use anyhow::bail;
use retrack_types::trackers::{
    TrackerExecutionLog, TrackerExecutionLogPhase, TrackerExecutionLogStatus,
};
use time::OffsetDateTime;
use uuid::Uuid;

fn status_to_db(status: TrackerExecutionLogStatus) -> i16 {
    match status {
        TrackerExecutionLogStatus::Success => 0,
        TrackerExecutionLogStatus::Failure => 1,
    }
}

fn status_from_db(value: i16) -> anyhow::Result<TrackerExecutionLogStatus> {
    match value {
        0 => Ok(TrackerExecutionLogStatus::Success),
        1 => Ok(TrackerExecutionLogStatus::Failure),
        other => bail!("Unknown execution log status: {other}"),
    }
}

#[derive(Debug, Eq, PartialEq, Clone)]
pub(super) struct RawTrackerExecutionLog {
    pub id: Uuid,
    pub tracker_id: Uuid,
    pub job_id: Option<Uuid>,
    pub started_at: OffsetDateTime,
    pub finished_at: OffsetDateTime,
    pub status: i16,
    pub error: Option<String>,
    pub is_manual: bool,
    pub retry_attempt: Option<i16>,
    pub max_retry_attempts: Option<i16>,
    pub revision_size: Option<i64>,
    pub has_changes: Option<bool>,
    pub duration_ms: i64,
    pub phases: Option<Vec<u8>>,
}

impl TryFrom<RawTrackerExecutionLog> for TrackerExecutionLog {
    type Error = anyhow::Error;

    fn try_from(raw: RawTrackerExecutionLog) -> Result<Self, Self::Error> {
        let phases = raw
            .phases
            .map(|bytes| serde_json::from_slice::<Vec<TrackerExecutionLogPhase>>(&bytes))
            .transpose()?;

        Ok(Self {
            id: raw.id,
            tracker_id: raw.tracker_id,
            job_id: raw.job_id,
            started_at: raw.started_at,
            finished_at: raw.finished_at,
            status: status_from_db(raw.status)?,
            error: raw.error,
            is_manual: raw.is_manual,
            retry_attempt: raw.retry_attempt.map(u16::try_from).transpose()?,
            max_retry_attempts: raw.max_retry_attempts.map(u16::try_from).transpose()?,
            revision_size: raw.revision_size,
            has_changes: raw.has_changes,
            duration_ms: raw.duration_ms as u64,
            phases,
        })
    }
}

impl TryFrom<&TrackerExecutionLog> for RawTrackerExecutionLog {
    type Error = anyhow::Error;

    fn try_from(item: &TrackerExecutionLog) -> Result<Self, Self::Error> {
        let phases = item.phases.as_ref().map(serde_json::to_vec).transpose()?;

        Ok(Self {
            id: item.id,
            tracker_id: item.tracker_id,
            job_id: item.job_id,
            started_at: item.started_at,
            finished_at: item.finished_at,
            status: status_to_db(item.status),
            error: item.error.clone(),
            is_manual: item.is_manual,
            retry_attempt: item.retry_attempt.map(|v| v as i16),
            max_retry_attempts: item.max_retry_attempts.map(|v| v as i16),
            revision_size: item.revision_size,
            has_changes: item.has_changes,
            duration_ms: item.duration_ms as i64,
            phases,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::RawTrackerExecutionLog;
    use retrack_types::trackers::{
        TrackerExecutionLog, TrackerExecutionLogPhase, TrackerExecutionLogStatus,
    };
    use serde_json::json;
    use time::OffsetDateTime;
    use uuid::uuid;

    #[test]
    fn can_convert_into_and_from_raw_execution_log() -> anyhow::Result<()> {
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
            duration_ms: 2340,
            phases: Some(vec![TrackerExecutionLogPhase {
                phase: "fetch_data".to_string(),
                duration_ms: 2340,
                status: TrackerExecutionLogStatus::Success,
                meta: Some(json!({"statusCode": 200})),
            }]),
        };
        assert_eq!(
            TrackerExecutionLog::try_from(RawTrackerExecutionLog::try_from(&log)?)?,
            log
        );

        let log_no_phases = TrackerExecutionLog {
            phases: None,
            revision_size: None,
            has_changes: None,
            error: Some("timeout".to_string()),
            status: TrackerExecutionLogStatus::Failure,
            job_id: None,
            retry_attempt: None,
            max_retry_attempts: None,
            is_manual: true,
            ..log
        };
        assert_eq!(
            TrackerExecutionLog::try_from(RawTrackerExecutionLog::try_from(&log_no_phases)?)?,
            log_no_phases
        );

        Ok(())
    }
}
