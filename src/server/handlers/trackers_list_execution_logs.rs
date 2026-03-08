use crate::{error::Error as RetrackError, server::ServerState};
use actix_web::{HttpResponse, get, web};
use retrack_types::trackers::{TrackerExecutionLog, TrackerListExecutionLogsParams};
use tracing::error;
use uuid::Uuid;

/// Gets a list of execution logs for a tracker with the specified ID.
#[utoipa::path(
    tags = ["trackers"],
    params(
        ("tracker_id" = Uuid, Path, description = "A unique tracker ID."),
        TrackerListExecutionLogsParams
    ),
    responses(
        (status = OK, description = "A list of execution log entries for the tracker.", body = [TrackerExecutionLog]),
        (status = BAD_REQUEST, description = "Cannot list execution logs for a tracker with the specified parameters.")
    )
)]
#[get("/api/trackers/{tracker_id}/execution-logs")]
pub async fn trackers_list_execution_logs(
    state: web::Data<ServerState>,
    tracker_id: web::Path<Uuid>,
    params: web::Query<TrackerListExecutionLogsParams>,
) -> Result<HttpResponse, RetrackError> {
    let trackers = state.api.trackers();
    match trackers
        .get_tracker_execution_logs(*tracker_id, params.into_inner())
        .await
    {
        Ok(logs) => Ok(HttpResponse::Ok().json(logs)),
        Err(err) => {
            error!("Failed to retrieve tracker execution logs: {err:?}");
            Err(err.into())
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        server::{
            handlers::trackers_list_execution_logs::trackers_list_execution_logs,
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
    use retrack_types::trackers::{
        TrackerExecutionLog, TrackerExecutionLogPhase, TrackerExecutionLogStatus,
    };
    use serde_json::json;
    use sqlx::PgPool;
    use std::str::from_utf8;
    use time::OffsetDateTime;
    use uuid::uuid;

    #[sqlx::test]
    async fn can_list_execution_logs(pool: PgPool) -> anyhow::Result<()> {
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
                revision_size: Some(1234),
                has_changes: Some(true),
                duration_ms: 2500,
                phases: Some(vec![TrackerExecutionLogPhase {
                    phase: "fetch_data".to_string(),
                    duration_ms: 2500,
                    status: TrackerExecutionLogStatus::Success,
                    meta: Some(json!({"statusCode": 200})),
                }]),
            })
            .await;

        let app = init_service(
            App::new()
                .app_data(server_state.clone())
                .service(trackers_list_execution_logs),
        )
        .await;

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
        assert_eq!(response.status(), 200);

        let logs = serde_json::from_slice::<Vec<TrackerExecutionLog>>(
            &response.into_body().try_into_bytes().unwrap(),
        )?;
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].status, TrackerExecutionLogStatus::Success);
        assert_eq!(logs[0].revision_size, Some(1234));
        assert!(logs[0].phases.is_some());

        Ok(())
    }

    #[sqlx::test]
    async fn returns_empty_for_tracker_with_no_logs(pool: PgPool) -> anyhow::Result<()> {
        let server_state = web::Data::new(mock_server_state(pool).await?);

        let trackers_api = server_state.api.trackers();
        let tracker = trackers_api
            .create_tracker(TrackerCreateParamsBuilder::new("test-tracker").build())
            .await?;

        let app = init_service(
            App::new()
                .app_data(server_state.clone())
                .service(trackers_list_execution_logs),
        )
        .await;

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
        assert_eq!(response.status(), 200);

        let logs = serde_json::from_slice::<Vec<TrackerExecutionLog>>(
            &response.into_body().try_into_bytes().unwrap(),
        )?;
        assert!(logs.is_empty());

        Ok(())
    }

    #[sqlx::test]
    async fn fails_with_bad_request_for_unknown_tracker(pool: PgPool) -> anyhow::Result<()> {
        let server_state = web::Data::new(mock_server_state(pool).await?);
        let app = init_service(
            App::new()
                .app_data(server_state.clone())
                .service(trackers_list_execution_logs),
        )
        .await;

        let response = call_service(
            &app,
            TestRequest::with_uri(&format!(
                "https://retrack.dev/api/trackers/{}/execution-logs",
                uuid!("00000000-0000-0000-0000-000000000001")
            ))
            .method(Method::GET)
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
