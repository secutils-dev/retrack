use crate::{error::Error as RetrackError, server::ServerState};
use actix_web::{delete, web, HttpResponse};
use tracing::error;
use uuid::Uuid;

/// Removes tracker data revision with the specified ID for a tracker with the specified ID.
#[utoipa::path(
    tags = ["trackers"],
    params(
        ("tracker_id" = Uuid, Path, description = "A unique tracker ID."),
        ("revision_id" = Uuid, Path, description = "A unique tracker data revision ID."),
    ),
    responses(
        (status = NO_CONTENT, description = "Specified data revision for a tracker with the specified ID was successfully removed."),
        (status = NOT_FOUND, description = "Revision with the specified ID was not found.")
    )
)]
#[delete("/api/trackers/{tracker_id}/revisions/{revision_id}")]
pub async fn trackers_remove_revision(
    state: web::Data<ServerState>,
    params: web::Path<(Uuid, Uuid)>,
) -> Result<HttpResponse, RetrackError> {
    let (tracker_id, revision_id) = params.into_inner();
    let trackers = state.api.trackers();
    match trackers
        .remove_tracker_data_revision(tracker_id, revision_id)
        .await
    {
        Ok(true) => Ok(HttpResponse::NoContent().finish()),
        Ok(false) => Ok(HttpResponse::NotFound().body(format!(
            "Tracker ('{tracker_id}') or data revision ('{revision_id}') is not found."
        ))),
        Err(err) => {
            error!("Failed to remove tracker data revision: {err:?}");
            Err(err.into())
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        server::{
            handlers::trackers_remove_revision::trackers_remove_revision,
            server_state::tests::mock_server_state,
        },
        tests::TrackerCreateParamsBuilder,
    };
    use actix_web::{
        body::MessageBody,
        http::Method,
        test::{call_service, init_service, TestRequest},
        web, App,
    };
    use retrack_types::trackers::{TrackerDataRevision, TrackerDataValue};
    use serde_json::json;
    use sqlx::PgPool;
    use std::str::from_utf8;
    use time::OffsetDateTime;
    use uuid::uuid;

    #[sqlx::test]
    async fn can_remove_tracker_data_revision(pool: PgPool) -> anyhow::Result<()> {
        let server_state = web::Data::new(mock_server_state(pool).await?);
        let app = init_service(
            App::new()
                .app_data(server_state.clone())
                .service(trackers_remove_revision),
        )
        .await;

        // Unknown tracker.
        let response = call_service(
            &app,
            TestRequest::with_uri(&format!(
                "https://retrack.dev/api/trackers/{}/revisions/{}",
                uuid!("00000000-0000-0000-0000-000000000001"),
                uuid!("00000000-0000-0000-0000-000000000002")
            ))
            .method(Method::DELETE)
            .to_request(),
        )
        .await;
        assert_eq!(response.status(), 404);
        assert_eq!(
            from_utf8(&response.into_body().try_into_bytes().unwrap())?,
            "Tracker ('00000000-0000-0000-0000-000000000001') or data revision ('00000000-0000-0000-0000-000000000002') is not found."
        );

        // Create tracker.
        let trackers_api = server_state.api.trackers();
        let tracker = trackers_api
            .create_tracker(TrackerCreateParamsBuilder::new("name_one").build())
            .await?;

        // Unknown tracker revision.
        let response = call_service(
            &app,
            TestRequest::with_uri(&format!(
                "https://retrack.dev/api/trackers/{}/revisions/{}",
                tracker.id,
                uuid!("00000000-0000-0000-0000-000000000002")
            ))
            .method(Method::DELETE)
            .to_request(),
        )
        .await;
        assert_eq!(response.status(), 404);
        assert_eq!(
            from_utf8(&response.into_body().try_into_bytes().unwrap())?,
            format!("Tracker ('{}') or data revision ('00000000-0000-0000-0000-000000000002') is not found.", tracker.id)
        );

        // Add tracker data revisions and check that it has been saved.
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
        assert_eq!(
            trackers_api
                .get_tracker_data_revisions(tracker.id, Default::default())
                .await?
                .len(),
            2
        );

        // Remove older revision.
        let response = call_service(
            &app,
            TestRequest::with_uri(&format!(
                "https://retrack.dev/api/trackers/{}/revisions/{}",
                tracker.id, data_revision_one.id
            ))
            .method(Method::DELETE)
            .to_request(),
        )
        .await;
        assert_eq!(response.status(), 204);
        assert_eq!(
            trackers_api
                .get_tracker_data_revisions(tracker.id, Default::default())
                .await?,
            vec![data_revision_two.clone()]
        );

        // Remove newer revision.
        let response = call_service(
            &app,
            TestRequest::with_uri(&format!(
                "https://retrack.dev/api/trackers/{}/revisions/{}",
                tracker.id, data_revision_two.id
            ))
            .method(Method::DELETE)
            .to_request(),
        )
        .await;
        assert_eq!(response.status(), 204);
        assert!(trackers_api
            .get_tracker_data_revisions(tracker.id, Default::default())
            .await?
            .is_empty());

        Ok(())
    }
}
