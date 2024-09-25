mod tracker_create_params;
mod tracker_list_revisions_params;
mod tracker_update_params;
mod trackers_list_params;

pub use self::{
    tracker_create_params::TrackerCreateParams,
    tracker_list_revisions_params::TrackerListRevisionsParams,
    tracker_update_params::TrackerUpdateParams, trackers_list_params::TrackersListParams,
};
use crate::{
    api::Api,
    config::TrackersConfig,
    error::Error as RetrackError,
    network::{DnsResolver, EmailTransport, EmailTransportError},
    scheduler::{CronExt, SchedulerJobRetryStrategy},
    tasks::{EmailContent, EmailTaskType, EmailTemplate, HttpTaskType, TaskType},
    trackers::{
        database_ext::TrackersDatabaseExt,
        tracker_data_revisions_diff::tracker_data_revisions_diff,
        web_scraper::{WebScraperContentRequest, WebScraperErrorResponse},
        ApiTarget, PageTarget, Tracker, TrackerAction, TrackerDataRevision, TrackerDataValue,
        TrackerTarget, WebhookAction,
    },
};
use anyhow::{anyhow, bail};
use croner::Cron;
use futures::Stream;
use http::Method;
use lettre::message::Mailbox;
use reqwest_middleware::ClientBuilder;
use reqwest_tracing::{SpanBackendWithUrl, TracingMiddleware};
use std::{collections::HashSet, str::FromStr, time::Duration};
use time::OffsetDateTime;
use tracing::{debug, info};
use uuid::Uuid;

/// Defines a maximum number of jobs that can be retrieved from the database at once.
const MAX_JOBS_PAGE_SIZE: usize = 1000;

/// Defines the maximum length of the extractor script.
const MAX_TRACKER_PAGE_EXTRACTOR_SCRIPT_LENGTH: usize = 2048;

/// Defines the maximum length of the user agent string.
const MAX_TRACKER_PAGE_USER_AGENT_LENGTH: usize = 200;

/// We currently wait up to 60 seconds for extractor script to execute.
const MAX_TRACKER_EXTRACTOR_TIMEOUT: Duration = Duration::from_secs(60);

/// We currently support up to 10 retry attempts for the tracker.
const MAX_TRACKER_RETRY_ATTEMPTS: u32 = 10;

/// We currently support minimum 60 seconds between retry attempts for the tracker.
const MIN_TRACKER_RETRY_INTERVAL: Duration = Duration::from_secs(60);

/// We currently support maximum 12 hours between retry attempts for the tracker.
const MAX_TRACKER_RETRY_INTERVAL: Duration = Duration::from_secs(12 * 3600);

/// Defines the maximum length of a tracker name.
pub const MAX_TRACKER_NAME_LENGTH: usize = 100;

/// Defines the maximum length of a tracker tag.
pub const MAX_TRACKER_TAG_LENGTH: usize = 50;

/// Defines the maximum count of tracker tags.
pub const MAX_TRACKER_TAGS_COUNT: usize = 10;

/// Defines the maximum count of tracker actions.
pub const MAX_TRACKER_ACTIONS_COUNT: usize = 10;

/// Defines the maximum count of tracker email action recipients.
pub const MAX_TRACKER_EMAIL_ACTION_RECIPIENTS_COUNT: usize = 10;

/// Defines the maximum count of tracker webhook action headers.
pub const MAX_TRACKER_WEBHOOK_ACTION_HEADERS_COUNT: usize = 20;

pub struct TrackersApiExt<'a, DR: DnsResolver, ET: EmailTransport>
where
    ET::Error: EmailTransportError,
{
    api: &'a Api<DR, ET>,
    trackers: TrackersDatabaseExt<'a>,
}

