use serde_derive::Serialize;
use serde_with::{serde_as, skip_serializing_none, DurationMilliSeconds};
use std::time::Duration;
use utoipa::ToSchema;

/// Scheduler status.
#[serde_as]
#[skip_serializing_none]
#[derive(Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SchedulerStatus {
    /// Indicates if the scheduler is operational.
    pub operational: bool,
    /// Indicates when the next job will be run. If there are no scheduled jobs, this field is `None`.
    #[serde_as(as = "Option<DurationMilliSeconds<u64>>")]
    pub time_till_next_job: Option<Duration>,
}

#[cfg(test)]
mod tests {
    use crate::server::SchedulerStatus;
    use insta::assert_json_snapshot;
    use std::time::Duration;

    #[test]
    fn serialization() -> anyhow::Result<()> {
        assert_json_snapshot!(SchedulerStatus {
            operational: true,
            time_till_next_job: Some(Duration::from_secs(10)),
        }, @r###"
        {
          "operational": true,
          "timeTillNextJob": 10000
        }
        "###);

        assert_json_snapshot!(SchedulerStatus {
            operational: false,
            time_till_next_job: None,
        }, @r###"
        {
          "operational": false
        }
        "###);

        Ok(())
    }
}
