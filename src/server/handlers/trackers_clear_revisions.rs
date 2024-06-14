use crate::{error::Error as RetrackError, server::ServerState};
use actix_web::{delete, web, HttpResponse};
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
        trackers::{TrackerCreateParams, TrackerDataRevision, TrackerSettings},
    };
    use actix_web::{
        http::Method,
        test::{call_service, init_service, TestRequest},
        web, App,
    };
    use sqlx::PgPool;
    use std::time::Duration;
    use time::OffsetDateTime;
    use url::Url;
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
        assert_eq!(response.status(), 204);

        // Create tracker.
        let trackers_api = server_state.api.trackers();
        let tracker = trackers_api
            .create_tracker(TrackerCreateParams {
                name: "name_one".to_string(),
                url: Url::parse("http://localhost:1234/my/app?q=2")?,
                settings: TrackerSettings {
                    revisions: 3,
                    delay: Duration::from_millis(2000),
                    extractor: Default::default(),
                    headers: Default::default(),
                },
                job_config: None,
            })
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
            data: "\"some-data\"".to_string(),
        };
        let data_revision_two = TrackerDataRevision {
            id: uuid!("00000000-0000-0000-0000-000000000002"),
            tracker_id: tracker.id,
            created_at: OffsetDateTime::from_unix_timestamp(946720900)?,
            data: "\"some-data\"".to_string(),
        };
        trackers_db
            .insert_tracker_data_revision(&data_revision_one)
            .await?;
        trackers_db
            .insert_tracker_data_revision(&data_revision_two)
            .await?;
        assert_eq!(
            trackers_api
                .get_tracker_data(tracker.id, Default::default())
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
        assert!(trackers_api
            .get_tracker_data(tracker.id, Default::default())
            .await?
            .is_empty());

        Ok(())
    }
}
