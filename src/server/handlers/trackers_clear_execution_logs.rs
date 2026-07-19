use crate::{error::Error as RetrackError, server::ServerState};
use actix_web::{HttpResponse, delete, web};
use tracing::error;
use uuid::Uuid;

/// Clears all execution logs for a tracker with the specified ID.
#[utoipa::path(
    tags = ["trackers"],
    params(
        ("tracker_id" = Uuid, Path, description = "A unique tracker ID."),
    ),
    responses(
        (status = NO_CONTENT, description = "Execution logs for a tracker with the specified ID were successfully cleared.")
    )
)]
#[delete("/api/trackers/{tracker_id}/execution-logs")]
pub async fn trackers_clear_execution_logs(
    state: web::Data<ServerState>,
    tracker_id: web::Path<Uuid>,
) -> Result<HttpResponse, RetrackError> {
    let trackers = state.api.trackers();
    match trackers.clear_tracker_execution_logs(*tracker_id).await {
        Ok(_) => Ok(HttpResponse::NoContent().finish()),
        Err(err) => {
            error!("Failed to clear tracker execution logs: {err:?}");
            Err(err.into())
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        server::{
            handlers::{
                trackers_clear_execution_logs::trackers_clear_execution_logs,
                trackers_list_execution_logs::trackers_list_execution_logs,
            },
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
    use retrack_types::trackers::{TrackerExecutionLog, TrackerExecutionLogStatus};
    use sqlx::PgPool;
    use std::str::from_utf8;
    use time::OffsetDateTime;
    use uuid::uuid;

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn can_clear_execution_logs(pool: PgPool) -> anyhow::Result<()> {
        let server_state = web::Data::new(mock_server_state(pool).await?);

        let trackers_api = server_state.api.trackers();
        let tracker = trackers_api
            .create_tracker(TrackerCreateParamsBuilder::new("test-tracker").build())
            .await?;

        trackers_api
            .log_tracker_execution(&TrackerExecutionLog {
                id: uuid!("00000000-0000-0000-0000-000000000010"),
                tracker_id: tracker.id,
                job_id: None,
                started_at: OffsetDateTime::from_unix_timestamp(946720800)?,
                finished_at: OffsetDateTime::from_unix_timestamp(946720803)?,
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

        let app = init_service(
            App::new()
                .app_data(server_state.clone())
                .service(trackers_clear_execution_logs)
                .service(trackers_list_execution_logs),
        )
        .await;

        let response = call_service(
            &app,
            TestRequest::with_uri(&format!(
                "https://retrack.dev/api/trackers/{}/execution-logs",
                tracker.id
            ))
            .method(Method::DELETE)
            .to_request(),
        )
        .await;
        assert_eq!(response.status(), 204);

        let response = call_service(
            &app,
            TestRequest::with_uri(&format!(
                "https://retrack.dev/api/trackers/{}/execution-logs",
                tracker.id
            ))
            .method(Method::GET)
            .to_request(),
        )
        .await;
        let logs = serde_json::from_slice::<Vec<TrackerExecutionLog>>(
            &response.into_body().try_into_bytes().unwrap(),
        )?;
        assert!(logs.is_empty());

        Ok(())
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn fails_with_bad_request_for_unknown_tracker(pool: PgPool) -> anyhow::Result<()> {
        let server_state = web::Data::new(mock_server_state(pool).await?);
        let app = init_service(
            App::new()
                .app_data(server_state.clone())
                .service(trackers_clear_execution_logs),
        )
        .await;

        let response = call_service(
            &app,
            TestRequest::with_uri(&format!(
                "https://retrack.dev/api/trackers/{}/execution-logs",
                uuid!("00000000-0000-0000-0000-000000000001")
            ))
            .method(Method::DELETE)
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
}
