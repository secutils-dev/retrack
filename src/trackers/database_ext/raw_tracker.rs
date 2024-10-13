use http::{HeaderMap, HeaderName, HeaderValue, Method};
use mediatype::MediaType;
use retrack_types::{
    scheduler::{SchedulerJobConfig, SchedulerJobRetryStrategy},
    trackers::{
        ApiTarget, EmailAction, PageTarget, Tracker, TrackerAction, TrackerConfig, TrackerTarget,
        WebhookAction,
    },
};
use serde_derive::{Deserialize, Serialize};
use serde_with::serde_as;
use std::{borrow::Cow, collections::HashMap, str::FromStr, time::Duration};
use time::OffsetDateTime;
use uuid::Uuid;

/// The type used to serialize and deserialize tracker database representation. There are a number
/// of helper structs here to help with the conversion from original types that are tailored for
/// JSON serialization to the types that are tailored for Postcard/database serialization.
#[derive(Debug, Eq, PartialEq, Clone)]
pub(super) struct RawTracker {
    pub id: Uuid,
    pub name: String,
    pub enabled: bool,
    pub config: Vec<u8>,
    pub tags: Vec<String>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
    pub job_id: Option<Uuid>,
    pub job_needed: bool,
}

#[serde_as]
#[derive(Serialize, Deserialize, Debug, Eq, PartialEq, Clone)]
pub struct RawTrackerConfig<'s> {
    revisions: usize,
    timeout: Option<Duration>,
    #[serde(borrow)]
    target: RawTrackerTarget<'s>,
    actions: Vec<RawTrackerAction<'s>>,
    job: Option<RawSchedulerJobConfig<'s>>,
}

#[derive(Serialize, Deserialize, Debug, Eq, PartialEq, Clone)]
struct RawSchedulerJobConfig<'s>(Cow<'s, str>, Option<RawSchedulerJobRetryStrategy>);

#[derive(Serialize, Deserialize, Copy, Clone, Debug, Eq, PartialEq)]
enum RawSchedulerJobRetryStrategy {
    Constant(Duration, u32),
    Exponential(Duration, u32, Duration, u32),
    Linear(Duration, Duration, Duration, u32),
}

#[derive(Serialize, Deserialize, Debug, Eq, PartialEq, Clone)]
enum RawTrackerTarget<'s> {
    Page(RawPageTarget<'s>),
    #[serde(borrow)]
    Api(RawApiTarget<'s>),
}

#[derive(Serialize, Deserialize, Debug, Eq, PartialEq, Clone)]
struct RawPageTarget<'s> {
    extractor: Cow<'s, str>,
    user_agent: Option<Cow<'s, str>>,
    ignore_https_errors: Option<bool>,
}

#[derive(Serialize, Deserialize, Debug, Eq, PartialEq, Clone)]
struct RawApiTarget<'s> {
    url: Cow<'s, str>,
    #[serde(with = "http_serde::option::method", default)]
    method: Option<Method>,
    headers: Option<HashMap<Cow<'s, str>, Cow<'s, str>>>,
    body: Option<Vec<u8>>,
    #[serde(borrow)]
    media_type: Option<MediaType<'s>>,
    configurator: Option<Cow<'s, str>>,
    extractor: Option<Cow<'s, str>>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Eq, PartialEq)]
enum RawTrackerAction<'s> {
    Email {
        to: Cow<'s, Vec<String>>,
    },
    Webhook {
        url: String,
        #[serde(with = "http_serde::option::method", default)]
        method: Option<Method>,
        headers: Option<HashMap<Cow<'s, str>, Cow<'s, str>>>,
    },
    ServerLog,
}

impl TryFrom<RawTracker> for Tracker {
    type Error = anyhow::Error;

