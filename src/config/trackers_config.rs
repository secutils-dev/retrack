use byte_unit::Byte;
use retrack_types::trackers::TrackerAction;
use serde::{Deserialize, Serialize};
use serde_with::{DurationMilliSeconds, serde_as};
use std::{collections::HashSet, time::Duration};

#[serde_as]
#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
pub struct TrackersConfig {
    /// The max number of tracker revisions per tracker.
    pub max_revisions: usize,
    /// The maximum timeout allowed for a tracker to retrieve new revision (default is 5 minutes).
    #[serde_as(as = "DurationMilliSeconds<u64>")]
    pub max_timeout: Duration,
    /// The list of allowed schedules for the trackers.
    pub schedules: Option<HashSet<String>>,
    /// The minimum interval between two consequent scheduled tracker checks.
    #[serde_as(as = "DurationMilliSeconds<u64>")]
    pub min_schedule_interval: Duration,
    /// The minimum interval between two consequent tracker check retries.
    #[serde_as(as = "DurationMilliSeconds<u64>")]
    pub min_retry_interval: Duration,
    /// Indicates whether to restrict the tracker to publicly reachable HTTP and HTTPS URLs only.
    pub restrict_to_public_urls: bool,
    /// The maximum size of any give tracker script (configurators, extractors, formatters, etc.).
    pub max_script_size: Byte,
    /// The default actions to be applied to all trackers. Always applied after the tracker-specific
    /// actions.
    pub default_actions: Option<Vec<TrackerAction>>,
}

impl Default for TrackersConfig {
    fn default() -> Self {
        Self {
            max_revisions: 30,
            // Default to None to allow all schedules.
            schedules: None,
            // Default to 10 seconds.
            max_timeout: Duration::from_secs(300),
            // Default to 10 seconds.
            min_schedule_interval: Duration::from_secs(10),
            // Default to 60 seconds.
            min_retry_interval: Duration::from_secs(60),
            restrict_to_public_urls: true,
            // Default is 4KiB.
            max_script_size: Byte::from_u64(4096),
            // Don't add any default actions by default.
            default_actions: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::config::TrackersConfig;
    use byte_unit::Byte;
    use insta::assert_toml_snapshot;
    use retrack_types::trackers::{EmailAction, TrackerAction};
    use std::time::Duration;

    #[test]
    fn serialization_and_default() {
        let config = TrackersConfig::default();
        assert_toml_snapshot!(config, @r###"
        max_revisions = 30
        max_timeout = 300000
        min_schedule_interval = 10000
        min_retry_interval = 60000
        restrict_to_public_urls = true
        max_script_size = '4 KiB'
        "###);

        let config = TrackersConfig {
            max_revisions: 10,
            schedules: Some(["@hourly".to_string()].into_iter().collect()),
            max_timeout: Duration::from_secs(100),
            min_schedule_interval: Duration::from_secs(2),
            min_retry_interval: Duration::from_secs(3),
            restrict_to_public_urls: false,
            max_script_size: Byte::from_u64(8192),
            default_actions: Some(vec![
                TrackerAction::ServerLog(Default::default()),
                TrackerAction::Email(EmailAction {
                    to: vec!["dev@retrack.dev".to_string()],
                    formatter: Some(
                        "(async () => Deno.core.encode(JSON.stringify({ key: 'value' })))();"
                            .to_string(),
                    ),
                }),
            ]),
        };
        assert_toml_snapshot!(config, @r###"
        max_revisions = 10
        max_timeout = 100000
        schedules = ['@hourly']
        min_schedule_interval = 2000
        min_retry_interval = 3000
        restrict_to_public_urls = false
        max_script_size = '8 KiB'

        [[default_actions]]
        type = 'log'

        [[default_actions]]
        type = 'email'
        to = ['dev@retrack.dev']
        formatter = '''(async () => Deno.core.encode(JSON.stringify({ key: 'value' })))();'''
        "###);
    }

    #[test]
    fn deserialization() {
        let config: TrackersConfig = toml::from_str(
            r#"
        max_revisions = 30
        max_timeout = 300_000
        min_schedule_interval = 10_000
        min_retry_interval = 60_000
        restrict_to_public_urls = true
        max_script_size = '4 KiB'
    "#,
        )
        .unwrap();
        assert_eq!(config, TrackersConfig::default());

        let config: TrackersConfig = toml::from_str(
            r#"
        max_revisions = 10
        max_timeout = 600_000
        min_schedule_interval = 2_000
        min_retry_interval = 3_000
        schedules = ['@', '@hourly']
        restrict_to_public_urls = false
        max_script_size = '8 KiB'
        default_actions = [{ type = 'log' }, { type = 'email', to = ['dev@retrack.dev'], formatter = '''(async () => Deno.core.encode(JSON.stringify({ key: 'value' })))();''' }]
    "#,
        )
        .unwrap();
        assert_eq!(
            config,
            TrackersConfig {
                max_revisions: 10,
                schedules: Some(
                    ["@".to_string(), "@hourly".to_string()]
                        .into_iter()
                        .collect(),
                ),
                max_timeout: Duration::from_secs(600),
                min_schedule_interval: Duration::from_secs(2),
                min_retry_interval: Duration::from_secs(3),
                restrict_to_public_urls: false,
                max_script_size: Byte::from_u64(8192),
                default_actions: Some(vec![
                    TrackerAction::ServerLog(Default::default()),
                    TrackerAction::Email(EmailAction {
                        to: vec!["dev@retrack.dev".to_string()],
                        formatter: Some(
                            "(async () => Deno.core.encode(JSON.stringify({ key: 'value' })))();"
                                .to_string(),
                        ),
                    }),
                ]),
            }
        );
    }
}
