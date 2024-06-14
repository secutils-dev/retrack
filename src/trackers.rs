mod api_ext;
mod database_ext;

mod tracker;
mod tracker_data_revision;
mod tracker_data_revisions_diff;
mod tracker_settings;
mod web_scraper;

pub use self::{
    api_ext::{TrackerCreateParams, TrackerListRevisionsParams, TrackerUpdateParams},
    tracker::Tracker,
    tracker_data_revision::TrackerDataRevision,
    tracker_settings::TrackerSettings,
};

#[cfg(test)]
pub mod tests {
    use crate::{
        scheduler::SchedulerJobConfig,
        trackers::{Tracker, TrackerSettings},
    };
    use std::time::Duration;
    use time::OffsetDateTime;
    use url::Url;
    use uuid::Uuid;

    pub use crate::trackers::web_scraper::{
        WebScraperContentRequest, WebScraperContentRequestScripts, WebScraperContentResponse,
        WebScraperErrorResponse,
    };

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
                    job_config: None,
                    url: Url::parse(url)?,
                    settings: TrackerSettings {
                        revisions,
                        delay: Duration::from_millis(2000),
                        extractor: Default::default(),
                        headers: Default::default(),
                    },
                    created_at: OffsetDateTime::from_unix_timestamp(946720800)?,
                },
            })
        }

        pub fn with_schedule<S: Into<String>>(mut self, schedule: S) -> Self {
            self.tracker.job_config = Some(SchedulerJobConfig {
                schedule: schedule.into(),
                retry_strategy: None,
                notifications: false,
            });
            self
        }

        pub fn with_job_config(mut self, job_config: SchedulerJobConfig) -> Self {
            self.tracker.job_config = Some(job_config);
            self
        }

        pub fn with_job_id(mut self, job_id: Uuid) -> Self {
            self.tracker.job_id = Some(job_id);
            self
        }

        pub fn with_delay_millis(mut self, millis: u64) -> Self {
            self.tracker.settings.delay = Duration::from_millis(millis);
            self
        }

        pub fn with_extractor(mut self, extractor: String) -> Self {
            self.tracker.settings.extractor = Some(extractor);
            self
        }

        pub fn build(self) -> Tracker {
            self.tracker
        }
    }
}