    fn try_from(raw: RawTracker) -> Result<Self, Self::Error> {
        let raw_config = postcard::from_bytes::<RawTrackerConfig>(&raw.config)?;

        let job_config =
            if let Some(RawSchedulerJobConfig(schedule, retry_strategy)) = raw_config.job {
                Some(SchedulerJobConfig {
                    schedule: schedule.into_owned(),
                    retry_strategy: retry_strategy.map(|retry_strategy| match retry_strategy {
                        RawSchedulerJobRetryStrategy::Constant(interval, max_attempts) => {
                            SchedulerJobRetryStrategy::Constant {
                                interval,
                                max_attempts,
                            }
                        }
                        RawSchedulerJobRetryStrategy::Exponential(
                            initial_interval,
                            multiplier,
                            max_interval,
                            max_attempts,
                        ) => SchedulerJobRetryStrategy::Exponential {
                            initial_interval,
                            multiplier,
                            max_interval,
                            max_attempts,
                        },
                        RawSchedulerJobRetryStrategy::Linear(
                            initial_interval,
                            increment,
                            max_interval,
                            max_attempts,
                        ) => SchedulerJobRetryStrategy::Linear {
                            initial_interval,
                            increment,
                            max_interval,
                            max_attempts,
                        },
                    }),
                })
            } else {
                None
            };

        Ok(Tracker {
            id: raw.id,
            name: raw.name,
            enabled: raw.enabled,
            target: match raw_config.target {
                RawTrackerTarget::Page(target) => TrackerTarget::Page(PageTarget {
                    extractor: target.extractor.into_owned(),
                    user_agent: target.user_agent.map(Cow::into_owned),
                    ignore_https_errors: target.ignore_https_errors.unwrap_or_default(),
                }),
                RawTrackerTarget::Api(target) => TrackerTarget::Api(ApiTarget {
                    url: target.url.into_owned().parse()?,
                    method: target.method,
                    headers: if let Some(headers) = target.headers {
                        let mut header_map = HeaderMap::new();
                        for (k, v) in headers {
                            header_map
                                .insert(HeaderName::from_str(&k)?, HeaderValue::from_str(&v)?);
                        }
                        Some(header_map)
                    } else {
                        None
                    },
                    body: target
                        .body
                        .map(|body| serde_json::from_slice(&body))
                        .transpose()?,
                    media_type: target.media_type.map(|media_type| media_type.into()),
                    configurator: target.configurator.map(Cow::into_owned),
                    extractor: target.extractor.map(Cow::into_owned),
                }),
            },
            actions: raw_config
                .actions
                .into_iter()
                .map(|action| action.try_into())
                .collect::<anyhow::Result<_>>()?,
            job_id: raw.job_id,
            config: TrackerConfig {
                revisions: raw_config.revisions,
                timeout: raw_config.timeout,
                job: job_config,
            },
            tags: raw.tags,
            created_at: raw.created_at,
            updated_at: raw.updated_at,
        })
    }
}

impl TryFrom<&Tracker> for RawTracker {
    type Error = anyhow::Error;

