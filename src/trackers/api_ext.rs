mod tracker_create_params;
mod tracker_list_revisions_params;
mod tracker_update_params;

pub use self::{
    tracker_create_params::TrackerCreateParams,
    tracker_list_revisions_params::TrackerListRevisionsParams,
    tracker_update_params::TrackerUpdateParams,
};
use crate::{
    api::Api,
    error::Error as RetrackError,
    network::{DnsResolver, EmailTransport},
    scheduler::{ScheduleExt, SchedulerJobRetryStrategy},
    trackers::{
        database_ext::TrackersDatabaseExt,
        tracker_data_revisions_diff::tracker_data_revisions_diff,
        web_scraper::{
            WebScraperContentRequest, WebScraperContentRequestScripts, WebScraperContentResponse,
            WebScraperErrorResponse,
        },
        Tracker, TrackerDataRevision,
    },
};
use anyhow::{anyhow, bail};
use cron::Schedule;
use futures::Stream;
use std::time::Duration;
use time::OffsetDateTime;
use tracing::debug;
use uuid::Uuid;

/// Defines a maximum number of jobs that can be retrieved from the database at once.
const MAX_JOBS_PAGE_SIZE: usize = 1000;

/// We currently wait up to 60 seconds before starting to track web page.
const MAX_TRACKER_WEB_PAGE_WAIT_DELAY: Duration = Duration::from_secs(60);

/// We currently support up to 10 retry attempts for the tracker.
const MAX_TRACKER_RETRY_ATTEMPTS: u32 = 10;

/// We currently support minimum 60 seconds between retry attempts for the tracker.
const MIN_TRACKER_RETRY_INTERVAL: Duration = Duration::from_secs(60);

/// We currently support maximum 12 hours between retry attempts for the tracker.
const MAX_TRACKER_RETRY_INTERVAL: Duration = Duration::from_secs(12 * 3600);

/// Defines the maximum length of a tracker name.
pub const MAX_TRACKER_NAME_LENGTH: usize = 100;

pub struct TrackersApiExt<'a, DR: DnsResolver, ET: EmailTransport> {
    api: &'a Api<DR, ET>,
    trackers: TrackersDatabaseExt<'a>,
}

impl<'a, DR: DnsResolver, ET: EmailTransport> TrackersApiExt<'a, DR, ET> {
    /// Creates Trackers API.
    pub fn new(api: &'a Api<DR, ET>) -> Self {
        Self {
            api,
            trackers: api.db.trackers(),
        }
    }

    /// Returns all trackers.
    pub async fn get_trackers(&self) -> anyhow::Result<Vec<Tracker>> {
        self.trackers.get_trackers().await
    }

    /// Returns tracker by its ID.
    pub async fn get_tracker(&self, id: Uuid) -> anyhow::Result<Option<Tracker>> {
        self.trackers.get_tracker(id).await
    }

    /// Creates a new web page content tracker.
    pub async fn create_tracker(&self, params: TrackerCreateParams) -> anyhow::Result<Tracker> {
        let tracker = Tracker {
            id: Uuid::now_v7(),
            name: params.name,
            url: params.url,
            settings: params.settings,
            job_id: None,
            job_config: params.job_config,
            // Preserve timestamp only up to seconds.
            created_at: OffsetDateTime::from_unix_timestamp(
                OffsetDateTime::now_utc().unix_timestamp(),
            )?,
        };

        self.validate_tracker(&tracker).await?;

        self.trackers.insert_tracker(&tracker).await?;

        Ok(tracker)
    }

    /// Updates existing tracker.
    pub async fn update_tracker(
        &self,
        id: Uuid,
        params: TrackerUpdateParams,
    ) -> anyhow::Result<Tracker> {
        if params.name.is_none()
            && params.url.is_none()
            && params.settings.is_none()
            && params.job_config.is_none()
        {
            bail!(RetrackError::client(format!(
                "Either new name, url, settings, or job config should be provided ({id})."
            )));
        }

        let Some(existing_tracker) = self.trackers.get_tracker(id).await? else {
            bail!(RetrackError::client(format!(
                "Tracker ('{id}') is not found."
            )));
        };

        let changed_url = params
            .url
            .as_ref()
            .map(|url| url != &existing_tracker.url)
            .unwrap_or_default();

        let disabled_revisions = params
            .settings
            .as_ref()
            .map(|settings| settings.revisions == 0)
            .unwrap_or_default();

        let changed_schedule = params
            .job_config
            .as_ref()
            .map(
                |job_config| match (&existing_tracker.job_config, job_config) {
                    (Some(existing_job_config), Some(job_config)) => {
                        job_config.schedule != existing_job_config.schedule
                    }
                    _ => true,
                },
            )
            .unwrap_or_default();

        let job_id = if disabled_revisions || changed_schedule {
            None
        } else {
            existing_tracker.job_id
        };

        let tracker = Tracker {
            name: params.name.unwrap_or(existing_tracker.name),
            url: params.url.unwrap_or(existing_tracker.url),
            settings: params.settings.unwrap_or(existing_tracker.settings),
            job_id,
            job_config: params.job_config.unwrap_or(existing_tracker.job_config),
            ..existing_tracker
        };

        self.validate_tracker(&tracker).await?;

        self.trackers.update_tracker(&tracker).await?;

        if changed_url {
            debug!(tracker.id = %id, "Tracker changed URL, clearing all data revisions.");
            self.trackers.clear_tracker_data(id).await?;
        }

        Ok(tracker)
    }

    /// Removes existing tracker and all data.
    pub async fn remove_tracker(&self, id: Uuid) -> anyhow::Result<()> {
        self.trackers.remove_tracker(id).await
    }

    /// Persists data revision for the specified tracker.
    pub async fn create_tracker_data_revision(
        &self,
        tracker_id: Uuid,
    ) -> anyhow::Result<Option<TrackerDataRevision>> {
        let Some(tracker) = self.get_tracker(tracker_id).await? else {
            bail!(RetrackError::client(format!(
                "Tracker ('{tracker_id}') is not found."
            )));
        };

        // Enforce revisions limit and displace old ones.
        let max_revisions = std::cmp::min(
            tracker.settings.revisions,
            self.api.config.trackers.max_revisions,
        );
        if max_revisions == 0 {
            return Ok(None);
        }

        let revisions = self.trackers.get_tracker_data(tracker.id).await?;

        let scraper_request = WebScraperContentRequest::with_default_parameters(&tracker.url)
            .set_delay(tracker.settings.delay);
        let scraper_request = if let Some(revision) = revisions.last() {
            scraper_request.set_previous_content(&revision.data)
        } else {
            scraper_request
        };

        let extract_content_script = tracker.settings.extractor.as_ref();
        let scraper_request = if let Some(extract_content) = extract_content_script {
            scraper_request.set_scripts(WebScraperContentRequestScripts {
                extract_content: Some(extract_content),
            })
        } else {
            scraper_request
        };

        let scraper_request = if let Some(headers) = tracker.settings.headers.as_ref() {
            scraper_request.set_headers(headers)
        } else {
            scraper_request
        };

        let scraper_response = reqwest::Client::new()
            .post(format!(
                "{}api/web_page/content",
                self.api.config.as_ref().components.web_scraper_url.as_str()
            ))
            .json(&scraper_request)
            .send()
            .await
            .map_err(|err| {
                anyhow!(
                    "Could not connect to the web scraper service to extract content for the tracker ('{}'): {err:?}",
                    tracker.id
                )
            })?;

        if !scraper_response.status().is_success() {
            let is_client_error = scraper_response.status().is_client_error();
            let scraper_error_response = scraper_response
                .json::<WebScraperErrorResponse>()
                .await
                .map_err(|err| {
                anyhow!(
                    "Could not deserialize scraper error response for the tracker ('{}'): {err:?}",
                    tracker.id
                )
            })?;
            if is_client_error {
                bail!(RetrackError::client(scraper_error_response.message));
            } else {
                bail!(
                    "Unexpected scraper error for the web tracker ('{}'): {:?}",
                    tracker.id,
                    scraper_error_response.message
                );
            }
        }

        let scraper_response = scraper_response
            .json::<WebScraperContentResponse>()
            .await
            .map_err(|err| {
                anyhow!(
                    "Could not deserialize scraper response for the web tracker ('{}'): {err:?}",
                    tracker.id
                )
            })?;

        // Check if there is a revision with the same timestamp. If so, drop newly fetched revision.
        if revisions
            .iter()
            .any(|revision| revision.created_at == scraper_response.timestamp)
        {
            return Ok(None);
        }

        // Check if content has changed.
        if let Some(revision) = revisions.last() {
            if revision.data == scraper_response.content {
                return Ok(None);
            }
        }

        let new_revision = TrackerDataRevision {
            id: Uuid::now_v7(),
            tracker_id: tracker.id,
            data: scraper_response.content,
            created_at: scraper_response.timestamp,
        };

        // Insert new revision.
        self.trackers
            .insert_tracker_data_revision(&new_revision)
            .await?;

        // Enforce revisions limit and displace old ones.
        if revisions.len() >= max_revisions {
            let revisions_to_remove = revisions.len() - max_revisions + 1;
            for revision in revisions.iter().take(revisions_to_remove) {
                self.trackers
                    .remove_tracker_data_revision(tracker.id, revision.id)
                    .await?;
            }
        }

        Ok(Some(new_revision))
    }

