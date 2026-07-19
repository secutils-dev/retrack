use crate::{error::Error as RetrackError, server::ServerState};
use actix_web::{HttpResponse, post, web};
use retrack_types::trackers::{TrackerDebugParams, TrackerDebugResult};
use tracing::error;

/// Runs the full tracker extraction + action dry-run pipeline without persisting anything.
#[utoipa::path(
    tags = ["trackers"],
    request_body = TrackerDebugParams,
    responses(
        (status = OK, description = "Debug result with diagnostic information.", body = TrackerDebugResult)
    )
)]
#[post("/api/trackers/_debug")]
pub async fn trackers_debug(
    state: web::Data<ServerState>,
    params: web::Json<TrackerDebugParams>,
) -> Result<HttpResponse, RetrackError> {
    let trackers = state.api.trackers();
    match trackers.debug_tracker(params.into_inner()).await {
        Ok(result) => Ok(HttpResponse::Ok().json(result)),
        Err(err) => {
            error!("Failed to debug tracker: {err:?}");
            Err(err.into())
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::server::{
        handlers::trackers_debug::trackers_debug, server_state::tests::mock_server_state,
    };
    use actix_web::{
        App,
        body::MessageBody,
        http::Method,
        test::{TestRequest, call_service, init_service},
        web,
    };
    use retrack_types::trackers::TrackerDebugResult;
    use serde_json::json;
    use sqlx::PgPool;
    use std::time::Duration;

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn returns_400_for_invalid_params(pool: PgPool) -> anyhow::Result<()> {
        let server_state = web::Data::new(mock_server_state(pool).await?);
        let app = init_service(
            App::new()
                .app_data(server_state.clone())
                .service(trackers_debug),
        )
        .await;

        let response = call_service(
            &app,
            TestRequest::with_uri("https://retrack.dev/api/trackers/_debug")
                .method(Method::POST)
                .set_json(json!({}))
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), 400);

        Ok(())
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn accepts_valid_page_target(pool: PgPool) -> anyhow::Result<()> {
        let server_state = web::Data::new(mock_server_state(pool).await?);
        let app = init_service(
            App::new()
                .app_data(server_state.clone())
                .service(trackers_debug),
        )
        .await;

        let response = call_service(
            &app,
            TestRequest::with_uri("https://retrack.dev/api/trackers/_debug")
                .method(Method::POST)
                .set_json(json!({
                    "target": {
                        "type": "page",
                        "extractor": "export async function execute(p) { return 'ok'; }"
                    }
                }))
                .to_request(),
        )
        .await;

        // The request will likely fail while connecting to web scraper, but it should still return
        // 200 since the debug endpoint captures errors inline.
        assert_eq!(response.status(), 200);
        let body_bytes = response.into_body().try_into_bytes().unwrap();
        let result: TrackerDebugResult = serde_json::from_slice(&body_bytes)?;
        assert!(result.duration_ms > Duration::ZERO || result.error.is_some());

        Ok(())
    }
}
