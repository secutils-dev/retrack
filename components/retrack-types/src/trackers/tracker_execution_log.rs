use crate::trackers::{TrackerExecutionLogPhase, TrackerExecutionLogStatus};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use utoipa::ToSchema;
use uuid::Uuid;

/// A log entry recording a single tracker execution (scheduled or manual).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TrackerExecutionLog {
    /// Unique log entry id (UUIDv7).
    pub id: Uuid,
    /// ID of the tracker that was executed.
    pub tracker_id: Uuid,
    /// ID of the scheduler job that triggered the execution (None for manual runs).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_id: Option<Uuid>,
    /// When the execution started.
    #[serde(with = "time::serde::timestamp")]
    pub started_at: OffsetDateTime,
    /// When the execution finished.
    #[serde(with = "time::serde::timestamp")]
    pub finished_at: OffsetDateTime,
    /// Whether the execution succeeded or failed.
    pub status: TrackerExecutionLogStatus,
    /// Error message if the execution failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Whether this was a manual (ad-hoc) run or a scheduled run.
    pub is_manual: bool,
    /// Retry attempt number (0 = initial attempt, None = not a retryable run).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_attempt: Option<u16>,
    /// Maximum retry attempts configured (None if no retry strategy).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_retry_attempts: Option<u16>,
    /// Size of the stored revision data in bytes (None if no revision was stored).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision_size: Option<i64>,
    /// Whether the fetched content changed compared to the previous revision.
    /// `None` when the execution failed before comparison could be performed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_changes: Option<bool>,
    /// Total execution duration in milliseconds (from monotonic clock).
    pub duration_ms: u64,
    /// Structured execution timeline capturing per-step timing and metadata.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phases: Option<Vec<TrackerExecutionLogPhase>>,
}

#[cfg(test)]
mod tests {
    use super::TrackerExecutionLog;
    use crate::trackers::{TrackerExecutionLogPhase, TrackerExecutionLogStatus};
    use insta::assert_json_snapshot;
    use serde_json::json;
    use time::OffsetDateTime;
    use uuid::uuid;

    #[test]
    fn serialization_success() -> anyhow::Result<()> {
        let log = TrackerExecutionLog {
            id: uuid!("00000000-0000-0000-0000-000000000001"),
            tracker_id: uuid!("00000000-0000-0000-0000-000000000002"),
            job_id: Some(uuid!("00000000-0000-0000-0000-000000000003")),
            started_at: OffsetDateTime::from_unix_timestamp(946720800)?,
            finished_at: OffsetDateTime::from_unix_timestamp(946720803)?,
            status: TrackerExecutionLogStatus::Success,
            error: None,
            is_manual: false,
            retry_attempt: Some(0),
            max_retry_attempts: Some(3),
            revision_size: Some(4521),
            has_changes: Some(true),
            duration_ms: 2342,
            phases: Some(vec![
                TrackerExecutionLogPhase {
                    phase: "fetch_data".to_string(),
                    duration_ms: 2340,
                    status: TrackerExecutionLogStatus::Success,
                    meta: Some(json!({"statusCode": 200, "bodySize": 4521})),
                },
                TrackerExecutionLogPhase {
                    phase: "compare".to_string(),
                    duration_ms: 2,
                    status: TrackerExecutionLogStatus::Success,
                    meta: Some(json!({"changed": true})),
                },
            ]),
        };
        assert_json_snapshot!(log, @r###"
        {
          "id": "00000000-0000-0000-0000-000000000001",
          "trackerId": "00000000-0000-0000-0000-000000000002",
          "jobId": "00000000-0000-0000-0000-000000000003",
          "startedAt": 946720800,
          "finishedAt": 946720803,
          "status": "success",
          "isManual": false,
          "retryAttempt": 0,
          "maxRetryAttempts": 3,
          "revisionSize": 4521,
          "hasChanges": true,
          "durationMs": 2342,
          "phases": [
            {
              "phase": "fetch_data",
              "durationMs": 2340,
              "status": "success",
              "meta": {
                "statusCode": 200,
                "bodySize": 4521
              }
            },
            {
              "phase": "compare",
              "durationMs": 2,
              "status": "success",
              "meta": {
                "changed": true
              }
            }
          ]
        }
        "###);

        Ok(())
    }

    #[test]
    fn serialization_failure_minimal() -> anyhow::Result<()> {
        let log = TrackerExecutionLog {
            id: uuid!("00000000-0000-0000-0000-000000000001"),
            tracker_id: uuid!("00000000-0000-0000-0000-000000000002"),
            job_id: None,
            started_at: OffsetDateTime::from_unix_timestamp(946720800)?,
            finished_at: OffsetDateTime::from_unix_timestamp(946720801)?,
            status: TrackerExecutionLogStatus::Failure,
            error: Some("Connection timeout".to_string()),
            is_manual: true,
            retry_attempt: None,
            max_retry_attempts: None,
            revision_size: None,
            has_changes: None,
            duration_ms: 1000,
            phases: None,
        };
        assert_json_snapshot!(log, @r###"
        {
          "id": "00000000-0000-0000-0000-000000000001",
          "trackerId": "00000000-0000-0000-0000-000000000002",
          "startedAt": 946720800,
          "finishedAt": 946720801,
          "status": "failure",
          "error": "Connection timeout",
          "isManual": true,
          "durationMs": 1000
        }
        "###);

        Ok(())
    }
}