    fn try_from(item: &Tracker) -> Result<Self, Self::Error> {
        let job_config = if let Some(SchedulerJobConfig {
            schedule,
            retry_strategy,
        }) = &item.config.job
        {
            Some(RawSchedulerJobConfig(
                Cow::Borrowed(schedule.as_ref()),
                retry_strategy.map(|retry_strategy| match retry_strategy {
                    SchedulerJobRetryStrategy::Constant {
                        interval,
                        max_attempts,
                    } => RawSchedulerJobRetryStrategy::Constant(interval, max_attempts),
                    SchedulerJobRetryStrategy::Exponential {
                        initial_interval,
                        multiplier,
                        max_interval,
                        max_attempts,
                    } => RawSchedulerJobRetryStrategy::Exponential(
                        initial_interval,
                        multiplier,
                        max_interval,
                        max_attempts,
                    ),
                    SchedulerJobRetryStrategy::Linear {
                        initial_interval,
                        increment,
                        max_interval,
                        max_attempts,
                    } => RawSchedulerJobRetryStrategy::Linear(
                        initial_interval,
                        increment,
                        max_interval,
                        max_attempts,
                    ),
                }),
            ))
        } else {
            None
        };

        Ok(RawTracker {
            id: item.id,
            name: item.name.clone(),
            enabled: item.enabled,
            config: postcard::to_stdvec(&RawTrackerConfig {
                revisions: item.config.revisions,
                timeout: item.config.timeout,
                target: match &item.target {
                    TrackerTarget::Page(target) => RawTrackerTarget::Page(RawPageTarget {
                        extractor: Cow::Borrowed(target.extractor.as_ref()),
                        user_agent: target
                            .user_agent
                            .as_ref()
                            .map(|ua| Cow::Borrowed(ua.as_ref())),
                        ignore_https_errors: if target.ignore_https_errors {
                            Some(true)
                        } else {
                            None
                        },
                    }),
                    TrackerTarget::Api(target) => RawTrackerTarget::Api(RawApiTarget {
                        url: target.url.as_str().into(),
                        method: target.method.clone(),
                        headers: target.headers.as_ref().map(|headers| {
                            headers
                                .iter()
                                .map(|(k, v)| {
                                    (
                                        Cow::Borrowed(k.as_str()),
                                        String::from_utf8_lossy(v.as_bytes()),
                                    )
                                })
                                .collect()
                        }),
                        body: target.body.as_ref().map(serde_json::to_vec).transpose()?,
                        media_type: target
                            .media_type
                            .as_ref()
                            .map(|media_type| media_type.to_ref()),
                        configurator: target
                            .configurator
                            .as_ref()
                            .map(|configurator| Cow::Borrowed(configurator.as_ref())),
                        extractor: target
                            .extractor
                            .as_ref()
                            .map(|extractor| Cow::Borrowed(extractor.as_ref())),
                    }),
                },
                actions: item.actions.iter().map(|action| action.into()).collect(),
                job: job_config,
            })?,
            tags: item.tags.clone(),
            created_at: item.created_at,
            updated_at: item.updated_at,
            job_id: item.job_id,
            job_needed: item.config.job.is_some() && item.config.revisions > 0 && item.enabled,
        })
    }
}

impl<'s> From<&'s TrackerAction> for RawTrackerAction<'s> {
    fn from(action: &'s TrackerAction) -> Self {
        match action {
            TrackerAction::Email(config) => Self::Email {
                to: Cow::Borrowed(config.to.as_ref()),
            },
            TrackerAction::Webhook(config) => Self::Webhook {
                url: config.url.to_string(),
                method: config.method.clone(),
                headers: config.headers.as_ref().map(|headers| {
                    headers
                        .iter()
                        .map(|(k, v)| {
                            (
                                Cow::Borrowed(k.as_str()),
                                String::from_utf8_lossy(v.as_bytes()),
                            )
                        })
                        .collect()
                }),
            },
            TrackerAction::ServerLog => Self::ServerLog,
        }
    }
}

impl<'s> TryFrom<RawTrackerAction<'s>> for TrackerAction {
    type Error = anyhow::Error;

