mod tracker;
mod tracker_action;
mod tracker_config;
mod tracker_create_params;
mod tracker_data_revision;
mod tracker_data_value;
mod tracker_list_revisions_params;
mod tracker_target;
mod tracker_update_params;
mod trackers_list_params;

pub use self::{
    tracker::Tracker,
    tracker_action::{
        EmailAction, FormatterScriptArgs, FormatterScriptResult, ServerLogAction, TrackerAction,
        WebhookAction,
    },
    tracker_config::TrackerConfig,
    tracker_create_params::TrackerCreateParams,
    tracker_data_revision::TrackerDataRevision,
    tracker_data_value::TrackerDataValue,
    tracker_list_revisions_params::TrackerListRevisionsParams,
    tracker_target::{
        ApiTarget, ConfiguratorScriptArgs, ConfiguratorScriptRequest, ConfiguratorScriptResult,
        ExtractorScriptArgs, ExtractorScriptResult, PageTarget, TargetRequest, TrackerTarget,
    },
    tracker_update_params::TrackerUpdateParams,
    trackers_list_params::TrackersListParams,
};

#[cfg(test)]
pub mod tests {
    use crate::{
        scheduler::SchedulerJobConfig,
        trackers::{PageTarget, Tracker, TrackerAction, TrackerConfig, TrackerTarget},
    };
    use std::time::Duration;
    use time::OffsetDateTime;
    use uuid::Uuid;

    pub struct MockTrackerBuilder {
        tracker: Tracker,
    }

    impl MockTrackerBuilder {
        pub fn create<N: Into<String>>(
            id: Uuid,
            name: N,
            revisions: usize,
        ) -> anyhow::Result<Self> {
            Ok(Self {
                tracker: Tracker {
                    id,
                    name: name.into(),
                    enabled: true,
                    job_id: None,
                    target: TrackerTarget::Page(PageTarget {
                        extractor: "export async function execute(p) { await p.goto('https://retrack.dev/'); return await p.content(); }".to_string(),
                        params: None,
                        user_agent: Some("Retrack/1.0.0".to_string()),
                        ignore_https_errors: false,
                    }),
                    config: TrackerConfig {
                        revisions,
                        timeout: Some(Duration::from_millis(2000)),
                        job: None,
                    },
                    tags: vec![],
                    actions: vec![TrackerAction::ServerLog(Default::default())],
                    created_at: OffsetDateTime::from_unix_timestamp(946720800)?,
                    updated_at: OffsetDateTime::from_unix_timestamp(946720810)?,
                },
            })
        }

        pub fn with_schedule<S: Into<String>>(mut self, schedule: S) -> Self {
            self.tracker.config.job = Some(SchedulerJobConfig {
                schedule: schedule.into(),
                retry_strategy: None,
            });
            self
        }

        pub fn with_job_config(mut self, job_config: SchedulerJobConfig) -> Self {
            self.tracker.config.job = Some(job_config);
            self
        }

        pub fn with_job_id(mut self, job_id: Uuid) -> Self {
            self.tracker.job_id = Some(job_id);
            self
        }

        pub fn with_target(mut self, target: TrackerTarget) -> Self {
            self.tracker.target = target;
            self
        }

        pub fn with_timeout(mut self, timeout: Duration) -> Self {
            self.tracker.config.timeout = Some(timeout);
            self
        }

        pub fn with_tags(mut self, tags: Vec<String>) -> Self {
            self.tracker.tags = tags;
            self
        }

        pub fn with_actions(mut self, actions: Vec<TrackerAction>) -> Self {
            self.tracker.actions = actions;
            self
        }

        pub fn build(self) -> Tracker {
            self.tracker
        }
    }
}
