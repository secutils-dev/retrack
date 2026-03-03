use crate::{
    api::Api,
    config::TrackersConfig,
    database::Database,
    error::Error as RetrackError,
    js_runtime::{ScriptBuilder, ScriptConfig},
    network::DnsResolver,
    scheduler::CronExt,
    tasks::{EmailContent, EmailTaskType, EmailTemplate, HttpTaskType, TaskType},
    trackers::{
        database_ext::TrackersDatabaseExt,
        parsers::{CsvParser, XlsParser},
        tracker_data_revisions_diff::tracker_data_revisions_diff,
        web_scraper::{
            WebScraperBackend, WebScraperContentRequest, WebScraperDebugOptions,
            WebScraperErrorResponse, WebScraperSuccessResponse,
        },
    },
};
use anyhow::{Context, anyhow, bail};
use byte_unit::Byte;
use croner::Cron;
use http::Method;
use http_cache_reqwest::{CACacheManager, Cache, CacheMode, HttpCache, HttpCacheOptions};
use lettre::message::Mailbox;
use mediatype::MediaTypeBuf;
use reqwest_middleware::{ClientBuilder, ClientWithMiddleware};
use reqwest_tracing::{SpanBackendWithUrl, TracingMiddleware};
use retrack_types::{
    scheduler::SchedulerJobRetryStrategy,
    trackers::{
        ActionDebugInfo, ActionDestinationDebugInfo, ApiRequestDebugInfo, ApiTarget,
        ApiTrackerDebugResult, AutoParseDebugInfo, ConfiguratorScriptArgs,
        ConfiguratorScriptResult, DebugOptions, EmailDestinationDebugInfo, ExtractorEngine,
        ExtractorScriptArgs, ExtractorScriptResult, FormatterScriptArgs, FormatterScriptResult,
        PageLogEntry, PageScreenshotEntry, PageTarget, PageTrackerDebugResult,
        RenderedEmailDebugInfo, ScriptDebugInfo, ServerLogAction, TargetRequest, TargetResponse,
        Tracker, TrackerAction, TrackerCreateParams, TrackerDataRevision, TrackerDataValue,
        TrackerDebugExistingParams, TrackerDebugParams, TrackerDebugResult,
        TrackerDebugTargetResult, TrackerListRevisionsParams, TrackerTarget, TrackerUpdateParams,
        TrackersListParams, WebhookAction, WebhookActionPayload, WebhookActionPayloadResult,
        WebhookDestinationDebugInfo,
    },
};
use serde_json::{Value as JsonValue, json};
use std::{
    borrow::Cow,
    cmp::min,
    collections::HashSet,
    str::FromStr,
    time::{Duration, Instant},
};
use tracing::{debug, error, info};
use url::Url;
use uuid::Uuid;

/// Defines the maximum length of the user agent string.
const MAX_TRACKER_PAGE_USER_AGENT_LENGTH: usize = 200;

/// We currently support up to 10 retry attempts for the tracker.
const MAX_TRACKER_RETRY_ATTEMPTS: u32 = 10;

/// We currently support a maximum 12 hours between retry attempts for the tracker.
const MAX_TRACKER_RETRY_INTERVAL: Duration = Duration::from_secs(12 * 3600);

/// Defines the maximum length of a tracker name.
pub const MAX_TRACKER_NAME_LENGTH: usize = 100;

/// Defines the maximum length of a tracker tag.
pub const MAX_TRACKER_TAG_LENGTH: usize = 100;

/// Defines the maximum count of tracker tags.
pub const MAX_TRACKER_TAGS_COUNT: usize = 20;

/// Defines the maximum count of tracker actions.
pub const MAX_TRACKER_ACTIONS_COUNT: usize = 10;

/// Defines the maximum count of tracker target requests.
pub const MAX_TRACKER_REQUEST_COUNT: usize = 10;

/// Defines the maximum count of tracker email action recipients.
pub const MAX_TRACKER_EMAIL_ACTION_RECIPIENTS_COUNT: usize = 10;

/// Defines the maximum count of tracker webhook action headers.
pub const MAX_TRACKER_WEBHOOK_ACTION_HEADERS_COUNT: usize = 20;

/// Defines a prefix for the tracker task tags.
const TRACKER_TASK_TAG_PREFIX: &str = "@retrack";

/// Well-known tracker ID used in logs for ad-hoc debug runs.
const DEBUG_TRACKER_ID: Uuid = Uuid::nil();
/// Well-known tracker name used in logs for ad-hoc debug runs.
const DEBUG_TRACKER_NAME: &str = "_debug";
/// Maximum raw response body bytes included in debug output.
const DEBUG_MAX_RAW_BODY_BYTES: usize = 64 * 1024;

pub struct TrackersApiExt<'a, DR: DnsResolver> {
    api: &'a Api<DR>,
    trackers: TrackersDatabaseExt<'a>,
}

impl<'a, DR: DnsResolver> TrackersApiExt<'a, DR> {
    /// Creates Trackers API.
    pub fn new(api: &'a Api<DR>) -> Self {
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
        let created_at = Database::utc_now()?;
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
            updated_at: Database::utc_now()?,
            job_id,
            ..existing_tracker
        };

        self.validate_tracker(&tracker).await?;

        self.trackers.update_tracker(&tracker).await?;

