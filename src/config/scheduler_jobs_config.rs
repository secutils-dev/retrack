use serde::{Deserialize, Serialize};

/// Configuration for the Retrack scheduler jobs.
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct SchedulerJobsConfig {
    /// The schedule to use for the `TrackersSchedule` job.
    pub trackers_schedule: String,
    /// The schedule to use for the `TrackersRun` job.
    pub trackers_run: String,
    /// The schedule to use for the `TasksRun` job.
    pub tasks_run: String,
}

impl Default for SchedulerJobsConfig {
    fn default() -> Self {
        Self {
            trackers_schedule: "0/10 * * * * *".to_string(),
            trackers_run: "0/10 * * * * *".to_string(),
            tasks_run: "0/30 * * * * *".to_string(),
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
        trackers_schedule = '0/10 * * * * *'
        trackers_run = '0/10 * * * * *'
        tasks_run = '0/30 * * * * *'
        "###);
    }

    #[test]
    fn deserialization() {
        let config: SchedulerJobsConfig = toml::from_str(
            r#"
        trackers_schedule = '0/10 * * * * *'
        trackers_run = '0/10 * * * * *'
        tasks_run = '0/30 * * * * *'
    "#,
        )
        .unwrap();
        assert_eq!(config, SchedulerJobsConfig::default());
    }
}