    fn try_from(raw: RawTrackerAction) -> Result<Self, Self::Error> {
        Ok(match raw {
            RawTrackerAction::Email { to } => TrackerAction::Email(EmailAction {
                to: to.into_owned(),
            }),
            RawTrackerAction::Webhook {
                url,
                method,
                headers,
            } => TrackerAction::Webhook(WebhookAction {
                url: url.parse()?,
                method,
                headers: if let Some(headers) = headers {
                    let mut header_map = HeaderMap::new();
                    for (k, v) in headers {
                        header_map.insert(HeaderName::from_str(&k)?, HeaderValue::from_str(&v)?);
                    }
                    Some(header_map)
                } else {
                    None
                },
            }),
            RawTrackerAction::ServerLog => TrackerAction::ServerLog,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::RawTracker;
    use http::{header::CONTENT_TYPE, Method};
    use retrack_types::{
        scheduler::{SchedulerJobConfig, SchedulerJobRetryStrategy},
        trackers::{
            ApiTarget, EmailAction, PageTarget, Tracker, TrackerAction, TrackerConfig,
            TrackerTarget, WebhookAction,
        },
    };
    use serde_json::json;
    use std::{collections::HashMap, time::Duration};
    use time::OffsetDateTime;
    use uuid::uuid;

    #[test]
    fn can_convert_into_and_from_raw_tracker() -> anyhow::Result<()> {
        let tracker = Tracker {
            id: uuid!("00000000-0000-0000-0000-000000000001"),
            name: "tk".to_string(),
            enabled: true,
            target: TrackerTarget::Page(PageTarget {
                extractor: "export async function execute(p) { await p.goto('https://retrack.dev/'); return await p.content(); }".to_string(),
                user_agent: None,
                ignore_https_errors: false,
            }),
            config: TrackerConfig {
                revisions: 1,
                timeout: Some(Duration::from_millis(2000)),
                job: None,
            },
            tags: vec!["tag".to_string()],
            actions: vec![],
            created_at: OffsetDateTime::from_unix_timestamp(946720800)?,
            updated_at: OffsetDateTime::from_unix_timestamp(946720810)?,
            job_id: None,
        };
        assert_eq!(Tracker::try_from(RawTracker::try_from(&tracker)?)?, tracker);

        let tracker = Tracker {
            enabled: false,
            target: TrackerTarget::Page(PageTarget {
                extractor: "export async function execute(p) { await p.goto('https://retrack.dev/'); return await p.content(); }".to_string(),
                user_agent: Some("Retrack/1.0.0".to_string()),
                ignore_https_errors: true,
            }),
            config: TrackerConfig {
                revisions: 1,
                timeout: Some(Duration::from_millis(2000)),
                job: Some(SchedulerJobConfig {
                    schedule: "0 0 * * *".to_string(),
                    retry_strategy: Some(SchedulerJobRetryStrategy::Exponential {
                        initial_interval: Duration::from_millis(1234),
                        multiplier: 2,
                        max_interval: Duration::from_secs(120),
                        max_attempts: 5,
                    })
                }),
            },
            actions: vec![TrackerAction::ServerLog, TrackerAction::Email(EmailAction {
                to: vec!["dev@retrack.dev".to_string()],
            }), TrackerAction::Webhook(WebhookAction {
                url: "https://retrack.dev".parse()?,
                method: Some(Method::GET),
                headers: Some(
                    (&[(CONTENT_TYPE, "text/plain".to_string())]
                        .into_iter()
                        .collect::<HashMap<_, _>>())
                        .try_into()?,
                ),
            })],
            job_id: Some(uuid!("00000000-0000-0000-0000-000000000002")),
            ..tracker.clone()
        };
        assert_eq!(Tracker::try_from(RawTracker::try_from(&tracker)?)?, tracker);

        let tracker = Tracker {
            target: TrackerTarget::Api(ApiTarget {
                url: "https://retrack.dev/".parse()?,
                method: None,
                headers: None,
                body: None,
                media_type: None,
                configurator: None,
                extractor: None,
            }),
            config: TrackerConfig::default(),
            actions: vec![TrackerAction::ServerLog],
            job_id: Some(uuid!("00000000-0000-0000-0000-000000000003")),
            ..tracker.clone()
        };
        assert_eq!(Tracker::try_from(RawTracker::try_from(&tracker)?)?, tracker);

        let tracker = Tracker {
            target: TrackerTarget::Api(ApiTarget {
                url: "https://retrack.dev/".parse()?,
                method: Some(Method::POST),
                headers: Some(
                    (&[(CONTENT_TYPE, "application/json".to_string())]
                        .into_iter()
                        .collect::<HashMap<_, _>>())
                        .try_into()?,
                ),
                body: Some(json!({ "key": "value" })),
                media_type: Some("application/json".parse()?),
                configurator: Some("(async () => ({ body: Deno.core.encode(JSON.stringify({ key: 'value' })) })();".to_string()),
                extractor: Some("((context) => ({ body: Deno.core.encode(JSON.stringify(context)) })();".to_string())
            }),
            config: TrackerConfig::default(),
            actions: vec![TrackerAction::ServerLog],
            job_id: Some(uuid!("00000000-0000-0000-0000-000000000003")),
            ..tracker.clone()
        };
        assert_eq!(Tracker::try_from(RawTracker::try_from(&tracker)?)?, tracker);

        Ok(())
    }
}
