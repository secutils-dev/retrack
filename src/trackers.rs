mod api_ext;
mod database_ext;

mod tracker;
mod tracker_config;
mod tracker_data_revision;
mod tracker_data_revisions_diff;
mod tracker_target;
mod web_scraper;

pub use self::{
    api_ext::{
        TrackerCreateParams, TrackerListRevisionsParams, TrackerUpdateParams, TrackersListParams,
    },
    tracker::Tracker,
    tracker_config::TrackerConfig,
    tracker_data_revision::TrackerDataRevision,
    tracker_target::{JsonApiTarget, TrackerTarget, WebPageTarget},
};

#[cfg(test)]
pub mod tests {
    pub use crate::trackers::web_scraper::{WebScraperContentRequest, WebScraperErrorResponse};
    use crate::{
        scheduler::SchedulerJobConfig,
        trackers::{Tracker, TrackerConfig, TrackerTarget, WebPageTarget},
    };
    use std::time::Duration;
    use time::OffsetDateTime;
    use uuid::Uuid;

    pub struct MockWebPageTrackerBuilder {
        tracker: Tracker,
    }

    impl MockWebPageTrackerBuilder {
        pub fn create<N: Into<String>>(
            id: Uuid,
            name: N,
            revisions: usize,
        ) -> anyhow::Result<Self> {
            Ok(Self {
                tracker: Tracker {
                    id,
                    name: name.into(),
                    job_id: None,
                    target: TrackerTarget::WebPage(WebPageTarget {
                        extractor: "export async function execute(p, r) { await p.goto('https://retrack.dev/'); return r.html(await p.content()); }".to_string(),
                        user_agent: Some("Retrack/1.0.0".to_string()),
                        ignore_https_errors: false,
                    }),
                    config: TrackerConfig {
                        revisions,
                        timeout: Some(Duration::from_millis(2000)),
                        headers: Default::default(),
                        job: None,
                    },
                    tags: vec![],
                    created_at: OffsetDateTime::from_unix_timestamp(946720800)?,
                    updated_at: OffsetDateTime::from_unix_timestamp(946720810)?,
                },
            })
        }

        pub fn with_schedule<S: Into<String>>(mut self, schedule: S) -> Self {
            self.tracker.config.job = Some(SchedulerJobConfig {
                schedule: schedule.into(),
                retry_strategy: None,
                notifications: None,
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

        pub fn build(self) -> Tracker {
            self.tracker
        }
    }
}
