use crate::{error::Error as RetrackError, server::ServerState};
use actix_web::{delete, web, HttpResponse};
use tracing::error;
use uuid::Uuid;

/// Removes a tracker with the specified ID.
#[utoipa::path(
    tags = ["trackers"],
    params(
        ("tracker_id" = Uuid, Path, description = "A unique tracker ID."),
    ),
    responses(
        (status = NO_CONTENT, description = "Tracker with the specified ID was successfully removed.")
    )
)]
#[delete("/api/trackers/{tracker_id}")]
pub async fn trackers_remove(
    state: web::Data<ServerState>,
    tracker_id: web::Path<Uuid>,
) -> Result<HttpResponse, RetrackError> {
    match state.api.trackers().remove_tracker(*tracker_id).await {
        Ok(_) => Ok(HttpResponse::NoContent().finish()),
        Err(err) => {
            error!("Failed to remove tracker: {err:?}");
            Err(err.into())
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        server::{
            handlers::trackers_remove::trackers_remove, server_state::tests::mock_server_state,
        },
        trackers::{TrackerConfig, TrackerCreateParams, TrackerTarget, WebPageTarget},
    };
    use actix_web::{
        http::Method,
        test::{call_service, init_service, TestRequest},
        web, App,
    };
    use sqlx::PgPool;
    use std::time::Duration;
    use url::Url;
    use uuid::uuid;

    #[sqlx::test]
    async fn can_remove_tracker(pool: PgPool) -> anyhow::Result<()> {
        let server_state = web::Data::new(mock_server_state(pool).await?);
        let app = init_service(
            App::new()
                .app_data(server_state.clone())
                .service(trackers_remove),
        )
        .await;

        let response = call_service(
            &app,
            TestRequest::with_uri(&format!(
                "https://retrack.dev/api/trackers/{}",
                uuid!("00000000-0000-0000-0000-000000000001")
            ))
            .method(Method::DELETE)
            .to_request(),
        )
        .await;
        assert_eq!(response.status(), 204);

        // Create tracker.
        let tracker = server_state
            .api
            .trackers()
            .create_tracker(TrackerCreateParams {
                name: "name_one".to_string(),
                url: Url::parse("http://localhost:1234/my/app?q=2")?,
                target: TrackerTarget::WebPage(WebPageTarget {
                    delay: Some(Duration::from_millis(2000)),
                    wait_for: Some("div".parse()?),
                }),
                config: TrackerConfig {
                    revisions: 3,
                    extractor: Default::default(),
                    headers: Default::default(),
                    job: None,
                },
                tags: vec!["tag".to_string()],
            })
            .await?;
        let trackers = server_state
            .api
            .trackers()
            .get_trackers(Default::default())
            .await?;
        assert_eq!(trackers.len(), 1);

        let response = call_service(
            &app,
            TestRequest::with_uri(&format!("https://retrack.dev/api/trackers/{}", tracker.id))
                .method(Method::DELETE)
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), 204);

        let trackers = server_state
            .api
            .trackers()
            .get_trackers(Default::default())
            .await?;
        assert!(trackers.is_empty());

        Ok(())
    }
}