impl<'a, DR: DnsResolver, ET: EmailTransport> TrackersApiExt<'a, DR, ET>
where
    ET::Error: EmailTransportError,
{
    /// Creates Trackers API.
    pub fn new(api: &'a Api<DR, ET>) -> Self {
        Self {
            api,
            trackers: api.db.trackers(),
        }
    }

    /// Returns all trackers.
    pub async fn get_trackers(&self, params: TrackersListParams) -> anyhow::Result<Vec<Tracker>> {
        let normalized_tags = Self::normalize_tracker_tags(params.tags);
        if normalized_tags.len() > MAX_TRACKER_TAGS_COUNT {
            bail!(RetrackError::client(format!(
                "Trackers filter params cannot use more than {MAX_TRACKER_TAGS_COUNT} tags."
            )));
        }
        Self::validate_tracker_tags(&normalized_tags)?;

        self.trackers.get_trackers(&normalized_tags).await
    }

    /// Returns tracker by its ID.
    pub async fn get_tracker(&self, id: Uuid) -> anyhow::Result<Option<Tracker>> {
        self.trackers.get_tracker(id).await
    }

    /// Creates a new web page content tracker.
    pub async fn create_tracker(&self, params: TrackerCreateParams) -> anyhow::Result<Tracker> {
        let created_at = OffsetDateTime::now_utc();
        let tracker = Tracker {
            id: Uuid::now_v7(),
            name: params.name,
            enabled: params.enabled,
            target: params.target,
            config: params.config,
            tags: Self::normalize_tracker_tags(params.tags),
            actions: params.actions,
            job_id: None,
            created_at,
            updated_at: created_at,
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
            && params.enabled.is_none()
            && params.target.is_none()
            && params.config.is_none()
            && params.tags.is_none()
            && params.actions.is_none()
        {
            bail!(RetrackError::client(format!(
                "At least one tracker property should be provided ({id})."
            )));
        }

        let Some(existing_tracker) = self.trackers.get_tracker(id).await? else {
            bail!(RetrackError::client(format!(
                "Tracker ('{id}') is not found."
            )));
        };

        let disabled_revisions = params
            .config
            .as_ref()
            .map(|config| config.revisions == 0)
            .unwrap_or_default();

        let changed_schedule = if let Some(config) = params.config.as_ref() {
            match (&existing_tracker.config.job, config.job.as_ref()) {
                (Some(existing_job_config), Some(job_config)) => {
                    job_config.schedule != existing_job_config.schedule
                }
                _ => true,
            }
        } else {
            false
        };

        let enabled = params.enabled.unwrap_or(existing_tracker.enabled);
        let job_id = if !enabled || disabled_revisions || changed_schedule {
            None
        } else {
            existing_tracker.job_id
        };

        let tracker = Tracker {
            name: params.name.unwrap_or(existing_tracker.name),
            enabled,
            target: params.target.unwrap_or(existing_tracker.target),
            config: params.config.unwrap_or(existing_tracker.config),
            tags: params
                .tags
                .map(Self::normalize_tracker_tags)
                .unwrap_or(existing_tracker.tags),
            actions: params.actions.unwrap_or(existing_tracker.actions),
            updated_at: OffsetDateTime::now_utc(),
            job_id,
            ..existing_tracker
        };

        self.validate_tracker(&tracker).await?;

        self.trackers.update_tracker(&tracker).await?;

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
            tracker.config.revisions,
            self.api.config.trackers.max_revisions,
        );
        if max_revisions == 0 {
            return Ok(None);
        }

        let revisions = self.trackers.get_tracker_data(tracker.id).await?;
        let mut new_revision = match tracker.target {
            TrackerTarget::Page(_) => {
                self.fetch_tracker_page_data_revision(&tracker, &revisions)
                    .await?
            }
            TrackerTarget::Api(_) => {
                self.fetch_tracker_api_data_revision(&tracker, &revisions)
                    .await?
            }
        };

        // If the last revision has the same original data value, drop newly fetched revision.
        let last_revision = revisions.last();
        if let Some(last_revision) = last_revision {
            if last_revision.data.original() == new_revision.data.original() {
                return Ok(None);
            }
        };

        // Iterate through all tracker actions and execute them.
        let previous_data_value = last_revision.as_ref().map(|r| &r.data);
        for action in tracker.actions.iter() {
            self.execute_tracker_action(
                &tracker,
                action,
                &mut new_revision.data,
                previous_data_value,
            )
            .await?
        }

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
    pub async fn get_trackers_to_schedule(&self) -> anyhow::Result<Vec<Tracker>> {
        self.trackers.get_trackers_to_schedule().await
    }

    /// Returns all trackers that have pending jobs.
    pub fn get_trackers_to_run(&self) -> impl Stream<Item = anyhow::Result<Tracker>> + '_ {
        self.trackers.get_trackers_to_run(MAX_JOBS_PAGE_SIZE)
    }

    /// Returns tracker by the corresponding job ID.
    pub async fn get_tracker_by_job_id(&self, job_id: Uuid) -> anyhow::Result<Option<Tracker>> {
        self.trackers.get_tracker_by_job_id(job_id).await
    }

    /// Updates tracker job ID reference (link or unlink).
    pub async fn update_tracker_job(&self, id: Uuid, job_id: Option<Uuid>) -> anyhow::Result<()> {
        self.trackers.update_tracker_job(id, job_id).await
    }

    /// Executes tracker action.
    async fn execute_tracker_action(
        &self,
        tracker: &Tracker,
        action: &TrackerAction,
        latest_data_value: &mut TrackerDataValue,
        previous_data_value: Option<&TrackerDataValue>,
    ) -> anyhow::Result<()> {
        // If the latest data value has no modifications, use previous original value as
        // previous value. Otherwise, use the modification from the previous data value based on
        // the highest index of the latest data value modifications.
        let previous_value = previous_data_value.and_then(|previous_data_value| {
            if latest_data_value.mods().is_none() {
                Some(previous_data_value.original())
            } else {
                previous_data_value
                    .mods()?
                    .get(latest_data_value.mods()?.len() - 1)
            }
        });

        let latest_value = latest_data_value.value();
        let changed = if let Some(previous_value) = previous_value {
            previous_value != latest_value
        } else {
            true
        };

        let tasks_api = self.api.tasks();
        match action {
            TrackerAction::Email(action) if changed => {
                let task = tasks_api
                    .schedule_task(
                        TaskType::Email(EmailTaskType {
                            to: action.to.clone(),
                            content: EmailContent::Template(EmailTemplate::TrackerChanges {
                                tracker_name: tracker.name.clone(),
                                content: Ok(latest_value.to_string()),
                            }),
                        }),
                        OffsetDateTime::now_utc(),
                    )
                    .await?;
                info!(
                    tracker.id = %tracker.id,
                    tracker.name = tracker.name,
                    task.id = %task.id,
                    "Scheduled email task."
                );
            }
            TrackerAction::Webhook(action) if changed => {
                let task = tasks_api
                    .schedule_task(
                        TaskType::Http(HttpTaskType {
                            url: action.url.clone(),
                            method: action.method.clone().unwrap_or(Method::POST),
                            headers: action.headers.clone(),
                            body: Some(serde_json::to_vec(&latest_value)?),
                        }),
                        OffsetDateTime::now_utc(),
                    )
                    .await?;
                info!(
                    tracker.id = %tracker.id,
                    tracker.name = tracker.name,
                    task.id = %task.id,
                    "Scheduled HTTP task."
                );
            }
            TrackerAction::ServerLog => {
                info!(
                    tracker.id = %tracker.id,
                    tracker.name = tracker.name,
                    "Fetched new data revision (data changed: {changed}): {:?}",
                    latest_value
                );
            }
            _ => {
                debug!(
                    tracker.id = %tracker.id,
                    tracker.name = tracker.name,
                    "Skipping action `{action:?}` for a new data revision (data changed: {changed}): {:?}",
                    latest_value
                );
            }
        };

        Ok(())
    }

    /// Normalizes tracker tags (trim, deduplicate, and lowercase).
    fn normalize_tracker_tags(tags: Vec<String>) -> Vec<String> {
        tags.into_iter()
            .map(|tag| tag.trim().to_lowercase())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect()
    }

    /// Validates tracker parameters.
    async fn validate_tracker(&self, tracker: &Tracker) -> anyhow::Result<()> {
        if tracker.name.is_empty() {
            bail!(RetrackError::client("Tracker name cannot be empty.",));
        }

        if tracker.name.len() > MAX_TRACKER_NAME_LENGTH {
            bail!(RetrackError::client(format!(
                "Tracker name cannot be longer than {MAX_TRACKER_NAME_LENGTH} characters."
            )));
        }

        if tracker.tags.len() > MAX_TRACKER_TAGS_COUNT {
            bail!(RetrackError::client(format!(
                "Tracker cannot have more than {MAX_TRACKER_TAGS_COUNT} tags."
            )));
        }

        if tracker.actions.len() > MAX_TRACKER_ACTIONS_COUNT {
            bail!(RetrackError::client(format!(
                "Tracker cannot have more than {MAX_TRACKER_ACTIONS_COUNT} actions."
            )));
        }

        Self::validate_tracker_actions(&tracker.actions)?;
        Self::validate_tracker_tags(&tracker.tags)?;

        let config = &self.api.config.trackers;
        if tracker.config.revisions > config.max_revisions {
            bail!(RetrackError::client(format!(
                "Tracker revisions count cannot be greater than {}.",
                config.max_revisions
            )));
        }

        match tracker.target {
            TrackerTarget::Page(ref target) => {
                Self::validate_page_target(target)?;
            }
            TrackerTarget::Api(ref target) => {
                self.validate_api_target(config, target).await?;
            }
        }

        if let Some(ref timeout) = tracker.config.timeout {
            if timeout > &MAX_TRACKER_EXTRACTOR_TIMEOUT {
                bail!(RetrackError::client(format!(
                    "Tracker timeout cannot be greater than {}ms.",
                    MAX_TRACKER_EXTRACTOR_TIMEOUT.as_millis()
                )));
            }
        }

        if let Some(job_config) = &tracker.config.job {
            // Validate that the schedule is a valid cron expression.
            let schedule = match Cron::parse_pattern(job_config.schedule.as_str()) {
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

        Ok(())
    }

    /// Validates tracker tags.
    fn validate_tracker_tags(tags: &[String]) -> anyhow::Result<()> {
        if tags
            .iter()
            .any(|tag| tag.is_empty() || tag.len() > MAX_TRACKER_TAG_LENGTH)
        {
            bail!(RetrackError::client(format!(
                "Tracker tags cannot be empty or longer than {MAX_TRACKER_TAG_LENGTH} characters."
            )));
        }

        Ok(())
    }

    /// Validates tracker actions.
    fn validate_tracker_actions(actions: &[TrackerAction]) -> anyhow::Result<()> {
        for action in actions {
            match action {
                TrackerAction::Email(action) => {
                    if action.to.is_empty() {
                        bail!(RetrackError::client(
                            "Tracker email action must have at least one recipient."
                        ));
                    }

                    if action.to.len() > MAX_TRACKER_EMAIL_ACTION_RECIPIENTS_COUNT {
                        bail!(RetrackError::client(format!(
                            "Tracker email action cannot have more than {MAX_TRACKER_EMAIL_ACTION_RECIPIENTS_COUNT} recipients."
                        )));
                    }

                    for recipient in &action.to {
                        if Mailbox::from_str(recipient).is_err() {
                            bail!(RetrackError::client(format!(
                                "Tracker email action recipient ('{}') is not a valid email address.",
                                recipient
                            )));
                        }
                    }
                }
                TrackerAction::Webhook(WebhookAction {
                    method, headers, ..
                }) => {
                    if let Some(method) = method {
                        if method != Method::GET && method != Method::POST && method != Method::PUT
                        {
                            bail!(RetrackError::client(
                                "Tracker webhook action method must be either `GET`, `POST`, or `PUT`."
                            ));
                        }
                    }

                    if let Some(headers) = headers {
                        if headers.len() > MAX_TRACKER_WEBHOOK_ACTION_HEADERS_COUNT {
                            bail!(RetrackError::client(format!(
                                "Tracker webhook action cannot have more than {MAX_TRACKER_WEBHOOK_ACTION_HEADERS_COUNT} headers."
                            )));
                        }
                    }
                }
                _ => {}
            }
        }

        Ok(())
    }

    /// Validates tracker's web page target parameters.
    fn validate_page_target(target: &PageTarget) -> anyhow::Result<()> {
        if target.extractor.is_empty() {
            bail!(RetrackError::client(
                "Tracker web page extractor script cannot be empty."
            ));
        }

        if target.extractor.len() > MAX_TRACKER_PAGE_EXTRACTOR_SCRIPT_LENGTH {
            bail!(RetrackError::client(format!(
                "Tracker web page extractor script cannot be longer than {MAX_TRACKER_PAGE_EXTRACTOR_SCRIPT_LENGTH} characters."
            )));
        }

        if let Some(ref user_agent) = target.user_agent {
            if user_agent.is_empty() {
                bail!(RetrackError::client(
                    "Tracker web page user-agent header cannot be empty.",
                ));
            }

            if user_agent.len() > MAX_TRACKER_PAGE_USER_AGENT_LENGTH {
                bail!(RetrackError::client(format!(
                    "Tracker web page user-agent cannot be longer than {MAX_TRACKER_PAGE_USER_AGENT_LENGTH} characters."
                )));
            }
        }

        Ok(())
    }

    /// Validates tracker's JSON api target parameters.
    async fn validate_api_target(
        &self,
        config: &TrackersConfig,
        target: &ApiTarget,
    ) -> anyhow::Result<()> {
        if config.restrict_to_public_urls && !self.api.network.is_public_web_url(&target.url).await
        {
            bail!(RetrackError::client(
                format!("Tracker JSON API target URL must be either `http` or `https` and have a valid public reachable domain name, but received {}.", target.url)
            ));
        }

        Ok(())
    }

    /// Fetches data revision for a tracker with `Page` target
    async fn fetch_tracker_page_data_revision(
        &self,
        tracker: &Tracker,
        revisions: &[TrackerDataRevision],
    ) -> anyhow::Result<TrackerDataRevision> {
        let scraper_request = WebScraperContentRequest::try_from(tracker)?;
        let scraper_request = if let Some(revision) = revisions.last() {
            scraper_request.set_previous_content(revision.data.original())
        } else {
            scraper_request
        };

        let client = ClientBuilder::new(reqwest::Client::new())
            .with(TracingMiddleware::<SpanBackendWithUrl>::new())
            .build();
        let scraper_response = client
            .post(format!(
                "{}api/web_page/execute",
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
                    "Unexpected scraper error for the tracker ('{}'): {:?}",
                    tracker.id,
                    scraper_error_response.message
                );
            }
        }

        Ok(TrackerDataRevision {
            id: Uuid::now_v7(),
            tracker_id: tracker.id,
            data: TrackerDataValue::new(scraper_response.json().await.map_err(|err| {
                anyhow!(
                    "Could not deserialize scraper response for the tracker ('{}'): {err:?}",
                    tracker.id
                )
            })?),
            created_at: OffsetDateTime::now_utc(),
        })
    }

    /// Fetches data revision for a tracker with `Api` target
    async fn fetch_tracker_api_data_revision(
        &self,
        tracker: &Tracker,
        _revisions: &[TrackerDataRevision],
    ) -> anyhow::Result<TrackerDataRevision> {
        let TrackerTarget::Api(_) = tracker.target else {
            bail!(RetrackError::client(format!(
                "Tracker ('{}') target is not an API.",
                tracker.id
            )));
        };

        unimplemented!("Api target is not implemented yet.");
    }
}

impl<'a, DR: DnsResolver, ET: EmailTransport> Api<DR, ET>
where
    ET::Error: EmailTransportError,
{
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
        tasks::{EmailContent, EmailTaskType, EmailTemplate, HttpTaskType, TaskType},
        tests::{
            mock_api, mock_api_with_config, mock_api_with_network, mock_config,
            mock_network_with_records, mock_scheduler_job, mock_upsert_scheduler_job,
            RawSchedulerJobStoredData, WebScraperContentRequest, WebScraperErrorResponse,
        },
        trackers::{
            ApiTarget, EmailAction, PageTarget, Tracker, TrackerAction, TrackerConfig,
            TrackerCreateParams, TrackerDataValue, TrackerListRevisionsParams, TrackerTarget,
            TrackerUpdateParams, TrackersListParams, WebhookAction,
        },
    };
    use actix_web::ResponseError;
    use futures::StreamExt;
    use http::{header::CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue, Method};
    use httpmock::MockServer;
    use insta::assert_debug_snapshot;
    use serde_json::json;
    use sqlx::PgPool;
    use std::{collections::HashMap, net::Ipv4Addr, str::FromStr, time::Duration};
    use time::OffsetDateTime;
    use trust_dns_resolver::{
        proto::rr::{rdata::A, RData, Record},
        Name,
    };
    use url::Url;
    use uuid::{uuid, Uuid};

    #[sqlx::test]
    async fn properly_creates_new_tracker(pool: PgPool) -> anyhow::Result<()> {
        let api = mock_api(pool).await?;
        let api = api.trackers();

        let tracker = api
            .create_tracker(
                TrackerCreateParams::new("name_one")
                    .with_config(TrackerConfig {
                        revisions: 3,
                        timeout: Some(Duration::from_millis(2500)),
                        headers: Default::default(),
                        job: Some(SchedulerJobConfig {
                            schedule: "@hourly".to_string(),
                            retry_strategy: Some(SchedulerJobRetryStrategy::Constant {
                                interval: Duration::from_secs(120),
                                max_attempts: 5,
                            }),
                        }),
                    })
                    .with_tags(vec![
                        "tag".to_string(),
                        "TAG".to_string(),
                        " tag".to_string(),
                    ]),
            )
            .await?;

        assert_eq!(tracker, api.get_tracker(tracker.id).await?.unwrap());
        assert_eq!(tracker.tags, vec!["tag".to_string()]);

        let tracker = api
            .create_tracker(
                TrackerCreateParams::new("name_two").with_target(TrackerTarget::Api(ApiTarget {
                    url: Url::parse("https://retrack.dev")?,
                    method: Some(Method::POST),
                    headers: Some(
                        (&[(CONTENT_TYPE, "application/json".to_string())]
                            .into_iter()
                            .collect::<HashMap<_, _>>())
                            .try_into()?,
                    ),
                    media_type: Some("application/json".parse()?),
                })),
            )
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

        let target = TrackerTarget::Page(PageTarget {
            extractor: "export async function execute(p) { await p.goto('https://retrack.dev/'); return await p.content(); }".to_string(),
            user_agent: Some("Retrack/1.0.0".to_string()),
            ignore_https_errors: true,
        });
        let config = TrackerConfig {
            revisions: 3,
            timeout: Some(Duration::from_millis(2500)),
            headers: Default::default(),
            job: None,
        };
        let tags = vec!["tag".to_string()];
        let actions = vec![TrackerAction::ServerLog];

        let create_and_fail = |result: anyhow::Result<_>| -> RetrackError {
            result.unwrap_err().downcast::<RetrackError>().unwrap()
        };

        // Empty name.
        assert_debug_snapshot!(
            create_and_fail(api.create_tracker(TrackerCreateParams {
                name: "".to_string(),
                enabled: true,
                target: target.clone(),
                config: config.clone(),
                tags: tags.clone(),
                actions: actions.clone()
            }).await),
            @r###""Tracker name cannot be empty.""###
        );

        // Very long name.
        assert_debug_snapshot!(
            create_and_fail(api.create_tracker(TrackerCreateParams {
                name: "a".repeat(101),
                enabled: true,
                target: target.clone(),
                config: config.clone(),
                tags: tags.clone(),
                actions: actions.clone()
            }).await),
            @r###""Tracker name cannot be longer than 100 characters.""###
        );

        // Too many revisions.
        assert_debug_snapshot!(
            create_and_fail(api.create_tracker(TrackerCreateParams {
                name: "name".to_string(),
                enabled: false,
                target: target.clone(),
                config: TrackerConfig {
                    revisions: 31,
                    ..config.clone()
                },
                tags: tags.clone(),
                actions: actions.clone()
            }).await),
            @r###""Tracker revisions count cannot be greater than 30.""###
        );

        // Very long tag.
        assert_debug_snapshot!(
            create_and_fail(api.create_tracker(TrackerCreateParams {
                name: "name".to_string(),
                enabled: true,
                target: target.clone(),
                config: config.clone(),
                tags: vec!["a".repeat(51)],
                actions: actions.clone()
            }).await),
            @r###""Tracker tags cannot be empty or longer than 50 characters.""###
        );

        // Empty tag.
        assert_debug_snapshot!(
            create_and_fail(api.create_tracker(TrackerCreateParams {
                name: "name".to_string(),
                enabled: false,
                target: target.clone(),
                config: config.clone(),
                tags: vec!["tag".to_string(), "".to_string()],
                actions: actions.clone()
            }).await),
            @r###""Tracker tags cannot be empty or longer than 50 characters.""###
        );

        // Too many tags.
        assert_debug_snapshot!(
            create_and_fail(api.create_tracker(TrackerCreateParams {
                name: "name".to_string(),
                enabled: true,
                target: target.clone(),
                config: config.clone(),
                tags: (0..11).map(|i| i.to_string()).collect(),
                actions: actions.clone()
            }).await),
            @r###""Tracker cannot have more than 10 tags.""###
        );

        // Too many actions.
        assert_debug_snapshot!(
            create_and_fail(api.create_tracker(TrackerCreateParams {
                name: "name".to_string(),
                enabled: false,
                target: target.clone(),
                config: config.clone(),
                tags: tags.clone(),
                actions: [const { TrackerAction::ServerLog }; 11].into_iter().collect()
            }).await),
            @r###""Tracker cannot have more than 10 actions.""###
        );

        // Empty email action recipient.
        assert_debug_snapshot!(
            create_and_fail(api.create_tracker(TrackerCreateParams {
                name: "name".to_string(),
                enabled: false,
                target: target.clone(),
                config: config.clone(),
                tags: tags.clone(),
                actions: vec![TrackerAction::Email(EmailAction {
                    to: vec!["".to_string()]
                })],
            }).await),
            @r###""Tracker email action recipient ('') is not a valid email address.""###
        );

        // Invalid email action recipient.
        assert_debug_snapshot!(
            create_and_fail(api.create_tracker(TrackerCreateParams {
                name: "name".to_string(),
                enabled: false,
                target: target.clone(),
                config: config.clone(),
                tags: tags.clone(),
                actions: vec![TrackerAction::Email(EmailAction {
                    to: vec!["alpha-beta-gamma".to_string()]
                })],
            }).await),
            @r###""Tracker email action recipient ('alpha-beta-gamma') is not a valid email address.""###
        );

        // Too many email action recipients.
        assert_debug_snapshot!(
            create_and_fail(api.create_tracker(TrackerCreateParams {
                name: "name".to_string(),
                enabled: false,
                target: target.clone(),
                config: config.clone(),
                tags: tags.clone(),
                actions: vec![TrackerAction::Email(EmailAction {
                    to: vec!["dev@retrack.dev".to_string(); 11]
                })],
            }).await),
            @r###""Tracker email action cannot have more than 10 recipients.""###
        );

        // Invalid webhook action method.
        assert_debug_snapshot!(
            create_and_fail(api.create_tracker(TrackerCreateParams {
                name: "name".to_string(),
                enabled: false,
                target: target.clone(),
                config: config.clone(),
                tags: tags.clone(),
                actions: vec![TrackerAction::Webhook(WebhookAction {
                    url: "https://retrack.dev".parse()?,
                    method: Some(Method::PATCH),
                    headers: None
                })],
            }).await),
            @r###""Tracker webhook action method must be either `GET`, `POST`, or `PUT`.""###
        );

        // Too many webhook action headers.
        let headers: [(HeaderName, String); 21] = core::array::from_fn(|i| {
            (
                HeaderName::from_str(&format!("x-header-{i}")).unwrap(),
                format!("application/{i}"),
            )
        });
        assert_debug_snapshot!(
            create_and_fail(api.create_tracker(TrackerCreateParams {
                name: "name".to_string(),
                enabled: false,
                target: target.clone(),
                config: config.clone(),
                tags: tags.clone(),
                actions: vec![TrackerAction::Webhook(WebhookAction {
                    url: "https://retrack.dev".parse()?,
                    method: None,
                    headers: Some((&headers.into_iter().collect::<HashMap<_, _>>()).try_into()?),
                })],
            }).await),
            @r###""Tracker webhook action cannot have more than 20 headers.""###
        );

        // Too long timeout.
        assert_debug_snapshot!(
            create_and_fail(api.create_tracker(TrackerCreateParams {
                name: "name".to_string(),
                enabled: true,
                target: target.clone(),
                config: TrackerConfig {
                    timeout: Some(Duration::from_secs(61)),
                    ..config.clone()
                },
                tags: tags.clone(),
                actions: actions.clone()
            }).await),
            @r###""Tracker timeout cannot be greater than 60000ms.""###
        );

        // Empty web page target extractor.
        assert_debug_snapshot!(
            create_and_fail(api.create_tracker(TrackerCreateParams {
                name: "name".to_string(),
                enabled: false,
                target: TrackerTarget::Page(PageTarget {
                   extractor: "".to_string(),
                    ..Default::default()
                }),
                config: config.clone(),
                tags: tags.clone(),
                actions: actions.clone()
            }).await),
            @r###""Tracker web page extractor script cannot be empty.""###
        );

        // Very long web page target extractor.
        assert_debug_snapshot!(
            create_and_fail(api.create_tracker(TrackerCreateParams {
                name: "name".to_string(),
                enabled: true,
                target: TrackerTarget::Page(PageTarget {
                    extractor: "a".repeat(2049),
                    ..Default::default()
                }),
                config: config.clone(),
                tags: tags.clone(),
                actions: actions.clone()
            }).await),
            @r###""Tracker web page extractor script cannot be longer than 2048 characters.""###
        );

        // Empty web page target user agent.
        assert_debug_snapshot!(
            create_and_fail(api.create_tracker(TrackerCreateParams {
                name: "name".to_string(),
                enabled: false,
                target: TrackerTarget::Page(PageTarget {
                   user_agent: Some("".to_string()),
                    ..Default::default()
                }),
                config: config.clone(),
                tags: tags.clone(),
                actions: actions.clone()
            }).await),
            @r###""Tracker web page extractor script cannot be empty.""###
        );

        // Very long web page target user agent.
        assert_debug_snapshot!(
            create_and_fail(api.create_tracker(TrackerCreateParams {
                name: "name".to_string(),
                enabled: true,
                target: TrackerTarget::Page(PageTarget {
                    user_agent: Some("a".repeat(201)),
                    ..Default::default()
                }),
                config: config.clone(),
                tags: tags.clone(),
                actions: actions.clone()
            }).await),
            @r###""Tracker web page extractor script cannot be empty.""###
        );

        // Invalid schedule.
        assert_debug_snapshot!(
            create_and_fail(api.create_tracker(TrackerCreateParams {
                name: "name".to_string(),
                enabled: false,
                target: target.clone(),
                config: TrackerConfig {
                    job: Some(SchedulerJobConfig {
                        schedule: "-".to_string(),
                        retry_strategy: None
                    }),
                    ..config.clone()
                },
                tags: tags.clone(),
                actions: actions.clone()
            }).await),
            @r###"
        Error {
            context: "Tracker schedule must be a valid cron expression.",
            source: "Failed to parse schedule `-`: Invalid pattern: Pattern must consist of five or six fields (minute, hour, day, month, day of week, and optional second).",
        }
        "###
        );

        // Invalid schedule interval.
        assert_debug_snapshot!(
            create_and_fail(api.create_tracker(TrackerCreateParams {
                name: "name".to_string(),
                enabled: true,
                target: target.clone(),
                config: TrackerConfig {
                    job: Some(SchedulerJobConfig {
                        schedule: "0/5 * * * * *".to_string(),
                        retry_strategy: None
                    }),
                    ..config.clone()
                },
                tags: tags.clone(),
                actions: actions.clone()
            }).await),
            @r###""Tracker schedule must have at least 10s between occurrences, but detected 5s.""###
        );

        // Too few retry attempts.
        assert_debug_snapshot!(
            create_and_fail(api.create_tracker(TrackerCreateParams {
                name: "name".to_string(),
                enabled: false,
                target: target.clone(),
                config: TrackerConfig {
                    job: Some(SchedulerJobConfig {
                       schedule: "@daily".to_string(),
                        retry_strategy: Some(SchedulerJobRetryStrategy::Constant {
                            interval: Duration::from_secs(120),
                            max_attempts: 0,
                        })
                    }),
                    ..config.clone()
                },
                tags: tags.clone(),
                actions: actions.clone()
            }).await),
            @r###""Tracker max retry attempts cannot be zero or greater than 10, but received 0.""###
        );

        // Too many retry attempts.
        assert_debug_snapshot!(
            create_and_fail(api.create_tracker(TrackerCreateParams {
                name: "name".to_string(),
                enabled: true,
                target: target.clone(),
                config: TrackerConfig {
                    job: Some(SchedulerJobConfig {
                        schedule: "@daily".to_string(),
                        retry_strategy: Some(SchedulerJobRetryStrategy::Constant {
                            interval: Duration::from_secs(120),
                            max_attempts: 11,
                        })
                    }),
                    ..config.clone()
                },
                tags: tags.clone(),
                actions: actions.clone()
            }).await),
            @r###""Tracker max retry attempts cannot be zero or greater than 10, but received 11.""###
        );

        // Too low retry interval.
        assert_debug_snapshot!(
            create_and_fail(api.create_tracker(TrackerCreateParams {
                name: "name".to_string(),
                enabled: true,
                target: target.clone(),
                config: TrackerConfig {
                    job: Some(SchedulerJobConfig {
                        schedule: "@daily".to_string(),
                        retry_strategy: Some(SchedulerJobRetryStrategy::Constant {
                            interval: Duration::from_secs(30),
                            max_attempts: 5,
                        })
                    }),
                    ..config.clone()
                },
                tags: tags.clone(),
                actions: actions.clone()
            }).await),
            @r###""Tracker min retry interval cannot be less than 1m, but received 30s.""###
        );

        // Too low max retry interval.
        assert_debug_snapshot!(
            create_and_fail(api.create_tracker(TrackerCreateParams {
                name: "name".to_string(),
                enabled: true,
                target: target.clone(),
                config: TrackerConfig {
                    job: Some(SchedulerJobConfig {
                        schedule: "@daily".to_string(),
                        retry_strategy: Some(SchedulerJobRetryStrategy::Linear {
                            initial_interval: Duration::from_secs(120),
                            increment: Duration::from_secs(10),
                            max_interval: Duration::from_secs(30),
                            max_attempts: 5,
                        })
                    }),
                    ..config.clone()
                },
                tags: tags.clone(),
                actions: actions.clone()
            }).await),
            @r###""Tracker retry strategy max interval cannot be less than 1m, but received 30s.""###
        );

        // Too high max retry interval.
        assert_debug_snapshot!(
            create_and_fail(api.create_tracker(TrackerCreateParams {
                name: "name".to_string(),
                enabled: true,
                target: target.clone(),
                config: TrackerConfig {
                    job: Some(SchedulerJobConfig {
                        schedule: "@monthly".to_string(),
                        retry_strategy: Some(SchedulerJobRetryStrategy::Linear {
                            initial_interval: Duration::from_secs(120),
                            increment: Duration::from_secs(10),
                            max_interval: Duration::from_secs(13 * 3600),
                            max_attempts: 5,
                        })
                    }),
                    ..config.clone()
                },
                tags: tags.clone(),
                actions: actions.clone()
            }).await),
            @r###""Tracker retry strategy max interval cannot be greater than 12h, but received 13h.""###
        );

        assert_debug_snapshot!(
            create_and_fail(api.create_tracker(TrackerCreateParams {
                name: "name".to_string(),
                enabled: true,
                target: target.clone(),
                config: TrackerConfig {
                    job: Some(SchedulerJobConfig {
                        schedule: "@hourly".to_string(),
                        retry_strategy: Some(SchedulerJobRetryStrategy::Linear {
                            initial_interval: Duration::from_secs(120),
                            increment: Duration::from_secs(10),
                            max_interval: Duration::from_secs(2 * 3600),
                            max_attempts: 5,
                        })
                    }),
                    ..config.clone()
                },
                tags: tags.clone(),
                actions: actions.clone()
            }).await),
            @r###""Tracker retry strategy max interval cannot be greater than 1h, but received 2h.""###
        );

        // Invalid JSON API target URL schema.
        assert_debug_snapshot!(
            create_and_fail(api.create_tracker(TrackerCreateParams {
                name: "name".to_string(),
                enabled: true,
                target: TrackerTarget::Api(ApiTarget {
                    url: Url::parse("ftp://retrack.dev")?,
                    method: None,
                    headers: None,
                    media_type: None,
                }),
                config: config.clone(),
                tags: tags.clone(),
                actions: actions.clone()
            }).await),
            @r###""Tracker JSON API target URL must be either `http` or `https` and have a valid public reachable domain name, but received ftp://retrack.dev/.""###
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

        // Non-public JSON API target URL.
        assert_debug_snapshot!(
            create_and_fail(api_with_local_network.trackers().create_tracker(TrackerCreateParams {
                name: "name".to_string(),
                enabled: true,
                target: TrackerTarget::Api(ApiTarget {
                    url: Url::parse("https://127.0.0.1")?,
                    method: None,
                    headers: None,
                    media_type: None,
                }),
                config: config.clone(),
                tags: tags.clone(),
                actions: actions.clone()
            }).await),
            @r###""Tracker JSON API target URL must be either `http` or `https` and have a valid public reachable domain name, but received https://127.0.0.1/.""###
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
                enabled: true,
                target: TrackerTarget::Page(PageTarget {
                    extractor: "export async function execute(p) { await p.goto('https://retrack.dev/'); return await p.content(); }".to_string(),
                    user_agent: Some("Retrack/1.0.0".to_string()),
                    ignore_https_errors: true,
                }),
                config: TrackerConfig {
                    revisions: 3,
                    timeout: Some(Duration::from_millis(2500)),
                    headers: Default::default(),
                    job: None,
                },
                tags: vec!["tag".to_string()],
                actions: vec![TrackerAction::ServerLog],
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
            updated_at: updated_tracker.updated_at,
            ..tracker.clone()
        };
        assert_eq!(expected_tracker, updated_tracker);
        assert_eq!(
            expected_tracker,
            api.get_tracker(tracker.id).await?.unwrap()
        );

        // Disable tracker
        let updated_tracker = api
            .update_tracker(
                tracker.id,
                TrackerUpdateParams {
                    enabled: Some(false),
                    ..Default::default()
                },
            )
            .await?;
        let expected_tracker = Tracker {
            name: "name_two".to_string(),
            enabled: false,
            updated_at: updated_tracker.updated_at,
            ..tracker.clone()
        };
        assert_eq!(expected_tracker, updated_tracker);
        assert_eq!(
            expected_tracker,
            api.get_tracker(tracker.id).await?.unwrap()
        );

        // Enable tracker
        let updated_tracker = api
            .update_tracker(
                tracker.id,
                TrackerUpdateParams {
                    enabled: Some(true),
                    ..Default::default()
                },
            )
            .await?;
        let expected_tracker = Tracker {
            name: "name_two".to_string(),
            enabled: true,
            updated_at: updated_tracker.updated_at,
            ..tracker.clone()
        };
        assert_eq!(expected_tracker, updated_tracker);
        assert_eq!(
            expected_tracker,
            api.get_tracker(tracker.id).await?.unwrap()
        );

        // Update config.
        let updated_tracker = api
            .update_tracker(
                tracker.id,
                TrackerUpdateParams {
                    config: Some(TrackerConfig {
                        revisions: 4,
                        ..tracker.config.clone()
                    }),
                    ..Default::default()
                },
            )
            .await?;
        let expected_tracker = Tracker {
            name: "name_two".to_string(),
            config: TrackerConfig {
                revisions: 4,
                ..tracker.config.clone()
            },
            updated_at: updated_tracker.updated_at,
            ..tracker.clone()
        };
        assert_eq!(expected_tracker, updated_tracker);
        assert_eq!(
            expected_tracker,
            api.get_tracker(tracker.id).await?.unwrap()
        );

        // Update tags.
        let updated_tracker = api
            .update_tracker(
                tracker.id,
                TrackerUpdateParams {
                    tags: Some(vec![
                        "tag_two".to_string(),
                        "TAG_TWO".to_string(),
                        " tag_two".to_string(),
                    ]),
                    ..Default::default()
                },
            )
            .await?;
        let expected_tracker = Tracker {
            name: "name_two".to_string(),
            config: TrackerConfig {
                revisions: 4,
                ..tracker.config.clone()
            },
            tags: vec!["tag_two".to_string()],
            updated_at: updated_tracker.updated_at,
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
                    config: Some(TrackerConfig {
                        revisions: 4,
                        job: Some(SchedulerJobConfig {
                            schedule: "@hourly".to_string(),
                            retry_strategy: Some(SchedulerJobRetryStrategy::Constant {
                                interval: Duration::from_secs(120),
                                max_attempts: 5,
                            }),
                        }),
                        ..tracker.config.clone()
                    }),
                    ..Default::default()
                },
            )
            .await?;
        let expected_tracker = Tracker {
            name: "name_two".to_string(),
            config: TrackerConfig {
                revisions: 4,
                job: Some(SchedulerJobConfig {
                    schedule: "@hourly".to_string(),
                    retry_strategy: Some(SchedulerJobRetryStrategy::Constant {
                        interval: Duration::from_secs(120),
                        max_attempts: 5,
                    }),
                }),
                ..tracker.config.clone()
            },
            tags: vec!["tag_two".to_string()],
            updated_at: updated_tracker.updated_at,
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
                    config: Some(TrackerConfig {
                        revisions: 4,
                        job: None,
                        ..tracker.config.clone()
                    }),
                    ..Default::default()
                },
            )
            .await?;
        let expected_tracker = Tracker {
            name: "name_two".to_string(),
            config: TrackerConfig {
                revisions: 4,
                job: None,
                ..tracker.config.clone()
            },
            tags: vec!["tag_two".to_string()],
            updated_at: updated_tracker.updated_at,
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
            .create_tracker(
                TrackerCreateParams::new("name_one").with_config(TrackerConfig {
                    timeout: Some(Duration::from_millis(2500)),
                    job: Some(SchedulerJobConfig {
                        schedule: "@hourly".to_string(),
                        retry_strategy: Some(SchedulerJobRetryStrategy::Constant {
                            interval: Duration::from_secs(120),
                            max_attempts: 5,
                        }),
                    }),
                    ..Default::default()
                }),
            )
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
                "At least one tracker property should be provided ({}).",
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
                config: Some(TrackerConfig {
                    revisions: 31,
                    ..tracker.config.clone()
                }),
                ..Default::default()
            }).await),
            @r###""Tracker revisions count cannot be greater than 30.""###
        );

        // Very long tag.
        assert_debug_snapshot!(
            update_and_fail(trackers.update_tracker(tracker.id, TrackerUpdateParams {
                tags: Some(vec!["a".repeat(51)]),
                ..Default::default()
            }).await),
            @r###""Tracker tags cannot be empty or longer than 50 characters.""###
        );

        // Empty tag.
        assert_debug_snapshot!(
            update_and_fail(trackers.update_tracker(tracker.id, TrackerUpdateParams {
                tags: Some(vec!["tag".to_string(), "".to_string()]),
                ..Default::default()
            }).await),
            @r###""Tracker tags cannot be empty or longer than 50 characters.""###
        );

        // Too many tags.
        assert_debug_snapshot!(
            update_and_fail(trackers.update_tracker(tracker.id, TrackerUpdateParams {
                tags: Some((0..11).map(|i| i.to_string()).collect()),
                ..Default::default()
            }).await),
            @r###""Tracker cannot have more than 10 tags.""###
        );

        // Too many actions.
        assert_debug_snapshot!(
            update_and_fail(trackers.update_tracker(tracker.id, TrackerUpdateParams {
                actions: Some([const { TrackerAction::ServerLog }; 11].into_iter().collect()),
                ..Default::default()
            }).await),
            @r###""Tracker cannot have more than 10 actions.""###
        );

        // Empty email action recipient.
        assert_debug_snapshot!(
            update_and_fail(trackers.update_tracker(tracker.id, TrackerUpdateParams {
                actions: Some(vec![TrackerAction::Email(EmailAction {
                    to: vec!["".to_string()]
                })]),
                ..Default::default()
            }).await),
             @r###""Tracker email action recipient ('') is not a valid email address.""###
        );

        // Invalid email action recipient.
        assert_debug_snapshot!(
            update_and_fail(trackers.update_tracker(tracker.id, TrackerUpdateParams {
                actions: Some(vec![TrackerAction::Email(EmailAction {
                    to: vec!["alpha-beta-gamma".to_string()]
                })]),
                ..Default::default()
            }).await),
            @r###""Tracker email action recipient ('alpha-beta-gamma') is not a valid email address.""###
        );

        // Too many email action recipients.
        assert_debug_snapshot!(
            update_and_fail(trackers.update_tracker(tracker.id, TrackerUpdateParams {
                actions: Some(vec![TrackerAction::Email(EmailAction {
                    to: vec!["dev@retrack.dev".to_string(); 11]
                })]),
                ..Default::default()
            }).await),
            @r###""Tracker email action cannot have more than 10 recipients.""###
        );

        // Invalid webhook action method.
        assert_debug_snapshot!(
            update_and_fail(trackers.update_tracker(tracker.id, TrackerUpdateParams {
                actions: Some(vec![TrackerAction::Webhook(WebhookAction {
                    url: "https://retrack.dev".parse()?,
                    method: Some(Method::PATCH),
                    headers: None
                })]),
                ..Default::default()
            }).await),
            @r###""Tracker webhook action method must be either `GET`, `POST`, or `PUT`.""###
        );

        // Too many webhook action headers.
        let headers: [(HeaderName, String); 21] = core::array::from_fn(|i| {
            (
                HeaderName::from_str(&format!("x-header-{i}")).unwrap(),
                format!("application/{i}"),
            )
        });
        assert_debug_snapshot!(
            update_and_fail(trackers.update_tracker(tracker.id, TrackerUpdateParams {
                actions: Some(vec![TrackerAction::Webhook(WebhookAction {
                   url: "https://retrack.dev".parse()?,
                    method: None,
                    headers: Some((&headers.into_iter().collect::<HashMap<_, _>>()).try_into()?),
                })]),
                ..Default::default()
            }).await),
            @r###""Tracker webhook action cannot have more than 20 headers.""###
        );

        // Too long timeout.
        assert_debug_snapshot!(
            update_and_fail(trackers.update_tracker(tracker.id, TrackerUpdateParams {
                config: Some(TrackerConfig {
                    timeout: Some(Duration::from_secs(61)),
                    ..tracker.config.clone()
                }),
                ..Default::default()
            }).await),
            @r###""Tracker timeout cannot be greater than 60000ms.""###
        );

        // Empty web page target extractor.
        assert_debug_snapshot!(
            update_and_fail(trackers.update_tracker(tracker.id, TrackerUpdateParams {
                target: Some(TrackerTarget::Page(PageTarget {
                    extractor: "".to_string(),
                    user_agent: None,
                    ignore_https_errors: false
                })),
                ..Default::default()
            }).await),
            @r###""Tracker web page extractor script cannot be empty.""###
        );

        // Very long web page target extractor.
        assert_debug_snapshot!(
            update_and_fail(trackers.update_tracker(tracker.id, TrackerUpdateParams {
                target: Some(TrackerTarget::Page(PageTarget {
                    extractor: "a".repeat(2049),
                    user_agent: None,
                    ignore_https_errors: false
                })),
                ..Default::default()
            }).await),
            @r###""Tracker web page extractor script cannot be longer than 2048 characters.""###
        );

        // Empty web page target user agent.
        assert_debug_snapshot!(
            update_and_fail(trackers.update_tracker(tracker.id, TrackerUpdateParams {
                target: Some(TrackerTarget::Page(PageTarget {
                    extractor: "export async function execute(p) { await p.goto('https://retrack.dev/'); return await p.content(); }".to_string(),
                    user_agent: Some("".to_string()),
                    ignore_https_errors: false,
                })),
                ..Default::default()
            }).await),
            @r###""Tracker web page user-agent header cannot be empty.""###
        );

        // Very long web page target user agent.
        assert_debug_snapshot!(
            update_and_fail(trackers.update_tracker(tracker.id, TrackerUpdateParams {
                target: Some(TrackerTarget::Page(PageTarget {
                    extractor: "export async function execute(p) { await p.goto('https://retrack.dev/'); return await p.content(); }".to_string(),
                    user_agent: Some("a".repeat(201)),
                    ignore_https_errors: false,
                })),
                ..Default::default()
            }).await),
            @r###""Tracker web page user-agent cannot be longer than 200 characters.""###
        );

        // Invalid schedule.
        assert_debug_snapshot!(
            update_and_fail(trackers.update_tracker(tracker.id, TrackerUpdateParams {
                config: Some(TrackerConfig {
                    job: Some(SchedulerJobConfig {
                        schedule: "-".to_string(),
                        retry_strategy: None
                    }),
                    ..tracker.config.clone()
                }),
                ..Default::default()
            }).await),
            @r###"
        Error {
            context: "Tracker schedule must be a valid cron expression.",
            source: "Failed to parse schedule `-`: Invalid pattern: Pattern must consist of five or six fields (minute, hour, day, month, day of week, and optional second).",
        }
        "###
        );

        // Invalid schedule interval.
        assert_debug_snapshot!(
            update_and_fail(trackers.update_tracker(tracker.id, TrackerUpdateParams {
                config: Some(TrackerConfig {
                    job: Some(SchedulerJobConfig {
                        schedule: "0/5 * * * * *".to_string(),
                        retry_strategy: None
                    }),
                    ..tracker.config.clone()
                }),
                ..Default::default()
            }).await),
            @r###""Tracker schedule must have at least 10s between occurrences, but detected 5s.""###
        );

        // Too few retry attempts.
        assert_debug_snapshot!(
            update_and_fail(trackers.update_tracker(tracker.id, TrackerUpdateParams {
                config: Some(TrackerConfig {
                    job: Some(SchedulerJobConfig {
                        schedule: "@daily".to_string(),
                        retry_strategy: Some(SchedulerJobRetryStrategy::Constant {
                            interval: Duration::from_secs(120),
                            max_attempts: 0,
                        })
                    }),
                    ..tracker.config.clone()
                }),
                ..Default::default()
            }).await),
            @r###""Tracker max retry attempts cannot be zero or greater than 10, but received 0.""###
        );

        // Too many retry attempts.
        assert_debug_snapshot!(
            update_and_fail(trackers.update_tracker(tracker.id, TrackerUpdateParams {
                config: Some(TrackerConfig {
                    job: Some(SchedulerJobConfig {
                        schedule: "@daily".to_string(),
                        retry_strategy: Some(SchedulerJobRetryStrategy::Constant {
                            interval: Duration::from_secs(120),
                            max_attempts: 11,
                        })
                    }),
                    ..tracker.config.clone()
                }),
                ..Default::default()
            }).await),
            @r###""Tracker max retry attempts cannot be zero or greater than 10, but received 11.""###
        );

        // Too low retry interval.
        assert_debug_snapshot!(
           update_and_fail(trackers.update_tracker(tracker.id, TrackerUpdateParams {
                config: Some(TrackerConfig {
                    job: Some(SchedulerJobConfig {
                        schedule: "@daily".to_string(),
                        retry_strategy: Some(SchedulerJobRetryStrategy::Constant {
                            interval: Duration::from_secs(30),
                            max_attempts: 5,
                        })
                    }),
                    ..tracker.config.clone()
                }),
                ..Default::default()
            }).await),
            @r###""Tracker min retry interval cannot be less than 1m, but received 30s.""###
        );

        // Too low max retry interval.
        assert_debug_snapshot!(
            update_and_fail(trackers.update_tracker(tracker.id, TrackerUpdateParams {
                config: Some(TrackerConfig {
                    job: Some(SchedulerJobConfig {
                        schedule: "@daily".to_string(),
                        retry_strategy: Some(SchedulerJobRetryStrategy::Linear {
                            initial_interval: Duration::from_secs(120),
                            increment: Duration::from_secs(10),
                            max_interval: Duration::from_secs(30),
                            max_attempts: 5,
                        })
                    }),
                    ..tracker.config.clone()
                }),
                ..Default::default()
            }).await),
            @r###""Tracker retry strategy max interval cannot be less than 1m, but received 30s.""###
        );

        // Too high max retry interval.
        assert_debug_snapshot!(
            update_and_fail(trackers.update_tracker(tracker.id, TrackerUpdateParams {
                config: Some(TrackerConfig {
                    job: Some(SchedulerJobConfig {
                        schedule: "@monthly".to_string(),
                        retry_strategy: Some(SchedulerJobRetryStrategy::Linear {
                            initial_interval: Duration::from_secs(120),
                            increment: Duration::from_secs(10),
                            max_interval: Duration::from_secs(13 * 3600),
                            max_attempts: 5,
                        })
                    }),
                    ..tracker.config.clone()
                }),
                ..Default::default()
            }).await),
            @r###""Tracker retry strategy max interval cannot be greater than 12h, but received 13h.""###
        );

        assert_debug_snapshot!(
            update_and_fail(trackers.update_tracker(tracker.id, TrackerUpdateParams {
                config: Some(TrackerConfig {
                    job: Some(SchedulerJobConfig {
                        schedule: "@hourly".to_string(),
                        retry_strategy: Some(SchedulerJobRetryStrategy::Linear {
                            initial_interval: Duration::from_secs(120),
                            increment: Duration::from_secs(10),
                            max_interval: Duration::from_secs(2 * 3600),
                            max_attempts: 5,
                        })
                    }),
                    ..tracker.config.clone()
                }),
               ..Default::default()
            }).await),
            @r###""Tracker retry strategy max interval cannot be greater than 1h, but received 2h.""###
        );

        // Invalid JSON API target URL schema.
        assert_debug_snapshot!(
            update_and_fail(trackers.update_tracker(tracker.id, TrackerUpdateParams {
                target: Some(TrackerTarget::Api(ApiTarget {
                    url: Url::parse("ftp://retrack.dev")?,
                    method: None,
                    headers: None,
                    media_type: None,
                })),
                ..Default::default()
            }).await),
            @r###""Tracker JSON API target URL must be either `http` or `https` and have a valid public reachable domain name, but received ftp://retrack.dev/.""###
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
                target: Some(TrackerTarget::Api(ApiTarget {
                    url: Url::parse("https://127.0.0.1")?,
                    method: None,
                    headers: None,
                    media_type: None,
                })),
                ..Default::default()
            }).await),
            @r###""Tracker JSON API target URL must be either `http` or `https` and have a valid public reachable domain name, but received https://127.0.0.1/.""###
        );

        Ok(())
    }

    #[sqlx::test]
    async fn properly_updates_tracker_job_id_at_update(pool: PgPool) -> anyhow::Result<()> {
        let api = mock_api(pool).await?;

        let trackers = api.trackers();
        let tracker = trackers
            .create_tracker(TrackerCreateParams::new("name_one").with_schedule("0 0 * * * *"))
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
            updated_at: updated_tracker.updated_at,
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
                    config: Some(TrackerConfig {
                        job: Some(SchedulerJobConfig {
                            schedule: "0 1 * * * *".to_string(),
                            retry_strategy: None,
                        }),
                        ..tracker.config.clone()
                    }),
                    ..Default::default()
                },
            )
            .await?;
        let expected_tracker = Tracker {
            name: "name_two".to_string(),
            job_id: None,
            config: TrackerConfig {
                job: Some(SchedulerJobConfig {
                    schedule: "0 1 * * * *".to_string(),
                    retry_strategy: None,
                }),
                ..tracker.config.clone()
            },
            updated_at: updated_tracker.updated_at,
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
            .create_tracker(TrackerCreateParams::new("name_one").with_schedule("0 0 * * * *"))
            .await?;
        let tracker_two = trackers
            .create_tracker(TrackerCreateParams::new("name_two").with_schedule("0 0 * * * *"))
            .await?;

        assert_eq!(
            trackers.get_trackers(Default::default()).await?,
            vec![tracker_one.clone(), tracker_two.clone()],
        );

        trackers.remove_tracker(tracker_one.id).await?;

        assert_eq!(
            trackers.get_trackers(Default::default()).await?,
            vec![tracker_two.clone()],
        );

        trackers.remove_tracker(tracker_two.id).await?;

        assert!(trackers.get_trackers(Default::default()).await?.is_empty());

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
            .create_tracker(TrackerCreateParams::new("name_one").with_schedule("0 0 * * * *"))
            .await?;
        assert_eq!(
            trackers.get_tracker(tracker_one.id).await?,
            Some(tracker_one.clone()),
        );

        let tracker_two = trackers
            .create_tracker(TrackerCreateParams::new("name_two").with_schedule("0 0 * * * *"))
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
        assert!(trackers.get_trackers(Default::default()).await?.is_empty(),);

        let tracker_one = trackers
            .create_tracker(
                TrackerCreateParams::new("name_one")
                    .with_schedule("0 0 * * * *")
                    .with_tags(vec!["tag:1".to_string(), "tag:common".to_string()])
                    .with_actions(vec![
                        TrackerAction::ServerLog,
                        TrackerAction::Email(EmailAction {
                            to: vec!["dev@retrack.dev".to_string()],
                        }),
                    ]),
            )
            .await?;
        assert_eq!(
            trackers.get_trackers(Default::default()).await?,
            vec![tracker_one.clone()],
        );
        let tracker_two = trackers
            .create_tracker(
                TrackerCreateParams::new("name_two")
                    .with_schedule("0 0 * * * *")
                    .with_tags(vec!["tag:2".to_string(), "tag:common".to_string()])
                    .with_actions(vec![
                        TrackerAction::ServerLog,
                        TrackerAction::Email(EmailAction {
                            to: vec!["dev@retrack.dev".to_string()],
                        }),
                    ]),
            )
            .await?;

        assert_eq!(
            trackers.get_trackers(Default::default()).await?,
            vec![tracker_one.clone(), tracker_two.clone()],
        );
        assert_eq!(
            trackers
                .get_trackers(TrackersListParams {
                    tags: vec!["tag:2".to_string()]
                })
                .await?,
            vec![tracker_two.clone()],
        );
        assert_eq!(
            trackers
                .get_trackers(TrackersListParams {
                    tags: vec!["tag:1".to_string()]
                })
                .await?,
            vec![tracker_one.clone()],
        );
        assert_eq!(
            trackers
                .get_trackers(TrackersListParams {
                    tags: vec!["tag:1".to_string(), "tag:common".to_string()]
                })
                .await?,
            vec![tracker_one.clone()],
        );
        assert_eq!(
            trackers
                .get_trackers(TrackersListParams {
                    tags: vec!["tag:2".to_string(), "tag:common".to_string()]
                })
                .await?,
            vec![tracker_two.clone()],
        );
        assert!(trackers
            .get_trackers(TrackersListParams {
                tags: vec!["tag:unknown".to_string(), "tag:common".to_string()]
            })
            .await?
            .is_empty());

        Ok(())
    }

    #[sqlx::test]
    async fn properly_validates_trackers_list_parameters(pool: PgPool) -> anyhow::Result<()> {
        let api = mock_api(pool).await?;
        let api = api.trackers();

        let list_and_fail = |result: anyhow::Result<_>| -> RetrackError {
            result.unwrap_err().downcast::<RetrackError>().unwrap()
        };

        // Very long tag.
        assert_debug_snapshot!(
            list_and_fail(api.get_trackers(TrackersListParams {
                tags: vec!["a".repeat(51)]
            }).await),
            @r###""Tracker tags cannot be empty or longer than 50 characters.""###
        );

        // Empty tag.
        assert_debug_snapshot!(
            list_and_fail(api.get_trackers(TrackersListParams {
                tags: vec!["tag".to_string(), "".to_string()]
            }).await),
            @r###""Tracker tags cannot be empty or longer than 50 characters.""###
        );

        // Too many tags.
        assert_debug_snapshot!(
            list_and_fail(api.get_trackers(TrackersListParams {
                tags: (0..11).map(|i| i.to_string()).collect()
            }).await),
            @r###""Trackers filter params cannot use more than 10 tags.""###
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
            .create_tracker(TrackerCreateParams::new("name_one").with_schedule("0 0 * * * *"))
            .await?;
        let tracker_two = trackers
            .create_tracker(TrackerCreateParams::new("name_two").with_schedule("0 0 * * * *"))
            .await?;

        let tracker_one_data = trackers
            .get_tracker_data(tracker_one.id, Default::default())
            .await?;
        let tracker_two_data = trackers
            .get_tracker_data(tracker_two.id, Default::default())
            .await?;
        assert!(tracker_one_data.is_empty());
        assert!(tracker_two_data.is_empty());

        let content_one = TrackerDataValue::new(json!("\"rev_1\""));
        let mut content_mock = server.mock(|when, then| {
            when.method(httpmock::Method::POST)
                .path("/api/web_page/execute")
                .json_body(
                    serde_json::to_value(WebScraperContentRequest::try_from(&tracker_one).unwrap())
                        .unwrap(),
                );
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body_obj(content_one.value());
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
        assert_eq!(tracker_one_data[0].data, content_one);
        assert!(tracker_two_data.is_empty());

        content_mock.assert();
        content_mock.delete();

        let content_two = TrackerDataValue::new(json!("\"rev_2\""));
        let mut content_mock = server.mock(|when, then| {
            when.method(httpmock::Method::POST)
                .path("/api/web_page/execute")
                .json_body(
                    serde_json::to_value(
                        WebScraperContentRequest::try_from(&tracker_one)
                            .unwrap()
                            .set_previous_content(&json!("\"rev_1\"")),
                    )
                    .unwrap(),
                );
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body_obj(content_two.value());
        });

        tokio::time::sleep(Duration::from_millis(1000)).await;

        let revision = trackers
            .create_tracker_data_revision(tracker_one.id)
            .await?
            .unwrap();
        assert_eq!(revision.data.value(), "\"rev_2\"");
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

        let content_two = TrackerDataValue::new(json!("\"rev_3\""));
        let content_mock = server.mock(|when, then| {
            when.method(httpmock::Method::POST)
                .path("/api/web_page/execute")
                .json_body(
                    serde_json::to_value(WebScraperContentRequest::try_from(&tracker_two).unwrap())
                        .unwrap(),
                );
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body_obj(content_two.value());
        });
        let revision = trackers
            .create_tracker_data_revision(tracker_two.id)
            .await?;
        assert_eq!(revision.unwrap().data.value(), "\"rev_3\"");
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
            TrackerDataValue {
                original: String("\"rev_1\""),
                mods: None,
            },
            TrackerDataValue {
                original: String("@@ -1 +1 @@\n-\"rev_1\"\n+\"rev_2\"\n"),
                mods: None,
            },
        ]
        "###
        );
        assert_debug_snapshot!(
            tracker_two_data.into_iter().map(|rev| rev.data).collect::<Vec<_>>(),
            @r###"
        [
            TrackerDataValue {
                original: String("\"rev_3\""),
                mods: None,
            },
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
            TrackerDataValue {
                original: String("\"rev_1\""),
                mods: None,
            },
            TrackerDataValue {
                original: String("\"rev_2\""),
                mods: None,
            },
        ]
        "###
        );

        Ok(())
    }

    #[sqlx::test]
    async fn can_execute_tracker_actions(pool: PgPool) -> anyhow::Result<()> {
        let server = MockServer::start();
        let mut config = mock_config()?;
        config.components.web_scraper_url = Url::parse(&server.base_url())?;

        let api = mock_api_with_config(pool, config).await?;

        let trackers = api.trackers();
        let tracker = trackers
            .create_tracker(
                TrackerCreateParams::new("tracker")
                    .with_schedule("0 0 * * * *")
                    .with_actions(vec![
                        TrackerAction::Email(EmailAction {
                            to: vec![
                                "dev@retrack.dev".to_string(),
                                "dev-2@retrack.dev".to_string(),
                            ],
                        }),
                        TrackerAction::Webhook(WebhookAction {
                            url: "https://retrack.dev".parse()?,
                            method: None,
                            headers: Some(HeaderMap::from_iter([(
                                CONTENT_TYPE,
                                HeaderValue::from_static("text/plain"),
                            )])),
                        }),
                    ]),
            )
            .await?;

        let content = TrackerDataValue::new(json!("\"rev_1\""));
        let mut server_mock = server.mock(|when, then| {
            when.method(httpmock::Method::POST)
                .path("/api/web_page/execute")
                .json_body(
                    serde_json::to_value(WebScraperContentRequest::try_from(&tracker).unwrap())
                        .unwrap(),
                );
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body_obj(content.value());
        });

        let scheduled_before_or_at = OffsetDateTime::now_utc()
            .checked_add(time::Duration::days(1))
            .unwrap();
        let tasks_ids = api
            .db
            .get_tasks_ids(scheduled_before_or_at, 2)
            .collect::<Vec<_>>()
            .await;
        assert!(tasks_ids.is_empty());

        let tracker_data = trackers
            .get_tracker_data(
                tracker.id,
                TrackerListRevisionsParams {
                    refresh: true,
                    calculate_diff: false,
                },
            )
            .await?;
        assert_eq!(tracker_data.len(), 1);
        assert_eq!(tracker_data[0].data.value(), &json!("\"rev_1\""));

        server_mock.assert();
        server_mock.delete();

        let mut tasks_ids = api
            .db
            .get_tasks_ids(scheduled_before_or_at, 2)
            .collect::<Vec<_>>()
            .await;
        assert_eq!(tasks_ids.len(), 2);

        let email_task = api.db.get_task(tasks_ids.remove(0)?).await?.unwrap();
        assert_eq!(
            email_task.task_type,
            TaskType::Email(EmailTaskType {
                to: vec![
                    "dev@retrack.dev".to_string(),
                    "dev-2@retrack.dev".to_string(),
                ],
                content: EmailContent::Template(EmailTemplate::TrackerChanges {
                    tracker_name: tracker.name.clone(),
                    content: Ok(json!("\"rev_1\"").to_string()),
                }),
            })
        );

        let http_task = api.db.get_task(tasks_ids.remove(0)?).await?.unwrap();
        assert_eq!(
            http_task.task_type,
            TaskType::Http(HttpTaskType {
                url: "https://retrack.dev".parse()?,
                method: Method::POST,
                headers: Some(HeaderMap::from_iter([(
                    CONTENT_TYPE,
                    HeaderValue::from_static("text/plain"),
                )])),
                body: Some(serde_json::to_vec(&json!("\"rev_1\""))?),
            })
        );

        // Clear action tasks.
        api.db.remove_task(email_task.id).await?;
        api.db.remove_task(http_task.id).await?;
        let tasks_ids = api
            .db
            .get_tasks_ids(scheduled_before_or_at, 2)
            .collect::<Vec<_>>()
            .await;
        assert!(tasks_ids.is_empty());

        let mut server_mock = server.mock(|when, then| {
            let mut scraper_request = WebScraperContentRequest::try_from(&tracker).unwrap();
            scraper_request.previous_content = Some(content.value());

            when.method(httpmock::Method::POST)
                .path("/api/web_page/execute")
                .json_body(serde_json::to_value(scraper_request).unwrap());
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body_obj(content.value());
        });

        let tracker_data = trackers
            .get_tracker_data(
                tracker.id,
                TrackerListRevisionsParams {
                    refresh: true,
                    calculate_diff: false,
                },
            )
            .await?;
        assert_eq!(tracker_data.len(), 1);
        assert_eq!(tracker_data[0].data.value(), &json!("\"rev_1\""));

        server_mock.assert();
        server_mock.delete();

        let tasks_ids = api
            .db
            .get_tasks_ids(scheduled_before_or_at, 2)
            .collect::<Vec<_>>()
            .await;
        assert!(tasks_ids.is_empty());

        // Now, let's change content.
        let new_content = TrackerDataValue::new(json!("\"rev_2\""));
        let server_mock = server.mock(|when, then| {
            let mut scraper_request = WebScraperContentRequest::try_from(&tracker).unwrap();
            scraper_request.previous_content = Some(content.value());

            when.method(httpmock::Method::POST)
                .path("/api/web_page/execute")
                .json_body(serde_json::to_value(scraper_request).unwrap());
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body_obj(new_content.value());
        });

        let tracker_data = trackers
            .get_tracker_data(
                tracker.id,
                TrackerListRevisionsParams {
                    refresh: true,
                    calculate_diff: false,
                },
            )
            .await?;
        assert_eq!(tracker_data.len(), 2);
        assert_eq!(tracker_data[0].data.value(), &json!("\"rev_1\""));
        assert_eq!(tracker_data[1].data.value(), &json!("\"rev_2\""));

        server_mock.assert();

        let mut tasks_ids = api
            .db
            .get_tasks_ids(scheduled_before_or_at, 2)
            .collect::<Vec<_>>()
            .await;
        assert_eq!(tasks_ids.len(), 2);

        let email_task = api.db.get_task(tasks_ids.remove(0)?).await?.unwrap();
        assert_eq!(
            email_task.task_type,
            TaskType::Email(EmailTaskType {
                to: vec![
                    "dev@retrack.dev".to_string(),
                    "dev-2@retrack.dev".to_string(),
                ],
                content: EmailContent::Template(EmailTemplate::TrackerChanges {
                    tracker_name: tracker.name.clone(),
                    content: Ok(json!("\"rev_2\"").to_string()),
                }),
            })
        );

        let http_task = api.db.get_task(tasks_ids.remove(0)?).await?.unwrap();
        assert_eq!(
            http_task.task_type,
            TaskType::Http(HttpTaskType {
                url: "https://retrack.dev".parse()?,
                method: Method::POST,
                headers: Some(HeaderMap::from_iter([(
                    CONTENT_TYPE,
                    HeaderValue::from_static("text/plain"),
                )])),
                body: Some(serde_json::to_vec(&json!("\"rev_2\""))?),
            })
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
            .create_tracker(TrackerCreateParams::new("name_one").with_schedule("0 0 * * * *"))
            .await?;

        let content_mock = server.mock(|when, then| {
            when.method(httpmock::Method::POST)
                .path("/api/web_page/execute")
                .json_body(
                    serde_json::to_value(WebScraperContentRequest::try_from(&tracker).unwrap())
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
            .downcast::<RetrackError>()?;
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
    async fn properly_ignores_revision_with_no_diff(pool: PgPool) -> anyhow::Result<()> {
        let server = MockServer::start();
        let mut config = mock_config()?;
        config.components.web_scraper_url = Url::parse(&server.base_url())?;

        let api = mock_api_with_config(pool, config).await?;

        let trackers = api.trackers();
        let tracker = trackers
            .create_tracker(TrackerCreateParams::new("name_one").with_schedule("0 0 * * * *"))
            .await?;

        let tracker_content = trackers
            .get_tracker_data(tracker.id, Default::default())
            .await?;
        assert!(tracker_content.is_empty());

        let content_one = TrackerDataValue::new(json!("\"rev_1\""));
        let mut content_mock = server.mock(|when, then| {
            when.method(httpmock::Method::POST)
                .path("/api/web_page/execute")
                .json_body(
                    serde_json::to_value(WebScraperContentRequest::try_from(&tracker).unwrap())
                        .unwrap(),
                );
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body_obj(content_one.value());
        });

        let revision = trackers.create_tracker_data_revision(tracker.id).await?;
        assert_eq!(revision.unwrap().data.value(), "\"rev_1\"");
        content_mock.assert();
        content_mock.delete();

        let content_two = json!("\"rev_1\"");
        let content_mock = server.mock(|when, then| {
            when.method(httpmock::Method::POST)
                .path("/api/web_page/execute")
                .json_body(
                    serde_json::to_value(
                        WebScraperContentRequest::try_from(&tracker)
                            .unwrap()
                            .set_previous_content(&json!("\"rev_1\"")),
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
            .create_tracker(TrackerCreateParams::new("name_one").with_schedule("0 0 * * * *"))
            .await?;

        let tracker_content = trackers
            .get_tracker_data(tracker.id, Default::default())
            .await?;
        assert!(tracker_content.is_empty());

        let content = json!("\"rev_1\"");
        let content_mock = server.mock(|when, then| {
            when.method(httpmock::Method::POST)
                .path("/api/web_page/execute")
                .json_body(
                    serde_json::to_value(WebScraperContentRequest::try_from(&tracker).unwrap())
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
            .create_tracker(TrackerCreateParams::new("name_one").with_schedule("0 0 * * * *"))
            .await?;

        let tracker_content = trackers
            .get_tracker_data(tracker.id, Default::default())
            .await?;
        assert!(tracker_content.is_empty());

        let content = json!("\"rev_1\"");
        let content_mock = server.mock(|when, then| {
            when.method(httpmock::Method::POST)
                .path("/api/web_page/execute")
                .json_body(
                    serde_json::to_value(WebScraperContentRequest::try_from(&tracker).unwrap())
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
    async fn properly_resets_job_id_when_tracker_schedule_changed(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        let api = mock_api(pool).await?;

        let trackers = api.trackers();
        let tracker = trackers
            .create_tracker(TrackerCreateParams::new("name_one").with_schedule("0 0 * * * *"))
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
                    enabled: Some(true),
                    target: Some(TrackerTarget::Page(PageTarget {
                        extractor: "export async function execute(p) { await p.goto('https://retrack.dev/222'); return await p.content(); }".to_string(),
                        user_agent: Some("Unknown/1.0.0".to_string()),
                        ignore_https_errors: true,
                    })),
                    config: Some(TrackerConfig {
                        revisions: 4,
                        timeout: Some(Duration::from_millis(3000)),
                        job: Some(SchedulerJobConfig {
                            schedule: "0 0 * * * *".to_string(),
                            retry_strategy: Some(SchedulerJobRetryStrategy::Constant {
                                interval: Duration::from_secs(120),
                                max_attempts: 5,
                            })
                        }),
                        ..tracker.config.clone()
                    }),
                    tags: Some(vec!["tag".to_string()]),
                    actions: Some(vec![TrackerAction::ServerLog]),
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
                    config: Some(TrackerConfig {
                        job: Some(SchedulerJobConfig {
                            schedule: "0 1 * * * *".to_string(),
                            retry_strategy: Some(SchedulerJobRetryStrategy::Constant {
                                interval: Duration::from_secs(120),
                                max_attempts: 5,
                            }),
                        }),
                        ..tracker.config.clone()
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
    async fn properly_removes_job_id_when_tracker_revisions_disabled(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        let api = mock_api(pool).await?;

        let trackers = api.trackers();
        let tracker = trackers
            .create_tracker(TrackerCreateParams::new("name_one").with_schedule("0 0 * * * *"))
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

        // Update everything except enabled and schedule and keep revisions enabled (job ID shouldn't be touched).
        trackers
            .update_tracker(
                tracker.id,
                TrackerUpdateParams {
                    name: Some("name_one_new".to_string()),
                    enabled: Some(true),
                    target: Some(TrackerTarget::Page(PageTarget {
                        extractor: "export async function execute(p) { await p.goto('https://retrack.dev/222'); return await p.content(); }".to_string(),
                        user_agent: Some("Unknown/1.0.0".to_string()),
                        ignore_https_errors: true,
                    })),
                    config: Some(TrackerConfig {
                        revisions: 4,
                        timeout: Some(Duration::from_millis(3000)),
                        job: Some(SchedulerJobConfig {
                            schedule: "0 0 * * * *".to_string(),
                            retry_strategy: Some(SchedulerJobRetryStrategy::Constant {
                                interval: Duration::from_secs(120),
                                max_attempts: 5,
                            })
                        }),
                        ..tracker.config.clone()
                    }),
                    tags: Some(vec!["tag_two".to_string()]),
                    actions: Some(vec![TrackerAction::Email(EmailAction {
                        to: vec!["dev@retrack.dev".to_string()]
                    })])
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
                    config: Some(TrackerConfig {
                        revisions: 0,
                        ..tracker.config.clone()
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
    async fn properly_removes_job_id_when_tracker_is_disabled(pool: PgPool) -> anyhow::Result<()> {
        let api = mock_api(pool).await?;

        let trackers = api.trackers();
        let tracker = trackers
            .create_tracker(TrackerCreateParams::new("name_one").with_schedule("0 0 * * * *"))
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

        // Update everything except enabled (job ID shouldn't be touched).
        trackers
            .update_tracker(
                tracker.id,
                TrackerUpdateParams {
                    name: Some("name_one_new".to_string()),
                    enabled: None,
                    target: Some(TrackerTarget::Page(PageTarget {
                        extractor: "export async function execute(p) { await p.goto('https://retrack.dev/222'); return await p.content(); }".to_string(),
                        user_agent: Some("Unknown/1.0.0".to_string()),
                        ignore_https_errors: true,
                    })),
                    config: Some(TrackerConfig {
                        revisions: 4,
                        timeout: Some(Duration::from_millis(3000)),
                        job: Some(SchedulerJobConfig {
                            schedule: "0 0 * * * *".to_string(),
                            retry_strategy: Some(SchedulerJobRetryStrategy::Constant {
                                interval: Duration::from_secs(120),
                                max_attempts: 5,
                            })
                        }),
                        ..tracker.config.clone()
                    }),
                    tags: Some(vec!["tag_two".to_string()]),
                    actions: Some(vec![TrackerAction::Email(EmailAction {
                        to: vec!["dev@retrack.dev".to_string()]
                    })])
                },
            )
            .await?;

        assert_eq!(
            trackers.get_tracker(tracker.id).await?.unwrap().job_id,
            Some(uuid!("00000000-0000-0000-0000-000000000001")),
        );

        // Disable tracker.
        trackers
            .update_tracker(
                tracker.id,
                TrackerUpdateParams {
                    enabled: Some(false),
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

        let unscheduled_trackers = api.trackers().get_trackers_to_schedule().await?;
        assert!(unscheduled_trackers.is_empty());
        let unscheduled_trackers = api.trackers().get_trackers_to_schedule().await?;
        assert!(unscheduled_trackers.is_empty());

        let tracker = trackers
            .create_tracker(TrackerCreateParams::new("name_one").with_schedule("0 0 * * * *"))
            .await?;

        let unscheduled_trackers = api.trackers().get_trackers_to_schedule().await?;
        assert_eq!(unscheduled_trackers, vec![tracker.clone()]);

        api.trackers()
            .update_tracker_job(
                tracker.id,
                Some(uuid!("11e55044-10b1-426f-9247-bb680e5fe0c9")),
            )
            .await?;

        let unscheduled_trackers = api.trackers().get_trackers_to_schedule().await?;
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
                    config: Some(TrackerConfig {
                        job: None,
                        ..tracker.config.clone()
                    }),
                    ..Default::default()
                },
            )
            .await?;

        let unscheduled_trackers = api.trackers().get_trackers_to_schedule().await?;
        assert!(unscheduled_trackers.is_empty());
        let unscheduled_trackers = api.trackers().get_trackers_to_schedule().await?;
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
            .get_trackers_to_run()
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
                .create_tracker(
                    TrackerCreateParams::new(format!("name_{}", n)).with_schedule("0 0 * * * *"),
                )
                .await?;
        }

        let pending_trackers = api
            .trackers()
            .get_trackers_to_run()
            .collect::<Vec<_>>()
            .await;
        assert!(pending_trackers.is_empty());

        // Assign job IDs to trackers.
        let all_trackers = trackers.get_trackers(Default::default()).await?;
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
            .get_trackers_to_run()
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<anyhow::Result<Vec<_>, _>>()?;
        assert_eq!(pending_trackers.len(), 2);

        let all_trackers = trackers.get_trackers(Default::default()).await?;
        assert_eq!(
            vec![all_trackers[0].clone(), all_trackers[2].clone()],
            pending_trackers,
        );

        let all_trackers = trackers.get_trackers(Default::default()).await?;
        assert_eq!(all_trackers.len(), 3);

        Ok(())
    }
}
