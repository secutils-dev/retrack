use crate::{error::Error as RetrackError, server::ServerState};
use actix_web::{HttpResponse, post, web};
use retrack_types::trackers::{TrackerExecutionLog, TrackerListExecutionLogsBatchParams};
use tracing::error;

/// Gets execution logs for multiple trackers in a single request.
#[utoipa::path(
    tags = ["trackers"],
    request_body = TrackerListExecutionLogsBatchParams,
    responses(
        (status = OK, description = "A map of tracker ID to execution log entries.", body = std::collections::HashMap<String, Vec<TrackerExecutionLog>>),
        (status = BAD_REQUEST, description = "Cannot list execution logs with the specified parameters.")
    )
)]
#[post("/api/trackers/execution-logs")]
pub async fn trackers_list_execution_logs_batch(
    state: web::Data<ServerState>,
    params: web::Json<TrackerListExecutionLogsBatchParams>,
) -> Result<HttpResponse, RetrackError> {
    let trackers = state.api.trackers();
    match trackers
        .get_tracker_execution_logs_batch(params.into_inner())
        .await
    {
        Ok(logs) => Ok(HttpResponse::Ok().json(logs)),
        Err(err) => {
            error!("Failed to retrieve batch tracker execution logs: {err:?}");
            Err(err.into())
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        server::{
            handlers::trackers_list_execution_logs_batch::trackers_list_execution_logs_batch,
            server_state::tests::mock_server_state,
        },
        tests::TrackerCreateParamsBuilder,
    };
    use actix_web::{
        App,
        test::{TestRequest, call_service, init_service},
        web,
    };
    use retrack_types::trackers::{TrackerExecutionLog, TrackerExecutionLogStatus};
    use sqlx::PgPool;
    use std::collections::HashMap;
    use time::OffsetDateTime;
    use uuid::{Uuid, uuid};

    #[sqlx::test]
    async fn returns_empty_map_for_empty_ids(pool: PgPool) -> anyhow::Result<()> {
        let server_state = web::Data::new(mock_server_state(pool).await?);
        let app = init_service(
            App::new()
                .app_data(server_state)
                .service(trackers_list_execution_logs_batch),
        )
        .await;

        let response = call_service(
            &app,
            TestRequest::post()
                .uri("/api/trackers/execution-logs")
                .set_json(serde_json::json!({"trackerIds": []}))
                .to_request(),
        )
        .await;

        assert_eq!(response.status(), 200);
        let body: HashMap<Uuid, Vec<TrackerExecutionLog>> =
            actix_web::test::read_body_json(response).await;
        assert!(body.is_empty());

        Ok(())
    }

    #[sqlx::test]
    async fn returns_logs_grouped_by_tracker(pool: PgPool) -> anyhow::Result<()> {
        let server_state = web::Data::new(mock_server_state(pool).await?);
        let trackers_api = server_state.api.trackers();

        let tracker_a = trackers_api
            .create_tracker(TrackerCreateParamsBuilder::new("tracker-a").build())
            .await?;
        let tracker_b = trackers_api
            .create_tracker(TrackerCreateParamsBuilder::new("tracker-b").build())
            .await?;

        trackers_api
            .log_tracker_execution(&TrackerExecutionLog {
                id: uuid!("00000000-0000-0000-0000-000000000010"),
                tracker_id: tracker_a.id,
                job_id: None,
                started_at: OffsetDateTime::from_unix_timestamp(946720800)?,
                finished_at: OffsetDateTime::from_unix_timestamp(946720803)?,
                status: TrackerExecutionLogStatus::Success,
                error: None,
                is_manual: true,
                retry_attempt: None,
                max_retry_attempts: None,
                revision_size: Some(100),
                has_changes: Some(true),
                duration_ms: 3000,
                phases: None,
            })
            .await;

        trackers_api
            .log_tracker_execution(&TrackerExecutionLog {
                id: uuid!("00000000-0000-0000-0000-000000000020"),
                tracker_id: tracker_b.id,
                job_id: None,
                started_at: OffsetDateTime::from_unix_timestamp(946720810)?,
                finished_at: OffsetDateTime::from_unix_timestamp(946720812)?,
                status: TrackerExecutionLogStatus::Failure,
                error: Some("timeout".to_string()),
                is_manual: false,
                retry_attempt: None,
                max_retry_attempts: None,
                revision_size: None,
                has_changes: None,
                duration_ms: 2000,
                phases: None,
            })
            .await;

        let app = init_service(
            App::new()
                .app_data(server_state)
                .service(trackers_list_execution_logs_batch),
        )
        .await;

        let response = call_service(
            &app,
            TestRequest::post()
                .uri("/api/trackers/execution-logs")
                .set_json(serde_json::json!({
                    "trackerIds": [tracker_a.id, tracker_b.id]
                }))
                .to_request(),
        )
        .await;

        assert_eq!(response.status(), 200);
        let body: HashMap<Uuid, Vec<TrackerExecutionLog>> =
            actix_web::test::read_body_json(response).await;
        assert_eq!(body.len(), 2);
        assert_eq!(body[&tracker_a.id].len(), 1);
        assert_eq!(
            body[&tracker_a.id][0].status,
            TrackerExecutionLogStatus::Success
        );
        assert_eq!(body[&tracker_b.id].len(), 1);
        assert_eq!(
            body[&tracker_b.id][0].status,
            TrackerExecutionLogStatus::Failure
        );

        Ok(())
    }
}
