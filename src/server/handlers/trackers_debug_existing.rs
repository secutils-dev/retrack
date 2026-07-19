use crate::{error::Error as RetrackError, server::ServerState};
use actix_web::{HttpResponse, post, web};
use retrack_types::trackers::{TrackerDebugExistingParams, TrackerDebugResult};
use tracing::error;
use uuid::Uuid;

/// Runs the full tracker extraction + action dry-run pipeline against a stored tracker,
/// with optional overrides, without persisting anything.
#[utoipa::path(
    tags = ["trackers"],
    params(
        ("tracker_id" = Uuid, Path, description = "A unique tracker ID.")
    ),
    request_body = TrackerDebugExistingParams,
    responses(
        (status = OK, description = "Debug result with diagnostic information.", body = TrackerDebugResult)
    )
)]
#[post("/api/trackers/{tracker_id}/_debug")]
pub async fn trackers_debug_existing(
    state: web::Data<ServerState>,
    tracker_id: web::Path<Uuid>,
    params: web::Json<TrackerDebugExistingParams>,
) -> Result<HttpResponse, RetrackError> {
    let trackers = state.api.trackers();
    match trackers
        .debug_existing_tracker(*tracker_id, params.into_inner())
        .await
    {
        Ok(result) => Ok(HttpResponse::Ok().json(result)),
        Err(err) => {
            error!(tracker.id = %tracker_id, "Failed to debug existing tracker: {err:?}");
            Err(err.into())
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        server::{
            handlers::trackers_debug_existing::trackers_debug_existing,
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
    use insta::assert_debug_snapshot;
    use serde_json::json;
    use sqlx::PgPool;
    use std::str::from_utf8;
    use uuid::uuid;

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn returns_400_for_unknown_tracker(pool: PgPool) -> anyhow::Result<()> {
        let server_state = web::Data::new(mock_server_state(pool).await?);
        let app = init_service(
            App::new()
                .app_data(server_state.clone())
                .service(trackers_debug_existing),
        )
        .await;

        let response = call_service(
            &app,
            TestRequest::with_uri(&format!(
                "https://retrack.dev/api/trackers/{}/_debug",
                uuid!("00000000-0000-0000-0000-000000000001")
            ))
            .method(Method::POST)
            .set_json(json!({}))
            .to_request(),
        )
        .await;
        assert_eq!(response.status(), 400);
        assert_debug_snapshot!(
            from_utf8(&response.into_body().try_into_bytes().unwrap())?,
            @r###""Tracker ('00000000-0000-0000-0000-000000000001') is not found.""###
        );

        Ok(())
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn accepts_existing_tracker(pool: PgPool) -> anyhow::Result<()> {
        let server_state = web::Data::new(mock_server_state(pool).await?);

        let trackers_api = server_state.api.trackers();
        let tracker = trackers_api
            .create_tracker(TrackerCreateParamsBuilder::new("debug-test").build())
            .await?;

        let app = init_service(
            App::new()
                .app_data(server_state.clone())
                .service(trackers_debug_existing),
        )
        .await;

        let response = call_service(
            &app,
            TestRequest::with_uri(&format!(
                "https://retrack.dev/api/trackers/{}/_debug",
                tracker.id
            ))
            .method(Method::POST)
            .set_json(json!({}))
            .to_request(),
        )
        .await;

        // Expect 200 even if the scraper fails.
        assert_eq!(response.status(), 200);

        Ok(())
    }
}
