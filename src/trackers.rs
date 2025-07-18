mod api_ext;
mod database_ext;

mod parsers;
mod tracker_data_revisions_diff;
mod web_scraper;

pub use api_ext::TrackersApiExt;

#[cfg(test)]
pub mod tests {
    pub use crate::trackers::{
        tracker_data_revisions_diff::tracker_data_revisions_diff,
        web_scraper::{WebScraperBackend, WebScraperContentRequest, WebScraperErrorResponse},
    };
    use anyhow::bail;
    use retrack_types::{
        scheduler::SchedulerJobConfig,
        trackers::{
            ExtractorEngine, PageTarget, Tracker, TrackerAction, TrackerConfig,
            TrackerCreateParams, TrackerDataValue, TrackerTarget,
        },
    };
    use std::time::Duration;
    use time::OffsetDateTime;
    use uuid::Uuid;

    pub struct TrackerCreateParamsBuilder {
        params: TrackerCreateParams,
    }
    impl TrackerCreateParamsBuilder {
        pub fn new(name: impl Into<String>) -> Self {
            Self {
                params: TrackerCreateParams {
                    name: name.into(),
                    enabled: true,
                    target: TrackerTarget::Page(PageTarget {
                        extractor: "export async function execute(p) { await p.goto('https://retrack.dev/'); return await p.content(); }".to_string(),
                        params: None,
                        engine: None,
                        user_agent: Some("Retrack/1.0.0".to_string()),
                        accept_invalid_certificates: true,
                    }),
                    config: Default::default(),
                    tags: vec!["tag".to_string()],
                    actions: vec![TrackerAction::ServerLog(Default::default())],
                }
            }
        }

        pub fn with_config(mut self, config: TrackerConfig) -> Self {
            self.params.config = config;
            self
        }

        pub fn with_target(mut self, target: TrackerTarget) -> Self {
            self.params.target = target;
            self
        }

        pub fn with_tags(mut self, tags: Vec<String>) -> Self {
            self.params.tags = tags;
            self
        }

        pub fn with_actions(mut self, actions: Vec<TrackerAction>) -> Self {
            self.params.actions = actions;
            self
        }

        pub fn with_schedule<S: Into<String>>(mut self, schedule: S) -> Self {
            self.params.config.job = Some(SchedulerJobConfig {
                schedule: schedule.into(),
                retry_strategy: None,
            });
            self
        }

        pub fn disable(mut self) -> Self {
            self.params.enabled = false;
            self
        }

        pub fn build(self) -> TrackerCreateParams {
            self.params
        }
    }

    impl<'a> WebScraperContentRequest<'a> {
        /// Sets the content that has been extracted from the page previously.
        pub fn set_previous_content(self, previous_content: &'a TrackerDataValue) -> Self {
            Self {
                previous_content: Some(previous_content),
                ..self
            }
        }
    }

    impl<'t> TryFrom<&'t Tracker> for WebScraperContentRequest<'t> {
        type Error = anyhow::Error;

        fn try_from(tracker: &'t Tracker) -> Result<Self, Self::Error> {
            let TrackerTarget::Page(ref target) = tracker.target else {
                bail!(
                    "Tracker ('{}') target is not web page, instead got: {:?}",
                    tracker.id,
                    tracker.target
                );
            };

            Ok(Self {
                // Target properties.
                extractor: target.extractor.as_str(),
                extractor_params: target.params.as_ref(),
                extractor_backend: Some(match target.engine {
                    // Use `chromium` backend by default.
                    Some(ExtractorEngine::Chromium) | None => WebScraperBackend::Chromium,
                    Some(ExtractorEngine::Camoufox) => WebScraperBackend::Firefox,
                }),
                tags: &tracker.tags,
                user_agent: target.user_agent.as_deref(),
                accept_invalid_certificates: target.accept_invalid_certificates,
                // Config properties.
                timeout: tracker.config.timeout,
                // Non-tracker properties.
                previous_content: None,
            })
        }
    }

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
                        engine: None,
                        user_agent: Some("Retrack/1.0.0".to_string()),
                        accept_invalid_certificates: false,
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

        pub fn disabled(mut self) -> Self {
            self.tracker.enabled = false;
            self
        }

        pub fn build(self) -> Tracker {
            self.tracker
        }
    }
}
