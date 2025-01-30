use crate::{error::Error as RetrackError, server::ServerState};
use actix_web::{get, web, HttpResponse};
use retrack_types::trackers::TrackerDataRevision;
use tracing::error;
use uuid::Uuid;

/// Retrieves tracker data revision with the specified ID for a tracker with the specified ID.
#[utoipa::path(
    tags = ["trackers"],
    params(
        ("tracker_id" = Uuid, Path, description = "A unique tracker ID."),
        ("revision_id" = Uuid, Path, description = "A unique tracker data revision ID."),
    ),
    responses(
        (status = OK, description = "Tracker data revision with the specified ID.", body = TrackerDataRevision),
        (status = NOT_FOUND, description = "Revision with the specified ID was not found."),
    )
)]
#[get("/api/trackers/{tracker_id}/revisions/{revision_id}")]
pub async fn trackers_get_revision(
    state: web::Data<ServerState>,
    params: web::Path<(Uuid, Uuid)>,
) -> Result<HttpResponse, RetrackError> {
    let (tracker_id, revision_id) = params.into_inner();
    let trackers = state.api.trackers();
    match trackers
        .get_tracker_data_revision(tracker_id, revision_id)
        .await
    {
        Ok(Some(revision)) => Ok(HttpResponse::Ok().json(revision)),
        Ok(None) => Ok(HttpResponse::NotFound().body(format!(
            "Tracker ('{tracker_id}') or data revision ('{revision_id}') is not found."
        ))),
        Err(err) => {
            error!("Failed to retrieve tracker data revision: {err:?}");
            Err(err.into())
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        server::{
            handlers::trackers_get_revision::trackers_get_revision,
            server_state::tests::mock_server_state,
        },
        tests::TrackerCreateParamsBuilder,
    };
    use actix_web::{
        body::MessageBody,
        test::{call_service, init_service, TestRequest},
        web, App,
    };
    use insta::assert_debug_snapshot;
    use retrack_types::trackers::{TrackerDataRevision, TrackerDataValue};
    use serde_json::json;
    use sqlx::PgPool;
    use std::str::from_utf8;
    use time::OffsetDateTime;
    use uuid::uuid;

    #[sqlx::test]
    async fn can_retrieve_tracker_data_revision(pool: PgPool) -> anyhow::Result<()> {
        let server_state = web::Data::new(mock_server_state(pool).await?);
        let app = init_service(
            App::new()
                .app_data(server_state.clone())
                .service(trackers_get_revision),
        )
        .await;

        // Create tracker.
        let trackers_api = server_state.api.trackers();
        let tracker = trackers_api
            .create_tracker(TrackerCreateParamsBuilder::new("name_one").build())
            .await?;

        // Unknown tracker.
        let response = call_service(
            &app,
            TestRequest::with_uri(&format!(
                "https://retrack.dev/api/trackers/{}/revisions/{}",
                uuid!("00000000-0000-0000-0000-000000000001"),
                uuid!("00000000-0000-0000-0000-000000000002")
            ))
            .to_request(),
        )
        .await;
        assert_eq!(response.status(), 404);
        assert_debug_snapshot!(from_utf8(&response.into_body().try_into_bytes().unwrap())?, @r###""Tracker ('00000000-0000-0000-0000-000000000001') or data revision ('00000000-0000-0000-0000-000000000002') is not found.""###);

        // Unknown tracker revision.
        let response = call_service(
            &app,
            TestRequest::with_uri(&format!(
                "https://retrack.dev/api/trackers/{}/revisions/{}",
                tracker.id,
                uuid!("00000000-0000-0000-0000-000000000002")
            ))
            .to_request(),
        )
        .await;
        assert_eq!(response.status(), 404);
        assert_eq!(
            from_utf8(&response.into_body().try_into_bytes().unwrap())?,
            format!("Tracker ('{}') or data revision ('00000000-0000-0000-0000-000000000002') is not found.", tracker.id)
        );

        // Add tracker data revisions.
        let trackers_db = server_state.api.db.trackers();
        let data_revision_one = TrackerDataRevision {
            id: uuid!("00000000-0000-0000-0000-000000000001"),
            tracker_id: tracker.id,
            created_at: OffsetDateTime::from_unix_timestamp(946720800)?,
            data: TrackerDataValue::new(json!("\"some-data\"")),
        };
        let data_revision_two = TrackerDataRevision {
            id: uuid!("00000000-0000-0000-0000-000000000002"),
            tracker_id: tracker.id,
            created_at: OffsetDateTime::from_unix_timestamp(946720900)?,
            data: TrackerDataValue::new(json!("\"some-data\"")),
        };
        trackers_db
            .insert_tracker_data_revision(&data_revision_one)
            .await?;
        trackers_db
            .insert_tracker_data_revision(&data_revision_two)
            .await?;

        // Get first revision.
        let response = call_service(
            &app,
            TestRequest::with_uri(&format!(
                "https://retrack.dev/api/trackers/{}/revisions/{}",
                tracker.id, data_revision_one.id
            ))
            .to_request(),
        )
        .await;
        assert_eq!(response.status(), 200);
        assert_eq!(
            serde_json::to_string(&data_revision_one)?,
            from_utf8(&response.into_body().try_into_bytes().unwrap())?
        );

        // Get second revision.
        let response = call_service(
            &app,
            TestRequest::with_uri(&format!(
                "https://retrack.dev/api/trackers/{}/revisions/{}",
                tracker.id, data_revision_two.id
            ))
            .to_request(),
        )
        .await;
        assert_eq!(response.status(), 200);
        assert_eq!(
            serde_json::to_string(&data_revision_two)?,
            from_utf8(&response.into_body().try_into_bytes().unwrap())?
        );

        Ok(())
    }
}
