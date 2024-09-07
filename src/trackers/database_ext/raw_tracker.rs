use crate::{
    scheduler::{SchedulerJobConfig, SchedulerJobRetryStrategy},
    trackers::{
        ApiTarget, EmailAction, PageTarget, Tracker, TrackerAction, TrackerConfig, TrackerTarget,
        WebhookAction,
    },
};
use http::{HeaderMap, HeaderName, HeaderValue, Method};
use mediatype::MediaType;
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
    headers: Option<Cow<'s, HashMap<String, String>>>,
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
    #[serde(borrow)]
    media_type: Option<MediaType<'s>>,
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
                    media_type: target.media_type.map(|media_type| media_type.into()),
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
                headers: raw_config.headers.map(Cow::into_owned),
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
                        media_type: target
                            .media_type
                            .as_ref()
                            .map(|media_type| media_type.to_ref()),
                    }),
                },
                actions: item.actions.iter().map(|action| action.into()).collect(),
                headers: item.config.headers.as_ref().map(Cow::Borrowed),
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
    use crate::{
        scheduler::{SchedulerJobConfig, SchedulerJobRetryStrategy},
        trackers::{
            ApiTarget, EmailAction, PageTarget, Tracker, TrackerAction, TrackerConfig,
            TrackerTarget, WebhookAction,
        },
    };
    use http::{header::CONTENT_TYPE, Method};
    use std::{collections::HashMap, time::Duration};
    use time::OffsetDateTime;
    use uuid::uuid;

    #[test]
    fn can_convert_into_tracker() -> anyhow::Result<()> {
        let raw_config = vec![
            1, 1, 2, 0, 0, 100, 101, 120, 112, 111, 114, 116, 32, 97, 115, 121, 110, 99, 32, 102,
            117, 110, 99, 116, 105, 111, 110, 32, 101, 120, 101, 99, 117, 116, 101, 40, 112, 41,
            32, 123, 32, 97, 119, 97, 105, 116, 32, 112, 46, 103, 111, 116, 111, 40, 39, 104, 116,
            116, 112, 115, 58, 47, 47, 114, 101, 116, 114, 97, 99, 107, 46, 100, 101, 118, 47, 39,
            41, 59, 32, 114, 101, 116, 117, 114, 110, 32, 97, 119, 97, 105, 116, 32, 112, 46, 99,
            111, 110, 116, 101, 110, 116, 40, 41, 59, 32, 125, 0, 0, 0, 0, 0,
        ];
        assert_eq!(
            Tracker::try_from(RawTracker {
                id: uuid!("00000000-0000-0000-0000-000000000001"),
                name: "tk".to_string(),
                enabled: true,
                config: raw_config,
                tags: vec!["tag".to_string()],
                // January 1, 2000 10:00:00
                created_at: OffsetDateTime::from_unix_timestamp(946720800)?,
                // January 1, 2000 10:00:10
                updated_at: OffsetDateTime::from_unix_timestamp(946720810)?,
                job_id: None,
                job_needed: false,
            })?,
            Tracker {
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
                    headers: Default::default(),
                    job: None,
                },
                tags: vec!["tag".to_string()],
                actions: vec![],
                created_at: OffsetDateTime::from_unix_timestamp(946720800)?,
                updated_at: OffsetDateTime::from_unix_timestamp(946720810)?,
                job_id: None,
            }
        );

        let raw_config = vec![
            1, 1, 2, 0, 0, 100, 101, 120, 112, 111, 114, 116, 32, 97, 115, 121, 110, 99, 32, 102,
            117, 110, 99, 116, 105, 111, 110, 32, 101, 120, 101, 99, 117, 116, 101, 40, 112, 41,
            32, 123, 32, 97, 119, 97, 105, 116, 32, 112, 46, 103, 111, 116, 111, 40, 39, 104, 116,
            116, 112, 115, 58, 47, 47, 114, 101, 116, 114, 97, 99, 107, 46, 100, 101, 118, 47, 39,
            41, 59, 32, 114, 101, 116, 117, 114, 110, 32, 97, 119, 97, 105, 116, 32, 112, 46, 99,
            111, 110, 116, 101, 110, 116, 40, 41, 59, 32, 125, 1, 13, 82, 101, 116, 114, 97, 99,
            107, 47, 49, 46, 48, 46, 48, 1, 1, 3, 2, 0, 1, 15, 100, 101, 118, 64, 114, 101, 116,
            114, 97, 99, 107, 46, 100, 101, 118, 1, 20, 104, 116, 116, 112, 115, 58, 47, 47, 114,
            101, 116, 114, 97, 99, 107, 46, 100, 101, 118, 47, 1, 3, 71, 69, 84, 1, 1, 12, 99, 111,
            110, 116, 101, 110, 116, 45, 116, 121, 112, 101, 10, 116, 101, 120, 116, 47, 112, 108,
            97, 105, 110, 1, 1, 6, 99, 111, 111, 107, 105, 101, 9, 109, 121, 45, 99, 111, 111, 107,
            105, 101, 1, 9, 48, 32, 48, 32, 42, 32, 42, 32, 42, 1, 1, 1, 128, 157, 202, 111, 2,
            120, 0, 5,
        ];
        assert_eq!(
            Tracker::try_from(RawTracker {
                id: uuid!("00000000-0000-0000-0000-000000000001"),
                name: "tk".to_string(),
                enabled: false,
                config: raw_config,
                tags: vec!["tag".to_string()],
                // January 1, 2000 10:00:00
                created_at: OffsetDateTime::from_unix_timestamp(946720800)?,
                // January 1, 2000 10:00:10
                updated_at: OffsetDateTime::from_unix_timestamp(946720810)?,
                job_id: Some(uuid!("00000000-0000-0000-0000-000000000002")),
                job_needed: true,
            })?,
            Tracker {
                id: uuid!("00000000-0000-0000-0000-000000000001"),
                name: "tk".to_string(),
                enabled: false,
                target: TrackerTarget::Page(PageTarget {
                    extractor: "export async function execute(p) { await p.goto('https://retrack.dev/'); return await p.content(); }".to_string(),
                    user_agent: Some("Retrack/1.0.0".to_string()),
                    ignore_https_errors: true,
                }),
                config: TrackerConfig {
                    revisions: 1,
                    timeout: Some(Duration::from_millis(2000)),
                    headers: Some(
                        [("cookie".to_string(), "my-cookie".to_string())]
                            .into_iter()
                            .collect()
                    ),
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
                tags: vec!["tag".to_string()],
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
                created_at: OffsetDateTime::from_unix_timestamp(946720800)?,
                updated_at: OffsetDateTime::from_unix_timestamp(946720810)?,
                job_id: Some(uuid!("00000000-0000-0000-0000-000000000002")),
            }
        );

        let raw_config = vec![
            3, 0, 1, 20, 104, 116, 116, 112, 115, 58, 47, 47, 114, 101, 116, 114, 97, 99, 107, 46,
            100, 101, 118, 47, 1, 4, 80, 79, 83, 84, 1, 1, 12, 99, 111, 110, 116, 101, 110, 116,
            45, 116, 121, 112, 101, 16, 97, 112, 112, 108, 105, 99, 97, 116, 105, 111, 110, 47,
            106, 115, 111, 110, 1, 16, 97, 112, 112, 108, 105, 99, 97, 116, 105, 111, 110, 47, 106,
            115, 111, 110, 1, 2, 0, 0,
        ];
        assert_eq!(
            Tracker::try_from(RawTracker {
                id: uuid!("00000000-0000-0000-0000-000000000001"),
                name: "tk".to_string(),
                enabled: true,
                config: raw_config,
                tags: vec!["tag".to_string()],
                // January 1, 2000 10:00:00
                created_at: OffsetDateTime::from_unix_timestamp(946720800)?,
                // January 1, 2000 10:00:10
                updated_at: OffsetDateTime::from_unix_timestamp(946720810)?,
                job_id: Some(uuid!("00000000-0000-0000-0000-000000000003")),
                job_needed: false,
            })?,
            Tracker {
                id: uuid!("00000000-0000-0000-0000-000000000001"),
                name: "tk".to_string(),
                enabled: true,
                target: TrackerTarget::Api(ApiTarget {
                    url: "https://retrack.dev/".parse()?,
                    method: Some(Method::POST),
                    headers: Some(
                        (&[(CONTENT_TYPE, "application/json".to_string())]
                            .into_iter()
                            .collect::<HashMap<_, _>>())
                            .try_into()?,
                    ),
                    media_type: Some("application/json".parse()?),
                }),
                config: TrackerConfig::default(),
                tags: vec!["tag".to_string()],
                actions: vec![TrackerAction::ServerLog],
                created_at: OffsetDateTime::from_unix_timestamp(946720800)?,
                updated_at: OffsetDateTime::from_unix_timestamp(946720810)?,
                job_id: Some(uuid!("00000000-0000-0000-0000-000000000003")),
            }
        );

        Ok(())
    }

    #[test]
    fn can_convert_into_raw_tracker() -> anyhow::Result<()> {
        let raw_config = vec![
            1, 1, 2, 0, 0, 100, 101, 120, 112, 111, 114, 116, 32, 97, 115, 121, 110, 99, 32, 102,
            117, 110, 99, 116, 105, 111, 110, 32, 101, 120, 101, 99, 117, 116, 101, 40, 112, 41,
            32, 123, 32, 97, 119, 97, 105, 116, 32, 112, 46, 103, 111, 116, 111, 40, 39, 104, 116,
            116, 112, 115, 58, 47, 47, 114, 101, 116, 114, 97, 99, 107, 46, 100, 101, 118, 47, 39,
            41, 59, 32, 114, 101, 116, 117, 114, 110, 32, 97, 119, 97, 105, 116, 32, 112, 46, 99,
            111, 110, 116, 101, 110, 116, 40, 41, 59, 32, 125, 0, 0, 0, 0, 0,
        ];
        assert_eq!(
            RawTracker::try_from(&Tracker {
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
                    headers: Default::default(),
                    job: None,
                },
                tags: vec!["tag".to_string()],
                actions: vec![],
                created_at: OffsetDateTime::from_unix_timestamp(946720800)?,
                updated_at: OffsetDateTime::from_unix_timestamp(946720810)?,
                job_id: None,
            })?,
            RawTracker {
                id: uuid!("00000000-0000-0000-0000-000000000001"),
                name: "tk".to_string(),
                enabled: true,
                config: raw_config,
                tags: vec!["tag".to_string()],
                // January 1, 2000 10:00:00
                created_at: OffsetDateTime::from_unix_timestamp(946720800)?,
                // January 1, 2000 10:00:10
                updated_at: OffsetDateTime::from_unix_timestamp(946720810)?,
                job_id: None,
                job_needed: false,
            }
        );

        let raw_config = vec![
            1, 1, 2, 0, 0, 100, 101, 120, 112, 111, 114, 116, 32, 97, 115, 121, 110, 99, 32, 102,
            117, 110, 99, 116, 105, 111, 110, 32, 101, 120, 101, 99, 117, 116, 101, 40, 112, 41,
            32, 123, 32, 97, 119, 97, 105, 116, 32, 112, 46, 103, 111, 116, 111, 40, 39, 104, 116,
            116, 112, 115, 58, 47, 47, 114, 101, 116, 114, 97, 99, 107, 46, 100, 101, 118, 47, 39,
            41, 59, 32, 114, 101, 116, 117, 114, 110, 32, 97, 119, 97, 105, 116, 32, 112, 46, 99,
            111, 110, 116, 101, 110, 116, 40, 41, 59, 32, 125, 1, 13, 82, 101, 116, 114, 97, 99,
            107, 47, 49, 46, 48, 46, 48, 1, 1, 3, 2, 0, 1, 15, 100, 101, 118, 64, 114, 101, 116,
            114, 97, 99, 107, 46, 100, 101, 118, 1, 20, 104, 116, 116, 112, 115, 58, 47, 47, 114,
            101, 116, 114, 97, 99, 107, 46, 100, 101, 118, 47, 1, 3, 71, 69, 84, 1, 1, 12, 99, 111,
            110, 116, 101, 110, 116, 45, 116, 121, 112, 101, 10, 116, 101, 120, 116, 47, 112, 108,
            97, 105, 110, 1, 1, 6, 99, 111, 111, 107, 105, 101, 9, 109, 121, 45, 99, 111, 111, 107,
            105, 101, 1, 9, 48, 32, 48, 32, 42, 32, 42, 32, 42, 1, 1, 1, 128, 157, 202, 111, 2,
            120, 0, 5,
        ];
        assert_eq!(
            RawTracker::try_from(&Tracker {
                id: uuid!("00000000-0000-0000-0000-000000000001"),
                name: "tk".to_string(),
                enabled: true,
                target: TrackerTarget::Page(PageTarget {
                    extractor: "export async function execute(p) { await p.goto('https://retrack.dev/'); return await p.content(); }".to_string(),
                    user_agent: Some("Retrack/1.0.0".to_string()),
                    ignore_https_errors: true,
                }),
                config: TrackerConfig {
                    revisions: 1,
                    timeout: Some(Duration::from_millis(2000)),
                    headers: Some(
                        [("cookie".to_string(), "my-cookie".to_string())]
                            .into_iter()
                            .collect()
                    ),
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
                tags: vec!["tag".to_string()],
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
                created_at: OffsetDateTime::from_unix_timestamp(946720800)?,
                updated_at: OffsetDateTime::from_unix_timestamp(946720810)?,
                job_id: Some(uuid!("00000000-0000-0000-0000-000000000002")),
            })?,
            RawTracker {
                id: uuid!("00000000-0000-0000-0000-000000000001"),
                name: "tk".to_string(),
                enabled: true,
                config: raw_config.clone(),
                tags: vec!["tag".to_string()],
                // January 1, 2000 10:00:00
                created_at: OffsetDateTime::from_unix_timestamp(946720800)?,
                // January 1, 2000 10:00:10
                updated_at: OffsetDateTime::from_unix_timestamp(946720810)?,
                job_id: Some(uuid!("00000000-0000-0000-0000-000000000002")),
                job_needed: true,
            }
        );

        assert_eq!(
            RawTracker::try_from(&Tracker {
                id: uuid!("00000000-0000-0000-0000-000000000001"),
                name: "tk".to_string(),
                enabled: false,
                target: TrackerTarget::Page(PageTarget {
                    extractor: "export async function execute(p) { await p.goto('https://retrack.dev/'); return await p.content(); }".to_string(),
                    user_agent: Some("Retrack/1.0.0".to_string()),
                    ignore_https_errors: true,
                }),
                config: TrackerConfig {
                    revisions: 1,
                    timeout: Some(Duration::from_millis(2000)),
                    headers: Some(
                        [("cookie".to_string(), "my-cookie".to_string())]
                            .into_iter()
                            .collect()
                    ),
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
                tags: vec!["tag".to_string()],
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
                created_at: OffsetDateTime::from_unix_timestamp(946720800)?,
                updated_at: OffsetDateTime::from_unix_timestamp(946720810)?,
                job_id: Some(uuid!("00000000-0000-0000-0000-000000000002")),
            })?,
            RawTracker {
                id: uuid!("00000000-0000-0000-0000-000000000001"),
                name: "tk".to_string(),
                enabled: false,
                config: raw_config,
                tags: vec!["tag".to_string()],
                // January 1, 2000 10:00:00
                created_at: OffsetDateTime::from_unix_timestamp(946720800)?,
                // January 1, 2000 10:00:10
                updated_at: OffsetDateTime::from_unix_timestamp(946720810)?,
                job_id: Some(uuid!("00000000-0000-0000-0000-000000000002")),
                job_needed: false,
            }
        );

        let raw_config = vec![
            3, 0, 1, 20, 104, 116, 116, 112, 115, 58, 47, 47, 114, 101, 116, 114, 97, 99, 107, 46,
            100, 101, 118, 47, 1, 4, 80, 79, 83, 84, 1, 1, 12, 99, 111, 110, 116, 101, 110, 116,
            45, 116, 121, 112, 101, 16, 97, 112, 112, 108, 105, 99, 97, 116, 105, 111, 110, 47,
            106, 115, 111, 110, 1, 16, 97, 112, 112, 108, 105, 99, 97, 116, 105, 111, 110, 47, 106,
            115, 111, 110, 1, 2, 0, 0,
        ];
        assert_eq!(
            RawTracker::try_from(&Tracker {
                id: uuid!("00000000-0000-0000-0000-000000000001"),
                name: "tk".to_string(),
                enabled: true,
                target: TrackerTarget::Api(ApiTarget {
                    url: "https://retrack.dev/".parse()?,
                    method: Some(Method::POST),
                    headers: Some(
                        (&[(CONTENT_TYPE, "application/json".to_string()),]
                            .into_iter()
                            .collect::<HashMap<_, _>>())
                            .try_into()?,
                    ),
                    media_type: Some("application/json".parse()?),
                }),
                config: TrackerConfig::default(),
                tags: vec!["tag".to_string()],
                actions: vec![TrackerAction::ServerLog,],
                created_at: OffsetDateTime::from_unix_timestamp(946720800)?,
                updated_at: OffsetDateTime::from_unix_timestamp(946720810)?,
                job_id: Some(uuid!("00000000-0000-0000-0000-000000000003")),
            })?,
            RawTracker {
                id: uuid!("00000000-0000-0000-0000-000000000001"),
                name: "tk".to_string(),
                enabled: true,
                config: raw_config,
                tags: vec!["tag".to_string()],
                // January 1, 2000 10:00:00
                created_at: OffsetDateTime::from_unix_timestamp(946720800)?,
                // January 1, 2000 10:00:10
                updated_at: OffsetDateTime::from_unix_timestamp(946720810)?,
                job_id: Some(uuid!("00000000-0000-0000-0000-000000000003")),
                job_needed: false,
            }
        );

        Ok(())
    }
}
