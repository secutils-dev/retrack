use cron::Schedule;
use serde_derive::{Deserialize, Serialize};
use serde_with::{serde_as, DisplayFromStr};
use std::str::FromStr;

/// Configuration for the Retrack scheduler jobs.
#[serde_as]
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct SchedulerJobsConfig {
    /// The schedule to use for the `TrackersSchedule` job.
    #[serde_as(as = "DisplayFromStr")]
    pub trackers_schedule: Schedule,
    /// The schedule to use for the `TrackersRun` job.
    #[serde_as(as = "DisplayFromStr")]
    pub trackers_run: Schedule,
    /// The schedule to use for the `TasksRun` job.
    #[serde_as(as = "DisplayFromStr")]
    pub tasks_run: Schedule,
}

impl Default for SchedulerJobsConfig {
    fn default() -> Self {
        Self {
            trackers_schedule: Schedule::from_str("0/10 * * * * * *")
                .expect("Cannot parse trackers schedule job schedule."),
            trackers_run: Schedule::from_str("0/10 * * * * * *")
                .expect("Cannot parse trackers run job schedule."),
            tasks_run: Schedule::from_str("0/30 * * * * * *")
                .expect("Cannot parse tasks run job schedule."),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::config::SchedulerJobsConfig;
    use insta::assert_toml_snapshot;

    #[test]
    fn serialization_and_default() {
        assert_toml_snapshot!(SchedulerJobsConfig::default(), @r###"
        trackers_schedule = '0/10 * * * * * *'
        trackers_run = '0/10 * * * * * *'
        tasks_run = '0/30 * * * * * *'
        "###);
    }

    #[test]
    fn deserialization() {
        let config: SchedulerJobsConfig = toml::from_str(
            r#"
        trackers_schedule = '0/10 * * * * * *'
        trackers_run = '0/10 * * * * * *'
        tasks_run = '0/30 * * * * * *'
    "#,
        )
        .unwrap();
        assert_eq!(config, SchedulerJobsConfig::default());
    }
}