        Ok(tracker)
    }

    /// Removes existing tracker and all data.
    pub async fn remove_tracker(&self, id: Uuid) -> anyhow::Result<bool> {
        self.trackers.remove_tracker(id).await
    }

    /// Removes all trackers that have all specified tags. If `tags` is empty, all trackers are removed.
    pub async fn remove_trackers(&self, params: TrackersListParams) -> anyhow::Result<u64> {
        let normalized_tags = Self::normalize_tracker_tags(params.tags);
        if normalized_tags.len() > MAX_TRACKER_TAGS_COUNT {
            bail!(RetrackError::client(format!(
                "Trackers filter params cannot use more than {MAX_TRACKER_TAGS_COUNT} tags."
            )));
        }
        Self::validate_tracker_tags(&normalized_tags)?;

        self.trackers.remove_trackers(&normalized_tags).await
    }

    /// Returns tracker data revision by its ID.
    pub async fn get_tracker_data_revision(
        &self,
        tracker_id: Uuid,
        id: Uuid,
    ) -> anyhow::Result<Option<TrackerDataRevision>> {
        self.trackers
            .get_tracker_data_revision(tracker_id, id)
            .await
    }

    /// Tries to fetch a new data revision for the specified tracker and persists it if allowed by
    /// config and if the data has changed. Executes actions regardless of the result.
    pub async fn create_tracker_data_revision(
        &self,
        tracker_id: Uuid,
    ) -> anyhow::Result<TrackerDataRevision> {
        self.create_tracker_data_revision_with_retry(tracker_id, None)
            .await
    }

    /// Tries to fetch a new data revision for the specified tracker and persists it if allowed by
    /// config and if the data has changed. When `retry_attempt` is specified, it suppresses error
    /// actions in failure scenarios if there are still retry attempts remaining.
    pub async fn create_tracker_data_revision_with_retry(
        &self,
        tracker_id: Uuid,
        retry_attempt: Option<u32>,
    ) -> anyhow::Result<TrackerDataRevision> {
        let Some(tracker) = self.get_tracker(tracker_id).await? else {
            bail!(RetrackError::client(format!(
                "Tracker ('{tracker_id}') is not found."
            )));
        };

        // Fetch the last revision to provide it to the extractor of the new revision.
        let last_revision = self
            .trackers
            .get_tracker_data_revisions(tracker.id, 1)
            .await?
            .pop();
        let new_revision = match tracker.target {
            TrackerTarget::Page(_) => self
                .create_tracker_page_data_revision(&tracker, &last_revision)
                .await
                .map_err(RetrackError::client_with_root_cause),
            TrackerTarget::Api(_) => self
                .create_tracker_api_data_revision(&tracker, &last_revision)
                .await
                .map_err(RetrackError::client_with_root_cause),
        };

        // Iterate through all tracker actions, including default ones, and execute them.
        let actions = tracker
            .actions
            .iter()
            .chain(self.api.config.trackers.default_actions.iter().flatten());
        let new_revision = match new_revision {
            Ok(new_revision) => new_revision,
            Err(err) => {
                // Don't execute actions if retries aren't exhausted yet.
                let has_retries_left = retry_attempt
                    .and_then(|attempt| {
                        Some(attempt < tracker.config.job.as_ref()?.retry_strategy?.max_attempts())
                    })
                    .unwrap_or_default();
                if !has_retries_left {
                    for action in actions {
                        self.execute_tracker_action(&tracker, action, Err(err.to_string()))
                            .await?
                    }
                }

                return Err(err.into());
            }
        };

        // If the last revision has the same original data value, drop a newly fetched revision.
        let last_revision = if let Some(last_revision) = last_revision {
            // Return the last revision without re-running actions if data hasn't changed.
            if last_revision.data.original() == new_revision.data.original() {
                debug!(
                    tracker.id = %tracker.id,
                    tracker.name = tracker.name,
                    tracker.tags = ?tracker.tags,
                    "Skipping actions for a new data revision since content hasn't changed."
                );
                return Ok(last_revision);
            }
            Some(last_revision)
        } else {
            None
        };

        for (action_index, action) in actions.enumerate() {
            match self
                .get_action_payload(action, &new_revision, last_revision.as_ref())
                .await
            {
                Ok(Some(action_payload)) => {
                    self.execute_tracker_action(&tracker, action, Ok(action_payload))
                        .await?
                }
                Ok(None) => {
                    debug!(
                        tracker.id = %tracker.id,
                        tracker.name = tracker.name,
                        tracker.action = action.type_tag(),
                        tracker.tags = ?tracker.tags,
                        "Skipping action for a new data revision as requested by action formatter ({action_index})."
                    );
                }
                Err(err) => {
                    error!(
                        tracker.id = %tracker.id,
                        tracker.name = tracker.name,
                        tracker.action = action.type_tag(),
                        tracker.tags = ?tracker.tags,
                        "Failed to retrieve payload for action ({action_index}): {err}"
                    );
                    bail!(RetrackError::client_with_root_cause(err));
                }
            }
        }

        // Insert a new revision if allowed by the config.
        let max_revisions = min(
            tracker.config.revisions,
            self.api.config.trackers.max_revisions,
        );
        if max_revisions > 0 {
            self.trackers
                .insert_tracker_data_revision(&new_revision)
                .await?;
        }

        // Enforce revision limit and displace old revisions if needed.
        self.trackers
            .enforce_tracker_data_revisions_limit(tracker.id, max_revisions)
            .await?;

        Ok(new_revision)
    }

    /// Returns all stored tracker data revisions.
    pub async fn get_tracker_data_revisions(
        &self,
        tracker_id: Uuid,
        params: TrackerListRevisionsParams,
    ) -> anyhow::Result<Vec<TrackerDataRevision>> {
        let Some(tracker) = self.get_tracker(tracker_id).await? else {
            bail!(RetrackError::client(format!(
                "Tracker ('{tracker_id}') is not found."
            )));
        };

        // If size isn't explicitly specified, use the `max_revisions` value from the global config.
        let max_revisions = params
            .size
            .map(|size| size.get())
            .unwrap_or(self.api.config.trackers.max_revisions);
        let revisions = self
            .trackers
            .get_tracker_data_revisions(tracker_id, min(max_revisions, tracker.config.revisions))
            .await?;
        if params.calculate_diff {
            tracker_data_revisions_diff(revisions, params.effective_context_radius())
        } else {
            Ok(revisions)
        }
    }

    /// Removes specified tracker data revision.
    pub async fn remove_tracker_data_revision(
        &self,
        tracker_id: Uuid,
        id: Uuid,
    ) -> anyhow::Result<bool> {
        self.trackers
            .remove_tracker_data_revision(tracker_id, id)
            .await
    }

    /// Removes all persisted tracker revisions data.
    pub async fn clear_tracker_data(&self, tracker_id: Uuid) -> anyhow::Result<()> {
        if self.get_tracker(tracker_id).await?.is_none() {
            bail!(RetrackError::client(format!(
                "Tracker ('{tracker_id}') is not found."
            )));
        }
        self.trackers.clear_tracker_data_revisions(tracker_id).await
    }

    /// Returns all tracker job references that have jobs that need to be scheduled.
    pub async fn get_trackers_to_schedule(&self) -> anyhow::Result<Vec<Tracker>> {
        self.trackers.get_trackers_to_schedule().await
    }

    /// Returns tracker by the corresponding job ID.
    pub async fn get_tracker_by_job_id(&self, job_id: Uuid) -> anyhow::Result<Option<Tracker>> {
        self.trackers.get_tracker_by_job_id(job_id).await
    }

    /// Updates tracker job ID reference.
    pub async fn set_tracker_job(&self, id: Uuid, job_id: Uuid) -> anyhow::Result<()> {
        self.trackers.update_tracker_job(id, Some(job_id)).await
    }

    /// Clears tracker job ID reference.
    pub async fn clear_tracker_job(&self, id: Uuid) -> anyhow::Result<()> {
        self.trackers.update_tracker_job(id, None).await
    }

    /// Returns tracker-specific tags for the task originated from the tracker.
    pub fn get_task_tags(tracker: &Tracker, task_type: &TaskType) -> Vec<String> {
        tracker
            .tags
            .iter()
            .cloned()
            .chain([
                format!("{TRACKER_TASK_TAG_PREFIX}:tracker:id:{}", tracker.id),
                format!(
                    "{TRACKER_TASK_TAG_PREFIX}:task:type:{}",
                    task_type.type_tag()
                ),
            ])
            .collect()
    }

    /// Calculates action payload for the success case.
    async fn get_action_payload<'r>(
        &self,
        action: &TrackerAction,
        new_revision: &'r TrackerDataRevision,
        previous_revision: Option<&TrackerDataRevision>,
    ) -> anyhow::Result<Option<Cow<'r, JsonValue>>> {
        // If no formatter is specified, use the new revision data value as an action payload.
        let Some(formatter) = action.formatter() else {
            return Ok(Some(Cow::Borrowed(new_revision.data.value())));
        };

        // Retrieve the formatter script content.
        let formatter = self
            .get_script_content(formatter)
            .await
            .with_context(|| {
                format!(
                    "Cannot retrieve tracker action formatter script ({})",
                    action.type_tag()
                )
            })
            .map_err(|err| anyhow!(RetrackError::client_with_root_cause(err)))?;

        // If the latest data value has no modifications, use previous original value as
        // previous value. Otherwise, use the modification from the previous data value based on
        // the highest index of the latest data value modifications.
        let new_data_value = &new_revision.data;
        let previous_value =
            previous_revision
                .map(|rev| &rev.data)
                .and_then(|previous_data_value| {
                    if new_data_value.mods().is_none() {
                        Some(previous_data_value.original())
                    } else {
                        previous_data_value
                            .mods()?
                            .get(new_data_value.mods()?.len() - 1)
                    }
                });

        // Format action payload, if needed.
        let formatter_result = self
            .execute_script::<FormatterScriptArgs, FormatterScriptResult>(
                formatter,
                FormatterScriptArgs {
                    action: action.type_tag(),
                    new_content: new_data_value.value().clone(),
                    previous_content: previous_value.cloned(),
                },
            )
            .await
            .with_context(|| {
                format!(
                    "Failed to execute action formatter script ({}).",
                    action.type_tag()
                )
            })
            .map_err(|err| anyhow!(RetrackError::client_with_root_cause(err)))?;

        // If the formatter doesn't return anything, use the original new revision value as an
        // action payload. If it returns content, use it as the action payload instead. If it
        // returns `null`, it's a signal to abort the action completely.
        Ok(formatter_result.map_or_else(
            || Some(Cow::Borrowed(new_data_value.value())),
            |formatter_result| formatter_result.content.map(Cow::Owned),
        ))
    }

    /// Executes tracker action.
    async fn execute_tracker_action(
        &self,
        tracker: &Tracker,
        action: &TrackerAction,
        payload: Result<Cow<'_, JsonValue>, String>,
    ) -> anyhow::Result<()> {
        let tasks_api = self.api.tasks();
        match action {
            TrackerAction::Email(action) => {
                let task_type = TaskType::Email(EmailTaskType {
                    to: action.to.clone(),
                    content: EmailContent::Template(EmailTemplate::TrackerCheckResult {
                        tracker_id: tracker.id,
                        tracker_name: tracker.name.clone(),
                        // If the payload is a JSON string, remove quotes, otherwise use
                        // JSON as is.
                        result: payload.map(|payload| {
                            payload
                                .as_str()
                                .map(|value| value.to_owned())
                                .unwrap_or_else(|| payload.to_string())
                        }),
                    }),
                });
                let task_tags = TrackersApiExt::<DR>::get_task_tags(tracker, &task_type);
                let task = tasks_api
                    .schedule_task(task_type, task_tags, Database::utc_now()?)
                    .await?;
                info!(
                    tracker.id = %tracker.id,
                    tracker.name = tracker.name,
                    tracker.tags = ?tracker.tags,
                    task.id = %task.id,
                    "Scheduled email task."
                );
            }
            TrackerAction::Webhook(action) => {
                let task_type = TaskType::Http(HttpTaskType {
                    url: action.url.clone(),
                    method: action.method.clone().unwrap_or(Method::POST),
                    headers: action.headers.clone(),
                    body: Some(serde_json::to_vec(&WebhookActionPayload {
                        tracker_id: tracker.id,
                        tracker_name: tracker.name.clone(),
                        result: payload
                            .map(|payload| {
                                WebhookActionPayloadResult::Success(payload.into_owned())
                            })
                            .unwrap_or_else(WebhookActionPayloadResult::Failure),
                    })?),
                });
                let task_tags = TrackersApiExt::<DR>::get_task_tags(tracker, &task_type);
                let task = tasks_api
                    .schedule_task(task_type, task_tags, Database::utc_now()?)
                    .await?;
                info!(
                    tracker.id = %tracker.id,
                    tracker.name = tracker.name,
                    tracker.tags = ?tracker.tags,
                    task.id = %task.id,
                    "Scheduled HTTP task."
                );
            }
            TrackerAction::ServerLog(_) => {
                info!(
                    tracker.id = %tracker.id,
                    tracker.name = tracker.name,
                    tracker.tags = ?tracker.tags,
                    "Fetched new data revision: {payload:?}"
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

        self.validate_tracker_actions(&tracker.actions).await?;
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
                self.validate_page_target(target).await?;
            }
            TrackerTarget::Api(ref target) => {
                self.validate_api_target(config, target).await?;
            }
        }

        if let Some(ref timeout) = tracker.config.timeout
            && timeout > &config.max_timeout
        {
            bail!(RetrackError::client(format!(
                "Tracker timeout cannot be greater than {}ms.",
                config.max_timeout.as_millis()
            )));
        }

        if let Some(job_config) = &tracker.config.job {
            // Validate that the schedule is a valid cron expression.
            let schedule = match Cron::parse_pattern(job_config.schedule.as_str()) {
                Ok(schedule) => schedule,
                Err(err) => {
                    bail!(RetrackError::client_with_root_cause(anyhow!(
                        "Tracker schedule must be a valid cron expression, but the provided schedule ({}) cannot be parsed: {err}",
                        job_config.schedule
                    )));
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
                    bail!(RetrackError::client(format!(
                        "Tracker max retry attempts cannot be zero or greater than {MAX_TRACKER_RETRY_ATTEMPTS}, but received {max_attempts}."
                    )));
                }

                let min_interval = *retry_strategy.min_interval();
                if min_interval < config.min_retry_interval {
                    bail!(RetrackError::client(format!(
                        "Tracker min retry interval cannot be less than {}, but received {}.",
                        humantime::format_duration(config.min_retry_interval),
                        humantime::format_duration(min_interval)
                    )));
                }

                if let SchedulerJobRetryStrategy::Linear { max_interval, .. }
                | SchedulerJobRetryStrategy::Exponential { max_interval, .. } = retry_strategy
                {
                    let max_interval = *max_interval;
                    if max_interval < min_interval {
                        bail!(RetrackError::client(format!(
                            "Tracker retry strategy max interval cannot be less than {}, but received {}.",
                            humantime::format_duration(min_interval),
                            humantime::format_duration(max_interval)
                        )));
                    }

                    if max_interval > MAX_TRACKER_RETRY_INTERVAL {
                        bail!(RetrackError::client(format!(
                            "Tracker retry strategy max interval cannot be greater than {}, but received {}.",
                            humantime::format_duration(MAX_TRACKER_RETRY_INTERVAL),
                            humantime::format_duration(max_interval)
                        )));
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
    async fn validate_tracker_actions(&self, actions: &[TrackerAction]) -> anyhow::Result<()> {
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
                                "Tracker email action recipient ('{recipient}') is not a valid email address.",
                            )));
                        }
                    }

                    if let Some(script) = &action.formatter {
                        self.validate_script(script, "email action formatter")
                            .await?;
                    }
                }
                TrackerAction::Webhook(WebhookAction {
                    method,
                    headers,
                    formatter,
                    ..
                }) => {
                    if let Some(method) = method
                        && method != Method::GET
                        && method != Method::POST
                        && method != Method::PUT
                    {
                        bail!(RetrackError::client(
                            "Tracker webhook action method must be either `GET`, `POST`, or `PUT`."
                        ));
                    }

                    if let Some(headers) = headers
                        && headers.len() > MAX_TRACKER_WEBHOOK_ACTION_HEADERS_COUNT
                    {
                        bail!(RetrackError::client(format!(
                            "Tracker webhook action cannot have more than {MAX_TRACKER_WEBHOOK_ACTION_HEADERS_COUNT} headers."
                        )));
                    }

                    if let Some(script) = &formatter {
                        self.validate_script(script, "webhook action formatter")
                            .await?;
                    }
                }
                TrackerAction::ServerLog(ServerLogAction { formatter }) => {
                    if let Some(script) = &formatter {
                        self.validate_script(script, "server log action formatter")
                            .await?;
                    }
                }
            }
        }

        Ok(())
    }

    /// Validates tracker's web page target parameters.
    async fn validate_page_target(&self, target: &PageTarget) -> anyhow::Result<()> {
        // Validate extractor script.
        self.validate_script(&target.extractor, "target extractor")
            .await?;

        if let Some(ref user_agent) = target.user_agent {
            if user_agent.is_empty() {
                bail!(RetrackError::client(
                    "Tracker target user-agent header cannot be empty.",
                ));
            }

            if user_agent.len() > MAX_TRACKER_PAGE_USER_AGENT_LENGTH {
                bail!(RetrackError::client(format!(
                    "Tracker target user-agent cannot be longer than {MAX_TRACKER_PAGE_USER_AGENT_LENGTH} characters."
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
        if target.requests.is_empty() {
            bail!(RetrackError::client(
                "Tracker target should have at least one request."
            ));
        }

        if target.requests.len() > MAX_TRACKER_REQUEST_COUNT {
            bail!(RetrackError::client(format!(
                "Tracker target cannot have more than {MAX_TRACKER_REQUEST_COUNT} requests."
            )));
        }

        if config.restrict_to_public_urls {
            for request in &target.requests {
                if !self.api.network.is_public_web_url(&request.url).await {
                    bail!(RetrackError::client(format!(
                        "Tracker target URL must be either `http` or `https` and have a valid public reachable domain name, but received {}.",
                        request.url
                    )));
                }
            }
        }

        // Validate configurator script.
        if let Some(script) = &target.configurator {
            self.validate_script(script, "target configurator").await?;
        }

        // Validate extractor script.
        if let Some(script) = &target.extractor {
            self.validate_script(script, "target extractor").await?;
        }

        Ok(())
    }

    /// Creates data revision for a tracker with a `Page` target
    async fn create_tracker_page_data_revision(
        &self,
        tracker: &Tracker,
        previous_revision: &Option<TrackerDataRevision>,
    ) -> anyhow::Result<TrackerDataRevision> {
        let TrackerTarget::Page(ref target) = tracker.target else {
            bail!("Tracker ('{}') target is not `Page`.", tracker.id);
        };

        let extractor = self
            .get_script_content(&target.extractor)
            .await
            .context("Cannot retrieve tracker extractor script")
            .map_err(|err| anyhow!(RetrackError::client_with_root_cause(err)))?;
        let scraper_request = WebScraperContentRequest {
            extractor: extractor.as_ref(),
            extractor_params: target.params.as_ref(),
            extractor_backend: Some(match target.engine {
                // Use `chromium` backend if engine isn't specified.
                Some(ExtractorEngine::Chromium) | None => WebScraperBackend::Chromium,
                Some(ExtractorEngine::Camoufox) => WebScraperBackend::Firefox,
            }),
            tags: &tracker.tags,
            user_agent: target.user_agent.as_deref(),
            accept_invalid_certificates: target.accept_invalid_certificates,
            timeout: tracker.config.timeout,
            previous_content: previous_revision.as_ref().map(|rev| &rev.data),
            debug: None,
        };

        let scraper_response = self
            .http_client(false)
            .post(format!(
                "{}api/web_page/execute",
                self.api.config.as_ref().components.web_scraper_url.as_str()
            ))
            .json(&scraper_request)
            .send()
            .await
            .context("Cannot connect to the web scraper service")?;

        if !scraper_response.status().is_success() {
            let scraper_error_response =
                scraper_response
                    .json::<WebScraperErrorResponse>()
                    .await
                    .context("Cannot deserialize the web scraper service error response")?;
            bail!(scraper_error_response.message);
        }

        let success_response: WebScraperSuccessResponse = scraper_response
            .json()
            .await
            .context("Cannot deserialize the web scraper service response")?;

        Ok(TrackerDataRevision {
            id: Uuid::now_v7(),
            tracker_id: tracker.id,
            data: TrackerDataValue::new(success_response.result),
            created_at: Database::utc_now()?,
        })
    }

    /// Creates data revision for a tracker with `Api` target.
    async fn create_tracker_api_data_revision(
        &self,
        tracker: &Tracker,
        last_revision: &Option<TrackerDataRevision>,
    ) -> anyhow::Result<TrackerDataRevision> {
        let TrackerTarget::Api(ref target) = tracker.target else {
            bail!("Tracker ('{}') target is not `Api`.", tracker.id);
        };

        // Run the configurator script, if specified, to check if there are any overrides to the
        // request parameters need to be made.
        let (requests_override, responses_override) =
            if let Some(ref configurator) = target.configurator {
                // Prepare requests for the configurator script.
                let mut configurator_requests = Vec::with_capacity(target.requests.len());
                for request in &target.requests {
                    configurator_requests.push(request.clone().try_into()?);
                }

                let result = self
                    .execute_script::<ConfiguratorScriptArgs, ConfiguratorScriptResult>(
                        self.get_script_content(configurator)
                            .await
                            .context("Cannot retrieve tracker configurator script")
                            .map_err(|err| anyhow!(RetrackError::client_with_root_cause(err)))?,
                        ConfiguratorScriptArgs {
                            tags: tracker.tags.clone(),
                            previous_content: last_revision.as_ref().map(|rev| rev.data.clone()),
                            requests: configurator_requests,
                            params: target.params.clone(),
                        },
                    )
                    .await
                    .context("Cannot execute tracker configurator script")
                    .map_err(|err| anyhow!(RetrackError::client_with_root_cause(err)))?;
                match result {
                    Some(ConfiguratorScriptResult::Requests(configurator_requests)) => {
                        // If the configurator script didn't return any request overrides, use the default requests.
                        if configurator_requests.is_empty() {
                            (None, None)
                        } else {
                            let mut requests = Vec::with_capacity(configurator_requests.len());
                            for request in configurator_requests {
                                requests.push(request.try_into()?);
                            }
                            (Some(requests), None)
                        }
                    }
                    Some(ConfiguratorScriptResult::Responses(responses)) => (None, Some(responses)),
                    _ => (None, None),
                }
            } else {
                (None, None)
            };

        // If the configurator overrides the response body, use it instead of making any requests.
        let responses = if let Some(responses_override) = responses_override {
            responses_override
        } else {
            let requests = requests_override.as_ref().unwrap_or(&target.requests);
            let mut responses = Vec::with_capacity(requests.len());
            for (request_index, request) in requests.iter().enumerate() {
                let client = self.http_client(request.accept_invalid_certificates);
                let request_builder = client.request(
                    request.method.as_ref().unwrap_or(&Method::GET).clone(),
                    request.url.clone(),
                );

                // Add headers, if any.
                let request_builder = if let Some(ref headers) = request.headers {
                    request_builder.headers(headers.clone())
                } else {
                    request_builder
                };

                // Add body, if any.
                let request_builder = if let Some(ref body) = request.body {
                    request_builder.body(serde_json::to_vec(body).with_context(|| {
                        format!("Cannot serialize a body of the API request ({request_index}).")
                    })?)
                } else {
                    request_builder
                };

                // Set timeout, if any.
                let request_builder = if let Some(ref timeout) = tracker.config.timeout {
                    request_builder.timeout(*timeout)
                } else {
                    request_builder
                };

                // Read response, parse, and extract data with the extractor script, if specified.
                let api_response = client.execute(request_builder.build()?).await?;

                // Check if status should be accepted.
                let status = api_response.status();
                let should_accept_status = request.accept_statuses.as_ref().map_or_else(
                    || status.is_success(),
                    |statuses| statuses.contains(&status),
                );
                if !should_accept_status {
                    bail!(
                        "Failed to execute the API request ({request_index}): {status} {}",
                        api_response.text().await?
                    );
                }

                let headers = api_response.headers().clone();
                let response_bytes = api_response
                    .bytes()
                    .await
                    .with_context(|| format!("Failed to read API response ({request_index})."))?;

                debug!(
                    tracker.id = %tracker.id,
                    tracker.name = tracker.name,
                    tracker.tags = ?tracker.tags,
                    "Fetched the API response ({request_index}) with {} bytes ({}, {} headers).",
                    response_bytes.len(),
                    status,
                    headers.len()
                );

                // Media-type-based parsing is only performed for success responses.
                let body = if status.is_success() {
                    let media_type = request
                        .media_type
                        .as_ref()
                        .map(|media_type| media_type.to_ref());
                    (match media_type {
                        Some(ref media_type) if XlsParser::supports(media_type) => {
                            XlsParser::parse(&response_bytes).with_context(|| format!("Failed to parse the API response as XLS file ({request_index})."))?
                        }
                        Some(ref media_type) if CsvParser::supports(media_type) => {
                            CsvParser::parse(&response_bytes).with_context(|| format!("Failed to parse the API response as CSV file ({request_index})."))?
                        }
                        _ => response_bytes,
                    }).to_vec()
                } else {
                    response_bytes.to_vec()
                };

                responses.push(TargetResponse {
                    status,
                    headers,
                    body,
                });
            }

            responses
        };

        // Process the response with the extractor script, if specified.
        let extractor_response_bytes = if let Some(ref extractor) = target.extractor {
            debug!(
                tracker.id = %tracker.id,
                tracker.name = tracker.name,
                "Extracting data with the API target extractor script ({extractor})."
            );

            let result = self
                .execute_script::<ExtractorScriptArgs, ExtractorScriptResult>(
                    self.get_script_content(extractor)
                        .await
                        .context("Cannot retrieve tracker extractor script")
                        .map_err(|err| anyhow!(RetrackError::client_with_root_cause(err)))?,
                    ExtractorScriptArgs {
                        tags: tracker.tags.clone(),
                        previous_content: last_revision.as_ref().map(|rev| rev.data.clone()),
                        responses: Some(responses.clone()),
                        params: target.params.clone(),
                    },
                )
                .await
                .context("Failed to execute tracker extractor script")
                .map_err(|err| anyhow!(RetrackError::client_with_root_cause(err)))?
                .unwrap_or_default();
            result.body
        } else {
            None
        };

        // Deserialize the response body or the extractor result.
        let tracker_data_value = if let Some(response_bytes) = extractor_response_bytes {
            debug!(
                tracker.id = %tracker.id,
                tracker.name = tracker.name,
                "Extracted data from the tracker extractor script with {} bytes.",
                response_bytes.len()
            );
            serde_json::from_slice(&response_bytes)
                .context("Cannot deserialize tracker extractor script result")?
        } else if responses.len() == 1 {
            serde_json::from_slice(&responses[0].body).context("Cannot deserialize API response")?
        } else {
            JsonValue::Array(
                responses
                    .iter()
                    .map(|r| {
                        serde_json::from_slice::<JsonValue>(&r.body).unwrap_or_else(|_| {
                            JsonValue::String(String::from_utf8_lossy(&r.body).into_owned())
                        })
                    })
                    .collect(),
            )
        };

        Ok(TrackerDataRevision {
            id: Uuid::now_v7(),
            tracker_id: tracker.id,
            data: TrackerDataValue::new(tracker_data_value),
            created_at: Database::utc_now()?,
        })
    }

    /// Executes JavaScript with Deno JS runtime.
    async fn execute_script<ScriptArgs: ScriptBuilder<ScriptArgs, ScriptResult>, ScriptResult>(
        &self,
        script_src: impl Into<String>,
        script_args: ScriptArgs,
    ) -> anyhow::Result<Option<ScriptResult>> {
        let now = Instant::now();
        match self
            .api
            .js_runtime
            .execute_script(
                script_src,
                script_args,
                ScriptConfig {
                    max_heap_size: self.api.config.js_runtime.max_heap_size,
                    max_execution_time: self.api.config.js_runtime.max_script_execution_time,
                },
            )
            .await
        {
            Ok(result) => {
                debug!(
                    metrics.script_execution_time = now.elapsed().as_secs_f64(),
                    "Successfully executed script.",
                );
                Ok(result)
            }
            Err(err) => {
                error!(
                    metrics.script_execution_time = now.elapsed().as_secs_f64(),
                    "Failed to execute script: {err:?}"
                );
                Err(err)
            }
        }
    }

    /// Validates remote script reference.
    async fn validate_script(&self, script: &str, script_ref: &str) -> anyhow::Result<()> {
        let config = &self.api.config.trackers;
        if script.is_empty() {
            bail!(RetrackError::client(format!(
                "Tracker {script_ref} script cannot be empty."
            )));
        }

        let script_size = Byte::from_u64(script.len() as u64);
        if script_size > config.max_script_size {
            bail!(RetrackError::client(format!(
                "Tracker {script_ref} script cannot be larger than {} bytes.",
                config.max_script_size
            )));
        }

        // No need to parse the URL if we don't restrict to public URLs.
        if !config.restrict_to_public_urls {
            return Ok(());
        }

        // Try to parse the URL and check if it's a valid URL. If it's not, script will be treated
        // as a script content.
        let Ok(script_url) = Url::parse(script) else {
            return Ok(());
        };

        if !self.api.network.is_public_web_url(&script_url).await {
            bail!(RetrackError::client(format!(
                "Tracker {script_ref} script URL must be either `http` or `https` and have a valid public reachable domain name, but received {script_url}."
            )));
        }

        Ok(())
    }

    /// Takes script reference saved as a tracker script and returns its content. If the script
    /// reference is a valid URL, its content will be fetched from the remote server.
    async fn get_script_content(&self, script_ref: &str) -> anyhow::Result<String> {
        // First check if script is a URL pointing to a remote script.
        let Ok(url) = Url::parse(script_ref) else {
            return Ok(script_ref.to_string());
        };

        // Make sure that URL is allowed.
        let config = &self.api.config.trackers;
        if config.restrict_to_public_urls && !self.api.network.is_public_web_url(&url).await {
            bail!("Attempted to fetch remote script from not allowed URL: {script_ref}");
        }

        Ok(self
            .http_client(false)
            .get(url)
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?)
    }

    /// Runs the full tracker pipeline (extraction + action dry-run) without persisting
    /// anything. Accepts the same shape as `TrackerCreateParams`.
    pub async fn debug_tracker(
        &self,
        params: TrackerDebugParams,
    ) -> anyhow::Result<TrackerDebugResult> {
        let tracker = Tracker {
            id: DEBUG_TRACKER_ID,
            name: DEBUG_TRACKER_NAME.to_string(),
            enabled: true,
            target: params.target,
            config: params.config,
            tags: Self::normalize_tracker_tags(params.tags),
            actions: params.actions,
            job_id: None,
            created_at: Database::utc_now()?,
            updated_at: Database::utc_now()?,
        };

        self.validate_debug_tracker(&tracker).await?;

        self.run_debug(&tracker, params.previous_content, params.debug)
            .await
    }

    /// Runs the full tracker pipeline against a stored tracker, with optional overrides.
    pub async fn debug_existing_tracker(
        &self,
        tracker_id: Uuid,
        params: TrackerDebugExistingParams,
    ) -> anyhow::Result<TrackerDebugResult> {
        let Some(mut tracker) = self.get_tracker(tracker_id).await? else {
            bail!(RetrackError::client(format!(
                "Tracker ('{tracker_id}') is not found."
            )));
        };

        if let Some(target) = params.target {
            tracker.target = target;
        }
        if let Some(config) = params.config {
            tracker.config = config;
        }
        if let Some(tags) = params.tags {
            tracker.tags = Self::normalize_tracker_tags(tags);
        }
        if let Some(actions) = params.actions {
            tracker.actions = actions;
        }

        self.validate_debug_tracker(&tracker).await?;

        let previous_content = if params.previous_content.is_some() {
            params.previous_content
        } else {
            self.trackers
                .get_tracker_data_revisions(tracker.id, 1)
                .await?
                .pop()
                .map(|rev| rev.data)
        };

        self.run_debug(&tracker, previous_content, params.debug)
            .await
    }

    /// Validates tracker fields relevant to a debug run (target + timeout), skipping
    /// name, schedule, revisions, and action recipient validation.
    async fn validate_debug_tracker(&self, tracker: &Tracker) -> anyhow::Result<()> {
        let config = &self.api.config.trackers;
        match tracker.target {
            TrackerTarget::Page(ref target) => {
                self.validate_page_target(target).await?;
            }
            TrackerTarget::Api(ref target) => {
                self.validate_api_target(config, target).await?;
            }
        }

        if let Some(ref timeout) = tracker.config.timeout
            && timeout > &config.max_timeout
        {
            bail!(RetrackError::client(format!(
                "Tracker timeout cannot be greater than {}ms.",
                config.max_timeout.as_millis()
            )));
        }

        Ok(())
    }

    /// Core debug execution: runs extraction pipeline then simulates actions.
    async fn run_debug(
        &self,
        tracker: &Tracker,
        previous_content: Option<TrackerDataValue>,
        debug_options: Option<DebugOptions>,
    ) -> anyhow::Result<TrackerDebugResult> {
        let overall_start = Instant::now();

        info!(
            tracker.id = %tracker.id,
            tracker.name = tracker.name,
            tracker.tags = ?tracker.tags,
            "Debug: starting tracker debug run"
        );

        let previous_revision = previous_content.map(|data| TrackerDataRevision {
            id: Uuid::nil(),
            tracker_id: tracker.id,
            data,
            created_at: tracker.created_at,
        });

        // Run extraction pipeline (returns target debug info + optional revision + optional error).
        let (target_result, new_revision, extraction_error) = match tracker.target {
            TrackerTarget::Page(_) => {
                self.debug_tracker_page(tracker, &previous_revision, debug_options)
                    .await
            }
            TrackerTarget::Api(_) => self.debug_tracker_api(tracker, &previous_revision).await,
        };

        // Simulate actions.
        let actions = self
            .debug_tracker_actions(tracker, new_revision.as_ref(), previous_revision.as_ref())
            .await;

        let result = new_revision.as_ref().map(|r| r.data.value().clone());
        let duration_ms = overall_start.elapsed();

        info!(
            tracker.id = %tracker.id,
            tracker.name = tracker.name,
            duration_ms = duration_ms.as_millis(),
            success = extraction_error.is_none(),
            "Debug: tracker debug run completed"
        );

        Ok(TrackerDebugResult {
            duration_ms,
            result,
            error: extraction_error,
            target: target_result,
            actions,
        })
    }

    /// Debug extraction for a Page tracker target.
    async fn debug_tracker_page(
        &self,
        tracker: &Tracker,
        previous_revision: &Option<TrackerDataRevision>,
        debug_options: Option<DebugOptions>,
    ) -> (
        TrackerDebugTargetResult,
        Option<TrackerDataRevision>,
        Option<String>,
    ) {
        let TrackerTarget::Page(ref target) = tracker.target else {
            return (
                TrackerDebugTargetResult::Page(PageTrackerDebugResult {
                    params: None,
                    engine: None,
                    extractor_source: String::new(),
                    logs: vec![],
                    screenshots: vec![],
                    duration_ms: Duration::ZERO,
                    error: Some("Tracker target is not `Page`.".to_string()),
                }),
                None,
                Some("Tracker target is not `Page`.".to_string()),
            );
        };

        let extractor_start = Instant::now();
        let extractor_source = match self.get_script_content(&target.extractor).await {
            Ok(content) => content,
            Err(err) => {
                let error_msg = format!("Cannot retrieve tracker extractor script: {err}");
                return (
                    TrackerDebugTargetResult::Page(PageTrackerDebugResult {
                        params: target.params.clone(),
                        engine: target.engine,
                        extractor_source: target.extractor.clone(),
                        logs: vec![],
                        screenshots: vec![],
                        duration_ms: extractor_start.elapsed(),
                        error: Some(error_msg.clone()),
                    }),
                    None,
                    Some(error_msg),
                );
            }
        };

        let scraper_request = WebScraperContentRequest {
            extractor: &extractor_source,
            extractor_params: target.params.as_ref(),
            extractor_backend: Some(match target.engine {
                Some(ExtractorEngine::Chromium) | None => WebScraperBackend::Chromium,
                Some(ExtractorEngine::Camoufox) => WebScraperBackend::Firefox,
            }),
            tags: &tracker.tags,
            user_agent: target.user_agent.as_deref(),
            accept_invalid_certificates: target.accept_invalid_certificates,
            timeout: tracker.config.timeout,
            previous_content: previous_revision.as_ref().map(|rev| &rev.data),
            debug: Some(WebScraperDebugOptions {
                enabled: true,
                max_screenshots_total_size: debug_options
                    .as_ref()
                    .and_then(|d| d.max_screenshots_total_size),
                auto_screenshots: debug_options.as_ref().and_then(|d| d.auto_screenshots),
            }),
        };

        let scraper_result = self
            .http_client(false)
            .post(format!(
                "{}api/web_page/execute",
                self.api.config.as_ref().components.web_scraper_url.as_str()
            ))
            .json(&scraper_request)
            .send()
            .await;

        let duration_ms = extractor_start.elapsed();
        let (result, error_msg, debug_info) = match scraper_result {
            Ok(response) => {
                let status = response.status();
                if status.is_success() {
                    match response.json::<WebScraperSuccessResponse>().await {
                        Ok(success_resp) => (Some(success_resp.result), None, success_resp.debug),
                        Err(err) => (
                            None,
                            Some(format!(
                                "Cannot deserialize web scraper response (HTTP {status}): {err}"
                            )),
                            None,
                        ),
                    }
                } else {
                    match response.json::<WebScraperErrorResponse>().await {
                        Ok(error_resp) => (None, Some(error_resp.message), error_resp.debug),
                        Err(err) => (
                            None,
                            Some(format!(
                                "Cannot deserialize web scraper error response (HTTP {status}): {err}"
                            )),
                            None,
                        ),
                    }
                }
            }
            Err(err) => (
                None,
                Some(format!("Cannot connect to the web scraper service: {err}")),
                None,
            ),
        };

        let (logs, screenshots) = match debug_info {
            Some(info) => (
                info.logs.into_iter().map(PageLogEntry::from).collect(),
                info.screenshots
                    .into_iter()
                    .map(PageScreenshotEntry::from)
                    .collect(),
            ),
            None => (vec![], vec![]),
        };

        (
            TrackerDebugTargetResult::Page(PageTrackerDebugResult {
                params: target.params.clone(),
                engine: target.engine,
                extractor_source,
                logs,
                screenshots,
                duration_ms,
                error: error_msg.clone(),
            }),
            result.map(|value| TrackerDataRevision {
                id: Uuid::nil(),
                tracker_id: tracker.id,
                data: TrackerDataValue::new(value),
                created_at: tracker.created_at,
            }),
            error_msg,
        )
    }

    /// Debug extraction for an API tracker target.
    async fn debug_tracker_api(
        &self,
        tracker: &Tracker,
        previous_revision: &Option<TrackerDataRevision>,
    ) -> (
        TrackerDebugTargetResult,
        Option<TrackerDataRevision>,
        Option<String>,
    ) {
        let TrackerTarget::Api(ref target) = tracker.target else {
            return (
                TrackerDebugTargetResult::Api(ApiTrackerDebugResult {
                    params: None,
                    configurator: None,
                    requests: vec![],
                    extractor: None,
                }),
                None,
                Some("Tracker target is not `Api`.".to_string()),
            );
        };

        // Stage 1: Configurator script.
        let mut configurator_debug: Option<ScriptDebugInfo> = None;
        let mut configurator_result_opt: Option<ConfiguratorScriptResult> = None;

        if let Some(ref configurator) = target.configurator {
            let configurator_start = Instant::now();
            let configurator_source = match self.get_script_content(configurator).await {
                Ok(c) => c,
                Err(err) => {
                    configurator_debug = Some(ScriptDebugInfo {
                        source: configurator.clone(),
                        duration_ms: configurator_start.elapsed(),
                        result: None,
                        error: Some(format!("Cannot retrieve configurator script: {err}")),
                    });
                    String::new()
                }
            };

            if configurator_debug.is_none() {
                let mut configurator_requests = Vec::with_capacity(target.requests.len());
                for request in &target.requests {
                    match request.clone().try_into() {
                        Ok(r) => configurator_requests.push(r),
                        Err(err) => {
                            configurator_debug = Some(ScriptDebugInfo {
                                source: configurator_source.clone(),
                                duration_ms: configurator_start.elapsed(),
                                result: None,
                                error: Some(format!(
                                    "Cannot convert request to configurator format: {err}"
                                )),
                            });
                            break;
                        }
                    }
                }

                if configurator_debug.is_none() {
                    let script_result = self
                        .execute_script::<ConfiguratorScriptArgs, ConfiguratorScriptResult>(
                            configurator_source.clone(),
                            ConfiguratorScriptArgs {
                                tags: tracker.tags.clone(),
                                previous_content: previous_revision
                                    .as_ref()
                                    .map(|rev| rev.data.clone()),
                                requests: configurator_requests,
                                params: target.params.clone(),
                            },
                        )
                        .await;

                    let script_result = match script_result {
                        Ok(Some(result)) => {
                            let result_json = Self::configurator_result_to_debug_json(&result);
                            configurator_result_opt = Some(result);
                            Ok(Some(result_json))
                        }
                        Ok(None) => Ok(None),
                        Err(err) => Err(format!("{err}")),
                    };

                    let duration_ms = configurator_start.elapsed();
                    configurator_debug = Some(match script_result {
                        Ok(result) => ScriptDebugInfo {
                            source: configurator_source,
                            duration_ms,
                            result,
                            error: None,
                        },
                        Err(err) => ScriptDebugInfo {
                            source: configurator_source,
                            duration_ms,
                            result: None,
                            error: Some(err),
                        },
                    });
                }
            }
        }

        // Stage 2: Execute requests (or use mock responses from the configurator).
        let mut request_debug_infos: Vec<ApiRequestDebugInfo> = Vec::new();
        let mut responses: Vec<TargetResponse> = Vec::new();

        let (effective_requests, request_source, mock_responses) = match &configurator_result_opt {
            Some(ConfiguratorScriptResult::Requests(reqs)) if !reqs.is_empty() => {
                let converted: Vec<_> = reqs
                    .iter()
                    .filter_map(|r| {
                        let tr: Result<TargetRequest, _> = r.clone().try_into();
                        tr.ok()
                    })
                    .collect();
                (converted, "configurator", None)
            }
            Some(ConfiguratorScriptResult::Responses(resps)) => {
                (vec![], "mock", Some(resps.clone()))
            }
            _ => (target.requests.clone(), "original", None),
        };

        if let Some(mock_resps) = mock_responses {
            for (i, mock) in mock_resps.iter().enumerate() {
                let req_start = Instant::now();
                let raw_size = mock.body.len() as u64;
                let truncated_body = Self::truncate_body_for_debug(&mock.body);
                let parsed = serde_json::from_slice::<JsonValue>(&mock.body).ok();

                responses.push(mock.clone());

                request_debug_infos.push(ApiRequestDebugInfo {
                    index: i,
                    source: "mock".to_string(),
                    url: None,
                    method: None,
                    request_headers: None,
                    request_body: None,
                    status_code: Some(mock.status.as_u16()),
                    response_headers: Some(mock.headers.clone()),
                    response_body_raw: Some(truncated_body),
                    response_body_raw_size: Some(raw_size),
                    response_body_parsed: parsed,
                    auto_parse: None,
                    duration_ms: req_start.elapsed(),
                    error: None,
                });
            }
        } else {
            for (i, request) in effective_requests.iter().enumerate() {
                let req_start = Instant::now();
                let request_body_json = request.body.clone();
                let method = request.method.as_ref().unwrap_or(&Method::GET);
                let client = self.http_client(request.accept_invalid_certificates);

                let mut request_builder = client.request(method.clone(), request.url.clone());
                if let Some(ref headers) = request.headers {
                    request_builder = request_builder.headers(headers.clone());
                }
                if let Some(ref body) = request.body {
                    match serde_json::to_vec(body) {
                        Ok(body_bytes) => {
                            request_builder = request_builder.body(body_bytes);
                        }
                        Err(err) => {
                            request_debug_infos.push(ApiRequestDebugInfo {
                                index: i,
                                source: request_source.to_string(),
                                url: Some(request.url.clone()),
                                method: Some(method.clone()),
                                request_headers: request.headers.clone(),
                                request_body: request_body_json,
                                status_code: None,
                                response_headers: None,
                                response_body_raw: None,
                                response_body_raw_size: None,
                                response_body_parsed: None,
                                auto_parse: None,
                                duration_ms: req_start.elapsed(),
                                error: Some(format!("Cannot serialize request body: {err}")),
                            });
                            continue;
                        }
                    }
                }

                if let Some(ref timeout) = tracker.config.timeout {
                    request_builder = request_builder.timeout(*timeout);
                }

                match request_builder.send().await {
                    Ok(response) => {
                        let status = response.status();
                        let response_headers_map = response.headers().clone();
                        let media_type = request.media_type.as_ref().map(|m| m.to_ref());

                        let body_bytes = match response.bytes().await {
                            Ok(b) => b.to_vec(),
                            Err(err) => {
                                request_debug_infos.push(ApiRequestDebugInfo {
                                    index: i,
                                    source: request_source.to_string(),
                                    url: Some(request.url.clone()),
                                    method: Some(method.clone()),
                                    request_headers: request.headers.clone(),
                                    request_body: request_body_json,
                                    status_code: Some(status.as_u16()),
                                    response_headers: Some(response_headers_map),
                                    response_body_raw: None,
                                    response_body_raw_size: None,
                                    response_body_parsed: None,
                                    auto_parse: None,
                                    duration_ms: req_start.elapsed(),
                                    error: Some(format!("Failed to read response body: {err}")),
                                });
                                continue;
                            }
                        };

                        let raw_size = body_bytes.len() as u64;
                        let truncated_body = Self::truncate_body_for_debug(&body_bytes);

                        let (parsed, auto_parse_info) = if status.is_success() {
                            Self::debug_auto_parse(&body_bytes, media_type.as_ref())
                        } else {
                            (serde_json::from_slice::<JsonValue>(&body_bytes).ok(), None)
                        };

                        let body_for_responses = if status.is_success() {
                            if let (Some(parsed_val), Some(info)) = (&parsed, &auto_parse_info) {
                                if info.success {
                                    serde_json::to_vec(parsed_val).unwrap_or(body_bytes.clone())
                                } else {
                                    body_bytes.clone()
                                }
                            } else {
                                body_bytes.clone()
                            }
                        } else {
                            body_bytes.clone()
                        };

                        responses.push(TargetResponse {
                            status,
                            headers: response_headers_map.clone(),
                            body: body_for_responses,
                        });

                        request_debug_infos.push(ApiRequestDebugInfo {
                            index: i,
                            source: request_source.to_string(),
                            url: Some(request.url.clone()),
                            method: Some(method.clone()),
                            request_headers: request.headers.clone(),
                            request_body: request_body_json,
                            status_code: Some(status.as_u16()),
                            response_headers: Some(response_headers_map),
                            response_body_raw: Some(truncated_body),
                            response_body_raw_size: Some(raw_size),
                            response_body_parsed: parsed,
                            auto_parse: auto_parse_info,
                            duration_ms: req_start.elapsed(),
                            error: None,
                        });
                    }
                    Err(err) => {
                        request_debug_infos.push(ApiRequestDebugInfo {
                            index: i,
                            source: request_source.to_string(),
                            url: Some(request.url.clone()),
                            method: Some(method.clone()),
                            request_headers: request.headers.clone(),
                            request_body: request_body_json,
                            status_code: None,
                            response_headers: None,
                            response_body_raw: None,
                            response_body_raw_size: None,
                            response_body_parsed: None,
                            auto_parse: None,
                            duration_ms: req_start.elapsed(),
                            error: Some(format!("Request failed: {err}")),
                        });
                    }
                }
            }
        }

        // Stage 3: Extractor script.
        let mut extractor_debug: Option<ScriptDebugInfo> = None;
        let mut final_result: Option<JsonValue> = None;
        let mut extraction_error: Option<String> = None;

        if let Some(ref extractor) = target.extractor {
            let extractor_start = Instant::now();
            let extractor_source = match self.get_script_content(extractor).await {
                Ok(c) => c,
                Err(err) => {
                    let error_msg = format!("Cannot retrieve extractor script: {err}");
                    extractor_debug = Some(ScriptDebugInfo {
                        source: extractor.clone(),
                        duration_ms: extractor_start.elapsed(),
                        result: None,
                        error: Some(error_msg.clone()),
                    });
                    extraction_error = Some(error_msg);
                    String::new()
                }
            };

            if extractor_debug.is_none() {
                let args = ExtractorScriptArgs {
                    tags: tracker.tags.clone(),
                    previous_content: previous_revision.as_ref().map(|rev| rev.data.clone()),
                    responses: Some(responses.clone()),
                    params: target.params.clone(),
                };

                let script_result = self
                    .execute_script::<ExtractorScriptArgs, ExtractorScriptResult>(
                        extractor_source.clone(),
                        args,
                    )
                    .await;
                let script_result = match script_result {
                    Ok(Some(result)) => {
                        let result_json = result
                            .body
                            .as_ref()
                            .and_then(|b| serde_json::from_slice::<JsonValue>(b).ok());
                        final_result = result_json.clone();
                        Ok(result_json)
                    }
                    Ok(None) => Ok(None),
                    Err(err) => {
                        let error_msg = format!("{err}");
                        extraction_error = Some(error_msg.clone());
                        Err(error_msg)
                    }
                };

                let duration_ms = extractor_start.elapsed();
                extractor_debug = Some(match script_result {
                    Ok(result) => ScriptDebugInfo {
                        source: extractor_source,
                        duration_ms,
                        result,
                        error: None,
                    },
                    Err(err) => ScriptDebugInfo {
                        source: extractor_source,
                        duration_ms,
                        result: None,
                        error: Some(err),
                    },
                });
            }
        } else if responses.len() == 1 {
            final_result = Some(
                serde_json::from_slice::<JsonValue>(&responses[0].body).unwrap_or_else(|_| {
                    JsonValue::String(String::from_utf8_lossy(&responses[0].body).into_owned())
                }),
            );
        } else if !responses.is_empty() {
            final_result = Some(JsonValue::Array(
                responses
                    .iter()
                    .map(|r| {
                        serde_json::from_slice::<JsonValue>(&r.body).unwrap_or_else(|_| {
                            JsonValue::String(String::from_utf8_lossy(&r.body).into_owned())
                        })
                    })
                    .collect(),
            ));
        }

        (
            TrackerDebugTargetResult::Api(ApiTrackerDebugResult {
                params: target.params.clone(),
                configurator: configurator_debug,
                requests: request_debug_infos,
                extractor: extractor_debug,
            }),
            final_result.map(|value| TrackerDataRevision {
                id: Uuid::nil(),
                tracker_id: tracker.id,
                data: TrackerDataValue::new(value),
                created_at: tracker.created_at,
            }),
            extraction_error,
        )
    }

    /// Simulates every configured action without dispatching side effects.
    async fn debug_tracker_actions(
        &self,
        tracker: &Tracker,
        new_revision: Option<&TrackerDataRevision>,
        previous_revision: Option<&TrackerDataRevision>,
    ) -> Vec<ActionDebugInfo> {
        let all_actions: Vec<_> = tracker
            .actions
            .iter()
            .chain(self.api.config.trackers.default_actions.iter().flatten())
            .collect();
        if all_actions.is_empty() {
            return vec![];
        }

        let mut result = Vec::with_capacity(all_actions.len());

        for (i, action) in all_actions.iter().enumerate() {
            let action_start = Instant::now();
            let type_tag = action.type_tag().to_string();

            let make_destination = |rendered_email: Option<RenderedEmailDebugInfo>| match action {
                TrackerAction::Email(email) => {
                    ActionDestinationDebugInfo::Email(EmailDestinationDebugInfo {
                        to: email.to.iter().map(|m| m.to_string()).collect(),
                        rendered_email,
                    })
                }
                TrackerAction::Webhook(wh) => {
                    ActionDestinationDebugInfo::Webhook(WebhookDestinationDebugInfo {
                        url: wh.url.clone(),
                        method: wh.method.clone().unwrap_or(Method::POST),
                        headers: wh.headers.clone(),
                    })
                }
                TrackerAction::ServerLog(_) => ActionDestinationDebugInfo::ServerLog {},
            };

            let Some(new_rev) = new_revision else {
                result.push(ActionDebugInfo {
                    type_tag,
                    index: i,
                    formatter: None,
                    skipped: false,
                    payload: None,
                    destination: make_destination(None),
                    duration_ms: action_start.elapsed(),
                    error: Some("Extraction failed, no revision available".to_string()),
                });
                continue;
            };

            // Run formatter if present.
            let (formatter_debug, payload_result, skipped) =
                if let Some(formatter_ref) = action.formatter() {
                    let fmt_start = Instant::now();
                    let formatter_source = match self.get_script_content(formatter_ref).await {
                        Ok(s) => s,
                        Err(err) => {
                            result.push(ActionDebugInfo {
                                type_tag,
                                index: i,
                                formatter: Some(ScriptDebugInfo {
                                    source: formatter_ref.to_string(),
                                    duration_ms: fmt_start.elapsed(),
                                    result: None,
                                    error: Some(format!("Cannot retrieve formatter script: {err}")),
                                }),
                                skipped: false,
                                payload: None,
                                destination: make_destination(None),
                                duration_ms: action_start.elapsed(),
                                error: Some(format!("Cannot retrieve formatter script: {err}")),
                            });
                            continue;
                        }
                    };

                    let new_data_value = &new_rev.data;
                    let previous_value =
                        previous_revision
                            .map(|rev| &rev.data)
                            .and_then(|previous_data_value| {
                                if new_data_value.mods().is_none() {
                                    Some(previous_data_value.original())
                                } else {
                                    previous_data_value
                                        .mods()?
                                        .get(new_data_value.mods()?.len() - 1)
                                }
                            });

                    let script_result = self
                        .execute_script::<FormatterScriptArgs, FormatterScriptResult>(
                            formatter_source.clone(),
                            FormatterScriptArgs {
                                action: action.type_tag(),
                                new_content: new_data_value.value().clone(),
                                previous_content: previous_value.cloned(),
                            },
                        )
                        .await;

                    let fmt_duration = fmt_start.elapsed();

                    match script_result {
                        Ok(Some(fmt_result)) => {
                            let result_json =
                                fmt_result.content.as_ref().map(|c| json!({"content": c}));
                            let is_skip = fmt_result.content.is_none();
                            let payload: Result<Option<JsonValue>, String> = Ok(fmt_result.content);
                            (
                                Some(ScriptDebugInfo {
                                    source: formatter_source,
                                    duration_ms: fmt_duration,
                                    result: result_json,
                                    error: None,
                                }),
                                payload,
                                is_skip,
                            )
                        }
                        Ok(None) => (
                            Some(ScriptDebugInfo {
                                source: formatter_source,
                                duration_ms: fmt_duration,
                                result: None,
                                error: None,
                            }),
                            Ok(Some(new_data_value.value().clone())),
                            false,
                        ),
                        Err(err) => (
                            Some(ScriptDebugInfo {
                                source: formatter_source,
                                duration_ms: fmt_duration,
                                result: None,
                                error: Some(format!("{err}")),
                            }),
                            Err(format!("{err}")),
                            false,
                        ),
                    }
                } else {
                    (
                        None,
                        Ok(Some(new_rev.data.value().clone())) as Result<Option<JsonValue>, String>,
                        false,
                    )
                };

            let (payload, error) = match payload_result {
                Ok(p) => (p, None),
                Err(e) => (None, Some(e)),
            };

            let rendered_email = if let TrackerAction::Email(_) = action {
                if let Some(ref payload_val) = payload {
                    let result_str = payload_val
                        .as_str()
                        .map(|s| s.to_owned())
                        .unwrap_or_else(|| payload_val.to_string());

                    match (EmailTemplate::TrackerCheckResult {
                        tracker_id: tracker.id,
                        tracker_name: tracker.name.clone(),
                        result: Ok(result_str),
                    })
                    .compile_to_email(self.api)
                    .await
                    {
                        Ok(email) => Some(RenderedEmailDebugInfo {
                            subject: email.subject,
                            text: email.text,
                            html: email.html,
                        }),
                        Err(err) => {
                            debug!("Debug: failed to render email preview: {err}");
                            None
                        }
                    }
                } else {
                    None
                }
            } else {
                None
            };

            result.push(ActionDebugInfo {
                type_tag,
                index: i,
                formatter: formatter_debug,
                skipped,
                payload,
                destination: make_destination(rendered_email),
                duration_ms: action_start.elapsed(),
                error,
            });
        }

        result
    }

    /// Attempt auto-parsing of a response body based on media type (CSV, XLS, JSON).
    fn debug_auto_parse(
        body: &[u8],
        media_type: Option<&mediatype::MediaType>,
    ) -> (Option<JsonValue>, Option<AutoParseDebugInfo>) {
        let Some(mt) = media_type else {
            let parsed = serde_json::from_slice::<JsonValue>(body).ok();
            return (parsed, None);
        };

        let essence = MediaTypeBuf::from_string(mt.essence().to_string())
            .expect("essence of a valid MediaType is always valid");
        if CsvParser::supports(mt) {
            match CsvParser::parse(body) {
                Ok(parsed) => (
                    serde_json::from_slice::<JsonValue>(&parsed).ok(),
                    Some(AutoParseDebugInfo {
                        media_type: essence,
                        success: true,
                        error: None,
                    }),
                ),
                Err(err) => {
                    let parsed = serde_json::from_slice::<JsonValue>(body).ok();
                    (
                        parsed,
                        Some(AutoParseDebugInfo {
                            media_type: essence,
                            success: false,
                            error: Some(format!("{err}")),
                        }),
                    )
                }
            }
        } else if XlsParser::supports(mt) {
            match XlsParser::parse(body) {
                Ok(parsed) => (
                    serde_json::from_slice::<JsonValue>(&parsed).ok(),
                    Some(AutoParseDebugInfo {
                        media_type: essence,
                        success: true,
                        error: None,
                    }),
                ),
                Err(err) => (
                    None,
                    Some(AutoParseDebugInfo {
                        media_type: essence,
                        success: false,
                        error: Some(format!("{err}")),
                    }),
                ),
            }
        } else {
            (serde_json::from_slice::<JsonValue>(body).ok(), None)
        }
    }

    /// Truncate a response body to fit in the debug output, returning a UTF-8 string.
    /// Invalid UTF-8 bytes are replaced with U+FFFD (�) so text-heavy responses
    /// (like HTML with a few stray bytes) remain readable instead of being hidden
    /// behind a "[binary]" placeholder.
    fn truncate_body_for_debug(body: &[u8]) -> String {
        let (slice, suffix) = if body.len() <= DEBUG_MAX_RAW_BODY_BYTES {
            (body, "")
        } else {
            (&body[..DEBUG_MAX_RAW_BODY_BYTES], "…[truncated]")
        };
        format!("{}{suffix}", String::from_utf8_lossy(slice))
    }

    /// Converts raw `body` bytes (`Vec<u8>`) into a debug-friendly JSON value:
    /// valid JSON is returned parsed, otherwise the bytes are decoded as a UTF-8 string.
    fn body_bytes_to_debug_json(body: &[u8]) -> JsonValue {
        serde_json::from_slice::<JsonValue>(body)
            .unwrap_or_else(|_| JsonValue::String(String::from_utf8_lossy(body).into_owned()))
    }

    /// Builds a debug-friendly JSON representation of a `ConfiguratorScriptResult`,
    /// converting raw byte bodies in requests/responses to readable strings or parsed JSON.
    fn configurator_result_to_debug_json(result: &ConfiguratorScriptResult) -> JsonValue {
        let mut json = serde_json::to_value(result).unwrap_or_default();
        // Replace raw byte arrays in body fields with human-readable JSON or UTF-8 strings.
        let items_key = match result {
            ConfiguratorScriptResult::Requests(_) => "requests",
            ConfiguratorScriptResult::Responses(_) => "responses",
        };
        if let Some(items) = json.get_mut(items_key).and_then(|v| v.as_array_mut()) {
            for item in items {
                if let Some(obj) = item.as_object_mut()
                    && let Some(body) = obj.get("body").and_then(|b| b.as_array()).cloned()
                {
                    let bytes: Vec<u8> = body
                        .iter()
                        .filter_map(|v| v.as_u64().map(|n| n as u8))
                        .collect();
                    obj.insert("body".into(), Self::body_bytes_to_debug_json(&bytes));
                }
            }
        }
        json
    }

    /// Constructs a new instance of the HTTP client with tracing and caching middleware.
    fn http_client(&self, accept_invalid_certificates: bool) -> ClientWithMiddleware {
        let client_builder = ClientBuilder::new(
            reqwest::Client::builder()
                .tls_danger_accept_invalid_certs(accept_invalid_certificates)
                .build()
                .expect("Failed to build http client"),
        )
        .with(TracingMiddleware::<SpanBackendWithUrl>::new());
        if let Some(ref path) = self.api.config.cache.http_cache_path {
            client_builder
                .with(Cache(HttpCache {
                    mode: CacheMode::Default,
                    manager: CACacheManager::new(path.to_path_buf(), true),
                    options: HttpCacheOptions::default(),
                }))
                .build()
        } else {
            client_builder.build()
        }
    }
}

impl<'a, DR: DnsResolver> Api<DR> {
    /// Returns an API to work with trackers.
    pub fn trackers(&'a self) -> TrackersApiExt<'a, DR> {
        TrackersApiExt::new(self)
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        config::{Config, TrackersConfig},
        error::Error as RetrackError,
        scheduler::SchedulerJob,
        tasks::{EmailContent, EmailTaskType, EmailTemplate, HttpTaskType, TaskType},
        tests::{
            TrackerCreateParamsBuilder, WebScraperBackend, WebScraperContentRequest,
            WebScraperErrorResponse, load_fixture, mock_api, mock_api_with_config,
            mock_api_with_network, mock_config, mock_network_with_records, mock_scheduler_job,
            mock_upsert_scheduler_job,
        },
    };
    use actix_web::ResponseError;
    use bytes::Bytes;
    use futures::StreamExt;
    use http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, header::CONTENT_TYPE};
    use httpmock::MockServer;
    use insta::{assert_debug_snapshot, assert_json_snapshot};
    use retrack_types::{
        scheduler::{SchedulerJobConfig, SchedulerJobRetryStrategy},
        trackers::{
            ApiTarget, EmailAction, ExtractorEngine, PageTarget, ServerLogAction, TargetRequest,
            Tracker, TrackerAction, TrackerConfig, TrackerCreateParams, TrackerDataValue,
            TrackerDebugExistingParams, TrackerDebugParams, TrackerDebugTargetResult,
            TrackerListRevisionsParams, TrackerTarget, TrackerUpdateParams, TrackersListParams,
            WebhookAction, WebhookActionPayload, WebhookActionPayloadResult,
        },
    };
    use serde_json::json;
    use sqlx::PgPool;
    use std::{collections::HashMap, iter, net::Ipv4Addr, str::FromStr, time::Duration};
    use time::OffsetDateTime;
    use trust_dns_resolver::{
        Name,
        proto::rr::{RData, Record, rdata::A},
    };
    use url::Url;
    use uuid::uuid;

    #[sqlx::test]
    async fn properly_creates_new_tracker(pool: PgPool) -> anyhow::Result<()> {
        let api = mock_api(pool).await?;
        let api = api.trackers();

        let tracker = api
            .create_tracker(
                TrackerCreateParamsBuilder::new("name_one")
                    .with_config(TrackerConfig {
                        revisions: 3,
                        timeout: Some(Duration::from_millis(2500)),
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
                    ])
                    .build(),
            )
            .await?;

        assert_eq!(tracker, api.get_tracker(tracker.id).await?.unwrap());
        assert_eq!(tracker.tags, vec!["tag".to_string()]);

        let tracker = api
            .create_tracker(
                TrackerCreateParamsBuilder::new("name_two").with_target(TrackerTarget::Api(ApiTarget {
                    requests: vec![TargetRequest {
                        url: Url::parse("https://retrack.dev")?,
                        method: Some(Method::POST),
                        headers: Some(
                            (&[(CONTENT_TYPE, "application/json".to_string())]
                                .into_iter()
                                .collect::<HashMap<_, _>>())
                                .try_into()?,
                        ),
                        body: Some(json!({ "key": "value" })),
                        media_type: Some("application/json".parse()?),
                        accept_statuses: Some([StatusCode::OK].into_iter().collect()),
                        accept_invalid_certificates: true,
                    }],
                    configurator: Some("(async () => ({ body: Deno.core.encode(JSON.stringify({ key: 'value' })) })();".to_string()),
                    extractor: Some("((context) => ({ body: Deno.core.encode(JSON.stringify({ key: 'value' })) })();".to_string()),
                    params: None,
                })).build(),
            )
            .await?;

        assert_eq!(tracker, api.get_tracker(tracker.id).await?.unwrap());

        let tracker = api
            .create_tracker(
                TrackerCreateParamsBuilder::new("name_three")
                    .with_target(TrackerTarget::Api(ApiTarget {
                        requests: vec![TargetRequest::new(
                            Url::parse("https://retrack.dev/api")?,
                        )],
                        configurator: None,
                        extractor: Some("((context) => ({ body: Deno.core.encode(JSON.stringify(context.params)) }))(context);".to_string()),
                        params: Some(json!({ "secrets": { "api_key": "s3cr3t", "token": "tok123" } })),
                    }))
                    .build(),
            )
            .await?;

        let retrieved = api.get_tracker(tracker.id).await?.unwrap();
        assert_eq!(tracker, retrieved);
        if let TrackerTarget::Api(ref api_target) = retrieved.target {
            assert_eq!(
                api_target.params,
                Some(json!({ "secrets": { "api_key": "s3cr3t", "token": "tok123" } }))
            );
        } else {
            panic!("Expected Api target");
        }

        Ok(())
    }

    #[sqlx::test]
    async fn properly_validates_tracker_at_creation(pool: PgPool) -> anyhow::Result<()> {
        let global_config = Config {
            trackers: TrackersConfig {
                restrict_to_public_urls: true,
                ..Default::default()
            },
            ..mock_config()?
        };
        let api = mock_api_with_config(pool.clone(), global_config.clone()).await?;

        let api = api.trackers();

        let target = TrackerTarget::Page(PageTarget {
            extractor: "export async function execute(p) { await p.goto('https://retrack.dev/'); return await p.content(); }".to_string(),
            params: None,
            engine: None,
            user_agent: Some("Retrack/1.0.0".to_string()),
            accept_invalid_certificates: true,
        });
        let config = TrackerConfig {
            revisions: 3,
            timeout: Some(Duration::from_millis(2500)),
            job: None,
        };
        let tags = vec!["tag".to_string()];
        let actions = vec![TrackerAction::ServerLog(Default::default())];

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
                tags: vec!["a".repeat(101)],
                actions: actions.clone()
            }).await),
            @r###""Tracker tags cannot be empty or longer than 100 characters.""###
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
            @r###""Tracker tags cannot be empty or longer than 100 characters.""###
        );

        // Too many tags.
        assert_debug_snapshot!(
            create_and_fail(api.create_tracker(TrackerCreateParams {
                name: "name".to_string(),
                enabled: true,
                target: target.clone(),
                config: config.clone(),
                tags: (0..21).map(|i| i.to_string()).collect(),
                actions: actions.clone()
            }).await),
            @r###""Tracker cannot have more than 20 tags.""###
        );

        // Too many actions.
        assert_debug_snapshot!(
            create_and_fail(api.create_tracker(TrackerCreateParams {
                name: "name".to_string(),
                enabled: false,
                target: target.clone(),
                config: config.clone(),
                tags: tags.clone(),
                actions: iter::repeat_n(TrackerAction::ServerLog(Default::default()), 11).collect()
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
                    to: vec!["".to_string()],
                    formatter: None
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
                    to: vec!["alpha-beta-gamma".to_string()],
                    formatter: None
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
                    to: vec!["dev@retrack.dev".to_string(); 11],
                    formatter: None
                })],
            }).await),
            @r###""Tracker email action cannot have more than 10 recipients.""###
        );

        // Empty email action formatter.
        assert_debug_snapshot!(
            create_and_fail(api.create_tracker(TrackerCreateParams {
                name: "name".to_string(),
                enabled: false,
                target: target.clone(),
                config: config.clone(),
                tags: tags.clone(),
                actions: vec![TrackerAction::Email(EmailAction {
                    to: vec!["dev@retrack.dev".to_string()],
                    formatter: Some("".to_string())
                })],
            }).await),
            @r###""Tracker email action formatter script cannot be empty.""###
        );

        // Very long email action formatter.
        assert_debug_snapshot!(
            create_and_fail(api.create_tracker(TrackerCreateParams {
                name: "name".to_string(),
                enabled: true,
                target: target.clone(),
                config: config.clone(),
                tags: tags.clone(),
                actions: vec![TrackerAction::Email(EmailAction {
                    to: vec!["dev@retrack.dev".to_string()],
                    formatter: Some("a".repeat(global_config.trackers.max_script_size.as_u64() as usize + 1))
                })],
            }).await),
            @r###""Tracker email action formatter script cannot be larger than 4096 bytes.""###
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
                    headers: None,
                    formatter: None
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
                    formatter: None
                })],
            }).await),
            @r###""Tracker webhook action cannot have more than 20 headers.""###
        );

        // Empty webhook action formatter.
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
                    headers: None,
                    formatter: Some("".to_string())
                })],
            }).await),
            @r###""Tracker webhook action formatter script cannot be empty.""###
        );

        // Very long webhook action formatter.
        assert_debug_snapshot!(
            create_and_fail(api.create_tracker(TrackerCreateParams {
                name: "name".to_string(),
                enabled: true,
                target: target.clone(),
                config: config.clone(),
                tags: tags.clone(),
                actions: vec![TrackerAction::Webhook(WebhookAction {
                    url: "https://retrack.dev".parse()?,
                    method: None,
                    headers: None,
                    formatter: Some("a".repeat(global_config.trackers.max_script_size.as_u64() as usize + 1))
                })],
            }).await),
            @r###""Tracker webhook action formatter script cannot be larger than 4096 bytes.""###
        );

        // Empty server log action formatter.
        assert_debug_snapshot!(
            create_and_fail(api.create_tracker(TrackerCreateParams {
                name: "name".to_string(),
                enabled: false,
                target: target.clone(),
                config: config.clone(),
                tags: tags.clone(),
                actions: vec![TrackerAction::ServerLog(ServerLogAction {
                    formatter: Some("".to_string())
                })],
            }).await),
            @r###""Tracker server log action formatter script cannot be empty.""###
        );

        // Very long server log action formatter.
        assert_debug_snapshot!(
            create_and_fail(api.create_tracker(TrackerCreateParams {
                name: "name".to_string(),
                enabled: true,
                target: target.clone(),
                config: config.clone(),
                tags: tags.clone(),
                actions: vec![TrackerAction::ServerLog(ServerLogAction {
                    formatter: Some("a".repeat(global_config.trackers.max_script_size.as_u64() as usize + 1))
                })],
            }).await),
            @r###""Tracker server log action formatter script cannot be larger than 4096 bytes.""###
        );

        // Too long timeout.
        assert_debug_snapshot!(
            create_and_fail(api.create_tracker(TrackerCreateParams {
                name: "name".to_string(),
                enabled: true,
                target: target.clone(),
                config: TrackerConfig {
                    timeout: Some(Duration::from_secs(301)),
                    ..config.clone()
                },
                tags: tags.clone(),
                actions: actions.clone()
            }).await),
            @r###""Tracker timeout cannot be greater than 300000ms.""###
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
            @r###""Tracker target extractor script cannot be empty.""###
        );

        // Very long web page target extractor.
        assert_debug_snapshot!(
            create_and_fail(api.create_tracker(TrackerCreateParams {
                name: "name".to_string(),
                enabled: true,
                target: TrackerTarget::Page(PageTarget {
                    extractor: "a".repeat(global_config.trackers.max_script_size.as_u64() as usize + 1),
                    ..Default::default()
                }),
                config: config.clone(),
                tags: tags.clone(),
                actions: actions.clone()
            }).await),
            @r###""Tracker target extractor script cannot be larger than 4096 bytes.""###
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
            @r###""Tracker target extractor script cannot be empty.""###
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
            @r###""Tracker target extractor script cannot be empty.""###
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
            @r###""Tracker schedule must be a valid cron expression, but the provided schedule (-) cannot be parsed: Invalid pattern: Pattern must have 6 or 7 fields when seconds are required and years are optional.""###
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
            @r###""Tracker retry strategy max interval cannot be less than 2m, but received 30s.""###
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

        // Too few requests.
        assert_debug_snapshot!(
            create_and_fail(api.create_tracker(TrackerCreateParams {
                name: "name".to_string(),
                enabled: true,
                target: TrackerTarget::Api(ApiTarget {
                    requests: vec![],
                    configurator: None,
                    extractor: None,
                    params: None,
                }),
                config: config.clone(),
                tags: tags.clone(),
                actions: actions.clone()
            }).await),
            @r###""Tracker target should have at least one request.""###
        );

        // Too many requests.
        assert_debug_snapshot!(
            create_and_fail(api.create_tracker(TrackerCreateParams {
                name: "name".to_string(),
                enabled: true,
                target: TrackerTarget::Api(ApiTarget {
                    requests: iter::repeat_n(TargetRequest {
                        url: "https://retrack.dev".parse()?,
                        method: None,
                        headers: None,
                        body: None,
                        media_type: None,
                        accept_statuses: None,
                        accept_invalid_certificates: false,
                    }, 11).collect::<Vec<_>>(),
                    configurator: None,
                    extractor: None,
                    params: None,
                }),
                config: config.clone(),
                tags: tags.clone(),
                actions: actions.clone()
            }).await),
            @r###""Tracker target cannot have more than 10 requests.""###
        );

        // Invalid API target URL schema.
        assert_debug_snapshot!(
            create_and_fail(api.create_tracker(TrackerCreateParams {
                name: "name".to_string(),
                enabled: true,
                target: TrackerTarget::Api(ApiTarget {
                    requests: vec![TargetRequest {
                        url:"ftp://retrack.dev".parse()?,
                        method: None,
                        headers: None,
                        body: None,
                        media_type: None,
                        accept_statuses: None,
                        accept_invalid_certificates: false,
                    }],
                    configurator: None,
                    extractor: None,
                    params: None,
                }),
                config: config.clone(),
                tags: tags.clone(),
                actions: actions.clone()
            }).await),
            @r###""Tracker target URL must be either `http` or `https` and have a valid public reachable domain name, but received ftp://retrack.dev/.""###
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

        // Non-public API target URL.
        assert_debug_snapshot!(
            create_and_fail(api_with_local_network.trackers().create_tracker(TrackerCreateParams {
                name: "name".to_string(),
                enabled: true,
                target: TrackerTarget::Api(ApiTarget {
                    requests: vec![TargetRequest {
                        url:"https://127.0.0.1".parse()?,
                        method: None,
                        headers: None,
                        body: None,
                        media_type: None,
                        accept_statuses: None,
                        accept_invalid_certificates: false,
                    }],
                    configurator: None,
                    extractor: None,
                    params: None,
                }),
                config: config.clone(),
                tags: tags.clone(),
                actions: actions.clone()
            }).await),
            @r###""Tracker target URL must be either `http` or `https` and have a valid public reachable domain name, but received https://127.0.0.1/.""###
        );

        // Empty API target configurator.
        assert_debug_snapshot!(
            create_and_fail(api.create_tracker(TrackerCreateParams {
                name: "name".to_string(),
                enabled: false,
                target: TrackerTarget::Api(ApiTarget {
                    requests: vec![TargetRequest {
                        url:"https://retrack.dev".parse()?,
                        method: None,
                        headers: None,
                        body: None,
                        media_type: None,
                        accept_statuses: None,
                        accept_invalid_certificates: false,
                    }],
                    configurator: Some("".to_string()),
                    extractor: None,
                    params: None,
                }),
                config: config.clone(),
                tags: tags.clone(),
                actions: actions.clone()
            }).await),
            @r###""Tracker target configurator script cannot be empty.""###
        );

        // Very long API target configurator.
        assert_debug_snapshot!(
            create_and_fail(api.create_tracker(TrackerCreateParams {
                name: "name".to_string(),
                enabled: true,
                target: TrackerTarget::Api(ApiTarget {
                    requests: vec![TargetRequest {
                        url:"https://retrack.dev".parse()?,
                        method: None,
                        headers: None,
                        body: None,
                        media_type: None,
                        accept_statuses: None,
                        accept_invalid_certificates: false,
                    }],
                    configurator: Some(
                        "a".repeat(global_config.trackers.max_script_size.as_u64() as usize + 1)
                    ),
                    extractor: None,
                    params: None,
                }),
                config: config.clone(),
                tags: tags.clone(),
                actions: actions.clone()
            }).await),
            @r###""Tracker target configurator script cannot be larger than 4096 bytes.""###
        );

        // Empty API target extractor.
        assert_debug_snapshot!(
            create_and_fail(api.create_tracker(TrackerCreateParams {
                name: "name".to_string(),
                enabled: false,
                target: TrackerTarget::Api(ApiTarget {
                    requests: vec![TargetRequest {
                        url:"https://retrack.dev".parse()?,
                        method: None,
                        headers: None,
                        body: None,
                        media_type: None,
                        accept_statuses: None,
                        accept_invalid_certificates: false,
                    }],
                    configurator: None,
                    extractor: Some("".to_string()),
                    params: None,
                }),
                config: config.clone(),
                tags: tags.clone(),
                actions: actions.clone()
            }).await),
            @r###""Tracker target extractor script cannot be empty.""###
        );

        // Very long API target extractor.
        assert_debug_snapshot!(
            create_and_fail(api.create_tracker(TrackerCreateParams {
                name: "name".to_string(),
                enabled: true,
                target: TrackerTarget::Api(ApiTarget {
                    requests: vec![TargetRequest {
                        url:"https://retrack.dev".parse()?,
                        method: None,
                        headers: None,
                        body: None,
                        media_type: None,
                        accept_statuses: None,
                        accept_invalid_certificates: false,
                    }],
                    configurator: None,
                    extractor: Some(
                        "a".repeat(global_config.trackers.max_script_size.as_u64() as usize + 1)
                    ),
                    params: None,
                }),
                config: config.clone(),
                tags: tags.clone(),
                actions: actions.clone()
            }).await),
            @r###""Tracker target extractor script cannot be larger than 4096 bytes.""###
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
                    params: None,
                    engine: None,
                    user_agent: Some("Retrack/1.0.0".to_string()),
                    accept_invalid_certificates: true,
                }),
                config: TrackerConfig {
                    revisions: 3,
                    timeout: Some(Duration::from_millis(2500)),
                    job: None,
                },
                tags: vec!["tag".to_string()],
                actions: vec![TrackerAction::ServerLog(Default::default())],
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
        let global_config = Config {
            trackers: TrackersConfig {
                restrict_to_public_urls: true,
                ..Default::default()
            },
            ..mock_config()?
        };
        let api = mock_api_with_config(pool.clone(), global_config.clone()).await?;

        let trackers = api.trackers();
        let tracker = trackers
            .create_tracker(
                TrackerCreateParamsBuilder::new("name_one")
                    .with_config(TrackerConfig {
                        timeout: Some(Duration::from_millis(2500)),
                        job: Some(SchedulerJobConfig {
                            schedule: "@hourly".to_string(),
                            retry_strategy: Some(SchedulerJobRetryStrategy::Constant {
                                interval: Duration::from_secs(120),
                                max_attempts: 5,
                            }),
                        }),
                        ..Default::default()
                    })
                    .build(),
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
                tags: Some(vec!["a".repeat(101)]),
                ..Default::default()
            }).await),
            @r###""Tracker tags cannot be empty or longer than 100 characters.""###
        );

        // Empty tag.
        assert_debug_snapshot!(
            update_and_fail(trackers.update_tracker(tracker.id, TrackerUpdateParams {
                tags: Some(vec!["tag".to_string(), "".to_string()]),
                ..Default::default()
            }).await),
            @r###""Tracker tags cannot be empty or longer than 100 characters.""###
        );

        // Too many tags.
        assert_debug_snapshot!(
            update_and_fail(trackers.update_tracker(tracker.id, TrackerUpdateParams {
                tags: Some((0..21).map(|i| i.to_string()).collect()),
                ..Default::default()
            }).await),
            @r###""Tracker cannot have more than 20 tags.""###
        );

        // Too many actions.
        assert_debug_snapshot!(
            update_and_fail(trackers.update_tracker(tracker.id, TrackerUpdateParams {
                actions: Some(
                    iter::repeat_n(TrackerAction::ServerLog(Default::default()), 11).collect()
                ),
                ..Default::default()
            }).await),
            @r###""Tracker cannot have more than 10 actions.""###
        );

        // Empty email action recipient.
        assert_debug_snapshot!(
            update_and_fail(trackers.update_tracker(tracker.id, TrackerUpdateParams {
                actions: Some(vec![TrackerAction::Email(EmailAction {
                    to: vec!["".to_string()],
                    formatter: None
                })]),
                ..Default::default()
            }).await),
             @r###""Tracker email action recipient ('') is not a valid email address.""###
        );

        // Invalid email action recipient.
        assert_debug_snapshot!(
            update_and_fail(trackers.update_tracker(tracker.id, TrackerUpdateParams {
                actions: Some(vec![TrackerAction::Email(EmailAction {
                    to: vec!["alpha-beta-gamma".to_string()],
                    formatter: None
                })]),
                ..Default::default()
            }).await),
            @r###""Tracker email action recipient ('alpha-beta-gamma') is not a valid email address.""###
        );

        // Too many email action recipients.
        assert_debug_snapshot!(
            update_and_fail(trackers.update_tracker(tracker.id, TrackerUpdateParams {
                actions: Some(vec![TrackerAction::Email(EmailAction {
                    to: vec!["dev@retrack.dev".to_string(); 11],
                    formatter: None
                })]),
                ..Default::default()
            }).await),
            @r###""Tracker email action cannot have more than 10 recipients.""###
        );

        // Empty email action formatter.
        assert_debug_snapshot!(
            update_and_fail(trackers.update_tracker(tracker.id, TrackerUpdateParams {
                actions: Some(vec![TrackerAction::Email(EmailAction {
                    to: vec!["dev@retrack.dev".to_string()],
                    formatter: Some("".to_string())
                })]),
                ..Default::default()
            }).await),
            @r###""Tracker email action formatter script cannot be empty.""###
        );

        // Very long email action formatter.
        assert_debug_snapshot!(
            update_and_fail(trackers.update_tracker(tracker.id, TrackerUpdateParams {
                actions: Some(vec![TrackerAction::Email(EmailAction {
                    to: vec!["dev@retrack.dev".to_string()],
                    formatter: Some("a".repeat(global_config.trackers.max_script_size.as_u64() as usize + 1))
                })]),
                ..Default::default()
            }).await),
            @r###""Tracker email action formatter script cannot be larger than 4096 bytes.""###
        );

        // Invalid webhook action method.
        assert_debug_snapshot!(
            update_and_fail(trackers.update_tracker(tracker.id, TrackerUpdateParams {
                actions: Some(vec![TrackerAction::Webhook(WebhookAction {
                    url: "https://retrack.dev".parse()?,
                    method: Some(Method::PATCH),
                    headers: None,
                    formatter: None
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
                    formatter: None
                })]),
                ..Default::default()
            }).await),
            @r###""Tracker webhook action cannot have more than 20 headers.""###
        );

        // Empty webhook action formatter.
        assert_debug_snapshot!(
            update_and_fail(trackers.update_tracker(tracker.id, TrackerUpdateParams {
                actions: Some(vec![TrackerAction::Webhook(WebhookAction {
                    url: "https://retrack.dev".parse()?,
                    method: None,
                    headers: None,
                    formatter: Some("".to_string())
                })]),
                ..Default::default()
            }).await),
            @r###""Tracker webhook action formatter script cannot be empty.""###
        );

        // Very long webhook action formatter.
        assert_debug_snapshot!(
            update_and_fail(trackers.update_tracker(tracker.id, TrackerUpdateParams {
                actions: Some(vec![TrackerAction::Webhook(WebhookAction {
                    url: "https://retrack.dev".parse()?,
                    method: None,
                    headers: None,
                    formatter: Some("a".repeat(global_config.trackers.max_script_size.as_u64() as usize + 1))
                })]),
                ..Default::default()
            }).await),
            @r###""Tracker webhook action formatter script cannot be larger than 4096 bytes.""###
        );

        // Empty server log action formatter.
        assert_debug_snapshot!(
            update_and_fail(trackers.update_tracker(tracker.id, TrackerUpdateParams {
                actions: Some(vec![TrackerAction::ServerLog(ServerLogAction {
                    formatter: Some("".to_string())
                })]),
                ..Default::default()
            }).await),
            @r###""Tracker server log action formatter script cannot be empty.""###
        );

        // Very long server log action formatter.
        assert_debug_snapshot!(
            update_and_fail(trackers.update_tracker(tracker.id, TrackerUpdateParams {
                actions: Some(vec![TrackerAction::ServerLog(ServerLogAction {
                    formatter: Some("a".repeat(global_config.trackers.max_script_size.as_u64() as usize + 1))
                })]),
                ..Default::default()
            }).await),
            @r###""Tracker server log action formatter script cannot be larger than 4096 bytes.""###
        );

        // Too long timeout.
        assert_debug_snapshot!(
            update_and_fail(trackers.update_tracker(tracker.id, TrackerUpdateParams {
                config: Some(TrackerConfig {
                    timeout: Some(Duration::from_secs(301)),
                    ..tracker.config.clone()
                }),
                ..Default::default()
            }).await),
            @r###""Tracker timeout cannot be greater than 300000ms.""###
        );

        // Empty web page target extractor.
        assert_debug_snapshot!(
            update_and_fail(trackers.update_tracker(tracker.id, TrackerUpdateParams {
                target: Some(TrackerTarget::Page(PageTarget {
                    extractor: "".to_string(),
                    params: None,
                    engine: None,
                    user_agent: None,
                    accept_invalid_certificates: false
                })),
                ..Default::default()
            }).await),
            @r###""Tracker target extractor script cannot be empty.""###
        );

        // Very long web page target extractor.
        assert_debug_snapshot!(
            update_and_fail(trackers.update_tracker(tracker.id, TrackerUpdateParams {
                target: Some(TrackerTarget::Page(PageTarget {
                    extractor: "a".repeat(global_config.trackers.max_script_size.as_u64() as usize + 1),
                    params: None,
                    engine: None,
                    user_agent: None,
                    accept_invalid_certificates: false
                })),
                ..Default::default()
            }).await),
            @r###""Tracker target extractor script cannot be larger than 4096 bytes.""###
        );

        // Empty web page target user agent.
        assert_debug_snapshot!(
            update_and_fail(trackers.update_tracker(tracker.id, TrackerUpdateParams {
                target: Some(TrackerTarget::Page(PageTarget {
                    extractor: "export async function execute(p) { await p.goto('https://retrack.dev/'); return await p.content(); }".to_string(),
                    params: None,
                    engine: None,
                    user_agent: Some("".to_string()),
                    accept_invalid_certificates: false,
                })),
                ..Default::default()
            }).await),
            @r###""Tracker target user-agent header cannot be empty.""###
        );

        // Very long web page target user agent.
        assert_debug_snapshot!(
            update_and_fail(trackers.update_tracker(tracker.id, TrackerUpdateParams {
                target: Some(TrackerTarget::Page(PageTarget {
                    extractor: "export async function execute(p) { await p.goto('https://retrack.dev/'); return await p.content(); }".to_string(),
                    params: None,
                    engine: None,
                    user_agent: Some("a".repeat(201)),
                    accept_invalid_certificates: false,
                })),
                ..Default::default()
            }).await),
            @r###""Tracker target user-agent cannot be longer than 200 characters.""###
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
            @r###""Tracker schedule must be a valid cron expression, but the provided schedule (-) cannot be parsed: Invalid pattern: Pattern must have 6 or 7 fields when seconds are required and years are optional.""###
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
            @r###""Tracker retry strategy max interval cannot be less than 2m, but received 30s.""###
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

        // Too few requests.
        assert_debug_snapshot!(
            update_and_fail(trackers.update_tracker(tracker.id, TrackerUpdateParams {
                target: Some(TrackerTarget::Api(ApiTarget {
                    requests: vec![],
                    configurator: None,
                    extractor: None,
                    params: None,
                })),
                ..Default::default()
            }).await),
            @r###""Tracker target should have at least one request.""###
        );

        // Too many requests.
        assert_debug_snapshot!(
            update_and_fail(trackers.update_tracker(tracker.id, TrackerUpdateParams {
                target: Some(TrackerTarget::Api(ApiTarget {
                    requests: iter::repeat_n(TargetRequest {
                        url: "https://retrack.dev".parse()?,
                        method: None,
                        headers: None,
                        body: None,
                        media_type: None,
                        accept_statuses: None,
                        accept_invalid_certificates: false,
                    }, 11).collect::<Vec<_>>(),
                    configurator: None,
                    extractor: None,
                    params: None,
                })),
                ..Default::default()
            }).await),
            @r###""Tracker target cannot have more than 10 requests.""###
        );

        // Invalid API target URL schema.
        assert_debug_snapshot!(
            update_and_fail(trackers.update_tracker(tracker.id, TrackerUpdateParams {
                target: Some(TrackerTarget::Api(ApiTarget {
                    requests: vec![TargetRequest {
                        url:"ftp://retrack.dev".parse()?,
                        method: None,
                        headers: None,
                        body: None,
                        media_type: None,
                        accept_statuses: None,
                        accept_invalid_certificates: false,
                    }],
                    configurator: None,
                    extractor: None,
                    params: None,
                })),
                ..Default::default()
            }).await),
            @r###""Tracker target URL must be either `http` or `https` and have a valid public reachable domain name, but received ftp://retrack.dev/.""###
        );

        // Empty API target configurator.
        assert_debug_snapshot!(
            update_and_fail(trackers.update_tracker(tracker.id, TrackerUpdateParams {
                target: Some(TrackerTarget::Api(ApiTarget {
                    requests: vec![TargetRequest {
                        url:"https://retrack.dev".parse()?,
                        method: None,
                        headers: None,
                        body: None,
                        media_type: None,
                        accept_statuses: None,
                        accept_invalid_certificates: false,
                    }],
                    configurator: Some("".to_string()),
                    extractor: None,
                    params: None,
                })),
                ..Default::default()
            }).await),
            @r###""Tracker target configurator script cannot be empty.""###
        );

        // Very long API target configurator.
        assert_debug_snapshot!(
            update_and_fail(trackers.update_tracker(tracker.id, TrackerUpdateParams {
                target: Some(TrackerTarget::Api(ApiTarget {
                    requests: vec![TargetRequest {
                        url:"https://retrack.dev".parse()?,
                        method: None,
                        headers: None,
                        body: None,
                        media_type: None,
                        accept_statuses: None,
                        accept_invalid_certificates: false,
                    }],
                    configurator: Some(
                        "a".repeat(global_config.trackers.max_script_size.as_u64() as usize + 1)
                    ),
                    extractor: None,
                    params: None,
                })),
                ..Default::default()
            }).await),
            @r###""Tracker target configurator script cannot be larger than 4096 bytes.""###
        );

        // Empty API target extractor.
        assert_debug_snapshot!(
            update_and_fail(trackers.update_tracker(tracker.id, TrackerUpdateParams {
                target: Some(TrackerTarget::Api(ApiTarget {
                    requests: vec![TargetRequest {
                        url:"https://retrack.dev".parse()?,
                        method: None,
                        headers: None,
                        body: None,
                        media_type: None,
                        accept_statuses: None,
                        accept_invalid_certificates: false,
                    }],
                    configurator: None,
                    extractor: Some("".to_string()),
                    params: None,
                })),
                ..Default::default()
            }).await),
            @r###""Tracker target extractor script cannot be empty.""###
        );

        // Very long API target extractor.
        assert_debug_snapshot!(
            update_and_fail(trackers.update_tracker(tracker.id, TrackerUpdateParams {
                target: Some(TrackerTarget::Api(ApiTarget {
                    requests: vec![TargetRequest {
                        url:"https://retrack.dev".parse()?,
                        method: None,
                        headers: None,
                        body: None,
                        media_type: None,
                        accept_statuses: None,
                        accept_invalid_certificates: false,
                    }],
                    configurator: None,
                    extractor: Some(
                        "a".repeat(global_config.trackers.max_script_size.as_u64() as usize + 1)
                    ),
                    params: None,
                })),
                ..Default::default()
            }).await),
            @r###""Tracker target extractor script cannot be larger than 4096 bytes.""###
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
                    requests: vec![TargetRequest {
                        url:"https://127.0.0.1".parse()?,
                        method: None,
                        headers: None,
                        body: None,
                        media_type: None,
                        accept_statuses: None,
                        accept_invalid_certificates: false,
                    }],
                    configurator: None,
                    extractor: None,
                    params: None,
                })),
                ..Default::default()
            }).await),
            @r###""Tracker target URL must be either `http` or `https` and have a valid public reachable domain name, but received https://127.0.0.1/.""###
        );

        Ok(())
    }

    #[sqlx::test]
    async fn properly_updates_tracker_job_id_at_update(pool: PgPool) -> anyhow::Result<()> {
        let api = mock_api(pool).await?;

        let trackers = api.trackers();
        let tracker = trackers
            .create_tracker(
                TrackerCreateParamsBuilder::new("name_one")
                    .with_schedule("0 0 * * * *")
                    .build(),
            )
            .await?;

        // Set job ID.
        api.trackers()
            .set_tracker_job(tracker.id, uuid!("00000000-0000-0000-0000-000000000001"))
            .await?;
        assert_eq!(
            Some(uuid!("00000000-0000-0000-0000-000000000001")),
            trackers.get_tracker(tracker.id).await?.unwrap().job_id
        );

        // Clears job ID.
        api.trackers().clear_tracker_job(tracker.id).await?;
        assert!(
            trackers
                .get_tracker(tracker.id)
                .await?
                .unwrap()
                .job_id
                .is_none()
        );

        // Set job ID.
        api.trackers()
            .set_tracker_job(tracker.id, uuid!("00000000-0000-0000-0000-000000000001"))
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
    async fn properly_removes_tracker(pool: PgPool) -> anyhow::Result<()> {
        let api = mock_api(pool).await?;

        let trackers = api.trackers();
        let tracker_one = trackers
            .create_tracker(
                TrackerCreateParamsBuilder::new("name_one")
                    .with_schedule("0 0 * * * *")
                    .build(),
            )
            .await?;
        let tracker_two = trackers
            .create_tracker(
                TrackerCreateParamsBuilder::new("name_two")
                    .with_schedule("0 0 * * * *")
                    .build(),
            )
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
        assert!(
            trackers
                .get_tracker(uuid!("00000000-0000-0000-0000-000000000001"))
                .await?
                .is_none()
        );

        let tracker_one = trackers
            .create_tracker(
                TrackerCreateParamsBuilder::new("name_one")
                    .with_schedule("0 0 * * * *")
                    .build(),
            )
            .await?;
        assert_eq!(
            trackers.get_tracker(tracker_one.id).await?,
            Some(tracker_one.clone()),
        );

        let tracker_two = trackers
            .create_tracker(
                TrackerCreateParamsBuilder::new("name_two")
                    .with_schedule("0 0 * * * *")
                    .build(),
            )
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
                TrackerCreateParamsBuilder::new("name_one")
                    .with_schedule("0 0 * * * *")
                    .with_tags(vec!["tag:1".to_string(), "tag:common".to_string()])
                    .with_actions(vec![
                        TrackerAction::ServerLog(Default::default()),
                        TrackerAction::Email(EmailAction {
                            to: vec!["dev@retrack.dev".to_string()],
                            formatter: None,
                        }),
                    ])
                    .build(),
            )
            .await?;
        assert_eq!(
            trackers.get_trackers(Default::default()).await?,
            vec![tracker_one.clone()],
        );
        let tracker_two = trackers
            .create_tracker(
                TrackerCreateParamsBuilder::new("name_two")
                    .with_schedule("0 0 * * * *")
                    .with_tags(vec!["tag:2".to_string(), "tag:common".to_string()])
                    .with_actions(vec![
                        TrackerAction::ServerLog(Default::default()),
                        TrackerAction::Email(EmailAction {
                            to: vec!["dev@retrack.dev".to_string()],
                            formatter: None,
                        }),
                    ])
                    .build(),
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
        assert!(
            trackers
                .get_trackers(TrackersListParams {
                    tags: vec!["tag:unknown".to_string(), "tag:common".to_string()]
                })
                .await?
                .is_empty()
        );

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
                tags: vec!["a".repeat(101)]
            }).await),
            @r###""Tracker tags cannot be empty or longer than 100 characters.""###
        );

        // Empty tag.
        assert_debug_snapshot!(
            list_and_fail(api.get_trackers(TrackersListParams {
                tags: vec!["tag".to_string(), "".to_string()]
            }).await),
            @r###""Tracker tags cannot be empty or longer than 100 characters.""###
        );

        // Too many tags.
        assert_debug_snapshot!(
            list_and_fail(api.get_trackers(TrackersListParams {
                tags: (0..21).map(|i| i.to_string()).collect()
            }).await),
            @r###""Trackers filter params cannot use more than 20 tags.""###
        );

        Ok(())
    }

    #[sqlx::test]
    async fn properly_removes_all_trackers(pool: PgPool) -> anyhow::Result<()> {
        let api = mock_api(pool).await?;

        let trackers = api.trackers();

        let tracker_one = trackers
            .create_tracker(
                TrackerCreateParamsBuilder::new("name_one")
                    .with_schedule("0 0 * * * *")
                    .with_tags(vec!["tag:1".to_string(), "tag:common".to_string()])
                    .with_actions(vec![
                        TrackerAction::ServerLog(Default::default()),
                        TrackerAction::Email(EmailAction {
                            to: vec!["dev@retrack.dev".to_string()],
                            formatter: None,
                        }),
                    ])
                    .build(),
            )
            .await?;
        let tracker_two = trackers
            .create_tracker(
                TrackerCreateParamsBuilder::new("name_two")
                    .with_schedule("0 0 * * * *")
                    .with_tags(vec!["tag:2".to_string(), "tag:common".to_string()])
                    .with_actions(vec![
                        TrackerAction::ServerLog(Default::default()),
                        TrackerAction::Email(EmailAction {
                            to: vec!["dev@retrack.dev".to_string()],
                            formatter: None,
                        }),
                    ])
                    .build(),
            )
            .await?;

        assert_eq!(
            trackers.get_trackers(Default::default()).await?,
            vec![tracker_one.clone(), tracker_two.clone()],
        );
        assert_eq!(
            trackers
                .remove_trackers(TrackersListParams {
                    tags: vec!["tag:2".to_string()]
                })
                .await?,
            1
        );
        assert_eq!(
            trackers.get_trackers(TrackersListParams::default()).await?,
            vec![tracker_one.clone()],
        );
        assert_eq!(
            trackers
                .remove_trackers(TrackersListParams {
                    tags: vec!["tag:1".to_string(), "tag:common".to_string()]
                })
                .await?,
            1
        );
        assert!(
            trackers
                .get_trackers(TrackersListParams::default())
                .await?
                .is_empty()
        );

        Ok(())
    }

    #[sqlx::test]
    async fn properly_saves_page_target_revision(pool: PgPool) -> anyhow::Result<()> {
        let server = MockServer::start();
        let mut config = mock_config()?;
        config.components.web_scraper_url = Url::parse(&server.base_url())?;

        let api = mock_api_with_config(pool, config).await?;

        let trackers = api.trackers();
        let tracker_one = trackers
            .create_tracker(
                TrackerCreateParamsBuilder::new("name_one")
                    .with_schedule("0 0 * * * *")
                    .with_tags(vec!["tag:1".to_string(), "tag:common".to_string()])
                    .build(),
            )
            .await?;
        let tracker_two = trackers
            .create_tracker(
                TrackerCreateParamsBuilder::new("name_two")
                    .with_schedule("0 0 * * * *")
                    .build(),
            )
            .await?;

        let tracker_one_data = trackers
            .get_tracker_data_revisions(tracker_one.id, Default::default())
            .await?;
        let tracker_two_data = trackers
            .get_tracker_data_revisions(tracker_two.id, Default::default())
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
                .json_body(json!({ "result": content_one.value() }));
        });

        trackers
            .create_tracker_data_revision(tracker_one.id)
            .await?;
        let tracker_one_data = trackers
            .get_tracker_data_revisions(tracker_one.id, Default::default())
            .await?;
        let tracker_two_data = trackers
            .get_tracker_data_revisions(tracker_two.id, Default::default())
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
                            .set_previous_content(&TrackerDataValue::new(json!("\"rev_1\""))),
                    )
                    .unwrap(),
                );
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body(json!({ "result": content_two.value() }));
        });

        let revision = trackers
            .create_tracker_data_revision(tracker_one.id)
            .await?;
        assert_eq!(revision.data.value(), "\"rev_2\"");
        content_mock.assert();
        content_mock.delete();

        let tracker_one_data = trackers
            .get_tracker_data_revisions(tracker_one.id, Default::default())
            .await?;
        let tracker_two_data = trackers
            .get_tracker_data_revisions(tracker_two.id, Default::default())
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
                .json_body(json!({ "result": content_two.value() }));
        });
        let revision = trackers
            .create_tracker_data_revision(tracker_two.id)
            .await?;
        assert_eq!(revision.data.value(), "\"rev_3\"");
        content_mock.assert();

        let tracker_one_data = trackers
            .get_tracker_data_revisions(
                tracker_one.id,
                TrackerListRevisionsParams {
                    calculate_diff: true,
                    ..Default::default()
                },
            )
            .await?;
        let tracker_two_data = trackers
            .get_tracker_data_revisions(tracker_two.id, Default::default())
            .await?;
        assert_eq!(tracker_one_data.len(), 2);
        assert_eq!(tracker_two_data.len(), 1);

        assert_debug_snapshot!(
            tracker_one_data.into_iter().map(|rev| rev.data).collect::<Vec<_>>(),
            @r###"
        [
            TrackerDataValue {
                original: String("@@ -1 +1 @@\n-\"rev_1\"\n+\"rev_2\"\n"),
                mods: None,
            },
            TrackerDataValue {
                original: String("\"rev_1\""),
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
            .get_tracker_data_revisions(tracker_one.id, Default::default())
            .await?;
        assert_debug_snapshot!(
            tracker_one_data.into_iter().map(|rev| rev.data).collect::<Vec<_>>(),
            @r###"
        [
            TrackerDataValue {
                original: String("\"rev_2\""),
                mods: None,
            },
            TrackerDataValue {
                original: String("\"rev_1\""),
                mods: None,
            },
        ]
        "###
        );

        Ok(())
    }

    #[sqlx::test]
    async fn properly_specifies_backend_for_page_target_revision(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        let server = MockServer::start();
        let mut config = mock_config()?;
        config.components.web_scraper_url = Url::parse(&server.base_url())?;

        let api = mock_api_with_config(pool, config).await?;

        let trackers = api.trackers();

        // Default engine.
        let tracker = trackers
            .create_tracker(
                TrackerCreateParamsBuilder::new("name_one")
                    .with_target(TrackerTarget::Page(PageTarget {
                        extractor: "some-script".to_string(),
                        engine: None,
                        ..Default::default()
                    }))
                    .build(),
            )
            .await?;

        let web_scraper_request = WebScraperContentRequest::try_from(&tracker)?;
        assert_eq!(
            web_scraper_request.extractor_backend,
            Some(WebScraperBackend::Chromium)
        );
        let mut content_mock = server.mock(|when, then| {
            when.method(httpmock::Method::POST)
                .path("/api/web_page/execute")
                .json_body(serde_json::to_value(web_scraper_request).unwrap());
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body(json!({ "result": "\"rev_1\"" }));
        });

        trackers.create_tracker_data_revision(tracker.id).await?;
        content_mock.assert();
        content_mock.delete();

        // Chromium engine.
        let tracker = trackers
            .create_tracker(
                TrackerCreateParamsBuilder::new("name_two")
                    .with_target(TrackerTarget::Page(PageTarget {
                        extractor: "some-script".to_string(),
                        engine: Some(ExtractorEngine::Chromium),
                        ..Default::default()
                    }))
                    .build(),
            )
            .await?;

        let web_scraper_request = WebScraperContentRequest::try_from(&tracker)?;
        assert_eq!(
            web_scraper_request.extractor_backend,
            Some(WebScraperBackend::Chromium)
        );
        let mut content_mock = server.mock(|when, then| {
            when.method(httpmock::Method::POST)
                .path("/api/web_page/execute")
                .json_body(serde_json::to_value(web_scraper_request).unwrap());
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body(json!({ "result": "\"rev_2\"" }));
        });

        trackers.create_tracker_data_revision(tracker.id).await?;
        content_mock.assert();
        content_mock.delete();

        // Camoufox engine.
        let tracker = trackers
            .create_tracker(
                TrackerCreateParamsBuilder::new("name_three")
                    .with_target(TrackerTarget::Page(PageTarget {
                        extractor: "some-script".to_string(),
                        engine: Some(ExtractorEngine::Camoufox),
                        ..Default::default()
                    }))
                    .build(),
            )
            .await?;
        let web_scraper_request = WebScraperContentRequest::try_from(&tracker)?;
        assert_eq!(
            web_scraper_request.extractor_backend,
            Some(WebScraperBackend::Firefox)
        );
        let content_mock = server.mock(|when, then| {
            when.method(httpmock::Method::POST)
                .path("/api/web_page/execute")
                .json_body(serde_json::to_value(web_scraper_request).unwrap());
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body(json!({ "result": "\"rev_3\"" }));
        });

        trackers.create_tracker_data_revision(tracker.id).await?;
        content_mock.assert();

        Ok(())
    }

    #[sqlx::test]
    async fn properly_saves_api_target_revision(pool: PgPool) -> anyhow::Result<()> {
        let server = MockServer::start();
        let config = mock_config()?;

        let api = mock_api_with_config(pool, config).await?;

        let trackers = api.trackers();
        let tracker_one = trackers
            .create_tracker(
                TrackerCreateParamsBuilder::new("name_one")
                    .with_schedule("0 0 * * * *")
                    .with_target(TrackerTarget::Api(ApiTarget {
                        requests: vec![TargetRequest {
                            url: server.url("/api/get-call").parse()?,
                            method: None,
                            headers: Some(HeaderMap::from_iter([(
                                HeaderName::from_static("x-custom-header"),
                                HeaderValue::from_static("x-custom-value"),
                            )])),
                            body: None,
                            media_type: Some("application/json".parse()?),
                            accept_statuses: Some([StatusCode::OK].into_iter().collect()),
                            accept_invalid_certificates: true,
                        }],
                        configurator: None,
                        extractor: None,
                        params: None,
                    }))
                    .build(),
            )
            .await?;
        let tracker_two = trackers
            .create_tracker(
                TrackerCreateParamsBuilder::new("name_two")
                    .with_schedule("0 0 * * * *")
                    .with_target(TrackerTarget::Api(ApiTarget {
                        requests: vec![TargetRequest {
                            url: server.url("/api/get-call").parse()?,
                            method: None,
                            headers: None,
                            body: Some(json!({ "key": "value" })),
                            media_type: Some("application/json".parse()?),
                            accept_statuses: Some([StatusCode::OK].into_iter().collect()),
                            accept_invalid_certificates: true
                        }],
                        configurator: Some(format!("((context) => ({{ requests: [{{ url: '{}', method: 'POST', headers: {{ 'x-custom-header': 'x-custom-value' }}, body: Deno.core.encode(JSON.stringify({{ key: `overridden-${{JSON.parse(Deno.core.decode(context.requests[0].body)).key}}` }})) }}] }}))(context);", server.url("/api/post-call"))),
                        extractor: None,
                        params: None,
                    })).build(),
            )
            .await?;

        let tracker_one_data = trackers
            .get_tracker_data_revisions(tracker_one.id, Default::default())
            .await?;
        let tracker_two_data = trackers
            .get_tracker_data_revisions(tracker_two.id, Default::default())
            .await?;
        assert!(tracker_one_data.is_empty());
        assert!(tracker_two_data.is_empty());

        let content_one = TrackerDataValue::new(json!("\"rev_1\""));
        let mut content_mock = server.mock(|when, then| {
            when.method(httpmock::Method::GET).path("/api/get-call");
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body_obj(content_one.value());
        });

        let revision_one = trackers
            .create_tracker_data_revision(tracker_one.id)
            .await?;
        let tracker_one_data = trackers
            .get_tracker_data_revisions(tracker_one.id, Default::default())
            .await?;
        let tracker_two_data = trackers
            .get_tracker_data_revisions(tracker_two.id, Default::default())
            .await?;
        assert_eq!(tracker_one_data.len(), 1);
        assert_eq!(tracker_one_data[0].tracker_id, tracker_one.id);
        assert_eq!(tracker_one_data[0].data, content_one);
        assert!(tracker_two_data.is_empty());

        content_mock.assert();
        content_mock.delete();

        let content_two = TrackerDataValue::new(json!("\"rev_2\""));
        let mut content_mock = server.mock(|when, then| {
            when.method(httpmock::Method::GET).path("/api/get-call");
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body_obj(content_two.value());
        });

        let revision_two = trackers
            .create_tracker_data_revision(tracker_one.id)
            .await?;
        assert_eq!(revision_two.data.value(), "\"rev_2\"");
        assert_ne!(revision_one, revision_two);
        content_mock.assert();
        content_mock.delete();

        let tracker_one_data = trackers
            .get_tracker_data_revisions(tracker_one.id, Default::default())
            .await?;
        let tracker_two_data = trackers
            .get_tracker_data_revisions(tracker_two.id, Default::default())
            .await?;
        assert_eq!(tracker_one_data.len(), 2);
        assert!(tracker_two_data.is_empty());

        let content_two = TrackerDataValue::new(json!("\"rev_3\""));
        let content_mock = server.mock(|when, then| {
            when.method(httpmock::Method::POST)
                .path("/api/post-call")
                .header("x-custom-header", "x-custom-value")
                // Make sure "configurator" script managed to override body.
                .json_body(json!({ "key": "overridden-value" }));
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body_obj(content_two.value());
        });

        let revision = trackers
            .create_tracker_data_revision(tracker_two.id)
            .await?;
        assert_eq!(revision.data.value(), "\"rev_3\"");

        content_mock.assert();

        let tracker_one_data = trackers
            .get_tracker_data_revisions(
                tracker_one.id,
                TrackerListRevisionsParams {
                    calculate_diff: true,
                    ..Default::default()
                },
            )
            .await?;
        let tracker_two_data = trackers
            .get_tracker_data_revisions(tracker_two.id, Default::default())
            .await?;
        assert_eq!(tracker_one_data.len(), 2);
        assert_eq!(tracker_two_data.len(), 1);

        assert_debug_snapshot!(
            tracker_one_data.into_iter().map(|rev| rev.data).collect::<Vec<_>>(),
            @r###"
        [
            TrackerDataValue {
                original: String("@@ -1 +1 @@\n-\"rev_1\"\n+\"rev_2\"\n"),
                mods: None,
            },
            TrackerDataValue {
                original: String("\"rev_1\""),
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
            .get_tracker_data_revisions(tracker_one.id, Default::default())
            .await?;
        assert_debug_snapshot!(
            tracker_one_data.into_iter().map(|rev| rev.data).collect::<Vec<_>>(),
            @r###"
        [
            TrackerDataValue {
                original: String("\"rev_2\""),
                mods: None,
            },
            TrackerDataValue {
                original: String("\"rev_1\""),
                mods: None,
            },
        ]
        "###
        );

        Ok(())
    }

    #[sqlx::test]
    async fn properly_saves_api_target_revision_with_extractor(pool: PgPool) -> anyhow::Result<()> {
        let server = MockServer::start();
        let config = mock_config()?;

        let api = mock_api_with_config(pool, config).await?;

        let trackers = api.trackers();
        let tracker = trackers
            .create_tracker(
                TrackerCreateParamsBuilder::new("name_one")
                    .with_schedule("0 0 * * * *")
                    .with_target(TrackerTarget::Api(ApiTarget {
                        requests: vec![TargetRequest {
                            url: server.url("/api/get-call").parse()?,
                            method: None,
                            headers: Some(HeaderMap::from_iter([(
                                HeaderName::from_static("x-custom-header"),
                                HeaderValue::from_static("x-custom-value"),
                            )])),
                            body: None,
                            media_type: Some("application/json".parse()?),
                            accept_statuses: None,
                            accept_invalid_certificates: false
                        }],
                        configurator: None,
                        extractor: Some(
                            r#"
((context) => {{
  const newBody = JSON.parse(Deno.core.decode(new Uint8Array(context.responses[0].body)));
  return {
    body: Deno.core.encode(
      JSON.stringify({ name: `${newBody}_modified_${JSON.stringify(context.previousContent)}`, value: 1 })
    )
  };
}})(context);"#
                                .to_string(),
                        ),
                        params: None,
                    })).build(),
            )
            .await?;

        let content = TrackerDataValue::new(json!("\"rev_1\""));
        let mut content_mock = server.mock(|when, then| {
            when.method(httpmock::Method::GET).path("/api/get-call");
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body_obj(content.value());
        });

        trackers.create_tracker_data_revision(tracker.id).await?;
        let tracker_data = trackers
            .get_tracker_data_revisions(tracker.id, Default::default())
            .await?;
        assert_eq!(tracker_data.len(), 1);
        assert_eq!(tracker_data[0].tracker_id, tracker.id);
        assert_eq!(
            tracker_data[0].data,
            TrackerDataValue::new(json!({ "name": "\"rev_1\"_modified_undefined", "value": 1 }))
        );

        content_mock.assert();
        content_mock.delete();

        let content_two = TrackerDataValue::new(json!("\"rev_2\""));
        let content_mock = server.mock(|when, then| {
            when.method(httpmock::Method::GET).path("/api/get-call");
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body_obj(content_two.value());
        });

        let revision = trackers.create_tracker_data_revision(tracker.id).await?;
        assert_eq!(
            revision.data.value(),
            &json!({ "name": "\"rev_2\"_modified_{\"original\":{\"name\":\"\\\"rev_1\\\"_modified_undefined\",\"value\":1}}", "value": 1 })
        );
        content_mock.assert();

        let tracker_data = trackers
            .get_tracker_data_revisions(
                tracker.id,
                TrackerListRevisionsParams {
                    calculate_diff: true,
                    ..Default::default()
                },
            )
            .await?;
        assert_eq!(tracker_data.len(), 2);

        assert_debug_snapshot!(
            tracker_data.into_iter().map(|rev| rev.data).collect::<Vec<_>>(),
            @r###"
        [
            TrackerDataValue {
                original: String("@@ -1,4 +1,4 @@\n {\n-  \"name\": \"\\\"rev_1\\\"_modified_undefined\",\n+  \"name\": \"\\\"rev_2\\\"_modified_{\\\"original\\\":{\\\"name\\\":\\\"\\\\\\\"rev_1\\\\\\\"_modified_undefined\\\",\\\"value\\\":1}}\",\n   \"value\": 1\n }\n"),
                mods: None,
            },
            TrackerDataValue {
                original: Object {
                    "name": String("\"rev_1\"_modified_undefined"),
                    "value": Number(1),
                },
                mods: None,
            },
        ]
        "###
        );

        Ok(())
    }

    #[sqlx::test]
    async fn properly_saves_api_target_revision_with_params(pool: PgPool) -> anyhow::Result<()> {
        let server = MockServer::start();
        let config = mock_config()?;

        let api = mock_api_with_config(pool, config).await?;

        let trackers = api.trackers();
        let tracker = trackers
            .create_tracker(
                TrackerCreateParamsBuilder::new("name_one")
                    .with_schedule("0 0 * * * *")
                    .with_target(TrackerTarget::Api(ApiTarget {
                        requests: vec![TargetRequest {
                            url: server.url("/api/get-call").parse()?,
                            method: None,
                            headers: None,
                            body: None,
                            media_type: None,
                            accept_statuses: None,
                            accept_invalid_certificates: false,
                        }],
                        configurator: None,
                        extractor: Some(
                            r#"
((context) => {{
  const body = JSON.parse(Deno.core.decode(new Uint8Array(context.responses[0].body)));
  const secret = context.params?.secrets?.api_key ?? "none";
  return {
    body: Deno.core.encode(JSON.stringify({ data: body, secret }))
  };
}})(context);"#
                                .to_string(),
                        ),
                        params: Some(json!({ "secrets": { "api_key": "s3cr3t" } })),
                    }))
                    .build(),
            )
            .await?;

        let content = TrackerDataValue::new(json!("\"hello\""));
        let content_mock = server.mock(|when, then| {
            when.method(httpmock::Method::GET).path("/api/get-call");
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body_obj(content.value());
        });

        trackers.create_tracker_data_revision(tracker.id).await?;
        let tracker_data = trackers
            .get_tracker_data_revisions(tracker.id, Default::default())
            .await?;
        assert_eq!(tracker_data.len(), 1);
        assert_eq!(tracker_data[0].tracker_id, tracker.id);
        assert_eq!(
            tracker_data[0].data,
            TrackerDataValue::new(json!({ "data": "\"hello\"", "secret": "s3cr3t" }))
        );

        content_mock.assert();

        Ok(())
    }

    #[sqlx::test]
    async fn properly_saves_api_target_revision_with_non_success_code(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        let server = MockServer::start();
        let config = mock_config()?;

        let api = mock_api_with_config(pool, config).await?;

        let trackers = api.trackers();
        let tracker = trackers
            .create_tracker(
                TrackerCreateParamsBuilder::new("name_one")
                    .with_schedule("0 0 * * * *")
                    .with_target(TrackerTarget::Api(ApiTarget {
                        requests: vec![
                            TargetRequest {
                                url: server.url("/api/get-call").parse()?,
                                method: None,
                                headers: None,
                                body: None,
                                media_type: Some("application/json".parse()?),
                                accept_statuses: None,
                                accept_invalid_certificates: false
                            },
                            TargetRequest {
                                url: server.url("/api/get-call-fail").parse()?,
                                method: None,
                                headers: None,
                                body: None,
                                media_type: Some("application/json".parse()?),
                                accept_statuses: Some(
                                    [StatusCode::FORBIDDEN].into_iter().collect(),
                                ),
                                accept_invalid_certificates: true
                            },
                        ],
                        configurator: None,
                        extractor: Some(
                            r#"
((context) => {{
  const successBody = JSON.parse(Deno.core.decode(new Uint8Array(context.responses[0].body)));
  const failBody = JSON.parse(Deno.core.decode(new Uint8Array(context.responses[1].body)));
  return {
    body: Deno.core.encode(
      JSON.stringify({
        success: `${successBody.result} (${context.responses[0].status}, ${context.responses[0].headers['x-response']})`,
        failure: `${failBody.error} (${context.responses[1].status}, ${context.responses[1].headers['x-response']})`
      })
    )
  };
}})(context);"#.to_string(),
                        ),
                        params: None,
                    }))
                    .build(),
            )
            .await?;

        let success_mock = server.mock(|when, then| {
            when.method(httpmock::Method::GET).path("/api/get-call");
            then.status(200)
                .header("Content-Type", "application/json")
                .header("x-response", "x-success")
                .json_body_obj(&json!({ "result": "Yahoo!" }));
        });

        let fail_mock = server.mock(|when, then| {
            when.method(httpmock::Method::GET)
                .path("/api/get-call-fail");
            then.status(403)
                .header("Content-Type", "application/json")
                .header("x-response", "x-failure")
                .json_body_obj(&json!({ "error": "Uh oh" }));
        });

        trackers.create_tracker_data_revision(tracker.id).await?;
        let tracker_data = trackers
            .get_tracker_data_revisions(tracker.id, Default::default())
            .await?;
        assert_eq!(tracker_data.len(), 1);
        assert_eq!(tracker_data[0].tracker_id, tracker.id);
        assert_eq!(
            tracker_data[0].data,
            TrackerDataValue::new(json!({
                "success": "Yahoo! (200, x-success)",
                "failure": "Uh oh (403, x-failure)"
            }))
        );

        success_mock.assert();
        fail_mock.assert();

        Ok(())
    }

    #[sqlx::test]
    async fn properly_saves_api_target_revision_with_configurator_response(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        let server = MockServer::start();
        let config = mock_config()?;

        let api = mock_api_with_config(pool, config).await?;

        let trackers = api.trackers();
        let tracker = trackers
            .create_tracker(
                TrackerCreateParamsBuilder::new("name_one")
                    .with_schedule("0 0 * * * *")
                    .with_target(TrackerTarget::Api(ApiTarget {
                        requests: vec![TargetRequest {
                            url: server.url("/api/get-call").parse()?,
                            method: None,
                            headers: Some(HeaderMap::from_iter([(
                                HeaderName::from_static("x-custom-header"),
                                HeaderValue::from_static("x-custom-value"),
                            )])),
                            body: Some(serde_json::Value::String("rev_1".to_string())),
                            media_type: Some("application/json".parse()?),
                            accept_statuses: Some([StatusCode::OK].into_iter().collect()),
                            accept_invalid_certificates: true
                        }],
                        configurator: Some(
                            r#"
((context) => {{
  const newBody = JSON.parse(Deno.core.decode(context.requests[0].body));
  return {
    responses: [{
      status: 200,
      headers: {},
      body: Deno.core.encode(
        JSON.stringify({ name: `${newBody}_modified_${JSON.stringify(context.previousContent)}`, value: 1 })
      )
    }]
  };
}})(context);"#
                                .to_string(),
                        ),
                        extractor: None,
                        params: None,
                    })).build(),
            )
            .await?;

        trackers.create_tracker_data_revision(tracker.id).await?;
        let tracker_data = trackers
            .get_tracker_data_revisions(tracker.id, Default::default())
            .await?;
        assert_eq!(tracker_data.len(), 1);
        assert_eq!(tracker_data[0].tracker_id, tracker.id);
        assert_eq!(
            tracker_data[0].data,
            TrackerDataValue::new(json!({ "name": "rev_1_modified_undefined", "value": 1 }))
        );

        let revision = trackers.create_tracker_data_revision(tracker.id).await?;
        assert_eq!(
            revision.data.value(),
            &json!({ "name": "rev_1_modified_{\"original\":{\"name\":\"rev_1_modified_undefined\",\"value\":1}}", "value": 1 })
        );

        let tracker_data = trackers
            .get_tracker_data_revisions(
                tracker.id,
                TrackerListRevisionsParams {
                    calculate_diff: true,
                    ..Default::default()
                },
            )
            .await?;
        assert_eq!(tracker_data.len(), 2);

        assert_debug_snapshot!(
            tracker_data.into_iter().map(|rev| rev.data).collect::<Vec<_>>(),
            @r###"
        [
            TrackerDataValue {
                original: String("@@ -1,4 +1,4 @@\n {\n-  \"name\": \"rev_1_modified_undefined\",\n+  \"name\": \"rev_1_modified_{\\\"original\\\":{\\\"name\\\":\\\"rev_1_modified_undefined\\\",\\\"value\\\":1}}\",\n   \"value\": 1\n }\n"),
                mods: None,
            },
            TrackerDataValue {
                original: Object {
                    "name": String("rev_1_modified_undefined"),
                    "value": Number(1),
                },
                mods: None,
            },
        ]
        "###
        );

        Ok(())
    }

    #[sqlx::test]
    async fn properly_saves_api_target_revision_with_parser_xlsx(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        let server = MockServer::start();
        let config = mock_config()?;

        let api = mock_api_with_config(pool, config).await?;

        let trackers = api.trackers();
        let tracker = trackers
            .create_tracker(
                TrackerCreateParamsBuilder::new("name_one")
                    .with_schedule("0 0 * * * *")
                    .with_target(TrackerTarget::Api(ApiTarget {
                        requests: vec![TargetRequest {
                            url: server.url("/api/get-call").parse()?,
                            method: None,
                            headers: None,
                            body: None,
                            media_type: Some(
                                "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
                                    .parse()?,
                            ),
                            accept_statuses: None,
                            accept_invalid_certificates: false,
                        }],
                        configurator: None,
                        extractor: None,
                        params: None,
                    }))
                    .build(),
            )
            .await?;

        let tracker_data = trackers
            .get_tracker_data_revisions(tracker.id, Default::default())
            .await?;
        assert!(tracker_data.is_empty());

        let content_mock = server.mock(|when, then| {
            when.method(httpmock::Method::GET).path("/api/get-call");
            then.status(200)
                .header("Content-Type", "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet;charset=UTF-8")
                .body(load_fixture("xlsx_fixture.xlsx").unwrap());
        });

        trackers.create_tracker_data_revision(tracker.id).await?;
        content_mock.assert();

        let revs = trackers
            .get_tracker_data_revisions(tracker.id, Default::default())
            .await?;
        assert_debug_snapshot!(
            revs.into_iter().map(|rev| rev.data).collect::<Vec<_>>(),
            @r###"
        [
            TrackerDataValue {
                original: Array [
                    Object {
                        "name": String("Sheet N1"),
                        "data": Array [
                            Array [
                                String("Header N1"),
                                String("Header N2"),
                                String(""),
                            ],
                            Array [
                                String("Some string"),
                                String("100500"),
                                String(""),
                            ],
                            Array [
                                String("500100"),
                                String("Some string 2"),
                                String("100"),
                            ],
                            Array [
                                String(""),
                                String(""),
                                String("Another string"),
                            ],
                        ],
                    },
                    Object {
                        "name": String("Sheet N2"),
                        "data": Array [
                            Array [
                                String("Header N3"),
                                String("Header N4"),
                                String(""),
                            ],
                            Array [
                                String("Some string 3"),
                                String("100500"),
                                String(""),
                            ],
                            Array [
                                String("600200"),
                                String("Some string 4"),
                                String("200"),
                            ],
                            Array [
                                String(""),
                                String(""),
                                String("Another string 2"),
                            ],
                        ],
                    },
                ],
                mods: None,
            },
        ]
        "###
        );

        Ok(())
    }

    #[sqlx::test]
    async fn properly_saves_api_target_revision_with_parser_csv(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        let server = MockServer::start();
        let config = mock_config()?;

        let api = mock_api_with_config(pool, config).await?;

        let trackers = api.trackers();
        let tracker = trackers
            .create_tracker(
                TrackerCreateParamsBuilder::new("name_one")
                    .with_schedule("0 0 * * * *")
                    .with_target(TrackerTarget::Api(ApiTarget {
                        requests: vec![TargetRequest {
                            url: server.url("/api/get-call").parse()?,
                            method: None,
                            headers: None,
                            body: None,
                            media_type: Some("text/csv".parse()?),
                            accept_statuses: None,
                            accept_invalid_certificates: false,
                        }],
                        configurator: None,
                        extractor: None,
                        params: None,
                    }))
                    .build(),
            )
            .await?;

        let tracker_data = trackers
            .get_tracker_data_revisions(tracker.id, Default::default())
            .await?;
        assert!(tracker_data.is_empty());

        let content_mock = server.mock(|when, then| {
            when.method(httpmock::Method::GET).path("/api/get-call");
            then.status(200)
                .header("Content-Type", "text/csv;charset=UTF-8")
                .body(load_fixture("csv_fixture.csv").unwrap());
        });

        trackers.create_tracker_data_revision(tracker.id).await?;
        content_mock.assert();

        let revs = trackers
            .get_tracker_data_revisions(tracker.id, Default::default())
            .await?;
        assert_debug_snapshot!(
            revs.into_iter().map(|rev| rev.data).collect::<Vec<_>>(),
            @r###"
        [
            TrackerDataValue {
                original: Array [
                    Array [
                        String("Header N1"),
                        String("Header N2"),
                        String(""),
                    ],
                    Array [
                        String("Some string"),
                        String("100500"),
                        String(""),
                    ],
                    Array [
                        String("500100"),
                        String("Some string 2"),
                        String("100"),
                    ],
                    Array [
                        String(""),
                        String(""),
                        String("Another string"),
                    ],
                ],
                mods: None,
            },
        ]
        "###
        );

        Ok(())
    }

    #[sqlx::test]
    async fn properly_saves_api_target_revision_with_multiple_requests(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        let server = MockServer::start();
        let config = mock_config()?;

        let api = mock_api_with_config(pool, config).await?;

        let trackers = api.trackers();
        let tracker = trackers
            .create_tracker(
                TrackerCreateParamsBuilder::new("name_one")
                    .with_schedule("0 0 * * * *")
                    .with_target(TrackerTarget::Api(ApiTarget {
                        requests: vec![
                            TargetRequest {
                                url: server.url("/api/csv-call").parse()?,
                                method: None,
                                headers: None,
                                body: None,
                                media_type: Some("text/csv".parse()?),
                                accept_statuses: None,
                                accept_invalid_certificates: false,
                            },
                            TargetRequest {
                                url: server.url("/api/json-call").parse()?,
                                method: Some(Method::POST),
                                headers: Some(HeaderMap::from_iter([(
                                    HeaderName::from_static("x-custom-header"),
                                    HeaderValue::from_static("x-custom-value"),
                                )])),
                                body: Some(json!({ "key": "value" })),
                                media_type: Some("application/json".parse()?),
                                accept_statuses: None,
                                accept_invalid_certificates: false,
                            },
                        ],
                        configurator: None,
                        extractor: Some(
                            r#"
((context) => {{
  const csvResponse = JSON.parse(Deno.core.decode(new Uint8Array(context.responses[0].body)));
  const jsonResponse = JSON.parse(Deno.core.decode(new Uint8Array(context.responses[1].body)));
  return {
    body: Deno.core.encode(
      JSON.stringify({ csv: csvResponse[0][1], json: jsonResponse.key })
    )
  };
}})(context);"#
                                .to_string(),
                        ),
                        params: None,
                    }))
                    .build(),
            )
            .await?;

        let tracker_data = trackers
            .get_tracker_data_revisions(tracker.id, Default::default())
            .await?;
        assert!(tracker_data.is_empty());

        let csv_mock = server.mock(|when, then| {
            when.method(httpmock::Method::GET).path("/api/csv-call");
            then.status(200)
                .header("Content-Type", "text/csv;charset=UTF-8")
                .body(Bytes::from("key,csv-value"));
        });

        let json_mock = server.mock(|when, then| {
            when.method(httpmock::Method::POST)
                .path("/api/json-call")
                .header("x-custom-header", "x-custom-value")
                .json_body(json!({ "key": "value" }));
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body_obj(&json!({ "key": "json-value" }));
        });

        trackers.create_tracker_data_revision(tracker.id).await?;
        csv_mock.assert();
        json_mock.assert();

        let revs = trackers
            .get_tracker_data_revisions(tracker.id, Default::default())
            .await?;
        assert_debug_snapshot!(
            revs.into_iter().map(|rev| rev.data).collect::<Vec<_>>(),
            @r###"
        [
            TrackerDataValue {
                original: Object {
                    "csv": String("csv-value"),
                    "json": String("json-value"),
                },
                mods: None,
            },
        ]
        "###
        );

        Ok(())
    }

    #[sqlx::test]
    async fn properly_saves_api_target_revision_with_remote_scripts(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        let server = MockServer::start();
        let config = mock_config()?;

        let api = mock_api_with_config(pool, config).await?;

        let trackers = api.trackers();
        let tracker = trackers
            .create_tracker(
                TrackerCreateParamsBuilder::new("name_one")
                    .with_schedule("0 0 * * * *")
                    .with_target(TrackerTarget::Api(ApiTarget {
                        requests: vec![TargetRequest {
                            url: server.url("/api/get-call").parse()?,
                            method: Some(Method::POST),
                            headers: Some(HeaderMap::from_iter([(
                                HeaderName::from_static("x-custom-header"),
                                HeaderValue::from_static("x-custom-value"),
                            )])),
                            body: Some(json!({ "key": "value" })),
                            media_type: Some("application/json".parse()?),
                            accept_statuses: None,
                            accept_invalid_certificates: false,
                        }],
                        configurator: Some(server.url("/configurator.js")),
                        extractor: Some(server.url("/extractor.js")),
                        params: None,
                    }))
                    .build(),
            )
            .await?;

        let configurator_mock = server.mock(|when, then| {
            when.method(httpmock::Method::GET).path("/configurator.js");
            then.status(200)
                .header("Content-Type", "text/javascript")
                .body(
                    r#"
((context) => ({
  requests: [{ ...context.requests[0], body: Deno.core.encode(JSON.stringify({ key: `overridden-${JSON.parse(Deno.core.decode(context.requests[0].body)).key}` })) }]
 }))(context);"#,
                );
        });

        let content = TrackerDataValue::new(json!("\"rev_1\""));
        let content_mock = server.mock(|when, then| {
            when.method(httpmock::Method::POST)
                .path("/api/get-call")
                .body(serde_json::to_string(&json!({ "key": "overridden-value" })).unwrap());
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body_obj(content.value());
        });

        let extractor_mock = server.mock(|when, then| {
            when.method(httpmock::Method::GET).path("/extractor.js");
            then.status(200)
                .header("Content-Type", "text/javascript")
                .body(
                    r#"
((context) => {{
  const newBody = JSON.parse(Deno.core.decode(new Uint8Array(context.responses[0].body)));
  return {
    body: Deno.core.encode(
      JSON.stringify(`${newBody}_modified_${JSON.stringify(context.previousContent)}`)
    )
  };
}})(context);"#,
                );
        });

        trackers.create_tracker_data_revision(tracker.id).await?;
        let tracker_data = trackers
            .get_tracker_data_revisions(tracker.id, Default::default())
            .await?;
        assert_eq!(tracker_data.len(), 1);
        assert_eq!(tracker_data[0].tracker_id, tracker.id);
        assert_eq!(
            tracker_data[0].data,
            TrackerDataValue::new(json!("\"rev_1\"_modified_undefined"))
        );

        configurator_mock.assert();
        content_mock.assert();
        extractor_mock.assert();

        let tracker_data = trackers
            .get_tracker_data_revisions(
                tracker.id,
                TrackerListRevisionsParams {
                    calculate_diff: true,
                    ..Default::default()
                },
            )
            .await?;
        assert_eq!(tracker_data.len(), 1);

        assert_debug_snapshot!(
            tracker_data.into_iter().map(|rev| rev.data).collect::<Vec<_>>(),
            @r###"
        [
            TrackerDataValue {
                original: String("\"rev_1\"_modified_undefined"),
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
                TrackerCreateParamsBuilder::new("tracker")
                    .with_schedule("0 0 * * * *")
                    .with_tags(vec!["tag".to_string()])
                    .with_actions(vec![
                        TrackerAction::Email(EmailAction {
                            to: vec![
                                "dev@retrack.dev".to_string(),
                                "dev-2@retrack.dev".to_string(),
                            ],
                            formatter: None,
                        }),
                        TrackerAction::Webhook(WebhookAction {
                            url: "https://retrack.dev".parse()?,
                            method: None,
                            headers: Some(HeaderMap::from_iter([(
                                CONTENT_TYPE,
                                HeaderValue::from_static("text/plain"),
                            )])),
                            formatter: None,
                        }),
                    ])
                    .build(),
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
                .json_body(json!({ "result": content.value() }));
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

        trackers.create_tracker_data_revision(tracker.id).await?;
        let tracker_data = trackers
            .get_tracker_data_revisions(tracker.id, Default::default())
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
                content: EmailContent::Template(EmailTemplate::TrackerCheckResult {
                    tracker_id: tracker.id,
                    tracker_name: tracker.name.clone(),
                    result: Ok("\"rev_1\"".to_string()),
                }),
            })
        );
        assert_eq!(
            email_task.tags,
            vec![
                "tag".to_string(),
                format!("@retrack:tracker:id:{}", tracker.id),
                "@retrack:task:type:email".to_string()
            ]
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
                body: Some(serde_json::to_vec(&WebhookActionPayload {
                    tracker_id: tracker.id,
                    tracker_name: tracker.name.clone(),
                    result: WebhookActionPayloadResult::Success(json!("\"rev_1\"")),
                })?),
            })
        );
        assert_eq!(
            http_task.tags,
            vec![
                "tag".to_string(),
                format!("@retrack:tracker:id:{}", tracker.id),
                "@retrack:task:type:http".to_string()
            ]
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
            scraper_request.previous_content = Some(&content);

            when.method(httpmock::Method::POST)
                .path("/api/web_page/execute")
                .json_body(serde_json::to_value(scraper_request).unwrap());
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body(json!({ "result": content.value() }));
        });

        trackers.create_tracker_data_revision(tracker.id).await?;
        let tracker_data = trackers
            .get_tracker_data_revisions(tracker.id, Default::default())
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
            scraper_request.previous_content = Some(&content);

            when.method(httpmock::Method::POST)
                .path("/api/web_page/execute")
                .json_body(serde_json::to_value(scraper_request).unwrap());
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body(json!({ "result": new_content.value() }));
        });

        trackers.create_tracker_data_revision(tracker.id).await?;
        let tracker_data = trackers
            .get_tracker_data_revisions(tracker.id, Default::default())
            .await?;
        assert_eq!(tracker_data.len(), 2);
        assert_eq!(tracker_data[0].data.value(), &json!("\"rev_2\""));
        assert_eq!(tracker_data[1].data.value(), &json!("\"rev_1\""));

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
                content: EmailContent::Template(EmailTemplate::TrackerCheckResult {
                    tracker_id: tracker.id,
                    tracker_name: tracker.name.clone(),
                    result: Ok("\"rev_2\"".to_string()),
                }),
            })
        );
        assert_eq!(
            email_task.tags,
            vec![
                "tag".to_string(),
                format!("@retrack:tracker:id:{}", tracker.id),
                "@retrack:task:type:email".to_string()
            ]
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
                body: Some(serde_json::to_vec(&WebhookActionPayload {
                    tracker_id: tracker.id,
                    tracker_name: tracker.name,
                    result: WebhookActionPayloadResult::Success(json!("\"rev_2\"")),
                })?),
            })
        );
        assert_eq!(
            http_task.tags,
            vec![
                "tag".to_string(),
                format!("@retrack:tracker:id:{}", tracker.id),
                "@retrack:task:type:http".to_string()
            ]
        );

        Ok(())
    }

    #[sqlx::test]
    async fn can_execute_tracker_actions_in_fail_scenarios(pool: PgPool) -> anyhow::Result<()> {
        let server = MockServer::start();
        let mut config = mock_config()?;
        config.components.web_scraper_url = Url::parse(&server.base_url())?;

        let api = mock_api_with_config(pool, config).await?;

        let trackers = api.trackers();
        let tracker = trackers
            .create_tracker(
                TrackerCreateParamsBuilder::new("tracker")
                    .with_schedule("0 0 * * * *")
                    .with_tags(vec!["tag".to_string()])
                    .with_actions(vec![
                        TrackerAction::Email(EmailAction {
                            to: vec![
                                "dev@retrack.dev".to_string(),
                                "dev-2@retrack.dev".to_string(),
                            ],
                            formatter: None,
                        }),
                        TrackerAction::Webhook(WebhookAction {
                            url: "https://retrack.dev".parse()?,
                            method: None,
                            headers: Some(HeaderMap::from_iter([(
                                CONTENT_TYPE,
                                HeaderValue::from_static("text/plain"),
                            )])),
                            formatter: None,
                        }),
                    ])
                    .build(),
            )
            .await?;

        let server_mock = server.mock(|when, then| {
            when.method(httpmock::Method::POST)
                .path("/api/web_page/execute")
                .json_body(
                    serde_json::to_value(WebScraperContentRequest::try_from(&tracker).unwrap())
                        .unwrap(),
                );
            then.status(400)
                .header("Content-Type", "application/json")
                .json_body_obj(&WebScraperErrorResponse {
                    message: "Uh oh".to_string(),
                    error: None,
                    debug: None,
                });
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

        assert_debug_snapshot!(trackers.create_tracker_data_revision(tracker.id).await.unwrap_err(), @r###""Uh oh""###);

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
                content: EmailContent::Template(EmailTemplate::TrackerCheckResult {
                    tracker_id: tracker.id,
                    tracker_name: tracker.name.clone(),
                    result: Err("Uh oh".to_string()),
                }),
            })
        );
        assert_eq!(
            email_task.tags,
            vec![
                "tag".to_string(),
                format!("@retrack:tracker:id:{}", tracker.id),
                "@retrack:task:type:email".to_string()
            ]
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
                body: Some(serde_json::to_vec(&WebhookActionPayload {
                    tracker_id: tracker.id,
                    tracker_name: tracker.name,
                    result: WebhookActionPayloadResult::Failure("Uh oh".to_string()),
                })?),
            })
        );
        assert_eq!(
            http_task.tags,
            vec![
                "tag".to_string(),
                format!("@retrack:tracker:id:{}", tracker.id),
                "@retrack:task:type:http".to_string()
            ]
        );

        Ok(())
    }

    #[sqlx::test]
    async fn dont_execute_tracker_actions_in_fail_scenarios_if_retries_left(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        let server = MockServer::start();
        let mut config = mock_config()?;
        config.components.web_scraper_url = Url::parse(&server.base_url())?;

        let api = mock_api_with_config(pool, config).await?;

        let trackers = api.trackers();
        let tracker = trackers
            .create_tracker(
                TrackerCreateParamsBuilder::new("tracker")
                    .with_config(TrackerConfig {
                        job: Some(SchedulerJobConfig {
                            schedule: "0 0 * * * *".to_string(),
                            retry_strategy: Some(SchedulerJobRetryStrategy::Constant {
                                interval: Duration::from_secs(60),
                                max_attempts: 1,
                            }),
                        }),
                        ..Default::default()
                    })
                    .with_tags(vec!["tag".to_string()])
                    .with_actions(vec![
                        TrackerAction::Email(EmailAction {
                            to: vec![
                                "dev@retrack.dev".to_string(),
                                "dev-2@retrack.dev".to_string(),
                            ],
                            formatter: None,
                        }),
                        TrackerAction::Webhook(WebhookAction {
                            url: "https://retrack.dev".parse()?,
                            method: None,
                            headers: Some(HeaderMap::from_iter([(
                                CONTENT_TYPE,
                                HeaderValue::from_static("text/plain"),
                            )])),
                            formatter: None,
                        }),
                    ])
                    .build(),
            )
            .await?;

        let server_mock = server.mock(|when, then| {
            when.method(httpmock::Method::POST)
                .path("/api/web_page/execute")
                .json_body(
                    serde_json::to_value(WebScraperContentRequest::try_from(&tracker).unwrap())
                        .unwrap(),
                );
            then.status(400)
                .header("Content-Type", "application/json")
                .json_body_obj(&WebScraperErrorResponse {
                    message: "Uh oh".to_string(),
                    error: None,
                    debug: None,
                });
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

        assert_debug_snapshot!(trackers.create_tracker_data_revision_with_retry(tracker.id, Some(0)).await.unwrap_err(), @r###""Uh oh""###);

        server_mock.assert();

        let tasks_ids = api
            .db
            .get_tasks_ids(scheduled_before_or_at, 2)
            .collect::<Vec<_>>()
            .await;
        assert!(tasks_ids.is_empty());

        Ok(())
    }

    #[sqlx::test]
    async fn can_execute_tracker_actions_combined_with_default_actions(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        let server = MockServer::start();
        let mut config = mock_config()?;
        config.components.web_scraper_url = Url::parse(&server.base_url())?;
        config.trackers.default_actions = Some(vec![TrackerAction::Webhook(WebhookAction {
            url: "https://retrack.dev".parse()?,
            method: None,
            headers: Some(HeaderMap::from_iter([(
                CONTENT_TYPE,
                HeaderValue::from_static("text/plain"),
            )])),
            formatter: None,
        })]);

        let api = mock_api_with_config(pool, config).await?;

        let trackers = api.trackers();
        let tracker = trackers
            .create_tracker(
                TrackerCreateParamsBuilder::new("tracker")
                    .with_schedule("0 0 * * * *")
                    .with_tags(vec!["tag".to_string()])
                    .with_actions(vec![TrackerAction::Email(EmailAction {
                        to: vec![
                            "dev@retrack.dev".to_string(),
                            "dev-2@retrack.dev".to_string(),
                        ],
                        formatter: None,
                    })])
                    .build(),
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
                .json_body(json!({ "result": content.value() }));
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

        trackers.create_tracker_data_revision(tracker.id).await?;
        let tracker_data = trackers
            .get_tracker_data_revisions(tracker.id, Default::default())
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
                content: EmailContent::Template(EmailTemplate::TrackerCheckResult {
                    tracker_id: tracker.id,
                    tracker_name: tracker.name.clone(),
                    result: Ok("\"rev_1\"".to_string()),
                }),
            })
        );
        assert_eq!(
            email_task.tags,
            vec![
                "tag".to_string(),
                format!("@retrack:tracker:id:{}", tracker.id),
                "@retrack:task:type:email".to_string()
            ]
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
                body: Some(serde_json::to_vec(&WebhookActionPayload {
                    tracker_id: tracker.id,
                    tracker_name: tracker.name.clone(),
                    result: WebhookActionPayloadResult::Success(json!("\"rev_1\"")),
                })?),
            })
        );
        assert_eq!(
            http_task.tags,
            vec![
                "tag".to_string(),
                format!("@retrack:tracker:id:{}", tracker.id),
                "@retrack:task:type:http".to_string()
            ]
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
            scraper_request.previous_content = Some(&content);

            when.method(httpmock::Method::POST)
                .path("/api/web_page/execute")
                .json_body(serde_json::to_value(scraper_request).unwrap());
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body(json!({ "result": content.value() }));
        });

        trackers.create_tracker_data_revision(tracker.id).await?;
        let tracker_data = trackers
            .get_tracker_data_revisions(tracker.id, Default::default())
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
            scraper_request.previous_content = Some(&content);

            when.method(httpmock::Method::POST)
                .path("/api/web_page/execute")
                .json_body(serde_json::to_value(scraper_request).unwrap());
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body(json!({ "result": new_content.value() }));
        });

        trackers.create_tracker_data_revision(tracker.id).await?;
        let tracker_data = trackers
            .get_tracker_data_revisions(tracker.id, Default::default())
            .await?;
        assert_eq!(tracker_data.len(), 2);
        assert_eq!(tracker_data[0].data.value(), &json!("\"rev_2\""));
        assert_eq!(tracker_data[1].data.value(), &json!("\"rev_1\""));

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
                content: EmailContent::Template(EmailTemplate::TrackerCheckResult {
                    tracker_id: tracker.id,
                    tracker_name: tracker.name.clone(),
                    result: Ok("\"rev_2\"".to_string()),
                }),
            })
        );
        assert_eq!(
            email_task.tags,
            vec![
                "tag".to_string(),
                format!("@retrack:tracker:id:{}", tracker.id),
                "@retrack:task:type:email".to_string()
            ]
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
                body: Some(serde_json::to_vec(&WebhookActionPayload {
                    tracker_id: tracker.id,
                    tracker_name: tracker.name,
                    result: WebhookActionPayloadResult::Success(json!("\"rev_2\"")),
                })?),
            })
        );
        assert_eq!(
            http_task.tags,
            vec![
                "tag".to_string(),
                format!("@retrack:tracker:id:{}", tracker.id),
                "@retrack:task:type:http".to_string()
            ]
        );

        Ok(())
    }

    #[sqlx::test]
    async fn can_execute_only_default_actions(pool: PgPool) -> anyhow::Result<()> {
        let server = MockServer::start();
        let mut config = mock_config()?;
        config.components.web_scraper_url = Url::parse(&server.base_url())?;
        config.trackers.default_actions = Some(vec![TrackerAction::Webhook(WebhookAction {
            url: "https://retrack.dev".parse()?,
            method: None,
            headers: Some(HeaderMap::from_iter([(
                CONTENT_TYPE,
                HeaderValue::from_static("text/plain"),
            )])),
            formatter: None,
        })]);

        let api = mock_api_with_config(pool, config).await?;

        let trackers = api.trackers();
        let tracker = trackers
            .create_tracker(
                TrackerCreateParamsBuilder::new("tracker")
                    .with_schedule("0 0 * * * *")
                    .with_tags(vec!["tag".to_string()])
                    .build(),
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
                .json_body(json!({ "result": content.value() }));
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

        trackers.create_tracker_data_revision(tracker.id).await?;
        let tracker_data = trackers
            .get_tracker_data_revisions(tracker.id, Default::default())
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
        assert_eq!(tasks_ids.len(), 1);

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
                body: Some(serde_json::to_vec(&WebhookActionPayload {
                    tracker_id: tracker.id,
                    tracker_name: tracker.name.clone(),
                    result: WebhookActionPayloadResult::Success(json!("\"rev_1\"")),
                })?),
            })
        );
        assert_eq!(
            http_task.tags,
            vec![
                "tag".to_string(),
                format!("@retrack:tracker:id:{}", tracker.id),
                "@retrack:task:type:http".to_string()
            ]
        );

        // Clear action tasks.
        api.db.remove_task(http_task.id).await?;
        let tasks_ids = api
            .db
            .get_tasks_ids(scheduled_before_or_at, 2)
            .collect::<Vec<_>>()
            .await;
        assert!(tasks_ids.is_empty());

        let mut server_mock = server.mock(|when, then| {
            let mut scraper_request = WebScraperContentRequest::try_from(&tracker).unwrap();
            scraper_request.previous_content = Some(&content);

            when.method(httpmock::Method::POST)
                .path("/api/web_page/execute")
                .json_body(serde_json::to_value(scraper_request).unwrap());
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body(json!({ "result": content.value() }));
        });

        trackers.create_tracker_data_revision(tracker.id).await?;
        let tracker_data = trackers
            .get_tracker_data_revisions(tracker.id, Default::default())
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
            scraper_request.previous_content = Some(&content);

            when.method(httpmock::Method::POST)
                .path("/api/web_page/execute")
                .json_body(serde_json::to_value(scraper_request).unwrap());
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body(json!({ "result": new_content.value() }));
        });

        trackers.create_tracker_data_revision(tracker.id).await?;
        let tracker_data = trackers
            .get_tracker_data_revisions(tracker.id, Default::default())
            .await?;
        assert_eq!(tracker_data.len(), 2);
        assert_eq!(tracker_data[0].data.value(), &json!("\"rev_2\""));
        assert_eq!(tracker_data[1].data.value(), &json!("\"rev_1\""));

        server_mock.assert();

        let mut tasks_ids = api
            .db
            .get_tasks_ids(scheduled_before_or_at, 2)
            .collect::<Vec<_>>()
            .await;
        assert_eq!(tasks_ids.len(), 1);

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
                body: Some(serde_json::to_vec(&WebhookActionPayload {
                    tracker_id: tracker.id,
                    tracker_name: tracker.name,
                    result: WebhookActionPayloadResult::Success(json!("\"rev_2\"")),
                })?),
            })
        );
        assert_eq!(
            http_task.tags,
            vec![
                "tag".to_string(),
                format!("@retrack:tracker:id:{}", tracker.id),
                "@retrack:task:type:http".to_string()
            ]
        );

        Ok(())
    }

    #[sqlx::test]
    async fn can_execute_tracker_actions_with_formatter(pool: PgPool) -> anyhow::Result<()> {
        let server = MockServer::start();
        let mut config = mock_config()?;
        config.components.web_scraper_url = Url::parse(&server.base_url())?;

        let api = mock_api_with_config(pool, config).await?;

        let trackers = api.trackers();
        let tracker = trackers
            .create_tracker(
                TrackerCreateParamsBuilder::new("tracker")
                    .with_schedule("0 0 * * * *")
                    .with_tags(vec![])
                    .with_actions(vec![
                        TrackerAction::Email(EmailAction {
                            to: vec![
                                "dev@retrack.dev".to_string(),
                                "dev-2@retrack.dev".to_string(),
                            ],
                            formatter: Some(
                                "(() => ({ content: `${context.newContent}_${context.action}` }))();".to_string(),
                            ),
                        }),
                        TrackerAction::Webhook(WebhookAction {
                            url: "https://retrack.dev".parse()?,
                            method: None,
                            headers: Some(HeaderMap::from_iter([(
                                CONTENT_TYPE,
                                HeaderValue::from_static("text/plain"),
                            )])),
                            formatter: Some(
                                "(() => ({ content: `${context.newContent}_${context.action}$` }))();".to_string(),
                            ),
                        }),
                    ])
                    .build(),
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
                .json_body(json!({ "result": content.value() }));
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

        trackers.create_tracker_data_revision(tracker.id).await?;
        let tracker_data = trackers
            .get_tracker_data_revisions(tracker.id, Default::default())
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
                content: EmailContent::Template(EmailTemplate::TrackerCheckResult {
                    tracker_id: tracker.id,
                    tracker_name: tracker.name.clone(),
                    result: Ok("\"rev_1\"_email".to_string()),
                }),
            })
        );
        assert_eq!(
            email_task.tags,
            vec![
                format!("@retrack:tracker:id:{}", tracker.id),
                "@retrack:task:type:email".to_string()
            ]
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
                body: Some(serde_json::to_vec(&WebhookActionPayload {
                    tracker_id: tracker.id,
                    tracker_name: tracker.name.clone(),
                    result: WebhookActionPayloadResult::Success(json!("\"rev_1\"_webhook$")),
                })?),
            })
        );
        assert_eq!(
            http_task.tags,
            vec![
                format!("@retrack:tracker:id:{}", tracker.id),
                "@retrack:task:type:http".to_string()
            ]
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
            scraper_request.previous_content = Some(&content);

            when.method(httpmock::Method::POST)
                .path("/api/web_page/execute")
                .json_body(serde_json::to_value(scraper_request).unwrap());
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body(json!({ "result": content.value() }));
        });

        trackers.create_tracker_data_revision(tracker.id).await?;
        let tracker_data = trackers
            .get_tracker_data_revisions(tracker.id, Default::default())
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
            scraper_request.previous_content = Some(&content);

            when.method(httpmock::Method::POST)
                .path("/api/web_page/execute")
                .json_body(serde_json::to_value(scraper_request).unwrap());
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body(json!({ "result": new_content.value() }));
        });

        trackers.create_tracker_data_revision(tracker.id).await?;
        let tracker_data = trackers
            .get_tracker_data_revisions(tracker.id, Default::default())
            .await?;
        assert_eq!(tracker_data.len(), 2);
        assert_eq!(tracker_data[0].data.value(), &json!("\"rev_2\""));
        assert_eq!(tracker_data[1].data.value(), &json!("\"rev_1\""));

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
                content: EmailContent::Template(EmailTemplate::TrackerCheckResult {
                    tracker_id: tracker.id,
                    tracker_name: tracker.name.clone(),
                    result: Ok("\"rev_2\"_email".to_string()),
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
                body: Some(serde_json::to_vec(&WebhookActionPayload {
                    tracker_id: tracker.id,
                    tracker_name: tracker.name,
                    result: WebhookActionPayloadResult::Success(json!("\"rev_2\"_webhook$")),
                })?),
            })
        );

        Ok(())
    }

    #[sqlx::test]
    async fn can_execute_tracker_actions_with_remote_formatter_script(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        let api = mock_api(pool).await?;
        let server = MockServer::start();

        let trackers = api.trackers();
        let tracker = trackers
            .create_tracker(
                TrackerCreateParamsBuilder::new("tracker")
                    .with_target(TrackerTarget::Api(ApiTarget {
                        requests: vec![TargetRequest::new(
                            server.url("/api/remote-formatter/get-call").parse()?,
                        )],
                        configurator: None,
                        extractor: None,
                        params: None,
                    }))
                    .with_actions(vec![TrackerAction::Email(EmailAction {
                        to: vec!["dev@retrack.dev".to_string()],
                        formatter: Some(server.url("/api/remote-formatter/formatter.js")),
                    })])
                    .build(),
            )
            .await?;

        let content = TrackerDataValue::new(json!("\"rev_1\""));
        let server_mock = server.mock(|when, then| {
            when.method(httpmock::Method::GET)
                .path("/api/remote-formatter/get-call");
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body_obj(content.value());
        });

        let formatter_mock = server.mock(|when, then| {
            when.method(httpmock::Method::GET)
                .path("/api/remote-formatter/formatter.js");
            then.status(200)
                .header("Content-Type", "text/javascript")
                .body("(() => ({ content: `${context.newContent}_${context.action}` }))();");
        });

        trackers.create_tracker_data_revision(tracker.id).await?;
        let tracker_data = trackers
            .get_tracker_data_revisions(tracker.id, Default::default())
            .await?;
        assert_eq!(tracker_data.len(), 1);
        assert_eq!(tracker_data[0].data.value(), &json!("\"rev_1\""));

        server_mock.assert();
        formatter_mock.assert();

        let mut tasks_ids = api
            .db
            .get_tasks_ids(OffsetDateTime::now_utc(), 2)
            .collect::<Vec<_>>()
            .await;
        assert_eq!(tasks_ids.len(), 1);

        let email_task = api.db.get_task(tasks_ids.remove(0)?).await?.unwrap();
        assert_eq!(
            email_task.task_type,
            TaskType::Email(EmailTaskType {
                to: vec!["dev@retrack.dev".to_string()],
                content: EmailContent::Template(EmailTemplate::TrackerCheckResult {
                    tracker_id: tracker.id,
                    tracker_name: tracker.name.clone(),
                    result: Ok("\"rev_1\"_email".to_string()),
                }),
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
            .create_tracker(
                TrackerCreateParamsBuilder::new("name_one")
                    .with_schedule("0 0 * * * *")
                    .build(),
            )
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
                    error: None,
                    debug: None,
                });
        });

        let scraper_error = trackers
            .create_tracker_data_revision(tracker.id)
            .await
            .unwrap_err()
            .downcast::<RetrackError>()?;
        assert_eq!(scraper_error.status_code(), 400);
        assert_debug_snapshot!(
            scraper_error,
            @r###""some client-error""###
        );

        let tracker_content = trackers
            .get_tracker_data_revisions(tracker.id, Default::default())
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
            .create_tracker(
                TrackerCreateParamsBuilder::new("name_one")
                    .with_schedule("0 0 * * * *")
                    .build(),
            )
            .await?;

        let tracker_content = trackers
            .get_tracker_data_revisions(tracker.id, Default::default())
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
                .json_body(json!({ "result": content_one.value() }));
        });

        let revision = trackers.create_tracker_data_revision(tracker.id).await?;
        assert_eq!(revision.data.value(), "\"rev_1\"");
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
                            .set_previous_content(&TrackerDataValue::new(json!("\"rev_1\""))),
                    )
                    .unwrap(),
                );
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body(json!({ "result": content_two }));
        });

        let revision = trackers.create_tracker_data_revision(tracker.id).await?;
        content_mock.assert();

        let tracker_content = trackers
            .get_tracker_data_revisions(tracker.id, Default::default())
            .await?;
        assert_eq!(tracker_content.len(), 1);
        assert_eq!(tracker_content, vec![revision]);

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
            .create_tracker(
                TrackerCreateParamsBuilder::new("name_one")
                    .with_schedule("0 0 * * * *")
                    .build(),
            )
            .await?;

        let tracker_content = trackers
            .get_tracker_data_revisions(tracker.id, Default::default())
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
                .json_body(json!({ "result": content }));
        });

        let revision = trackers.create_tracker_data_revision(tracker.id).await?;
        let tracker_content = trackers
            .get_tracker_data_revisions(tracker.id, Default::default())
            .await?;
        assert_eq!(tracker_content.len(), 1);
        assert_eq!(tracker_content, vec![revision]);
        content_mock.assert();

        trackers.clear_tracker_data(tracker.id).await?;

        let tracker_content = trackers
            .get_tracker_data_revisions(tracker.id, Default::default())
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
            .create_tracker(
                TrackerCreateParamsBuilder::new("name_one")
                    .with_schedule("0 0 * * * *")
                    .build(),
            )
            .await?;

        let tracker_content = trackers
            .get_tracker_data_revisions(tracker.id, Default::default())
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
                .json_body(json!({ "result": content }));
        });

        let revision = trackers.create_tracker_data_revision(tracker.id).await?;
        let tracker_content = trackers
            .get_tracker_data_revisions(tracker.id, Default::default())
            .await?;
        assert_eq!(tracker_content.len(), 1);
        assert_eq!(tracker_content, vec![revision]);
        content_mock.assert();

        trackers.remove_tracker(tracker.id).await?;

        let tracker_content = api
            .db
            .trackers()
            .get_tracker_data_revisions(tracker.id, 100)
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
            .create_tracker(
                TrackerCreateParamsBuilder::new("name_one")
                    .with_schedule("0 0 * * * *")
                    .build(),
            )
            .await?;
        api.trackers()
            .set_tracker_job(tracker.id, uuid!("00000000-0000-0000-0000-000000000001"))
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
                        params: None,
                        engine: None,
                        user_agent: Some("Unknown/1.0.0".to_string()),
                        accept_invalid_certificates: true,
                    })),
                    config: Some(TrackerConfig {
                        revisions: 4,
                        timeout: Some(Duration::from_millis(3000)),
                        job: Some(SchedulerJobConfig {
                            schedule: "0 0 * * * *".to_string(),
                            retry_strategy: Some(SchedulerJobRetryStrategy::Constant {
                                interval: Duration::from_secs(120),
                                max_attempts: 5,
                            }),
                        }),
                    }),
                    tags: Some(vec!["tag".to_string()]),
                    actions: Some(vec![TrackerAction::ServerLog(Default::default())]),
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
            .create_tracker(
                TrackerCreateParamsBuilder::new("name_one")
                    .with_schedule("0 0 * * * *")
                    .build(),
            )
            .await?;
        api.trackers()
            .set_tracker_job(tracker.id, uuid!("00000000-0000-0000-0000-000000000001"))
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
                        params: None,
                        engine: None,
                        user_agent: Some("Unknown/1.0.0".to_string()),
                        accept_invalid_certificates: true,
                    })),
                    config: Some(TrackerConfig {
                        revisions: 4,
                        timeout: Some(Duration::from_millis(3000)),
                        job: Some(SchedulerJobConfig {
                            schedule: "0 0 * * * *".to_string(),
                            retry_strategy: Some(SchedulerJobRetryStrategy::Constant {
                                interval: Duration::from_secs(120),
                                max_attempts: 5,
                            }),
                        }),
                    }),
                    tags: Some(vec!["tag_two".to_string()]),
                    actions: Some(vec![TrackerAction::Email(EmailAction {
                        to: vec!["dev@retrack.dev".to_string()],
                        formatter: None,
                    })]),
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
            .create_tracker(
                TrackerCreateParamsBuilder::new("name_one")
                    .with_schedule("0 0 * * * *")
                    .build(),
            )
            .await?;
        api.trackers()
            .set_tracker_job(tracker.id, uuid!("00000000-0000-0000-0000-000000000001"))
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
                        params: None,
                        engine: None,
                        user_agent: Some("Unknown/1.0.0".to_string()),
                        accept_invalid_certificates: true,
                    })),
                    config: Some(TrackerConfig {
                        revisions: 4,
                        timeout: Some(Duration::from_millis(3000)),
                        job: Some(SchedulerJobConfig {
                            schedule: "0 0 * * * *".to_string(),
                            retry_strategy: Some(SchedulerJobRetryStrategy::Constant {
                                interval: Duration::from_secs(120),
                                max_attempts: 5,
                            }),
                        }),
                    }),
                    tags: Some(vec!["tag_two".to_string()]),
                    actions: Some(vec![TrackerAction::Email(EmailAction {
                        to: vec!["dev@retrack.dev".to_string()],
                        formatter: None,
                    })]),
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

        let tracker = trackers
            .create_tracker(
                TrackerCreateParamsBuilder::new("name_one")
                    .with_schedule("0 0 * * * *")
                    .build(),
            )
            .await?;

        let unscheduled_trackers = api.trackers().get_trackers_to_schedule().await?;
        assert_eq!(unscheduled_trackers, vec![tracker.clone()]);

        api.trackers()
            .set_tracker_job(tracker.id, uuid!("11e55044-10b1-426f-9247-bb680e5fe0c9"))
            .await?;
        mock_upsert_scheduler_job(
            &api.db,
            &mock_scheduler_job(
                uuid!("11e55044-10b1-426f-9247-bb680e5fe0c9"),
                SchedulerJob::TrackersRun,
                "0 0 * * * *",
            ),
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

        let scheduled_tracker = api
            .trackers()
            .get_tracker_by_job_id(uuid!("11e55044-10b1-426f-9247-bb680e5fe0c8"))
            .await?;
        assert!(scheduled_tracker.is_none());

        Ok(())
    }

    #[sqlx::test]
    async fn debug_api_target_happy_path(pool: PgPool) -> anyhow::Result<()> {
        let server = MockServer::start();
        let mut config = mock_config()?;
        config.components.web_scraper_url = Url::parse(&server.base_url())?;

        let api = mock_api_with_config(pool, config).await?;
        let trackers = api.trackers();

        let api_mock = server.mock(|when, then| {
            when.method(httpmock::Method::GET).path("/data");
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body(json!({"key": "value"}));
        });

        let result = trackers
            .debug_tracker(TrackerDebugParams {
                target: TrackerTarget::Api(ApiTarget {
                    requests: vec![TargetRequest::new(
                        format!("{}/data", server.base_url()).parse()?,
                    )],
                    configurator: None,
                    extractor: None,
                    params: None,
                }),
                config: Default::default(),
                tags: vec![],
                actions: vec![],
                previous_content: None,
                debug: None,
            })
            .await?;

        api_mock.assert();

        assert_json_snapshot!(result, {
            ".durationMs" => 10,
            ".target.requests[0].durationMs" => 10,
            ".target.requests[0].url" => insta::dynamic_redaction(|value, _| {
                assert!(value.as_str().unwrap().ends_with("/data"));
                "[url]"
            }),
            ".target.requests[0].responseHeaders.date" => "[date]",
            r#".target.requests[0].responseHeaders["x-cache"]"# => "[cache]",
            r#".target.requests[0].responseHeaders["x-cache-lookup"]"# => "[cache]",
            ".target.requests[0].responseHeaders" => insta::sorted_redaction(),
        }, @r###"
        {
          "durationMs": 10,
          "result": {
            "key": "value"
          },
          "target": {
            "type": "api",
            "requests": [
              {
                "index": 0,
                "source": "original",
                "url": "[url]",
                "method": "GET",
                "statusCode": 200,
                "responseHeaders": {
                  "content-length": "15",
                  "content-type": "application/json",
                  "date": "[date]",
                  "x-cache": "[cache]",
                  "x-cache-lookup": "[cache]"
                },
                "responseBodyRaw": "{\"key\":\"value\"}",
                "responseBodyRawSize": 15,
                "responseBodyParsed": {
                  "key": "value"
                },
                "durationMs": 10
              }
            ]
          }
        }
        "###);

        Ok(())
    }

    #[sqlx::test]
    async fn debug_api_target_with_request_failure(pool: PgPool) -> anyhow::Result<()> {
        let api = mock_api(pool).await?;
        let trackers = api.trackers();

        let result = trackers
            .debug_tracker(TrackerDebugParams {
                target: TrackerTarget::Api(ApiTarget {
                    requests: vec![TargetRequest::new(
                        "http://127.0.0.1:1/nonexistent".parse()?,
                    )],
                    configurator: None,
                    extractor: None,
                    params: None,
                }),
                config: TrackerConfig {
                    revisions: 0,
                    timeout: Some(Duration::from_millis(500)),
                    job: None,
                },
                tags: vec![],
                actions: vec![],
                previous_content: None,
                debug: None,
            })
            .await?;

        assert_json_snapshot!(result, {
            ".durationMs" => 10,
            ".target.requests[0].durationMs" => 10
        }, @r###"
        {
          "durationMs": 10,
          "target": {
            "type": "api",
            "requests": [
              {
                "index": 0,
                "source": "original",
                "url": "http://127.0.0.1:1/nonexistent",
                "method": "GET",
                "durationMs": 10,
                "error": "Request failed: Cache error: error sending request for url (http://127.0.0.1:1/nonexistent)"
              }
            ]
          }
        }
        "###);

        Ok(())
    }

    #[sqlx::test]
    async fn debug_existing_tracker_not_found(pool: PgPool) -> anyhow::Result<()> {
        let api = mock_api(pool).await?;
        let trackers = api.trackers();

        let result = trackers
            .debug_existing_tracker(
                uuid!("00000000-0000-0000-0000-000000000001"),
                TrackerDebugExistingParams::default(),
            )
            .await;

        assert_eq!(
            result.unwrap_err().to_string(),
            "Tracker ('00000000-0000-0000-0000-000000000001') is not found."
        );

        Ok(())
    }

    #[sqlx::test]
    async fn debug_existing_tracker_with_overrides(pool: PgPool) -> anyhow::Result<()> {
        let server = MockServer::start();
        let mut config = mock_config()?;
        config.components.web_scraper_url = Url::parse(&server.base_url())?;

        let api = mock_api_with_config(pool, config).await?;
        let trackers = api.trackers();

        let tracker = trackers
            .create_tracker(
                TrackerCreateParamsBuilder::new("test-debug")
                    .with_target(TrackerTarget::Api(ApiTarget {
                        requests: vec![TargetRequest::new(
                            format!("{}/original", server.base_url()).parse()?,
                        )],
                        configurator: None,
                        extractor: None,
                        params: None,
                    }))
                    .build(),
            )
            .await?;

        let override_mock = server.mock(|when, then| {
            when.method(httpmock::Method::GET).path("/override");
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body(json!({"overridden": true}));
        });

        let result = trackers
            .debug_existing_tracker(
                tracker.id,
                TrackerDebugExistingParams {
                    target: Some(TrackerTarget::Api(ApiTarget {
                        requests: vec![TargetRequest::new(
                            format!("{}/override", server.base_url()).parse()?,
                        )],
                        configurator: None,
                        extractor: None,
                        params: None,
                    })),
                    tags: Some(vec!["overridden-tag".to_string()]),
                    ..Default::default()
                },
            )
            .await?;

        override_mock.assert();

        assert_json_snapshot!(result, {
            ".durationMs" => 10,
            ".target.requests[0].durationMs" => 10,
            ".target.requests[0].url" => insta::dynamic_redaction(|value, _| {
                assert!(value.as_str().unwrap().ends_with("/override"));
                "[url]"
            }),
            ".target.requests[0].responseHeaders.date" => "[date]",
            r#".target.requests[0].responseHeaders["x-cache"]"# => "[cache]",
            r#".target.requests[0].responseHeaders["x-cache-lookup"]"# => "[cache]",
            ".target.requests[0].responseHeaders" => insta::sorted_redaction(),
        }, @r###"
        {
          "durationMs": 10,
          "result": {
            "overridden": true
          },
          "target": {
            "type": "api",
            "requests": [
              {
                "index": 0,
                "source": "original",
                "url": "[url]",
                "method": "GET",
                "statusCode": 200,
                "responseHeaders": {
                  "content-length": "19",
                  "content-type": "application/json",
                  "date": "[date]",
                  "x-cache": "[cache]",
                  "x-cache-lookup": "[cache]"
                },
                "responseBodyRaw": "{\"overridden\":true}",
                "responseBodyRawSize": 19,
                "responseBodyParsed": {
                  "overridden": true
                },
                "durationMs": 10
              }
            ]
          },
          "actions": [
            {
              "typeTag": "log",
              "index": 0,
              "skipped": false,
              "payload": {
                "overridden": true
              },
              "destination": {
                "type": "log"
              },
              "durationMs": 0
            }
          ]
        }
        "###);

        Ok(())
    }

    #[sqlx::test]
    async fn debug_action_dry_run_no_formatter(pool: PgPool) -> anyhow::Result<()> {
        let server = MockServer::start();
        let mut config = mock_config()?;
        config.components.web_scraper_url = Url::parse(&server.base_url())?;

        let api = mock_api_with_config(pool, config).await?;
        let trackers = api.trackers();

        let api_mock = server.mock(|when, then| {
            when.method(httpmock::Method::GET).path("/data");
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body(json!("test-data"));
        });

        let result = trackers
            .debug_tracker(TrackerDebugParams {
                target: TrackerTarget::Api(ApiTarget {
                    requests: vec![TargetRequest::new(
                        format!("{}/data", server.base_url()).parse()?,
                    )],
                    configurator: None,
                    extractor: None,
                    params: None,
                }),
                config: Default::default(),
                tags: vec![],
                actions: vec![TrackerAction::ServerLog(Default::default())],
                previous_content: None,
                debug: None,
            })
            .await?;

        api_mock.assert();

        assert_json_snapshot!(result, {
            ".durationMs" => 10,
            ".target.requests[0].durationMs" => 10,
            ".target.requests[0].url" => insta::dynamic_redaction(|value, _| {
                assert!(value.as_str().unwrap().ends_with("/data"));
                "[url]"
            }),
            ".target.requests[0].responseHeaders.date" => "[date]",
            r#".target.requests[0].responseHeaders["x-cache"]"# => "[cache]",
            r#".target.requests[0].responseHeaders["x-cache-lookup"]"# => "[cache]",
            ".target.requests[0].responseHeaders" => insta::sorted_redaction(),
        }, @r###"
        {
          "durationMs": 10,
          "result": "test-data",
          "target": {
            "type": "api",
            "requests": [
              {
                "index": 0,
                "source": "original",
                "url": "[url]",
                "method": "GET",
                "statusCode": 200,
                "responseHeaders": {
                  "content-length": "11",
                  "content-type": "application/json",
                  "date": "[date]",
                  "x-cache": "[cache]",
                  "x-cache-lookup": "[cache]"
                },
                "responseBodyRaw": "\"test-data\"",
                "responseBodyRawSize": 11,
                "responseBodyParsed": "test-data",
                "durationMs": 10
              }
            ]
          },
          "actions": [
            {
              "typeTag": "log",
              "index": 0,
              "skipped": false,
              "payload": "test-data",
              "destination": {
                "type": "log"
              },
              "durationMs": 0
            }
          ]
        }
        "###);

        Ok(())
    }

    #[sqlx::test]
    async fn debug_action_dry_run_with_formatter(pool: PgPool) -> anyhow::Result<()> {
        let server = MockServer::start();
        let mut config = mock_config()?;
        config.components.web_scraper_url = Url::parse(&server.base_url())?;

        let api = mock_api_with_config(pool, config).await?;
        let trackers = api.trackers();

        let api_mock = server.mock(|when, then| {
            when.method(httpmock::Method::GET).path("/data");
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body(json!("original-data"));
        });

        let result = trackers
            .debug_tracker(TrackerDebugParams {
                target: TrackerTarget::Api(ApiTarget {
                    requests: vec![TargetRequest::new(
                        format!("{}/data", server.base_url()).parse()?,
                    )],
                    configurator: None,
                    extractor: None,
                    params: None,
                }),
                config: Default::default(),
                tags: vec![],
                actions: vec![TrackerAction::ServerLog(ServerLogAction {
                    formatter: Some(
                        "(async () => ({ content: JSON.stringify({ formatted: true }) }))();"
                            .to_string(),
                    ),
                })],
                previous_content: None,
                debug: None,
            })
            .await?;

        api_mock.assert();

        assert_json_snapshot!(result, {
            ".durationMs" => 10,
            ".target.requests[0].durationMs" => 11,
            ".target.requests[0].url" => insta::dynamic_redaction(|value, _| {
                assert!(value.as_str().unwrap().ends_with("/data"));
                "[url]"
            }),
            ".target.requests[0].responseHeaders.date" => "[date]",
            r#".target.requests[0].responseHeaders["x-cache"]"# => "[cache]",
            r#".target.requests[0].responseHeaders["x-cache-lookup"]"# => "[cache]",
            ".target.requests[0].responseHeaders" => insta::sorted_redaction(),
            ".actions[0].durationMs" => 12,
            ".actions[0].formatter.durationMs" => 13,
        }, @r###"
        {
          "durationMs": 10,
          "result": "original-data",
          "target": {
            "type": "api",
            "requests": [
              {
                "index": 0,
                "source": "original",
                "url": "[url]",
                "method": "GET",
                "statusCode": 200,
                "responseHeaders": {
                  "content-length": "15",
                  "content-type": "application/json",
                  "date": "[date]",
                  "x-cache": "[cache]",
                  "x-cache-lookup": "[cache]"
                },
                "responseBodyRaw": "\"original-data\"",
                "responseBodyRawSize": 15,
                "responseBodyParsed": "original-data",
                "durationMs": 11
              }
            ]
          },
          "actions": [
            {
              "typeTag": "log",
              "index": 0,
              "formatter": {
                "source": "(async () => ({ content: JSON.stringify({ formatted: true }) }))();",
                "durationMs": 13,
                "result": {
                  "content": "{\"formatted\":true}"
                }
              },
              "skipped": false,
              "payload": "{\"formatted\":true}",
              "destination": {
                "type": "log"
              },
              "durationMs": 12
            }
          ]
        }
        "###);

        Ok(())
    }

    #[sqlx::test]
    async fn debug_action_dry_run_formatter_returns_null_skips(pool: PgPool) -> anyhow::Result<()> {
        let server = MockServer::start();
        let mut config = mock_config()?;
        config.components.web_scraper_url = Url::parse(&server.base_url())?;

        let api = mock_api_with_config(pool, config).await?;
        let trackers = api.trackers();

        let api_mock = server.mock(|when, then| {
            when.method(httpmock::Method::GET).path("/data");
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body(json!("test"));
        });

        let result = trackers
            .debug_tracker(TrackerDebugParams {
                target: TrackerTarget::Api(ApiTarget {
                    requests: vec![TargetRequest::new(
                        format!("{}/data", server.base_url()).parse()?,
                    )],
                    configurator: None,
                    extractor: None,
                    params: None,
                }),
                config: Default::default(),
                tags: vec![],
                actions: vec![TrackerAction::ServerLog(ServerLogAction {
                    formatter: Some("(async () => ({ content: null }))();".to_string()),
                })],
                previous_content: None,
                debug: None,
            })
            .await?;

        api_mock.assert();

        assert_json_snapshot!(result, {
            ".durationMs" => 10,
            ".target.requests[0].durationMs" => 11,
            ".target.requests[0].url" => insta::dynamic_redaction(|value, _| {
                assert!(value.as_str().unwrap().ends_with("/data"));
                "[url]"
            }),
            ".target.requests[0].responseHeaders.date" => "[date]",
            r#".target.requests[0].responseHeaders["x-cache"]"# => "[cache]",
            r#".target.requests[0].responseHeaders["x-cache-lookup"]"# => "[cache]",
            ".target.requests[0].responseHeaders" => insta::sorted_redaction(),
            ".actions[0].durationMs" => 12,
            ".actions[0].formatter.durationMs" => 13,
        }, @r###"
        {
          "durationMs": 10,
          "result": "test",
          "target": {
            "type": "api",
            "requests": [
              {
                "index": 0,
                "source": "original",
                "url": "[url]",
                "method": "GET",
                "statusCode": 200,
                "responseHeaders": {
                  "content-length": "6",
                  "content-type": "application/json",
                  "date": "[date]",
                  "x-cache": "[cache]",
                  "x-cache-lookup": "[cache]"
                },
                "responseBodyRaw": "\"test\"",
                "responseBodyRawSize": 6,
                "responseBodyParsed": "test",
                "durationMs": 11
              }
            ]
          },
          "actions": [
            {
              "typeTag": "log",
              "index": 0,
              "formatter": {
                "source": "(async () => ({ content: null }))();",
                "durationMs": 13
              },
              "skipped": true,
              "destination": {
                "type": "log"
              },
              "durationMs": 12
            }
          ]
        }
        "###);

        Ok(())
    }

    #[sqlx::test]
    async fn debug_action_dry_run_webhook_destination(pool: PgPool) -> anyhow::Result<()> {
        let server = MockServer::start();
        let mut config = mock_config()?;
        config.components.web_scraper_url = Url::parse(&server.base_url())?;

        let api = mock_api_with_config(pool, config).await?;
        let trackers = api.trackers();

        let api_mock = server.mock(|when, then| {
            when.method(httpmock::Method::GET).path("/data");
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body(json!("data"));
        });

        let result = trackers
            .debug_tracker(TrackerDebugParams {
                target: TrackerTarget::Api(ApiTarget {
                    requests: vec![TargetRequest::new(
                        format!("{}/data", server.base_url()).parse()?,
                    )],
                    configurator: None,
                    extractor: None,
                    params: None,
                }),
                config: Default::default(),
                tags: vec![],
                actions: vec![TrackerAction::Webhook(WebhookAction {
                    url: "https://hooks.example.com/trigger".parse()?,
                    method: Some(Method::PUT),
                    headers: None,
                    formatter: None,
                })],
                previous_content: None,
                debug: None,
            })
            .await?;

        api_mock.assert();

        assert_json_snapshot!(result, {
            ".durationMs" => 10,
            ".target.requests[0].durationMs" => 11,
            ".target.requests[0].url" => insta::dynamic_redaction(|value, _| {
                assert!(value.as_str().unwrap().ends_with("/data"));
                "[url]"
            }),
            ".target.requests[0].responseHeaders.date" => "[date]",
            r#".target.requests[0].responseHeaders["x-cache"]"# => "[cache]",
            r#".target.requests[0].responseHeaders["x-cache-lookup"]"# => "[cache]",
            ".target.requests[0].responseHeaders" => insta::sorted_redaction(),
            ".actions[0].durationMs" => 12,
        }, @r###"
        {
          "durationMs": 10,
          "result": "data",
          "target": {
            "type": "api",
            "requests": [
              {
                "index": 0,
                "source": "original",
                "url": "[url]",
                "method": "GET",
                "statusCode": 200,
                "responseHeaders": {
                  "content-length": "6",
                  "content-type": "application/json",
                  "date": "[date]",
                  "x-cache": "[cache]",
                  "x-cache-lookup": "[cache]"
                },
                "responseBodyRaw": "\"data\"",
                "responseBodyRawSize": 6,
                "responseBodyParsed": "data",
                "durationMs": 11
              }
            ]
          },
          "actions": [
            {
              "typeTag": "webhook",
              "index": 0,
              "skipped": false,
              "payload": "data",
              "destination": {
                "type": "webhook",
                "url": "https://hooks.example.com/trigger",
                "method": "PUT"
              },
              "durationMs": 12
            }
          ]
        }
        "###);

        Ok(())
    }

    #[sqlx::test]
    async fn debug_action_dry_run_extraction_failed(pool: PgPool) -> anyhow::Result<()> {
        let api = mock_api(pool).await?;
        let trackers = api.trackers();

        let result = trackers
            .debug_tracker(TrackerDebugParams {
                target: TrackerTarget::Api(ApiTarget {
                    requests: vec![TargetRequest::new(
                        "http://127.0.0.1:1/nonexistent".parse()?,
                    )],
                    configurator: None,
                    extractor: None,
                    params: None,
                }),
                config: TrackerConfig {
                    revisions: 0,
                    timeout: Some(Duration::from_millis(500)),
                    job: None,
                },
                tags: vec![],
                actions: vec![TrackerAction::ServerLog(Default::default())],
                previous_content: None,
                debug: None,
            })
            .await?;

        assert_json_snapshot!(result, {
            ".durationMs" => 10,
            ".target.requests[0].durationMs" => 11,
            ".actions[0].durationMs" => 12,
        }, @r###"
        {
          "durationMs": 10,
          "target": {
            "type": "api",
            "requests": [
              {
                "index": 0,
                "source": "original",
                "url": "http://127.0.0.1:1/nonexistent",
                "method": "GET",
                "durationMs": 11,
                "error": "Request failed: Cache error: error sending request for url (http://127.0.0.1:1/nonexistent)"
              }
            ]
          },
          "actions": [
            {
              "typeTag": "log",
              "index": 0,
              "skipped": false,
              "destination": {
                "type": "log"
              },
              "durationMs": 12,
              "error": "Extraction failed, no revision available"
            }
          ]
        }
        "###);

        Ok(())
    }

    #[sqlx::test]
    async fn debug_api_target_with_extractor(pool: PgPool) -> anyhow::Result<()> {
        let server = MockServer::start();
        let mut config = mock_config()?;
        config.components.web_scraper_url = Url::parse(&server.base_url())?;

        let api = mock_api_with_config(pool, config).await?;
        let trackers = api.trackers();

        let api_mock = server.mock(|when, then| {
            when.method(httpmock::Method::GET).path("/data");
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body(json!({"items": [1, 2, 3]}));
        });

        let extractor_script =
            "(async () => ({ body: Deno.core.encode(JSON.stringify({ extracted: true })) }))();";

        let result = trackers
            .debug_tracker(TrackerDebugParams {
                target: TrackerTarget::Api(ApiTarget {
                    requests: vec![TargetRequest::new(
                        format!("{}/data", server.base_url()).parse()?,
                    )],
                    configurator: None,
                    extractor: Some(extractor_script.to_string()),
                    params: None,
                }),
                config: Default::default(),
                tags: vec![],
                actions: vec![],
                previous_content: None,
                debug: None,
            })
            .await?;

        api_mock.assert();

        assert_json_snapshot!(result, {
            ".durationMs" => 10,
            ".target.requests[0].durationMs" => 11,
            ".target.requests[0].url" => insta::dynamic_redaction(|value, _| {
                assert!(value.as_str().unwrap().ends_with("/data"));
                "[url]"
            }),
            ".target.requests[0].responseHeaders.date" => "[date]",
            r#".target.requests[0].responseHeaders["x-cache"]"# => "[cache]",
            r#".target.requests[0].responseHeaders["x-cache-lookup"]"# => "[cache]",
            ".target.requests[0].responseHeaders" => insta::sorted_redaction(),
            ".target.extractor.durationMs" => 14,
        }, @r###"
        {
          "durationMs": 10,
          "result": {
            "extracted": true
          },
          "target": {
            "type": "api",
            "requests": [
              {
                "index": 0,
                "source": "original",
                "url": "[url]",
                "method": "GET",
                "statusCode": 200,
                "responseHeaders": {
                  "content-length": "17",
                  "content-type": "application/json",
                  "date": "[date]",
                  "x-cache": "[cache]",
                  "x-cache-lookup": "[cache]"
                },
                "responseBodyRaw": "{\"items\":[1,2,3]}",
                "responseBodyRawSize": 17,
                "responseBodyParsed": {
                  "items": [
                    1,
                    2,
                    3
                  ]
                },
                "durationMs": 11
              }
            ],
            "extractor": {
              "source": "(async () => ({ body: Deno.core.encode(JSON.stringify({ extracted: true })) }))();",
              "durationMs": 14,
              "result": {
                "extracted": true
              }
            }
          }
        }
        "###);

        Ok(())
    }

    #[sqlx::test]
    async fn debug_api_target_with_configurator(pool: PgPool) -> anyhow::Result<()> {
        let server = MockServer::start();
        let mut config = mock_config()?;
        config.components.web_scraper_url = Url::parse(&server.base_url())?;

        let api = mock_api_with_config(pool, config).await?;
        let trackers = api.trackers();

        let configurator_url = format!("{}/modified-data", server.base_url());
        let configurator_script = format!(
            "(async () => ({{ requests: [{{ url: '{}' }}] }}))();",
            configurator_url
        );

        let modified_mock = server.mock(|when, then| {
            when.method(httpmock::Method::GET).path("/modified-data");
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body(json!({"configured": true}));
        });

        let result = trackers
            .debug_tracker(TrackerDebugParams {
                target: TrackerTarget::Api(ApiTarget {
                    requests: vec![TargetRequest::new(
                        format!("{}/original-data", server.base_url()).parse()?,
                    )],
                    configurator: Some(configurator_script),
                    extractor: None,
                    params: None,
                }),
                config: Default::default(),
                tags: vec![],
                actions: vec![],
                previous_content: None,
                debug: None,
            })
            .await?;

        modified_mock.assert();

        assert_json_snapshot!(result, {
            ".durationMs" => 10,
            ".target.configurator.durationMs" => 14,
            ".target.requests[0].durationMs" => 11,
            ".target.requests[0].url" => insta::dynamic_redaction(|value, _| {
                assert!(value.as_str().unwrap().ends_with("/modified-data"));
                "[url]"
            }),
            ".target.requests[0].responseHeaders.date" => "[date]",
            r#".target.requests[0].responseHeaders["x-cache"]"# => "[cache]",
            r#".target.requests[0].responseHeaders["x-cache-lookup"]"# => "[cache]",
            ".target.requests[0].responseHeaders" => insta::sorted_redaction(),
           ".target.configurator.source" => "[script]",
            ".target.configurator.result.requests[0].url" => insta::dynamic_redaction(|value, _| {
                assert!(value.as_str().unwrap().ends_with("/modified-data"));
                "[url]"
            }),
        }, @r###"
        {
          "durationMs": 10,
          "result": {
            "configured": true
          },
          "target": {
            "type": "api",
            "configurator": {
              "source": "[script]",
              "durationMs": 14,
              "result": {
                "requests": [
                  {
                    "url": "[url]"
                  }
                ]
              }
            },
            "requests": [
              {
                "index": 0,
                "source": "configurator",
                "url": "[url]",
                "method": "GET",
                "statusCode": 200,
                "responseHeaders": {
                  "content-length": "19",
                  "content-type": "application/json",
                  "date": "[date]",
                  "x-cache": "[cache]",
                  "x-cache-lookup": "[cache]"
                },
                "responseBodyRaw": "{\"configured\":true}",
                "responseBodyRawSize": 19,
                "responseBodyParsed": {
                  "configured": true
                },
                "durationMs": 11
              }
            ]
          }
        }
        "###);

        Ok(())
    }

    #[sqlx::test]
    async fn debug_api_target_with_mock_responses(pool: PgPool) -> anyhow::Result<()> {
        let server = MockServer::start();
        let mut config = mock_config()?;
        config.components.web_scraper_url = Url::parse(&server.base_url())?;

        let api = mock_api_with_config(pool, config).await?;
        let trackers = api.trackers();

        let configurator_script = r#"(async () => ({ responses: [{ status: 200, headers: {}, body: Deno.core.encode(JSON.stringify({"mock":true})) }] }))();"#;

        let result = trackers
            .debug_tracker(TrackerDebugParams {
                target: TrackerTarget::Api(ApiTarget {
                    requests: vec![TargetRequest::new(
                        format!("{}/original", server.base_url()).parse()?,
                    )],
                    configurator: Some(configurator_script.to_string()),
                    extractor: None,
                    params: None,
                }),
                config: Default::default(),
                tags: vec![],
                actions: vec![],
                previous_content: None,
                debug: None,
            })
            .await?;

        assert_json_snapshot!(result, {
            ".durationMs" => 10,
            ".target.configurator.durationMs" => 14,
            ".target.requests[0].durationMs" => 11,
        }, @r###"
        {
          "durationMs": 10,
          "result": {
            "mock": true
          },
          "target": {
            "type": "api",
            "configurator": {
              "source": "(async () => ({ responses: [{ status: 200, headers: {}, body: Deno.core.encode(JSON.stringify({\"mock\":true})) }] }))();",
              "durationMs": 14,
              "result": {
                "responses": [
                  {
                    "status": 200,
                    "headers": {},
                    "body": {
                      "mock": true
                    }
                  }
                ]
              }
            },
            "requests": [
              {
                "index": 0,
                "source": "mock",
                "statusCode": 200,
                "responseHeaders": {},
                "responseBodyRaw": "{\"mock\":true}",
                "responseBodyRawSize": 13,
                "responseBodyParsed": {
                  "mock": true
                },
                "durationMs": 11
              }
            ]
          }
        }
        "###);

        Ok(())
    }

    #[sqlx::test]
    async fn debug_api_target_multiple_requests(pool: PgPool) -> anyhow::Result<()> {
        let server = MockServer::start();
        let mut config = mock_config()?;
        config.components.web_scraper_url = Url::parse(&server.base_url())?;

        let api = mock_api_with_config(pool, config).await?;
        let trackers = api.trackers();

        let mock_one = server.mock(|when, then| {
            when.method(httpmock::Method::GET).path("/data1");
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body(json!({"page": 1}));
        });

        let mock_two = server.mock(|when, then| {
            when.method(httpmock::Method::GET).path("/data2");
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body(json!({"page": 2}));
        });

        let result = trackers
            .debug_tracker(TrackerDebugParams {
                target: TrackerTarget::Api(ApiTarget {
                    requests: vec![
                        TargetRequest::new(format!("{}/data1", server.base_url()).parse()?),
                        TargetRequest::new(format!("{}/data2", server.base_url()).parse()?),
                    ],
                    configurator: None,
                    extractor: None,
                    params: None,
                }),
                config: Default::default(),
                tags: vec![],
                actions: vec![],
                previous_content: None,
                debug: None,
            })
            .await?;

        mock_one.assert();
        mock_two.assert();

        assert_json_snapshot!(result, {
            ".durationMs" => 10,
            ".target.requests[0].durationMs" => 11,
            ".target.requests[0].url" => insta::dynamic_redaction(|value, _| {
                assert!(value.as_str().unwrap().ends_with("/data1"));
                "[url1]"
            }),
            ".target.requests[0].responseHeaders.date" => "[date]",
            r#".target.requests[0].responseHeaders["x-cache"]"# => "[cache]",
            r#".target.requests[0].responseHeaders["x-cache-lookup"]"# => "[cache]",
            ".target.requests[0].responseHeaders" => insta::sorted_redaction(),
            ".target.requests[1].durationMs" => 15,
            ".target.requests[1].url" => insta::dynamic_redaction(|value, _| {
                assert!(value.as_str().unwrap().ends_with("/data2"));
                "[url2]"
            }),
            ".target.requests[1].responseHeaders.date" => "[date]",
            r#".target.requests[1].responseHeaders["x-cache"]"# => "[cache]",
            r#".target.requests[1].responseHeaders["x-cache-lookup"]"# => "[cache]",
            ".target.requests[1].responseHeaders" => insta::sorted_redaction(),
        }, @r###"
        {
          "durationMs": 10,
          "result": [
            {
              "page": 1
            },
            {
              "page": 2
            }
          ],
          "target": {
            "type": "api",
            "requests": [
              {
                "index": 0,
                "source": "original",
                "url": "[url1]",
                "method": "GET",
                "statusCode": 200,
                "responseHeaders": {
                  "content-length": "10",
                  "content-type": "application/json",
                  "date": "[date]",
                  "x-cache": "[cache]",
                  "x-cache-lookup": "[cache]"
                },
                "responseBodyRaw": "{\"page\":1}",
                "responseBodyRawSize": 10,
                "responseBodyParsed": {
                  "page": 1
                },
                "durationMs": 11
              },
              {
                "index": 1,
                "source": "original",
                "url": "[url2]",
                "method": "GET",
                "statusCode": 200,
                "responseHeaders": {
                  "content-length": "10",
                  "content-type": "application/json",
                  "date": "[date]",
                  "x-cache": "[cache]",
                  "x-cache-lookup": "[cache]"
                },
                "responseBodyRaw": "{\"page\":2}",
                "responseBodyRawSize": 10,
                "responseBodyParsed": {
                  "page": 2
                },
                "durationMs": 15
              }
            ]
          }
        }
        "###);

        Ok(())
    }

    #[sqlx::test]
    async fn debug_api_target_multiple_requests_html(pool: PgPool) -> anyhow::Result<()> {
        let server = MockServer::start();
        let mut config = mock_config()?;
        config.components.web_scraper_url = Url::parse(&server.base_url())?;

        let api = mock_api_with_config(pool, config).await?;
        let trackers = api.trackers();

        let mock_one = server.mock(|when, then| {
            when.method(httpmock::Method::GET).path("/page1");
            then.status(200)
                .header("Content-Type", "text/html")
                .body("<html><body><h1>Page One</h1></body></html>");
        });

        let mock_two = server.mock(|when, then| {
            when.method(httpmock::Method::GET).path("/page2");
            then.status(200)
                .header("Content-Type", "text/html")
                .body("<html><body><h1>Page Two</h1></body></html>");
        });

        let result = trackers
            .debug_tracker(TrackerDebugParams {
                target: TrackerTarget::Api(ApiTarget {
                    requests: vec![
                        TargetRequest::new(format!("{}/page1", server.base_url()).parse()?),
                        TargetRequest::new(format!("{}/page2", server.base_url()).parse()?),
                    ],
                    configurator: None,
                    extractor: None,
                    params: None,
                }),
                config: Default::default(),
                tags: vec![],
                actions: vec![],
                previous_content: None,
                debug: None,
            })
            .await?;

        mock_one.assert();
        mock_two.assert();

        assert_json_snapshot!(result, {
            ".durationMs" => 10,
            ".target.requests[0].durationMs" => 11,
            ".target.requests[0].url" => insta::dynamic_redaction(|value, _| {
                assert!(value.as_str().unwrap().ends_with("/page1"));
                "[url1]"
            }),
            ".target.requests[0].responseHeaders.date" => "[date]",
            r#".target.requests[0].responseHeaders["x-cache"]"# => "[cache]",
            r#".target.requests[0].responseHeaders["x-cache-lookup"]"# => "[cache]",
            ".target.requests[0].responseHeaders" => insta::sorted_redaction(),
            ".target.requests[1].durationMs" => 15,
            ".target.requests[1].url" => insta::dynamic_redaction(|value, _| {
                assert!(value.as_str().unwrap().ends_with("/page2"));
                "[url2]"
            }),
            ".target.requests[1].responseHeaders.date" => "[date]",
            r#".target.requests[1].responseHeaders["x-cache"]"# => "[cache]",
            r#".target.requests[1].responseHeaders["x-cache-lookup"]"# => "[cache]",
            ".target.requests[1].responseHeaders" => insta::sorted_redaction(),
        }, @r###"
        {
          "durationMs": 10,
          "result": [
            "<html><body><h1>Page One</h1></body></html>",
            "<html><body><h1>Page Two</h1></body></html>"
          ],
          "target": {
            "type": "api",
            "requests": [
              {
                "index": 0,
                "source": "original",
                "url": "[url1]",
                "method": "GET",
                "statusCode": 200,
                "responseHeaders": {
                  "content-length": "43",
                  "content-type": "text/html",
                  "date": "[date]",
                  "x-cache": "[cache]",
                  "x-cache-lookup": "[cache]"
                },
                "responseBodyRaw": "<html><body><h1>Page One</h1></body></html>",
                "responseBodyRawSize": 43,
                "durationMs": 11
              },
              {
                "index": 1,
                "source": "original",
                "url": "[url2]",
                "method": "GET",
                "statusCode": 200,
                "responseHeaders": {
                  "content-length": "43",
                  "content-type": "text/html",
                  "date": "[date]",
                  "x-cache": "[cache]",
                  "x-cache-lookup": "[cache]"
                },
                "responseBodyRaw": "<html><body><h1>Page Two</h1></body></html>",
                "responseBodyRawSize": 43,
                "durationMs": 15
              }
            ]
          }
        }
        "###);

        Ok(())
    }

    #[sqlx::test]
    async fn debug_matches_revision_api_simple(pool: PgPool) -> anyhow::Result<()> {
        let server = MockServer::start();
        let mut config = mock_config()?;
        config.components.web_scraper_url = Url::parse(&server.base_url())?;

        let api = mock_api_with_config(pool, config).await?;
        let trackers = api.trackers();

        let api_mock = server.mock(|when, then| {
            when.method(httpmock::Method::GET).path("/data");
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body(json!({"key": "value", "count": 42}));
        });

        let target = TrackerTarget::Api(ApiTarget {
            requests: vec![TargetRequest::new(
                format!("{}/data", server.base_url()).parse()?,
            )],
            configurator: None,
            extractor: None,
            params: None,
        });

        let debug_result = trackers
            .debug_tracker(TrackerDebugParams {
                target: target.clone(),
                config: Default::default(),
                tags: vec![],
                actions: vec![],
                previous_content: None,
                debug: None,
            })
            .await?;

        let tracker = trackers
            .create_tracker(
                TrackerCreateParamsBuilder::new("debug-vs-real")
                    .with_schedule("0 0 * * * *")
                    .with_target(target)
                    .with_actions(vec![])
                    .build(),
            )
            .await?;
        let revision = trackers.create_tracker_data_revision(tracker.id).await?;

        api_mock.assert_calls(2);
        assert_eq!(
            debug_result.result.as_ref(),
            Some(revision.data.original()),
            "debug result must equal the real revision"
        );

        Ok(())
    }

    #[sqlx::test]
    async fn debug_matches_revision_api_with_extractor(pool: PgPool) -> anyhow::Result<()> {
        let server = MockServer::start();
        let mut config = mock_config()?;
        config.components.web_scraper_url = Url::parse(&server.base_url())?;

        let api = mock_api_with_config(pool, config).await?;
        let trackers = api.trackers();

        let api_mock = server.mock(|when, then| {
            when.method(httpmock::Method::GET).path("/data");
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body(json!({"items": [1, 2, 3], "total": 3}));
        });

        let extractor =
            "(async () => ({ body: Deno.core.encode(JSON.stringify({ extracted: true })) }))();";
        let target = TrackerTarget::Api(ApiTarget {
            requests: vec![TargetRequest::new(
                format!("{}/data", server.base_url()).parse()?,
            )],
            configurator: None,
            extractor: Some(extractor.to_string()),
            params: None,
        });

        let debug_result = trackers
            .debug_tracker(TrackerDebugParams {
                target: target.clone(),
                config: Default::default(),
                tags: vec![],
                actions: vec![],
                previous_content: None,
                debug: None,
            })
            .await?;

        let tracker = trackers
            .create_tracker(
                TrackerCreateParamsBuilder::new("debug-vs-real-extractor")
                    .with_schedule("0 0 * * * *")
                    .with_target(target)
                    .with_actions(vec![])
                    .build(),
            )
            .await?;
        let revision = trackers.create_tracker_data_revision(tracker.id).await?;

        api_mock.assert_calls(2);
        assert_eq!(
            debug_result.result.as_ref(),
            Some(revision.data.original()),
            "debug result with extractor must equal the real revision"
        );

        let TrackerDebugTargetResult::Api(ref api_debug) = debug_result.target else {
            panic!("expected API target debug result");
        };
        assert!(api_debug.extractor.is_some());
        assert!(api_debug.extractor.as_ref().unwrap().error.is_none());
        assert_eq!(
            api_debug.extractor.as_ref().unwrap().result,
            Some(json!({"extracted": true}))
        );

        Ok(())
    }

    #[sqlx::test]
    async fn debug_matches_revision_api_with_configurator(pool: PgPool) -> anyhow::Result<()> {
        let server = MockServer::start();
        let mut config = mock_config()?;
        config.components.web_scraper_url = Url::parse(&server.base_url())?;

        let api = mock_api_with_config(pool, config).await?;
        let trackers = api.trackers();

        let redirect_url = format!("{}/redirected", server.base_url());
        let configurator_script = format!(
            "(async () => ({{ requests: [{{ url: '{}' }}] }}))();",
            redirect_url
        );

        let redirect_mock = server.mock(|when, then| {
            when.method(httpmock::Method::GET).path("/redirected");
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body(json!({"from_configurator": true}));
        });

        let target = TrackerTarget::Api(ApiTarget {
            requests: vec![TargetRequest::new(
                format!("{}/original", server.base_url()).parse()?,
            )],
            configurator: Some(configurator_script),
            extractor: None,
            params: None,
        });

        let debug_result = trackers
            .debug_tracker(TrackerDebugParams {
                target: target.clone(),
                config: Default::default(),
                tags: vec![],
                actions: vec![],
                previous_content: None,
                debug: None,
            })
            .await?;

        let tracker = trackers
            .create_tracker(
                TrackerCreateParamsBuilder::new("debug-vs-real-configurator")
                    .with_schedule("0 0 * * * *")
                    .with_target(target)
                    .with_actions(vec![])
                    .build(),
            )
            .await?;
        let revision = trackers.create_tracker_data_revision(tracker.id).await?;

        redirect_mock.assert_calls(2);
        assert_eq!(
            debug_result.result.as_ref(),
            Some(revision.data.original()),
            "debug result with configurator must equal the real revision"
        );

        let TrackerDebugTargetResult::Api(ref api_debug) = debug_result.target else {
            panic!("expected API target debug result");
        };
        assert!(api_debug.configurator.is_some());
        assert!(api_debug.configurator.as_ref().unwrap().error.is_none());
        assert_eq!(api_debug.requests[0].source, "configurator");

        Ok(())
    }

    #[sqlx::test]
    async fn debug_matches_revision_api_with_configurator_and_extractor(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        let server = MockServer::start();
        let mut config = mock_config()?;
        config.components.web_scraper_url = Url::parse(&server.base_url())?;

        let api = mock_api_with_config(pool, config).await?;
        let trackers = api.trackers();

        let redirect_url = format!("{}/cfg-data", server.base_url());
        let configurator_script = format!(
            "(async () => ({{ requests: [{{ url: '{}' }}] }}))();",
            redirect_url
        );
        let extractor = "(async () => ({ body: Deno.core.encode(JSON.stringify({ processed: context.responses[0].status })) }))();";

        let cfg_mock = server.mock(|when, then| {
            when.method(httpmock::Method::GET).path("/cfg-data");
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body(json!({"raw": "data"}));
        });

        let target = TrackerTarget::Api(ApiTarget {
            requests: vec![TargetRequest::new(
                format!("{}/original", server.base_url()).parse()?,
            )],
            configurator: Some(configurator_script),
            extractor: Some(extractor.to_string()),
            params: None,
        });

        let debug_result = trackers
            .debug_tracker(TrackerDebugParams {
                target: target.clone(),
                config: Default::default(),
                tags: vec![],
                actions: vec![],
                previous_content: None,
                debug: None,
            })
            .await?;

        let tracker = trackers
            .create_tracker(
                TrackerCreateParamsBuilder::new("debug-vs-real-both")
                    .with_schedule("0 0 * * * *")
                    .with_target(target)
                    .with_actions(vec![])
                    .build(),
            )
            .await?;
        let revision = trackers.create_tracker_data_revision(tracker.id).await?;

        cfg_mock.assert_calls(2);
        assert_eq!(
            debug_result.result.as_ref(),
            Some(revision.data.original()),
            "debug result with configurator+extractor must equal the real revision"
        );

        let TrackerDebugTargetResult::Api(ref api_debug) = debug_result.target else {
            panic!("expected API target debug result");
        };
        assert!(api_debug.configurator.is_some());
        assert!(api_debug.extractor.is_some());

        Ok(())
    }

    #[sqlx::test]
    async fn debug_matches_revision_api_with_mock_responses(pool: PgPool) -> anyhow::Result<()> {
        let server = MockServer::start();
        let mut config = mock_config()?;
        config.components.web_scraper_url = Url::parse(&server.base_url())?;

        let api = mock_api_with_config(pool, config).await?;
        let trackers = api.trackers();

        let configurator_script = r#"(async () => ({ responses: [{ status: 200, headers: {}, body: Deno.core.encode(JSON.stringify({"mocked":true})) }] }))();"#;

        let target = TrackerTarget::Api(ApiTarget {
            requests: vec![TargetRequest::new(
                format!("{}/unused", server.base_url()).parse()?,
            )],
            configurator: Some(configurator_script.to_string()),
            extractor: None,
            params: None,
        });

        let debug_result = trackers
            .debug_tracker(TrackerDebugParams {
                target: target.clone(),
                config: Default::default(),
                tags: vec![],
                actions: vec![],
                previous_content: None,
                debug: None,
            })
            .await?;

        let tracker = trackers
            .create_tracker(
                TrackerCreateParamsBuilder::new("debug-vs-real-mock")
                    .with_schedule("0 0 * * * *")
                    .with_target(target)
                    .with_actions(vec![])
                    .build(),
            )
            .await?;
        let revision = trackers.create_tracker_data_revision(tracker.id).await?;

        assert_eq!(
            debug_result.result.as_ref(),
            Some(revision.data.original()),
            "debug result with mock responses must equal the real revision"
        );

        let TrackerDebugTargetResult::Api(ref api_debug) = debug_result.target else {
            panic!("expected API target debug result");
        };
        assert!(api_debug.configurator.is_some());
        assert_eq!(api_debug.requests[0].source, "mock");

        Ok(())
    }

    #[sqlx::test]
    async fn debug_matches_revision_api_with_actions(pool: PgPool) -> anyhow::Result<()> {
        let server = MockServer::start();
        let mut config = mock_config()?;
        config.components.web_scraper_url = Url::parse(&server.base_url())?;

        let api = mock_api_with_config(pool, config).await?;
        let trackers = api.trackers();

        let api_mock = server.mock(|when, then| {
            when.method(httpmock::Method::GET).path("/data");
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body(json!("action-test-data"));
        });

        let formatter =
            "(async () => ({ content: JSON.stringify({ formatted: true }) }))();".to_string();
        let actions = vec![
            TrackerAction::ServerLog(ServerLogAction {
                formatter: Some(formatter),
            }),
            TrackerAction::Webhook(WebhookAction {
                url: "https://hooks.example.com/test".parse()?,
                method: Some(Method::POST),
                headers: None,
                formatter: None,
            }),
        ];

        let target = TrackerTarget::Api(ApiTarget {
            requests: vec![TargetRequest::new(
                format!("{}/data", server.base_url()).parse()?,
            )],
            configurator: None,
            extractor: None,
            params: None,
        });

        let debug_result = trackers
            .debug_tracker(TrackerDebugParams {
                target: target.clone(),
                config: Default::default(),
                tags: vec![],
                actions: actions.clone(),
                previous_content: None,
                debug: None,
            })
            .await?;

        api_mock.assert();
        assert_eq!(
            debug_result.result,
            Some(json!("action-test-data")),
            "debug result must match the expected data"
        );

        assert_eq!(debug_result.actions.len(), 2, "expected 2 action dry-runs");
        assert_eq!(debug_result.actions[0].type_tag, "log");
        assert!(!debug_result.actions[0].skipped);
        assert_eq!(
            debug_result.actions[0].payload,
            Some(json!("{\"formatted\":true}"))
        );

        assert_eq!(debug_result.actions[1].type_tag, "webhook");
        assert!(!debug_result.actions[1].skipped);
        assert_eq!(
            debug_result.actions[1].payload,
            Some(json!("action-test-data"))
        );

        Ok(())
    }
}