    /// Returns all stored tracker data revisions.
    pub async fn get_tracker_data(
        &self,
        tracker_id: Uuid,
        params: TrackerListRevisionsParams,
    ) -> anyhow::Result<Vec<TrackerDataRevision>> {
        if params.refresh {
            self.create_tracker_data_revision(tracker_id).await?;
        } else if self.get_tracker(tracker_id).await?.is_none() {
            bail!(RetrackError::client(format!(
                "Tracker ('{tracker_id}') is not found."
            )));
        }

        let revisions = self.trackers.get_tracker_data(tracker_id).await?;
        if params.calculate_diff {
            tracker_data_revisions_diff(revisions)
        } else {
            Ok(revisions)
        }
    }

    /// Removes all persisted tracker revisions data.
    pub async fn clear_tracker_data(&self, tracker_id: Uuid) -> anyhow::Result<()> {
        self.trackers.clear_tracker_data(tracker_id).await
    }

    /// Returns all tracker job references that have jobs that need to be scheduled.
    pub async fn get_unscheduled_trackers(&self) -> anyhow::Result<Vec<Tracker>> {
        self.trackers.get_unscheduled_trackers().await
    }

    /// Returns all trackers that have pending jobs.
    pub fn get_pending_trackers(&self) -> impl Stream<Item = anyhow::Result<Tracker>> + '_ {
        self.trackers.get_pending_trackers(MAX_JOBS_PAGE_SIZE)
    }

    /// Returns tracker by the corresponding job ID.
    pub async fn get_tracker_by_job_id(&self, job_id: Uuid) -> anyhow::Result<Option<Tracker>> {
        self.trackers.get_tracker_by_job_id(job_id).await
    }

    /// Update tracker job ID reference (link or unlink).
    pub async fn update_tracker_job(&self, id: Uuid, job_id: Option<Uuid>) -> anyhow::Result<()> {
        self.trackers.update_tracker_job(id, job_id).await
    }

    async fn validate_tracker(&self, tracker: &Tracker) -> anyhow::Result<()> {
        if tracker.name.is_empty() {
            bail!(RetrackError::client("Tracker name cannot be empty.",));
        }

        if tracker.name.len() > MAX_TRACKER_NAME_LENGTH {
            bail!(RetrackError::client(format!(
                "Tracker name cannot be longer than {MAX_TRACKER_NAME_LENGTH} characters."
            )));
        }

        let config = &self.api.config.trackers;
        if tracker.settings.revisions > config.max_revisions {
            bail!(RetrackError::client(format!(
                "Tracker revisions count cannot be greater than {}.",
                config.max_revisions
            )));
        }

        if tracker.settings.delay > MAX_TRACKER_WEB_PAGE_WAIT_DELAY {
            bail!(RetrackError::client(format!(
                "Tracker delay cannot be greater than {}ms.",
                MAX_TRACKER_WEB_PAGE_WAIT_DELAY.as_millis()
            )));
        }

        if let Some(ref script) = tracker.settings.extractor {
            if script.is_empty() {
                bail!(RetrackError::client(
                    "Tracker extractor script cannot be empty."
                ));
            }
        }

        if let Some(job_config) = &tracker.job_config {
            // Validate that the schedule is a valid cron expression.
            let schedule = match Schedule::try_from(job_config.schedule.as_str()) {
                Ok(schedule) => schedule,
                Err(err) => {
                    bail!(RetrackError::client_with_root_cause(
                        anyhow!(
                            "Failed to parse schedule `{}`: {err:?}",
                            job_config.schedule
                        )
                        .context("Tracker schedule must be a valid cron expression.")
                    ));
                }
            };

            // Check if the interval between next occurrences is greater or equal to minimum
            // interval defined by the subscription.
            let min_schedule_interval = schedule.min_interval()?;
            if min_schedule_interval < config.min_schedule_interval {
                bail!(RetrackError::client(format!(
                    "Tracker schedule must have at least {} between occurrences, but detected {}.",
                    humantime::format_duration(config.min_schedule_interval),
                    humantime::format_duration(min_schedule_interval)
                )));
            }

            // Validate retry strategy.
            if let Some(retry_strategy) = &job_config.retry_strategy {
                let max_attempts = retry_strategy.max_attempts();
                if max_attempts == 0 || max_attempts > MAX_TRACKER_RETRY_ATTEMPTS {
                    bail!(RetrackError::client(
                        format!("Tracker max retry attempts cannot be zero or greater than {MAX_TRACKER_RETRY_ATTEMPTS}, but received {max_attempts}.")
                    ));
                }

                let min_interval = *retry_strategy.min_interval();
                if min_interval < MIN_TRACKER_RETRY_INTERVAL {
                    bail!(RetrackError::client(format!(
                        "Tracker min retry interval cannot be less than {}, but received {}.",
                        humantime::format_duration(MIN_TRACKER_RETRY_INTERVAL),
                        humantime::format_duration(min_interval)
                    )));
                }

                if let SchedulerJobRetryStrategy::Linear { max_interval, .. }
                | SchedulerJobRetryStrategy::Exponential { max_interval, .. } = retry_strategy
                {
                    let max_interval = *max_interval;
                    if max_interval < MIN_TRACKER_RETRY_INTERVAL {
                        bail!(RetrackError::client(
                            format!(
                                "Tracker retry strategy max interval cannot be less than {}, but received {}.",
                                humantime::format_duration(MIN_TRACKER_RETRY_INTERVAL),
                                humantime::format_duration(max_interval)
                            )
                        ));
                    }

                    if max_interval > MAX_TRACKER_RETRY_INTERVAL
                        || max_interval > min_schedule_interval
                    {
                        bail!(RetrackError::client(
                            format!(
                                "Tracker retry strategy max interval cannot be greater than {}, but received {}.",
                                humantime::format_duration(MAX_TRACKER_RETRY_INTERVAL.min(min_schedule_interval)),
                                humantime::format_duration(max_interval)
                            )
                        ));
                    }
                }
            }
        }

        if config.restrict_to_public_urls && !self.api.network.is_public_web_url(&tracker.url).await
        {
            bail!(RetrackError::client(
                format!("Tracker URL must be either `http` or `https` and have a valid public reachable domain name, but received {}.", tracker.url)
            ));
        }

        Ok(())
    }
}

