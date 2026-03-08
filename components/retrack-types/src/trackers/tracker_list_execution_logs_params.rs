use serde::Deserialize;
use std::num::NonZero;
use utoipa::IntoParams;

/// Default maximum number of execution log entries returned.
pub const DEFAULT_EXECUTION_LOGS_PAGE_SIZE: usize = 50;

/// Parameters for getting a list of execution logs for a tracker.
#[derive(Deserialize, Default, Debug, Copy, Clone, PartialEq, Eq, IntoParams)]
#[serde(rename_all = "camelCase")]
pub struct TrackerListExecutionLogsParams {
    /// The number of execution log entries to return. The value should be a positive, non-zero
    /// number. If not set, defaults to 50. Log entries are sorted by start time, with the most
    /// recent entry returned first.
    #[serde(default)]
    #[param(value_type = usize, minimum = 1)]
    pub size: Option<NonZero<usize>>,
}

#[cfg(test)]
mod tests {
    use super::TrackerListExecutionLogsParams;

    #[test]
    fn deserialization() -> anyhow::Result<()> {
        assert_eq!(
            serde_json::from_str::<TrackerListExecutionLogsParams>(r#"{}"#)?,
            Default::default()
        );

        assert_eq!(
            serde_json::from_str::<TrackerListExecutionLogsParams>(r#"{"size": 10}"#)?,
            TrackerListExecutionLogsParams {
                size: Some(10.try_into()?)
            }
        );

        Ok(())
    }
}
