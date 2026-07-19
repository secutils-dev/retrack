use crate::{error::Error as RetrackError, server::ServerState};
use actix_web::{HttpResponse, delete, web};
use tracing::error;

/// Clears all execution logs for all trackers.
#[utoipa::path(
    tags = ["trackers"],
    responses(
        (status = NO_CONTENT, description = "All tracker execution logs were successfully cleared.")
    )
)]
#[delete("/api/trackers/execution-logs")]
pub async fn trackers_clear_all_execution_logs(
    state: web::Data<ServerState>,
) -> Result<HttpResponse, RetrackError> {
    let trackers = state.api.trackers();
    match trackers.clear_all_tracker_execution_logs().await {
        Ok(_) => Ok(HttpResponse::NoContent().finish()),
        Err(err) => {
            error!("Failed to clear all tracker execution logs: {err:?}");
            Err(err.into())
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        server::{
            handlers::trackers_clear_all_execution_logs::trackers_clear_all_execution_logs,
            server_state::tests::mock_server_state,
        },
        tests::TrackerCreateParamsBuilder,
    };
    use actix_web::{
        App,
        http::Method,
        test::{TestRequest, call_service, init_service},
        web,
    };
    use retrack_types::trackers::{TrackerExecutionLog, TrackerExecutionLogStatus};
    use sqlx::PgPool;
    use time::OffsetDateTime;
    use uuid::Uuid;

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn can_clear_all_execution_logs(pool: PgPool) -> anyhow::Result<()> {
        let server_state = web::Data::new(mock_server_state(pool).await?);

        let trackers_api = server_state.api.trackers();
        let tracker1 = trackers_api
            .create_tracker(TrackerCreateParamsBuilder::new("tracker-1").build())
            .await?;
        let tracker2 = trackers_api
            .create_tracker(TrackerCreateParamsBuilder::new("tracker-2").build())
            .await?;

        for (i, tracker_id) in [tracker1.id, tracker2.id].iter().enumerate() {
            trackers_api
                .log_tracker_execution(&TrackerExecutionLog {
                    id: Uuid::now_v7(),
                    tracker_id: *tracker_id,
                    job_id: None,
                    started_at: OffsetDateTime::from_unix_timestamp(946720800 + i as i64)?,
                    finished_at: OffsetDateTime::from_unix_timestamp(946720803 + i as i64)?,
                    status: TrackerExecutionLogStatus::Success,
                    error: None,
                    is_manual: true,
                    retry_attempt: None,
                    max_retry_attempts: None,
                    revision_size: None,
                    has_changes: None,
                    duration_ms: 3000,
                    phases: None,
                })
                .await;
        }

        let app = init_service(
            App::new()
                .app_data(server_state.clone())
                .service(trackers_clear_all_execution_logs),
        )
        .await;

        let response = call_service(
            &app,
            TestRequest::with_uri("https://retrack.dev/api/trackers/execution-logs")
                .method(Method::DELETE)
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), 204);

        let logs1 = trackers_api
            .get_tracker_execution_logs(tracker1.id, Default::default())
            .await?;
        let logs2 = trackers_api
            .get_tracker_execution_logs(tracker2.id, Default::default())
            .await?;
        assert!(logs1.is_empty());
        assert!(logs2.is_empty());

        Ok(())
    }
}