impl<'a, DR: DnsResolver, ET: EmailTransport> Api<DR, ET> {
    /// Returns an API to work with trackers.
    pub fn trackers(&'a self) -> TrackersApiExt<'a, DR, ET> {
        TrackersApiExt::new(self)
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        config::{Config, TrackersConfig},
        error::Error as RetrackError,
        scheduler::{SchedulerJob, SchedulerJobConfig, SchedulerJobRetryStrategy},
        tests::{
            mock_api, mock_api_with_config, mock_api_with_network, mock_config,
            mock_network_with_records, mock_scheduler_job, mock_upsert_scheduler_job,
            RawSchedulerJobStoredData, WebScraperContentRequest, WebScraperContentResponse,
            WebScraperErrorResponse,
        },
        trackers::{
            Tracker, TrackerCreateParams, TrackerListRevisionsParams, TrackerSettings,
            TrackerUpdateParams,
        },
    };
    use actix_web::ResponseError;
    use futures::StreamExt;
    use httpmock::MockServer;
    use insta::assert_debug_snapshot;
    use sqlx::PgPool;
    use std::{net::Ipv4Addr, time::Duration};
    use time::OffsetDateTime;
    use trust_dns_resolver::{
        proto::rr::{rdata::A, RData, Record},
        Name,
    };
    use url::Url;
    use uuid::{uuid, Uuid};

    fn get_content(timestamp: i64, label: &str) -> anyhow::Result<WebScraperContentResponse> {
        Ok(WebScraperContentResponse {
            timestamp: OffsetDateTime::from_unix_timestamp(timestamp)?,
            content: label.to_string(),
        })
    }

    #[sqlx::test]
    async fn properly_creates_new_tracker(pool: PgPool) -> anyhow::Result<()> {
        let api = mock_api(pool).await?;
        let api = api.trackers();

        let tracker = api
            .create_tracker(TrackerCreateParams {
                name: "name_one".to_string(),
                url: Url::parse("http://localhost:1234/my/app?q=2")?,
                settings: TrackerSettings {
                    revisions: 3,
                    delay: Duration::from_millis(2000),
                    extractor: Default::default(),
                    headers: Default::default(),
                },
                job_config: Some(SchedulerJobConfig {
                    schedule: "@hourly".to_string(),
                    retry_strategy: Some(SchedulerJobRetryStrategy::Constant {
                        interval: Duration::from_secs(120),
                        max_attempts: 5,
                    }),
                    notifications: false,
                }),
            })
            .await?;

        assert_eq!(tracker, api.get_tracker(tracker.id).await?.unwrap());

        Ok(())
    }

    #[sqlx::test]
    async fn properly_validates_tracker_at_creation(pool: PgPool) -> anyhow::Result<()> {
        let api = mock_api_with_config(
            pool.clone(),
            Config {
                trackers: TrackersConfig {
                    restrict_to_public_urls: true,
                    ..Default::default()
                },
                ..mock_config()?
            },
        )
        .await?;

        let api = api.trackers();

        let settings = TrackerSettings {
            revisions: 3,
            delay: Duration::from_millis(2000),
            extractor: Default::default(),
            headers: Default::default(),
        };
        let url = Url::parse("https://retrack.dev")?;

        let create_and_fail = |result: anyhow::Result<_>| -> RetrackError {
            result.unwrap_err().downcast::<RetrackError>().unwrap()
        };

        // Empty name.
        assert_debug_snapshot!(
            create_and_fail(api.create_tracker(TrackerCreateParams {
                name: "".to_string(),
                url: url.clone(),
                settings: settings.clone(),
                job_config: None
            }).await),
            @r###""Tracker name cannot be empty.""###
        );

        // Very long name.
        assert_debug_snapshot!(
            create_and_fail(api.create_tracker(TrackerCreateParams {
                name: "a".repeat(101),
                url: url.clone(),
                settings: settings.clone(),
                job_config: None
            }).await),
            @r###""Tracker name cannot be longer than 100 characters.""###
        );

        // Too many revisions.
        assert_debug_snapshot!(
            create_and_fail(api.create_tracker(TrackerCreateParams {
                name: "name".to_string(),
                url: url.clone(),
                settings: TrackerSettings {
                    revisions: 31,
                    ..settings.clone()
                },
                job_config: None
            }).await),
            @r###""Tracker revisions count cannot be greater than 30.""###
        );

        // Too long delay.
        assert_debug_snapshot!(
            create_and_fail(api.create_tracker(TrackerCreateParams {
                name: "name".to_string(),
                url: url.clone(),
                settings: TrackerSettings {
                    delay: Duration::from_secs(61),
                    ..settings.clone()
                },
                job_config: None
            }).await),
            @r###""Tracker delay cannot be greater than 60000ms.""###
        );

        // Empty extractor script.
        assert_debug_snapshot!(
            create_and_fail(api.create_tracker(TrackerCreateParams {
                name: "name".to_string(),
                url: url.clone(),
                settings: TrackerSettings {
                    extractor: Some("".to_string()),
                    ..settings.clone()
                },
                job_config: None
            }).await),
            @r###""Tracker extractor script cannot be empty.""###
        );

        // Invalid schedule.
        assert_debug_snapshot!(
            create_and_fail(api.create_tracker(TrackerCreateParams {
                name: "name".to_string(),
                url: url.clone(),
                settings: settings.clone(),
                job_config: Some(SchedulerJobConfig {
                    schedule: "-".to_string(),
                    retry_strategy: None,
                    notifications: false,
                }),
            }).await),
            @r###"
        Error {
            context: "Tracker schedule must be a valid cron expression.",
            source: "Failed to parse schedule `-`: Error { kind: Expression(\"Invalid cron expression.\") }",
        }
        "###
        );

        // Invalid schedule interval.
        assert_debug_snapshot!(
            create_and_fail(api.create_tracker(TrackerCreateParams {
                name: "name".to_string(),
                url: url.clone(),
                settings: settings.clone(),
                job_config: Some(SchedulerJobConfig {
                    schedule: "0/5 * * * * *".to_string(),
                    retry_strategy: None,
                    notifications: false,
                }),
            }).await),
            @r###""Tracker schedule must have at least 10s between occurrences, but detected 5s.""###
        );

        // Too few retry attempts.
        assert_debug_snapshot!(
            create_and_fail(api.create_tracker(TrackerCreateParams {
                name: "name".to_string(),
                url: url.clone(),
                settings: settings.clone(),
                job_config: Some(SchedulerJobConfig {
                    schedule: "@daily".to_string(),
                    retry_strategy: Some(SchedulerJobRetryStrategy::Constant {
                        interval: Duration::from_secs(120),
                        max_attempts: 0,
                    }),
                    notifications: false,
                }),
            }).await),
            @r###""Tracker max retry attempts cannot be zero or greater than 10, but received 0.""###
        );

        // Too many retry attempts.
        assert_debug_snapshot!(
            create_and_fail(api.create_tracker(TrackerCreateParams {
                name: "name".to_string(),
                url: url.clone(),
                settings: settings.clone(),
                job_config: Some(SchedulerJobConfig {
                    schedule: "@daily".to_string(),
                    retry_strategy: Some(SchedulerJobRetryStrategy::Constant {
                        interval: Duration::from_secs(120),
                        max_attempts: 11,
                    }),
                    notifications: false,
                }),
            }).await),
            @r###""Tracker max retry attempts cannot be zero or greater than 10, but received 11.""###
        );

        // Too low retry interval.
        assert_debug_snapshot!(
            create_and_fail(api.create_tracker(TrackerCreateParams {
                name: "name".to_string(),
                url: url.clone(),
                settings: settings.clone(),
                job_config: Some(SchedulerJobConfig {
                    schedule: "@daily".to_string(),
                    retry_strategy: Some(SchedulerJobRetryStrategy::Constant {
                        interval: Duration::from_secs(30),
                        max_attempts: 5,
                    }),
                    notifications: false,
                }),
            }).await),
            @r###""Tracker min retry interval cannot be less than 1m, but received 30s.""###
        );

        // Too low max retry interval.
        assert_debug_snapshot!(
            create_and_fail(api.create_tracker(TrackerCreateParams {
                name: "name".to_string(),
                url: url.clone(),
                settings: settings.clone(),
                job_config: Some(SchedulerJobConfig {
                    schedule: "@daily".to_string(),
                    retry_strategy: Some(SchedulerJobRetryStrategy::Linear {
                        initial_interval: Duration::from_secs(120),
                        increment: Duration::from_secs(10),
                        max_interval: Duration::from_secs(30),
                        max_attempts: 5,
                    }),
                    notifications: false,
                }),
            }).await),
            @r###""Tracker retry strategy max interval cannot be less than 1m, but received 30s.""###
        );

        // Too high max retry interval.
        assert_debug_snapshot!(
            create_and_fail(api.create_tracker(TrackerCreateParams {
                name: "name".to_string(),
                url: url.clone(),
                settings: settings.clone(),
                job_config: Some(SchedulerJobConfig {
                    schedule: "@monthly".to_string(),
                    retry_strategy: Some(SchedulerJobRetryStrategy::Linear {
                        initial_interval: Duration::from_secs(120),
                        increment: Duration::from_secs(10),
                        max_interval: Duration::from_secs(13 * 3600),
                        max_attempts: 5,
                    }),
                    notifications: false,
                }),
            }).await),
            @r###""Tracker retry strategy max interval cannot be greater than 12h, but received 13h.""###
        );

        assert_debug_snapshot!(
            create_and_fail(api.create_tracker(TrackerCreateParams {
                name: "name".to_string(),
                url: url.clone(),
                settings: settings.clone(),
                job_config: Some(SchedulerJobConfig {
                    schedule: "@hourly".to_string(),
                    retry_strategy: Some(SchedulerJobRetryStrategy::Linear {
                        initial_interval: Duration::from_secs(120),
                        increment: Duration::from_secs(10),
                        max_interval: Duration::from_secs(2 * 3600),
                        max_attempts: 5,
                    }),
                    notifications: false,
                }),
            }).await),
            @r###""Tracker retry strategy max interval cannot be greater than 1h, but received 2h.""###
        );

        // Invalid URL schema.
        assert_debug_snapshot!(
            create_and_fail(api.create_tracker(TrackerCreateParams {
                name: "name".to_string(),
                url: Url::parse("ftp://retrack.dev")?,
                settings: settings.clone(),
                job_config: None
            }).await),
            @r###""Tracker URL must be either `http` or `https` and have a valid public reachable domain name, but received ftp://retrack.dev/.""###
        );

        let mut api_with_local_network = mock_api_with_network(
            pool,
            mock_network_with_records::<1>(vec![Record::from_rdata(
                Name::new(),
                300,
                RData::A(A(Ipv4Addr::new(127, 0, 0, 1))),
            )]),
        )
        .await?;
        api_with_local_network
            .config
            .trackers
            .restrict_to_public_urls = true;

        // Non-public URL.
        assert_debug_snapshot!(
            create_and_fail(api_with_local_network.trackers().create_tracker(TrackerCreateParams {
                name: "name".to_string(),
                url: Url::parse("https://127.0.0.1")?,
                settings: settings.clone(),
                job_config: None
            }).await),
            @r###""Tracker URL must be either `http` or `https` and have a valid public reachable domain name, but received https://127.0.0.1/.""###
        );

        Ok(())
    }

    #[sqlx::test]
    async fn properly_updates_tracker(pool: PgPool) -> anyhow::Result<()> {
        let api = mock_api(pool).await?;

        let api = api.trackers();
        let tracker = api
            .create_tracker(TrackerCreateParams {
                name: "name_one".to_string(),
                url: Url::parse("http://localhost:1234/my/app?q=2")?,
                settings: TrackerSettings {
                    revisions: 3,
                    delay: Duration::from_millis(2000),
                    extractor: Default::default(),
                    headers: Default::default(),
                },
                job_config: None,
            })
            .await?;

        // Update name.
        let updated_tracker = api
            .update_tracker(
                tracker.id,
                TrackerUpdateParams {
                    name: Some("name_two".to_string()),
                    ..Default::default()
                },
            )
            .await?;
        let expected_tracker = Tracker {
            name: "name_two".to_string(),
            ..tracker.clone()
        };
        assert_eq!(expected_tracker, updated_tracker);
        assert_eq!(
            expected_tracker,
            api.get_tracker(tracker.id).await?.unwrap()
        );

        // Update URL.
        let updated_tracker = api
            .update_tracker(
                tracker.id,
                TrackerUpdateParams {
                    url: Some("http://localhost:1234/my/app?q=3".parse()?),
                    ..Default::default()
                },
            )
            .await?;
        let expected_tracker = Tracker {
            name: "name_two".to_string(),
            url: "http://localhost:1234/my/app?q=3".parse()?,
            ..tracker.clone()
        };
        assert_eq!(expected_tracker, updated_tracker);
        assert_eq!(
            expected_tracker,
            api.get_tracker(tracker.id).await?.unwrap()
        );

        // Update settings.
        let updated_tracker = api
            .update_tracker(
                tracker.id,
                TrackerUpdateParams {
                    settings: Some(TrackerSettings {
                        revisions: 4,
                        ..tracker.settings.clone()
                    }),
                    ..Default::default()
                },
            )
            .await?;
        let expected_tracker = Tracker {
            name: "name_two".to_string(),
            url: "http://localhost:1234/my/app?q=3".parse()?,
            settings: TrackerSettings {
                revisions: 4,
                ..tracker.settings.clone()
            },
            ..tracker.clone()
        };
        assert_eq!(expected_tracker, updated_tracker);
        assert_eq!(
            expected_tracker,
            api.get_tracker(tracker.id).await?.unwrap()
        );

        // Update job config.
        let updated_tracker = api
            .update_tracker(
                tracker.id,
                TrackerUpdateParams {
                    job_config: Some(Some(SchedulerJobConfig {
                        schedule: "@hourly".to_string(),
                        retry_strategy: Some(SchedulerJobRetryStrategy::Constant {
                            interval: Duration::from_secs(120),
                            max_attempts: 5,
                        }),
                        notifications: false,
                    })),
                    ..Default::default()
                },
            )
            .await?;
        let expected_tracker = Tracker {
            name: "name_two".to_string(),
            url: "http://localhost:1234/my/app?q=3".parse()?,
            settings: TrackerSettings {
                revisions: 4,
                ..tracker.settings.clone()
            },
            job_config: Some(SchedulerJobConfig {
                schedule: "@hourly".to_string(),
                retry_strategy: Some(SchedulerJobRetryStrategy::Constant {
                    interval: Duration::from_secs(120),
                    max_attempts: 5,
                }),
                notifications: false,
            }),
            ..tracker.clone()
        };
        assert_eq!(expected_tracker, updated_tracker);
        assert_eq!(
            expected_tracker,
            api.get_tracker(tracker.id).await?.unwrap()
        );

        // Remove job config.
        let updated_tracker = api
            .update_tracker(
                tracker.id,
                TrackerUpdateParams {
                    job_config: Some(None),
                    ..Default::default()
                },
            )
            .await?;
        let expected_tracker = Tracker {
            name: "name_two".to_string(),
            url: "http://localhost:1234/my/app?q=3".parse()?,
            settings: TrackerSettings {
                revisions: 4,
                ..tracker.settings.clone()
            },
            job_config: None,
            ..tracker.clone()
        };
        assert_eq!(expected_tracker, updated_tracker);
        assert_eq!(
            expected_tracker,
            api.get_tracker(tracker.id).await?.unwrap()
        );

        Ok(())
    }

    #[sqlx::test]
    async fn properly_validates_tracker_at_update(pool: PgPool) -> anyhow::Result<()> {
        let api = mock_api_with_config(
            pool.clone(),
            Config {
                trackers: TrackersConfig {
                    restrict_to_public_urls: true,
                    ..Default::default()
                },
                ..mock_config()?
            },
        )
        .await?;

        let trackers = api.trackers();
        let tracker = trackers
            .create_tracker(TrackerCreateParams {
                name: "name_one".to_string(),
                url: Url::parse("https://retrack.dev")?,
                settings: TrackerSettings {
                    revisions: 3,
                    delay: Duration::from_millis(2000),
                    extractor: Default::default(),
                    headers: Default::default(),
                },
                job_config: Some(SchedulerJobConfig {
                    schedule: "@hourly".to_string(),
                    retry_strategy: Some(SchedulerJobRetryStrategy::Constant {
                        interval: Duration::from_secs(120),
                        max_attempts: 5,
                    }),
                    notifications: false,
                }),
            })
            .await?;

        let update_and_fail = |result: anyhow::Result<_>| -> RetrackError {
            result.unwrap_err().downcast::<RetrackError>().unwrap()
        };

        // Empty parameters.
        let update_result = update_and_fail(
            trackers
                .update_tracker(tracker.id, Default::default())
                .await,
        );
        assert_eq!(
            update_result.to_string(),
            format!(
                "Either new name, url, settings, or job config should be provided ({}).",
                tracker.id
            )
        );

        // Non-existent tracker.
        let update_result = update_and_fail(
            trackers
                .update_tracker(
                    uuid!("00000000-0000-0000-0000-000000000002"),
                    TrackerUpdateParams {
                        name: Some("name".to_string()),
                        ..Default::default()
                    },
                )
                .await,
        );
        assert_eq!(
            update_result.to_string(),
            "Tracker ('00000000-0000-0000-0000-000000000002') is not found."
        );

        // Empty name.
        assert_debug_snapshot!(
            update_and_fail(trackers.update_tracker(tracker.id, TrackerUpdateParams {
                name: Some("".to_string()),
                ..Default::default()
            }).await),
            @r###""Tracker name cannot be empty.""###
        );

        // Very long name.
        assert_debug_snapshot!(
            update_and_fail(trackers.update_tracker(tracker.id, TrackerUpdateParams {
                name: Some("a".repeat(101)),
                ..Default::default()
            }).await),
            @r###""Tracker name cannot be longer than 100 characters.""###
        );

        // Too many revisions.
        assert_debug_snapshot!(
            update_and_fail(trackers.update_tracker(tracker.id, TrackerUpdateParams {
                settings: Some(TrackerSettings {
                    revisions: 31,
                    ..tracker.settings.clone()
                }),
                ..Default::default()
            }).await),
            @r###""Tracker revisions count cannot be greater than 30.""###
        );

        // Too long delay.
        assert_debug_snapshot!(
            update_and_fail(trackers.update_tracker(tracker.id, TrackerUpdateParams {
                settings: Some(TrackerSettings {
                    delay: Duration::from_secs(61),
                    ..tracker.settings.clone()
                }),
                ..Default::default()
            }).await),
            @r###""Tracker delay cannot be greater than 60000ms.""###
        );

        // Empty extractor script
        assert_debug_snapshot!(
            update_and_fail(trackers.update_tracker(tracker.id, TrackerUpdateParams {
                settings: Some(TrackerSettings {
                    extractor: Some("".to_string()),
                   ..tracker.settings.clone()
                }),
                ..Default::default()
            }).await),
            @r###""Tracker extractor script cannot be empty.""###
        );

        // Invalid schedule.
        assert_debug_snapshot!(
            update_and_fail(trackers.update_tracker(tracker.id, TrackerUpdateParams {
                job_config: Some(Some(SchedulerJobConfig {
                    schedule: "-".to_string(),
                    retry_strategy: None,
                    notifications: false,
                })),
                ..Default::default()
            }).await),
            @r###"
        Error {
            context: "Tracker schedule must be a valid cron expression.",
            source: "Failed to parse schedule `-`: Error { kind: Expression(\"Invalid cron expression.\") }",
        }
        "###
        );

        // Invalid schedule interval.
        assert_debug_snapshot!(
            update_and_fail(trackers.update_tracker(tracker.id, TrackerUpdateParams {
                job_config: Some(Some(SchedulerJobConfig {
                    schedule: "0/5 * * * * *".to_string(),
                    retry_strategy: None,
                    notifications: false,
                })),
                ..Default::default()
            }).await),
            @r###""Tracker schedule must have at least 10s between occurrences, but detected 5s.""###
        );

        // Too few retry attempts.
        assert_debug_snapshot!(
            update_and_fail(trackers.update_tracker(tracker.id, TrackerUpdateParams {
                job_config: Some(Some(SchedulerJobConfig {
                    schedule: "@daily".to_string(),
                    retry_strategy: Some(SchedulerJobRetryStrategy::Constant {
                        interval: Duration::from_secs(120),
                        max_attempts: 0,
                    }),
                    notifications: false,
                })),
                ..Default::default()
            }).await),
            @r###""Tracker max retry attempts cannot be zero or greater than 10, but received 0.""###
        );

        // Too many retry attempts.
        assert_debug_snapshot!(
            update_and_fail(trackers.update_tracker(tracker.id, TrackerUpdateParams {
                job_config: Some(Some(SchedulerJobConfig {
                    schedule: "@daily".to_string(),
                    retry_strategy: Some(SchedulerJobRetryStrategy::Constant {
                        interval: Duration::from_secs(120),
                        max_attempts: 11,
                    }),
                    notifications: false,
                })),
                ..Default::default()
            }).await),
            @r###""Tracker max retry attempts cannot be zero or greater than 10, but received 11.""###
        );

        // Too low retry interval.
        assert_debug_snapshot!(
           update_and_fail(trackers.update_tracker(tracker.id, TrackerUpdateParams {
                job_config: Some(Some(SchedulerJobConfig {
                    schedule: "@daily".to_string(),
                    retry_strategy: Some(SchedulerJobRetryStrategy::Constant {
                        interval: Duration::from_secs(30),
                        max_attempts: 5,
                    }),
                    notifications: false,
                })),
                ..Default::default()
            }).await),
            @r###""Tracker min retry interval cannot be less than 1m, but received 30s.""###
        );

        // Too low max retry interval.
        assert_debug_snapshot!(
            update_and_fail(trackers.update_tracker(tracker.id, TrackerUpdateParams {
                job_config: Some(Some(SchedulerJobConfig {
                    schedule: "@daily".to_string(),
                    retry_strategy: Some(SchedulerJobRetryStrategy::Linear {
                        initial_interval: Duration::from_secs(120),
                        increment: Duration::from_secs(10),
                        max_interval: Duration::from_secs(30),
                        max_attempts: 5,
                    }),
                    notifications: false,
                })),
                ..Default::default()
            }).await),
            @r###""Tracker retry strategy max interval cannot be less than 1m, but received 30s.""###
        );

        // Too high max retry interval.
        assert_debug_snapshot!(
            update_and_fail(trackers.update_tracker(tracker.id, TrackerUpdateParams {
                job_config: Some(Some(SchedulerJobConfig {
                    schedule: "@monthly".to_string(),
                    retry_strategy: Some(SchedulerJobRetryStrategy::Linear {
                        initial_interval: Duration::from_secs(120),
                        increment: Duration::from_secs(10),
                        max_interval: Duration::from_secs(13 * 3600),
                        max_attempts: 5,
                    }),
                    notifications: false,
                })),
                ..Default::default()
            }).await),
            @r###""Tracker retry strategy max interval cannot be greater than 12h, but received 13h.""###
        );

        assert_debug_snapshot!(
            update_and_fail(trackers.update_tracker(tracker.id, TrackerUpdateParams {
                job_config: Some(Some(SchedulerJobConfig {
                    schedule: "@hourly".to_string(),
                    retry_strategy: Some(SchedulerJobRetryStrategy::Linear {
                        initial_interval: Duration::from_secs(120),
                        increment: Duration::from_secs(10),
                        max_interval: Duration::from_secs(2 * 3600),
                        max_attempts: 5,
                    }),
                    notifications: false,
                })),
               ..Default::default()
            }).await),
            @r###""Tracker retry strategy max interval cannot be greater than 1h, but received 2h.""###
        );

        // Invalid URL schema.
        assert_debug_snapshot!(
            update_and_fail(trackers.update_tracker(tracker.id, TrackerUpdateParams {
                url: Some(Url::parse("ftp://retrack.dev")?),
                ..Default::default()
            }).await),
            @r###""Tracker URL must be either `http` or `https` and have a valid public reachable domain name, but received ftp://retrack.dev/.""###
        );

        let mut api_with_local_network = mock_api_with_network(
            pool,
            mock_network_with_records::<1>(vec![Record::from_rdata(
                Name::new(),
                300,
                RData::A(A(Ipv4Addr::new(127, 0, 0, 1))),
            )]),
        )
        .await?;
        api_with_local_network
            .config
            .trackers
            .restrict_to_public_urls = true;

        // Non-public URL.
        let trackers = api_with_local_network.trackers();
        assert_debug_snapshot!(
            update_and_fail(trackers.update_tracker(tracker.id, TrackerUpdateParams {
                url: Some(Url::parse("https://127.0.0.1")?),
                ..Default::default()
            }).await),
            @r###""Tracker URL must be either `http` or `https` and have a valid public reachable domain name, but received https://127.0.0.1/.""###
        );

        Ok(())
    }

    #[sqlx::test]
    async fn properly_updates_tracker_job_id_at_update(pool: PgPool) -> anyhow::Result<()> {
        let api = mock_api(pool).await?;

        let trackers = api.trackers();
        let tracker = trackers
            .create_tracker(TrackerCreateParams {
                name: "name_one".to_string(),
                url: Url::parse("https://retrack.dev")?,
                settings: TrackerSettings {
                    revisions: 3,
                    delay: Duration::from_millis(2000),
                    extractor: Default::default(),
                    headers: Default::default(),
                },
                job_config: Some(SchedulerJobConfig {
                    schedule: "0 0 * * * *".to_string(),
                    retry_strategy: None,
                    notifications: true,
                }),
            })
            .await?;

        // Set job ID.
        api.trackers()
            .update_tracker_job(
                tracker.id,
                Some(uuid!("00000000-0000-0000-0000-000000000001")),
            )
            .await?;
        assert_eq!(
            Some(uuid!("00000000-0000-0000-0000-000000000001")),
            trackers.get_tracker(tracker.id).await?.unwrap().job_id
        );

        let updated_tracker = trackers
            .update_tracker(
                tracker.id,
                TrackerUpdateParams {
                    name: Some("name_two".to_string()),
                    ..Default::default()
                },
            )
            .await?;
        let expected_tracker = Tracker {
            name: "name_two".to_string(),
            job_id: Some(uuid!("00000000-0000-0000-0000-000000000001")),
            ..tracker.clone()
        };
        assert_eq!(expected_tracker, updated_tracker);
        assert_eq!(
            expected_tracker,
            trackers.get_tracker(tracker.id).await?.unwrap()
        );

        // Change in schedule will reset job ID.
        let updated_tracker = trackers
            .update_tracker(
                tracker.id,
                TrackerUpdateParams {
                    job_config: Some(Some(SchedulerJobConfig {
                        schedule: "0 1 * * * *".to_string(),
                        retry_strategy: None,
                        notifications: true,
                    })),
                    ..Default::default()
                },
            )
            .await?;
        let expected_tracker = Tracker {
            name: "name_two".to_string(),
            job_id: None,
            job_config: Some(SchedulerJobConfig {
                schedule: "0 1 * * * *".to_string(),
                retry_strategy: None,
                notifications: true,
            }),
            ..tracker.clone()
        };
        assert_eq!(expected_tracker, updated_tracker);
        assert_eq!(
            expected_tracker,
            trackers.get_tracker(tracker.id).await?.unwrap()
        );

        Ok(())
    }

    #[sqlx::test]
    async fn properly_removes_trackers(pool: PgPool) -> anyhow::Result<()> {
        let api = mock_api(pool).await?;

        let trackers = api.trackers();
        let tracker_one = trackers
            .create_tracker(TrackerCreateParams {
                name: "name_one".to_string(),
                url: Url::parse("https://retrack.dev")?,
                settings: TrackerSettings {
                    revisions: 3,
                    delay: Duration::from_millis(2000),
                    extractor: Default::default(),
                    headers: Default::default(),
                },
                job_config: Some(SchedulerJobConfig {
                    schedule: "0 0 * * * *".to_string(),
                    retry_strategy: None,
                    notifications: true,
                }),
            })
            .await?;
        let tracker_two = trackers
            .create_tracker(TrackerCreateParams {
                name: "name_two".to_string(),
                url: Url::parse("https://retrack.dev")?,
                settings: tracker_one.settings.clone(),
                job_config: tracker_one.job_config.clone(),
            })
            .await?;

        assert_eq!(
            trackers.get_trackers().await?,
            vec![tracker_one.clone(), tracker_two.clone()],
        );

        trackers.remove_tracker(tracker_one.id).await?;

        assert_eq!(trackers.get_trackers().await?, vec![tracker_two.clone()],);

        trackers.remove_tracker(tracker_two.id).await?;

        assert!(trackers.get_trackers().await?.is_empty());

        Ok(())
    }

    #[sqlx::test]
    async fn properly_returns_trackers_by_id(pool: PgPool) -> anyhow::Result<()> {
        let api = mock_api(pool).await?;

        let trackers = api.trackers();
        assert!(trackers
            .get_tracker(uuid!("00000000-0000-0000-0000-000000000001"))
            .await?
            .is_none());

        let tracker_one = trackers
            .create_tracker(TrackerCreateParams {
                name: "name_one".to_string(),
                url: Url::parse("https://retrack.dev")?,
                settings: TrackerSettings {
                    revisions: 3,
                    delay: Duration::from_millis(2000),
                    extractor: Default::default(),
                    headers: Default::default(),
                },
                job_config: Some(SchedulerJobConfig {
                    schedule: "0 0 * * * *".to_string(),
                    retry_strategy: None,
                    notifications: true,
                }),
            })
            .await?;
        assert_eq!(
            trackers.get_tracker(tracker_one.id).await?,
            Some(tracker_one.clone()),
        );

        let tracker_two = trackers
            .create_tracker(TrackerCreateParams {
                name: "name_two".to_string(),
                url: Url::parse("https://retrack.dev")?,
                settings: tracker_one.settings.clone(),
                job_config: tracker_one.job_config.clone(),
            })
            .await?;

        assert_eq!(
            trackers.get_tracker(tracker_two.id).await?,
            Some(tracker_two.clone()),
        );

        Ok(())
    }

    #[sqlx::test]
    async fn properly_returns_all_trackers(pool: PgPool) -> anyhow::Result<()> {
        let api = mock_api(pool).await?;

        let trackers = api.trackers();
        assert!(trackers.get_trackers().await?.is_empty(),);

        let tracker_one = trackers
            .create_tracker(TrackerCreateParams {
                name: "name_one".to_string(),
                url: Url::parse("https://retrack.dev")?,
                settings: TrackerSettings {
                    revisions: 3,
                    delay: Duration::from_millis(2000),
                    extractor: Default::default(),
                    headers: Default::default(),
                },
                job_config: Some(SchedulerJobConfig {
                    schedule: "0 0 * * * *".to_string(),
                    retry_strategy: None,
                    notifications: true,
                }),
            })
            .await?;
        assert_eq!(trackers.get_trackers().await?, vec![tracker_one.clone()],);
        let tracker_two = trackers
            .create_tracker(TrackerCreateParams {
                name: "name_two".to_string(),
                url: Url::parse("https://retrack.dev")?,
                settings: tracker_one.settings.clone(),
                job_config: tracker_one.job_config.clone(),
            })
            .await?;

        assert_eq!(
            trackers.get_trackers().await?,
            vec![tracker_one.clone(), tracker_two.clone()],
        );

        Ok(())
    }

    #[sqlx::test]
    async fn properly_saves_revision(pool: PgPool) -> anyhow::Result<()> {
        let server = MockServer::start();
        let mut config = mock_config()?;
        config.components.web_scraper_url = Url::parse(&server.base_url())?;

        let api = mock_api_with_config(pool, config).await?;

        let trackers = api.trackers();
        let tracker_one = trackers
            .create_tracker(TrackerCreateParams {
                name: "name_one".to_string(),
                url: Url::parse("https://retrack.dev/one")?,
                settings: TrackerSettings {
                    revisions: 3,
                    delay: Duration::from_millis(2000),
                    extractor: Default::default(),
                    headers: Default::default(),
                },
                job_config: Some(SchedulerJobConfig {
                    schedule: "0 0 * * * *".to_string(),
                    retry_strategy: None,
                    notifications: true,
                }),
            })
            .await?;
        let tracker_two = trackers
            .create_tracker(TrackerCreateParams {
                name: "name_two".to_string(),
                url: Url::parse("https://retrack.dev/two")?,
                settings: tracker_one.settings.clone(),
                job_config: tracker_one.job_config.clone(),
            })
            .await?;

        let tracker_one_data = trackers
            .get_tracker_data(tracker_one.id, Default::default())
            .await?;
        let tracker_two_data = trackers
            .get_tracker_data(tracker_two.id, Default::default())
            .await?;
        assert!(tracker_one_data.is_empty());
        assert!(tracker_two_data.is_empty());

        let content_one = get_content(946720800, "\"rev_1\"")?;
        let mut content_mock = server.mock(|when, then| {
            when.method(httpmock::Method::POST)
                .path("/api/web_page/content")
                .json_body(
                    serde_json::to_value(
                        WebScraperContentRequest::with_default_parameters(&tracker_one.url)
                            .set_delay(Duration::from_millis(2000)),
                    )
                    .unwrap(),
                );
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body_obj(&content_one);
        });

        let tracker_one_data = trackers
            .get_tracker_data(
                tracker_one.id,
                TrackerListRevisionsParams {
                    refresh: true,
                    calculate_diff: false,
                },
            )
            .await?;
        let tracker_two_data = trackers
            .get_tracker_data(tracker_two.id, Default::default())
            .await?;
        assert_eq!(tracker_one_data.len(), 1);
        assert_eq!(tracker_one_data[0].tracker_id, tracker_one.id);
        assert_eq!(tracker_one_data[0].data, content_one.content);
        assert!(tracker_two_data.is_empty());

        content_mock.assert();
        content_mock.delete();

        let content_two = get_content(946720900, "\"rev_2\"")?;
        let mut content_mock = server.mock(|when, then| {
            when.method(httpmock::Method::POST)
                .path("/api/web_page/content")
                .json_body(
                    serde_json::to_value(
                        WebScraperContentRequest::with_default_parameters(&tracker_one.url)
                            .set_delay(Duration::from_millis(2000))
                            .set_previous_content("\"rev_1\""),
                    )
                    .unwrap(),
                );
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body_obj(&content_two);
        });
        let revision = trackers
            .create_tracker_data_revision(tracker_one.id)
            .await?
            .unwrap();
        assert_eq!(
            revision.created_at,
            OffsetDateTime::from_unix_timestamp(946720900)?
        );
        assert_eq!(revision.data, "\"rev_2\"");
        content_mock.assert();
        content_mock.delete();

        let tracker_one_data = trackers
            .get_tracker_data(tracker_one.id, Default::default())
            .await?;
        let tracker_two_data = trackers
            .get_tracker_data(tracker_two.id, Default::default())
            .await?;
        assert_eq!(tracker_one_data.len(), 2);
        assert!(tracker_two_data.is_empty());

        let content_two = get_content(946720900, "\"rev_3\"")?;
        let content_mock = server.mock(|when, then| {
            when.method(httpmock::Method::POST)
                .path("/api/web_page/content")
                .json_body(
                    serde_json::to_value(
                        WebScraperContentRequest::with_default_parameters(&tracker_two.url)
                            .set_delay(Duration::from_millis(2000)),
                    )
                    .unwrap(),
                );
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body_obj(&content_two);
        });
        let revision = trackers
            .create_tracker_data_revision(tracker_two.id)
            .await?;
        assert_eq!(revision.unwrap().data, "\"rev_3\"");
        content_mock.assert();

        let tracker_one_data = trackers
            .get_tracker_data(
                tracker_one.id,
                TrackerListRevisionsParams {
                    refresh: false,
                    calculate_diff: true,
                },
            )
            .await?;
        let tracker_two_data = trackers
            .get_tracker_data(tracker_two.id, Default::default())
            .await?;
        assert_eq!(tracker_one_data.len(), 2);
        assert_eq!(tracker_two_data.len(), 1);

        assert_debug_snapshot!(
            tracker_one_data.into_iter().map(|rev| rev.data).collect::<Vec<_>>(),
            @r###"
        [
            "\"rev_1\"",
            "@@ -1 +1 @@\n-rev_1\n+rev_2\n",
        ]
        "###
        );
        assert_debug_snapshot!(
            tracker_two_data.into_iter().map(|rev| rev.data).collect::<Vec<_>>(),
            @r###"
        [
            "\"rev_3\"",
        ]
        "###
        );

        let tracker_one_data = trackers
            .get_tracker_data(
                tracker_one.id,
                TrackerListRevisionsParams {
                    refresh: false,
                    calculate_diff: false,
                },
            )
            .await?;
        assert_debug_snapshot!(
            tracker_one_data.into_iter().map(|rev| rev.data).collect::<Vec<_>>(),
            @r###"
        [
            "\"rev_1\"",
            "\"rev_2\"",
        ]
        "###
        );

        Ok(())
    }

    #[sqlx::test]
    async fn properly_forwards_error_if_revision_extraction_fails(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        let server = MockServer::start();
        let mut config = mock_config()?;
        config.components.web_scraper_url = Url::parse(&server.base_url())?;

        let api = mock_api_with_config(pool, config).await?;

        let trackers = api.trackers();
        let tracker = trackers
            .create_tracker(TrackerCreateParams {
                name: "name_one".to_string(),
                url: Url::parse("https://retrack.dev/one")?,
                settings: TrackerSettings {
                    revisions: 3,
                    delay: Duration::from_millis(2000),
                    extractor: Default::default(),
                    headers: Default::default(),
                },
                job_config: Some(SchedulerJobConfig {
                    schedule: "0 0 * * * *".to_string(),
                    retry_strategy: None,
                    notifications: true,
                }),
            })
            .await?;

        let content_mock = server.mock(|when, then| {
            when.method(httpmock::Method::POST)
                .path("/api/web_page/content")
                .json_body(
                    serde_json::to_value(
                        WebScraperContentRequest::with_default_parameters(&tracker.url)
                            .set_delay(Duration::from_millis(2000)),
                    )
                    .unwrap(),
                );
            then.status(400)
                .header("Content-Type", "application/json")
                .json_body_obj(&WebScraperErrorResponse {
                    message: "some client-error".to_string(),
                });
        });

        let scraper_error = trackers
            .get_tracker_data(
                tracker.id,
                TrackerListRevisionsParams {
                    refresh: true,
                    calculate_diff: false,
                },
            )
            .await
            .unwrap_err()
            .downcast::<RetrackError>()
            .unwrap();
        assert_eq!(scraper_error.status_code(), 400);
        assert_debug_snapshot!(
            scraper_error,
            @r###""some client-error""###
        );

        let tracker_content = trackers
            .get_tracker_data(tracker.id, Default::default())
            .await?;
        assert!(tracker_content.is_empty());
        content_mock.assert();

        Ok(())
    }

    #[sqlx::test]
    async fn properly_ignores_revision_with_the_same_timestamp(pool: PgPool) -> anyhow::Result<()> {
        let server = MockServer::start();
        let mut config = mock_config()?;
        config.components.web_scraper_url = Url::parse(&server.base_url())?;

        let api = mock_api_with_config(pool, config).await?;

        let trackers = api.trackers();
        let tracker = trackers
            .create_tracker(TrackerCreateParams {
                name: "name_one".to_string(),
                url: Url::parse("https://retrack.dev/one")?,
                settings: TrackerSettings {
                    revisions: 3,
                    delay: Duration::from_millis(2000),
                    extractor: Default::default(),
                    headers: Default::default(),
                },
                job_config: Some(SchedulerJobConfig {
                    schedule: "0 0 * * * *".to_string(),
                    retry_strategy: None,
                    notifications: true,
                }),
            })
            .await?;

        let tracker_content = trackers
            .get_tracker_data(tracker.id, Default::default())
            .await?;
        assert!(tracker_content.is_empty());

        let content_one = get_content(946720800, "\"rev_1\"")?;
        let mut content_mock = server.mock(|when, then| {
            when.method(httpmock::Method::POST)
                .path("/api/web_page/content")
                .json_body(
                    serde_json::to_value(
                        WebScraperContentRequest::with_default_parameters(&tracker.url)
                            .set_delay(Duration::from_millis(2000)),
                    )
                    .unwrap(),
                );
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body_obj(&content_one);
        });

        let revision = trackers.create_tracker_data_revision(tracker.id).await?;
        assert_eq!(revision.unwrap().data, "\"rev_1\"");
        content_mock.assert_hits(1);

        content_mock.delete();
        let content_mock = server.mock(|when, then| {
            when.method(httpmock::Method::POST)
                .path("/api/web_page/content")
                .json_body(
                    serde_json::to_value(
                        WebScraperContentRequest::with_default_parameters(&tracker.url)
                            .set_delay(Duration::from_millis(2000))
                            .set_previous_content("\"rev_1\""),
                    )
                    .unwrap(),
                );
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body_obj(&content_one);
        });

        let revision = trackers.create_tracker_data_revision(tracker.id).await?;
        assert!(revision.is_none());
        content_mock.assert_hits(1);

        let tracker_content = trackers
            .get_tracker_data(tracker.id, Default::default())
            .await?;
        assert_eq!(tracker_content.len(), 1);

        Ok(())
    }

    #[sqlx::test]
    async fn properly_ignores_revision_with_no_diff(pool: PgPool) -> anyhow::Result<()> {
        let server = MockServer::start();
        let mut config = mock_config()?;
        config.components.web_scraper_url = Url::parse(&server.base_url())?;

        let api = mock_api_with_config(pool, config).await?;

        let trackers = api.trackers();
        let tracker = trackers
            .create_tracker(TrackerCreateParams {
                name: "name_one".to_string(),
                url: Url::parse("https://retrack.dev/one")?,
                settings: TrackerSettings {
                    revisions: 3,
                    delay: Duration::from_millis(2000),
                    extractor: Default::default(),
                    headers: Default::default(),
                },
                job_config: Some(SchedulerJobConfig {
                    schedule: "0 0 * * * *".to_string(),
                    retry_strategy: None,
                    notifications: true,
                }),
            })
            .await?;

        let tracker_content = trackers
            .get_tracker_data(tracker.id, Default::default())
            .await?;
        assert!(tracker_content.is_empty());

        let content_one = get_content(946720800, "\"rev_1\"")?;
        let mut content_mock = server.mock(|when, then| {
            when.method(httpmock::Method::POST)
                .path("/api/web_page/content")
                .json_body(
                    serde_json::to_value(
                        WebScraperContentRequest::with_default_parameters(&tracker.url)
                            .set_delay(Duration::from_millis(2000)),
                    )
                    .unwrap(),
                );
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body_obj(&content_one);
        });

        let revision = trackers.create_tracker_data_revision(tracker.id).await?;
        assert_eq!(revision.unwrap().data, "\"rev_1\"");
        content_mock.assert();
        content_mock.delete();

        let content_two = get_content(946720900, "\"rev_1\"")?;
        let content_mock = server.mock(|when, then| {
            when.method(httpmock::Method::POST)
                .path("/api/web_page/content")
                .json_body(
                    serde_json::to_value(
                        WebScraperContentRequest::with_default_parameters(&tracker.url)
                            .set_delay(Duration::from_millis(2000))
                            .set_previous_content("\"rev_1\""),
                    )
                    .unwrap(),
                );
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body_obj(&content_two);
        });

        let revision = trackers.create_tracker_data_revision(tracker.id).await?;
        assert!(revision.is_none());
        content_mock.assert();

        let tracker_content = trackers
            .get_tracker_data(tracker.id, Default::default())
            .await?;
        assert_eq!(tracker_content.len(), 1);

        Ok(())
    }

    #[sqlx::test]
    async fn properly_removes_revision(pool: PgPool) -> anyhow::Result<()> {
        let server = MockServer::start();
        let mut config = mock_config()?;
        config.components.web_scraper_url = Url::parse(&server.base_url())?;

        let api = mock_api_with_config(pool, config).await?;

        let trackers = api.trackers();
        let tracker = trackers
            .create_tracker(TrackerCreateParams {
                name: "name_one".to_string(),
                url: Url::parse("https://retrack.dev/one")?,
                settings: TrackerSettings {
                    revisions: 3,
                    delay: Duration::from_millis(2000),
                    extractor: Default::default(),
                    headers: Default::default(),
                },
                job_config: Some(SchedulerJobConfig {
                    schedule: "0 0 * * * *".to_string(),
                    retry_strategy: None,
                    notifications: true,
                }),
            })
            .await?;

        let tracker_content = trackers
            .get_tracker_data(tracker.id, Default::default())
            .await?;
        assert!(tracker_content.is_empty());

        let content = get_content(946720800, "\"rev_1\"")?;
        let content_mock = server.mock(|when, then| {
            when.method(httpmock::Method::POST)
                .path("/api/web_page/content")
                .json_body(
                    serde_json::to_value(
                        WebScraperContentRequest::with_default_parameters(&tracker.url)
                            .set_delay(Duration::from_millis(2000)),
                    )
                    .unwrap(),
                );
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body_obj(&content);
        });

        let revision = trackers.create_tracker_data_revision(tracker.id).await?;
        assert!(revision.is_some());
        let tracker_content = trackers
            .get_tracker_data(tracker.id, Default::default())
            .await?;
        assert_eq!(tracker_content.len(), 1);
        content_mock.assert();

        trackers.clear_tracker_data(tracker.id).await?;

        let tracker_content = trackers
            .get_tracker_data(tracker.id, Default::default())
            .await?;
        assert!(tracker_content.is_empty());

        Ok(())
    }

    #[sqlx::test]
    async fn properly_removes_revisions_when_tracker_is_removed(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        let server = MockServer::start();
        let mut config = mock_config()?;
        config.components.web_scraper_url = Url::parse(&server.base_url())?;

        let api = mock_api_with_config(pool, config).await?;

        let trackers = api.trackers();
        let tracker = trackers
            .create_tracker(TrackerCreateParams {
                name: "name_one".to_string(),
                url: Url::parse("https://retrack.dev/one")?,
                settings: TrackerSettings {
                    revisions: 3,
                    delay: Duration::from_millis(2000),
                    extractor: Default::default(),
                    headers: Default::default(),
                },
                job_config: Some(SchedulerJobConfig {
                    schedule: "0 0 * * * *".to_string(),
                    retry_strategy: None,
                    notifications: true,
                }),
            })
            .await?;

        let tracker_content = trackers
            .get_tracker_data(tracker.id, Default::default())
            .await?;
        assert!(tracker_content.is_empty());

        let content = get_content(946720800, "\"rev_1\"")?;
        let content_mock = server.mock(|when, then| {
            when.method(httpmock::Method::POST)
                .path("/api/web_page/content")
                .json_body(
                    serde_json::to_value(
                        WebScraperContentRequest::with_default_parameters(&tracker.url)
                            .set_delay(Duration::from_millis(2000)),
                    )
                    .unwrap(),
                );
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body_obj(&content);
        });

        let revision = trackers.create_tracker_data_revision(tracker.id).await?;
        assert!(revision.is_some());
        let tracker_content = trackers
            .get_tracker_data(tracker.id, Default::default())
            .await?;
        assert_eq!(tracker_content.len(), 1);
        content_mock.assert();

        trackers.remove_tracker(tracker.id).await?;

        let tracker_content = api.db.trackers().get_tracker_data(tracker.id).await?;
        assert!(tracker_content.is_empty());

        Ok(())
    }

    #[sqlx::test]
    async fn properly_removes_revisions_when_tracker_url_changed(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        let server = MockServer::start();
        let mut config = mock_config()?;
        config.components.web_scraper_url = Url::parse(&server.base_url())?;

        let api = mock_api_with_config(pool, config).await?;

        let trackers = api.trackers();
        let tracker = trackers
            .create_tracker(TrackerCreateParams {
                name: "name_one".to_string(),
                url: Url::parse("https://retrack.dev/one")?,
                settings: TrackerSettings {
                    revisions: 3,
                    delay: Duration::from_millis(2000),
                    extractor: Default::default(),
                    headers: Default::default(),
                },
                job_config: Some(SchedulerJobConfig {
                    schedule: "0 0 * * * *".to_string(),
                    retry_strategy: None,
                    notifications: true,
                }),
            })
            .await?;

        let tracker_content = trackers
            .get_tracker_data(tracker.id, Default::default())
            .await?;
        assert!(tracker_content.is_empty());

        let content = get_content(946720800, "\"rev_1\"")?;
        let content_mock = server.mock(|when, then| {
            when.method(httpmock::Method::POST)
                .path("/api/web_page/content")
                .json_body(
                    serde_json::to_value(
                        WebScraperContentRequest::with_default_parameters(&tracker.url)
                            .set_delay(Duration::from_millis(2000)),
                    )
                    .unwrap(),
                );
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body_obj(&content);
        });

        let revision = trackers.create_tracker_data_revision(tracker.id).await?;
        assert!(revision.is_some());
        let tracker_content = trackers
            .get_tracker_data(tracker.id, Default::default())
            .await?;
        assert_eq!(tracker_content.len(), 1);
        content_mock.assert();

        // Update name (content shouldn't be touched).
        trackers
            .update_tracker(
                tracker.id,
                TrackerUpdateParams {
                    name: Some("name_one_new".to_string()),
                    ..Default::default()
                },
            )
            .await?;

        let tracker_content = trackers
            .get_tracker_data(tracker.id, Default::default())
            .await?;
        assert_eq!(tracker_content.len(), 1);

        // Update URL.
        trackers
            .update_tracker(
                tracker.id,
                TrackerUpdateParams {
                    url: Some("https://retrack.dev/two".parse()?),
                    ..Default::default()
                },
            )
            .await?;

        let tracker_content = trackers
            .get_tracker_data(tracker.id, Default::default())
            .await?;
        assert!(tracker_content.is_empty());

        Ok(())
    }

    #[sqlx::test]
    async fn properly_resets_job_id_when_tracker_schedule_changed(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        let api = mock_api(pool).await?;

        let trackers = api.trackers();
        let tracker = trackers
            .create_tracker(TrackerCreateParams {
                name: "name_one".to_string(),
                url: Url::parse("https://retrack.dev/one")?,
                settings: TrackerSettings {
                    revisions: 3,
                    delay: Duration::from_millis(2000),
                    extractor: Default::default(),
                    headers: Default::default(),
                },
                job_config: Some(SchedulerJobConfig {
                    schedule: "0 0 * * * *".to_string(),
                    retry_strategy: None,
                    notifications: true,
                }),
            })
            .await?;
        api.trackers()
            .update_tracker_job(
                tracker.id,
                Some(uuid!("00000000-0000-0000-0000-000000000001")),
            )
            .await?;
        assert_eq!(
            trackers.get_tracker(tracker.id).await?.unwrap().job_id,
            Some(uuid!("00000000-0000-0000-0000-000000000001")),
        );

        // Update everything except schedule (job ID shouldn't be touched).
        trackers
            .update_tracker(
                tracker.id,
                TrackerUpdateParams {
                    name: Some("name_one_new".to_string()),
                    url: Some(Url::parse("https://retrack.dev/two")?),
                    settings: Some(TrackerSettings {
                        revisions: 4,
                        delay: Duration::from_millis(3000),
                        extractor: Some("some".to_string()),
                        ..tracker.settings.clone()
                    }),
                    job_config: Some(Some(SchedulerJobConfig {
                        schedule: "0 0 * * * *".to_string(),
                        retry_strategy: Some(SchedulerJobRetryStrategy::Constant {
                            interval: Duration::from_secs(120),
                            max_attempts: 5,
                        }),
                        notifications: false,
                    })),
                },
            )
            .await?;

        assert_eq!(
            trackers.get_tracker(tracker.id).await?.unwrap().job_id,
            Some(uuid!("00000000-0000-0000-0000-000000000001")),
        );

        // Update schedule.
        trackers
            .update_tracker(
                tracker.id,
                TrackerUpdateParams {
                    job_config: Some(Some(SchedulerJobConfig {
                        schedule: "0 1 * * * *".to_string(),
                        retry_strategy: Some(SchedulerJobRetryStrategy::Constant {
                            interval: Duration::from_secs(120),
                            max_attempts: 5,
                        }),
                        notifications: false,
                    })),
                    ..Default::default()
                },
            )
            .await?;

        assert_eq!(
            trackers.get_tracker(tracker.id).await?.unwrap().job_id,
            None,
        );

        Ok(())
    }

    #[sqlx::test]
    async fn properly_removes_job_id_when_tracker_revisions_disabled(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        let api = mock_api(pool).await?;

        let trackers = api.trackers();
        let tracker = trackers
            .create_tracker(TrackerCreateParams {
                name: "name_one".to_string(),
                url: Url::parse("https://retrack.dev/one")?,
                settings: TrackerSettings {
                    revisions: 3,
                    delay: Duration::from_millis(2000),
                    extractor: Default::default(),
                    headers: Default::default(),
                },
                job_config: Some(SchedulerJobConfig {
                    schedule: "0 0 * * * *".to_string(),
                    retry_strategy: None,
                    notifications: true,
                }),
            })
            .await?;
        api.trackers()
            .update_tracker_job(
                tracker.id,
                Some(uuid!("00000000-0000-0000-0000-000000000001")),
            )
            .await?;
        assert_eq!(
            trackers.get_tracker(tracker.id).await?.unwrap().job_id,
            Some(uuid!("00000000-0000-0000-0000-000000000001")),
        );

        // Update everything except schedule and keep revisions enabled (job ID shouldn't be touched).
        trackers
            .update_tracker(
                tracker.id,
                TrackerUpdateParams {
                    name: Some("name_one_new".to_string()),
                    url: Some(Url::parse("https://retrack.dev/two")?),
                    settings: Some(TrackerSettings {
                        revisions: 4,
                        delay: Duration::from_millis(3000),
                        extractor: Some("some".to_string()),
                        ..tracker.settings.clone()
                    }),
                    job_config: Some(Some(SchedulerJobConfig {
                        schedule: "0 0 * * * *".to_string(),
                        retry_strategy: Some(SchedulerJobRetryStrategy::Constant {
                            interval: Duration::from_secs(120),
                            max_attempts: 5,
                        }),
                        notifications: false,
                    })),
                },
            )
            .await?;

        assert_eq!(
            trackers.get_tracker(tracker.id).await?.unwrap().job_id,
            Some(uuid!("00000000-0000-0000-0000-000000000001")),
        );

        // Disable revisions.
        trackers
            .update_tracker(
                tracker.id,
                TrackerUpdateParams {
                    settings: Some(TrackerSettings {
                        revisions: 0,
                        ..tracker.settings.clone()
                    }),
                    ..Default::default()
                },
            )
            .await?;

        assert_eq!(
            trackers.get_tracker(tracker.id).await?.unwrap().job_id,
            None,
        );

        Ok(())
    }

    #[sqlx::test]
    async fn can_manipulate_tracker_jobs(pool: PgPool) -> anyhow::Result<()> {
        let api = mock_api(pool).await?;

        let trackers = api.trackers();

        let unscheduled_trackers = api.trackers().get_unscheduled_trackers().await?;
        assert!(unscheduled_trackers.is_empty());
        let unscheduled_trackers = api.trackers().get_unscheduled_trackers().await?;
        assert!(unscheduled_trackers.is_empty());

        let tracker = trackers
            .create_tracker(TrackerCreateParams {
                name: "name_one".to_string(),
                url: Url::parse("https://retrack.dev")?,
                settings: TrackerSettings {
                    revisions: 3,
                    delay: Duration::from_millis(2000),
                    extractor: Default::default(),
                    headers: Default::default(),
                },
                job_config: Some(SchedulerJobConfig {
                    schedule: "0 0 * * * *".to_string(),
                    retry_strategy: None,
                    notifications: true,
                }),
            })
            .await?;

        let unscheduled_trackers = api.trackers().get_unscheduled_trackers().await?;
        assert_eq!(unscheduled_trackers, vec![tracker.clone()]);

        api.trackers()
            .update_tracker_job(
                tracker.id,
                Some(uuid!("11e55044-10b1-426f-9247-bb680e5fe0c9")),
            )
            .await?;

        let unscheduled_trackers = api.trackers().get_unscheduled_trackers().await?;
        assert!(unscheduled_trackers.is_empty());

        let scheduled_tracker = api
            .trackers()
            .get_tracker_by_job_id(uuid!("11e55044-10b1-426f-9247-bb680e5fe0c9"))
            .await?;
        assert_eq!(
            scheduled_tracker,
            Some(Tracker {
                job_id: Some(uuid!("11e55044-10b1-426f-9247-bb680e5fe0c9")),
                ..tracker.clone()
            })
        );

        // Remove schedule to make sure that job is removed.
        trackers
            .update_tracker(
                tracker.id,
                TrackerUpdateParams {
                    job_config: Some(None),
                    ..Default::default()
                },
            )
            .await?;

        let unscheduled_trackers = api.trackers().get_unscheduled_trackers().await?;
        assert!(unscheduled_trackers.is_empty());
        let unscheduled_trackers = api.trackers().get_unscheduled_trackers().await?;
        assert!(unscheduled_trackers.is_empty());

        let scheduled_tracker = api
            .trackers()
            .get_tracker_by_job_id(uuid!("11e55044-10b1-426f-9247-bb680e5fe0c8"))
            .await?;
        assert!(scheduled_tracker.is_none());

        let scheduled_tracker = api
            .trackers()
            .get_tracker_by_job_id(uuid!("11e55044-10b1-426f-9247-bb680e5fe0c9"))
            .await?;
        assert!(scheduled_tracker.is_none());

        Ok(())
    }

    #[sqlx::test]
    async fn can_return_pending_tracker_jobs(pool: PgPool) -> anyhow::Result<()> {
        let api = mock_api(pool).await?;

        let trackers = api.trackers();

        let pending_trackers = api
            .trackers()
            .get_pending_trackers()
            .collect::<Vec<_>>()
            .await;
        assert!(pending_trackers.is_empty());

        for n in 0..=2 {
            let job = RawSchedulerJobStoredData {
                last_updated: Some(946720800 + n),
                last_tick: Some(946720700),
                next_tick: Some(946720900),
                ran: Some(true),
                stopped: Some(n != 1),
                ..mock_scheduler_job(
                    Uuid::parse_str(&format!("67e55044-10b1-426f-9247-bb680e5fe0c{}", n))?,
                    SchedulerJob::TrackersTrigger,
                    format!("{} 0 0 1 1 * *", n),
                )
            };

            mock_upsert_scheduler_job(&api.db, &job).await?;
        }

        for n in 0..=2 {
            trackers
                .create_tracker(TrackerCreateParams {
                    name: format!("name_{}", n),
                    url: Url::parse("https://retrack.dev")?,
                    settings: TrackerSettings {
                        revisions: 3,
                        delay: Duration::from_millis(2000),
                        extractor: Default::default(),
                        headers: Default::default(),
                    },
                    job_config: Some(SchedulerJobConfig {
                        schedule: "0 0 * * * *".to_string(),
                        retry_strategy: None,
                        notifications: true,
                    }),
                })
                .await?;
        }

        let pending_trackers = api
            .trackers()
            .get_pending_trackers()
            .collect::<Vec<_>>()
            .await;
        assert!(pending_trackers.is_empty());

        // Assign job IDs to trackers.
        let all_trackers = trackers.get_trackers().await?;
        for (n, tracker) in all_trackers.iter().enumerate() {
            api.trackers()
                .update_tracker_job(
                    tracker.id,
                    Some(Uuid::parse_str(&format!(
                        "67e55044-10b1-426f-9247-bb680e5fe0c{}",
                        n
                    ))?),
                )
                .await?;
        }

        let pending_trackers = api
            .trackers()
            .get_pending_trackers()
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<anyhow::Result<Vec<_>, _>>()?;
        assert_eq!(pending_trackers.len(), 2);

        let all_trackers = trackers.get_trackers().await?;
        assert_eq!(
            vec![all_trackers[0].clone(), all_trackers[2].clone()],
            pending_trackers,
        );

        let all_trackers = trackers.get_trackers().await?;
        assert_eq!(all_trackers.len(), 3);

        Ok(())
    }
}
