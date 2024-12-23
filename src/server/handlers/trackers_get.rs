use crate::{error::Error as RetrackError, server::ServerState};
use actix_web::{get, web, HttpResponse};
use retrack_types::trackers::Tracker;
use tracing::error;
use uuid::Uuid;

/// Gets a tracker with the specified ID.
#[utoipa::path(
    tags = ["trackers"],
    params(
        ("tracker_id" = Uuid, Path, description = "A unique tracker ID."),
    ),
    responses(
        (status = 200, description = "Tracker with the specified ID.", body = Tracker),
        (status = NOT_FOUND, description = "Tracker with the specified ID was not found or the ID is not a valid UUID.")
    )
)]
#[get("/api/trackers/{tracker_id}")]
pub async fn trackers_get(
    state: web::Data<ServerState>,
    tracker_id: web::Path<Uuid>,
) -> Result<HttpResponse, RetrackError> {
    match state.api.trackers().get_tracker(*tracker_id).await {
        Ok(Some(tracker)) => Ok(HttpResponse::Ok().json(tracker)),
        Ok(None) => Ok(HttpResponse::NotFound().finish()),
        Err(err) => {
            error!("Failed to retrieve tracker: {err:?}");
            Err(err.into())
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        server::{handlers::trackers_get::trackers_get, server_state::tests::mock_server_state},
        tests::TrackerCreateParamsBuilder,
    };
    use actix_web::{
        body::MessageBody,
        test::{call_service, init_service, TestRequest},
        web, App,
    };
    use sqlx::PgPool;
    use std::str::from_utf8;
    use uuid::uuid;

    #[sqlx::test]
    async fn can_get_tracker(pool: PgPool) -> anyhow::Result<()> {
        let server_state = web::Data::new(mock_server_state(pool).await?);

        // Create tracker.
        let tracker = server_state
            .api
            .trackers()
            .create_tracker(TrackerCreateParamsBuilder::new("name_one").build())
            .await?;

        let app = init_service(
            App::new()
                .app_data(server_state.clone())
                .service(trackers_get),
        )
        .await;

        let response = call_service(
            &app,
            TestRequest::with_uri(&format!("https://retrack.dev/api/trackers/{}", tracker.id))
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), 200);

        assert_eq!(
            serde_json::to_string(&tracker)?,
            from_utf8(&response.into_body().try_into_bytes().unwrap())?
        );

        Ok(())
    }

    #[sqlx::test]
    async fn returns_not_found_if_tracker_is_not_found(pool: PgPool) -> anyhow::Result<()> {
        let server_state = web::Data::new(mock_server_state(pool).await?);

        // Create tracker.
        server_state
            .api
            .trackers()
            .create_tracker(TrackerCreateParamsBuilder::new("name_one").build())
            .await?;

        let app = init_service(
            App::new()
                .app_data(server_state.clone())
                .service(trackers_get),
        )
        .await;

        let response = call_service(
            &app,
            TestRequest::with_uri(&format!(
                "https://retrack.dev/api/trackers/{}",
                uuid!("00000000-0000-0000-0000-000000000021")
            ))
            .to_request(),
        )
        .await;
        assert_eq!(response.status(), 404);

        assert_eq!(
            from_utf8(&response.into_body().try_into_bytes().unwrap())?,
            ""
        );

        Ok(())
    }
}
