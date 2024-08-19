mod api_ext;
mod database_ext;

mod tracker;
mod tracker_config;
mod tracker_data_revision;
mod tracker_data_revisions_diff;
mod tracker_target;
mod web_scraper;

pub use self::{
    api_ext::{TrackerCreateParams, TrackerListRevisionsParams, TrackerUpdateParams},
    tracker::Tracker,
    tracker_config::TrackerConfig,
    tracker_data_revision::TrackerDataRevision,
    tracker_target::{
        JsonApiTarget, TrackerTarget, WebPageTarget, WebPageWaitFor, WebPageWaitForState,
    },
};

#[cfg(test)]
pub mod tests {
    pub use crate::trackers::web_scraper::{
        WebScraperContentRequest, WebScraperContentResponse, WebScraperErrorResponse,
    };
    use crate::{
        scheduler::SchedulerJobConfig,
        trackers::{Tracker, TrackerConfig, TrackerTarget, WebPageTarget},
    };
    use std::time::Duration;
    use time::OffsetDateTime;
    use url::Url;
    use uuid::Uuid;

    pub struct MockWebPageTrackerBuilder {
        tracker: Tracker,
    }

    impl MockWebPageTrackerBuilder {
        pub fn create<N: Into<String>>(
            id: Uuid,
            name: N,
            url: &str,
            revisions: usize,
        ) -> anyhow::Result<Self> {
            Ok(Self {
                tracker: Tracker {
                    id,
                    name: name.into(),
                    job_id: None,
                    url: Url::parse(url)?,
                    target: TrackerTarget::WebPage(WebPageTarget {
                        delay: Some(Duration::from_millis(2000)),
                        wait_for: Some("div".parse()?),
                    }),
                    config: TrackerConfig {
                        revisions,
                        extractor: Default::default(),
                        headers: Default::default(),
                        job: None,
                    },
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

        pub fn with_extractor(mut self, extractor: String) -> Self {
            self.tracker.config.extractor = Some(extractor);
            self
        }

        pub fn build(self) -> Tracker {
            self.tracker
        }
    }
}
