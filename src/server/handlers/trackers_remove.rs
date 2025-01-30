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
        (status = NO_CONTENT, description = "Tracker with the specified ID was successfully removed."),
        (status = NOT_FOUND, description = "Tracker with the specified ID was not found.")
    )
)]
#[delete("/api/trackers/{tracker_id}")]
pub async fn trackers_remove(
    state: web::Data<ServerState>,
    tracker_id: web::Path<Uuid>,
) -> Result<HttpResponse, RetrackError> {
    match state.api.trackers().remove_tracker(*tracker_id).await {
        Ok(true) => Ok(HttpResponse::NoContent().finish()),
        Ok(false) => {
            Ok(HttpResponse::NotFound().body(format!("Tracker ('{tracker_id}') is not found.")))
        }
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
        tests::TrackerCreateParamsBuilder,
    };
    use actix_web::{
        body::MessageBody,
        http::Method,
        test::{call_service, init_service, TestRequest},
        web, App,
    };
    use sqlx::PgPool;
    use std::str::from_utf8;
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

        // Create tracker.
        let tracker = server_state
            .api
            .trackers()
            .create_tracker(TrackerCreateParamsBuilder::new("name_one").build())
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

    #[sqlx::test]
    async fn returns_not_found_if_tracker_is_not_found(pool: PgPool) -> anyhow::Result<()> {
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
                uuid!("00000000-0000-0000-0000-000000000021")
            ))
            .method(Method::DELETE)
            .to_request(),
        )
        .await;
        assert_eq!(
            from_utf8(&response.into_body().try_into_bytes().unwrap())?,
            "Tracker ('00000000-0000-0000-0000-000000000021') is not found."
        );

        Ok(())
    }
}
