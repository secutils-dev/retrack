use crate::{error::Error as RetrackError, server::ServerState};
use actix_web::{HttpResponse, post, web};
use retrack_types::trackers::{Tracker, TrackersBulkGetParams};
use tracing::error;

/// Retrieves multiple trackers by their IDs in a single request.
#[utoipa::path(
    tags = ["trackers"],
    request_body = TrackersBulkGetParams,
    responses(
        (status = OK, description = "A list of trackers matching the given IDs.", body = [Tracker]),
    )
)]
#[post("/api/trackers/_bulk_get")]
pub async fn trackers_bulk_get(
    state: web::Data<ServerState>,
    params: web::Json<TrackersBulkGetParams>,
) -> Result<HttpResponse, RetrackError> {
    let trackers = state.api.trackers();
    match trackers.bulk_get_trackers(&params.ids).await {
        Ok(trackers) => Ok(HttpResponse::Ok().json(trackers)),
        Err(err) => {
            error!("Failed to bulk-retrieve trackers: {err:?}");
            Err(err.into())
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        server::{
            handlers::trackers_bulk_get::trackers_bulk_get, server_state::tests::mock_server_state,
        },
        tests::TrackerCreateParamsBuilder,
    };
    use actix_web::{
        App,
        test::{TestRequest, call_service, init_service},
        web,
    };
    use retrack_types::trackers::Tracker;
    use sqlx::PgPool;

    #[sqlx::test]
    async fn returns_empty_list_for_empty_ids(pool: PgPool) -> anyhow::Result<()> {
        let server_state = web::Data::new(mock_server_state(pool).await?);
        let app = init_service(
            App::new()
                .app_data(server_state.clone())
                .service(trackers_bulk_get),
        )
        .await;

        let response = call_service(
            &app,
            TestRequest::post()
                .uri("/api/trackers/_bulk_get")
                .set_json(serde_json::json!({"ids": []}))
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), 200);
        let body: Vec<Tracker> = actix_web::test::read_body_json(response).await;
        assert!(body.is_empty());

        Ok(())
    }

    #[sqlx::test]
    async fn returns_only_requested_trackers(pool: PgPool) -> anyhow::Result<()> {
        let server_state = web::Data::new(mock_server_state(pool).await?);
        let trackers_api = server_state.api.trackers();

        let tracker_one = trackers_api
            .create_tracker(TrackerCreateParamsBuilder::new("tracker-one").build())
            .await?;
        let tracker_two = trackers_api
            .create_tracker(TrackerCreateParamsBuilder::new("tracker-two").build())
            .await?;
        let tracker_three = trackers_api
            .create_tracker(TrackerCreateParamsBuilder::new("tracker-three").build())
            .await?;

        let app = init_service(
            App::new()
                .app_data(server_state.clone())
                .service(trackers_bulk_get),
        )
        .await;

        // Fetch only tracker_one and tracker_three - tracker_two is excluded.
        let response = call_service(
            &app,
            TestRequest::post()
                .uri("/api/trackers/_bulk_get")
                .set_json(serde_json::json!({
                    "ids": [tracker_one.id, tracker_three.id]
                }))
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), 200);
        let mut body: Vec<Tracker> = actix_web::test::read_body_json(response).await;
        body.sort_by_key(|t| t.name.clone());
        assert_eq!(body.len(), 2);
        assert_eq!(body[0].id, tracker_one.id);
        assert_eq!(body[1].id, tracker_three.id);

        // Unknown IDs are silently ignored.
        let response = call_service(
            &app,
            TestRequest::post()
                .uri("/api/trackers/_bulk_get")
                .set_json(serde_json::json!({
                    "ids": [tracker_two.id, uuid::uuid!("00000000-0000-0000-0000-000000000099")]
                }))
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), 200);
        let body: Vec<Tracker> = actix_web::test::read_body_json(response).await;
        assert_eq!(body.len(), 1);
        assert_eq!(body[0].id, tracker_two.id);

        Ok(())
    }
}
