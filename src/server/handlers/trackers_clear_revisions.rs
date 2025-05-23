use crate::{error::Error as RetrackError, server::ServerState};
use actix_web::{HttpResponse, delete, web};
use tracing::error;
use uuid::Uuid;

/// Clears all data revisions for a tracker with the specified ID.
#[utoipa::path(
    tags = ["trackers"],
    params(
        ("tracker_id" = Uuid, Path, description = "A unique tracker ID."),
    ),
    responses(
        (status = NO_CONTENT, description = "Data revisions for a tracker with the specified ID were successfully cleared.")
    )
)]
#[delete("/api/trackers/{tracker_id}/revisions")]
pub async fn trackers_clear_revisions(
    state: web::Data<ServerState>,
    tracker_id: web::Path<Uuid>,
) -> Result<HttpResponse, RetrackError> {
    let trackers = state.api.trackers();
    match trackers.clear_tracker_data(*tracker_id).await {
        Ok(_) => Ok(HttpResponse::NoContent().finish()),
        Err(err) => {
            error!("Failed to clear tracker data revisions: {err:?}");
            Err(err.into())
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        server::{
            handlers::trackers_clear_revisions::trackers_clear_revisions,
            server_state::tests::mock_server_state,
        },
        tests::TrackerCreateParamsBuilder,
    };
    use actix_web::{
        App,
        body::MessageBody,
        http::Method,
        test::{TestRequest, call_service, init_service},
        web,
    };
    use retrack_types::trackers::{TrackerDataRevision, TrackerDataValue};
    use serde_json::json;
    use sqlx::PgPool;
    use std::str::from_utf8;
    use time::OffsetDateTime;
    use uuid::uuid;

    #[sqlx::test]
    async fn can_list_tracker_data(pool: PgPool) -> anyhow::Result<()> {
        let server_state = web::Data::new(mock_server_state(pool).await?);
        let app = init_service(
            App::new()
                .app_data(server_state.clone())
                .service(trackers_clear_revisions),
        )
        .await;

        // Unknown tracker.
        let response = call_service(
            &app,
            TestRequest::with_uri(&format!(
                "https://retrack.dev/api/trackers/{}/revisions",
                uuid!("00000000-0000-0000-0000-000000000001")
            ))
            .method(Method::DELETE)
            .to_request(),
        )
        .await;
        assert_eq!(response.status(), 400);
        assert_eq!(
            from_utf8(&response.into_body().try_into_bytes().unwrap())?,
            "Tracker ('00000000-0000-0000-0000-000000000001') is not found."
        );

        // Create tracker.
        let trackers_api = server_state.api.trackers();
        let tracker = trackers_api
            .create_tracker(TrackerCreateParamsBuilder::new("name_one").build())
            .await?;

        // Tracker without revisions.
        let response = call_service(
            &app,
            TestRequest::with_uri(&format!(
                "https://retrack.dev/api/trackers/{}/revisions",
                tracker.id
            ))
            .method(Method::DELETE)
            .to_request(),
        )
        .await;
        assert_eq!(response.status(), 204);

        // Add tracker data revision and check that it has been saved..
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

        // Finally clean all revisions.
        let response = call_service(
            &app,
            TestRequest::with_uri(&format!(
                "https://retrack.dev/api/trackers/{}/revisions",
                tracker.id
            ))
            .method(Method::DELETE)
            .to_request(),
        )
        .await;
        assert_eq!(response.status(), 204);
        assert!(
            trackers_api
                .get_tracker_data_revisions(tracker.id, Default::default())
                .await?
                .is_empty()
        );

        Ok(())
    }
}
