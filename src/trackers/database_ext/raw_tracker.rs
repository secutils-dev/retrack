use crate::{
    scheduler::{SchedulerJobConfig, SchedulerJobRetryStrategy},
    trackers::{JsonApiTarget, Tracker, TrackerConfig, TrackerTarget, WebPageTarget},
};
use serde_derive::{Deserialize, Serialize};
use serde_with::serde_as;
use std::{borrow::Cow, collections::HashMap, time::Duration};
use time::OffsetDateTime;
use uuid::Uuid;

/// The type used to serialize and deserialize tracker database representation. There are a number
/// of helper structs here to help with the conversion from original types that are tailored for
/// JSON serialization to the types that are tailored for Postcard/database serialization.
#[derive(Debug, Eq, PartialEq, Clone)]
pub(super) struct RawTracker {
    pub id: Uuid,
    pub name: String,
    pub config: Vec<u8>,
    pub target: Vec<u8>,
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
    headers: Option<Cow<'s, HashMap<String, String>>>,
    job: Option<RawSchedulerJobConfig<'s>>,
}

#[derive(Serialize, Deserialize, Debug, Eq, PartialEq, Clone)]
struct RawSchedulerJobConfig<'s>(
    Cow<'s, str>,
    Option<RawSchedulerJobRetryStrategy>,
    Option<bool>,
);

#[derive(Serialize, Deserialize, Copy, Clone, Debug, Eq, PartialEq)]
enum RawSchedulerJobRetryStrategy {
    Constant(Duration, u32),
    Exponential(Duration, u32, Duration, u32),
    Linear(Duration, Duration, Duration, u32),
}

#[derive(Serialize, Deserialize, Debug, Eq, PartialEq, Clone)]
enum RawTrackerTarget<'s> {
    WebPage(RawWebPageTarget<'s>),
    JsonApi(RawJsonApiTarget<'s>),
}

#[derive(Serialize, Deserialize, Debug, Eq, PartialEq, Clone)]
struct RawWebPageTarget<'s> {
    extractor: Cow<'s, str>,
    user_agent: Option<Cow<'s, str>>,
    ignore_https_errors: Option<bool>,
}

#[derive(Serialize, Deserialize, Debug, Eq, PartialEq, Clone)]
struct RawJsonApiTarget<'s> {
    url: Cow<'s, str>,
}

impl TryFrom<RawTracker> for Tracker {
    type Error = anyhow::Error;

    fn try_from(raw: RawTracker) -> Result<Self, Self::Error> {
        let raw_config = postcard::from_bytes::<RawTrackerConfig>(&raw.config)?;

        let job_config =
            if let Some(RawSchedulerJobConfig(schedule, retry_strategy, notifications)) =
                raw_config.job
            {
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
                    notifications,
                })
            } else {
                None
            };

        Ok(Tracker {
            id: raw.id,
            name: raw.name,
            target: match postcard::from_bytes::<RawTrackerTarget>(&raw.target)? {
                RawTrackerTarget::WebPage(target) => TrackerTarget::WebPage(WebPageTarget {
                    extractor: target.extractor.into_owned(),
                    user_agent: target.user_agent.map(Cow::into_owned),
                    ignore_https_errors: target.ignore_https_errors.unwrap_or_default(),
                }),
                RawTrackerTarget::JsonApi(target) => TrackerTarget::JsonApi(JsonApiTarget {
                    url: target.url.into_owned().parse()?,
                }),
            },
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
            notifications,
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
                *notifications,
            ))
        } else {
            None
        };

        Ok(RawTracker {
            id: item.id,
            name: item.name.clone(),
            target: postcard::to_stdvec(
                &(match &item.target {
                    TrackerTarget::WebPage(target) => RawTrackerTarget::WebPage(RawWebPageTarget {
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
                    TrackerTarget::JsonApi(target) => RawTrackerTarget::JsonApi(RawJsonApiTarget {
                        url: target.url.as_str().into(),
                    }),
                }),
            )?,
            config: postcard::to_stdvec(&RawTrackerConfig {
                revisions: item.config.revisions,
                timeout: item.config.timeout,
                headers: item.config.headers.as_ref().map(Cow::Borrowed),
                job: job_config,
            })?,
            tags: item.tags.clone(),
            created_at: item.created_at,
            updated_at: item.updated_at,
            job_id: item.job_id,
            job_needed: item.config.job.is_some(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::RawTracker;
    use crate::{
        scheduler::{SchedulerJobConfig, SchedulerJobRetryStrategy},
        trackers::{Tracker, TrackerConfig, TrackerTarget, WebPageTarget},
    };
    use std::time::Duration;
    use time::OffsetDateTime;
    use uuid::uuid;

    #[test]
    fn can_convert_into_tracker() -> anyhow::Result<()> {
        assert_eq!(
            Tracker::try_from(RawTracker {
                id: uuid!("00000000-0000-0000-0000-000000000001"),
                name: "tk".to_string(),
                target: vec![0, 111, 101, 120, 112, 111, 114, 116, 32, 97, 115, 121, 110, 99, 32, 102, 117, 110, 99, 116, 105, 111, 110, 32, 101, 120, 101, 99, 117, 116, 101, 40, 112, 44, 32, 114, 41, 32, 123, 32, 97, 119, 97, 105, 116, 32, 112, 46, 103, 111, 116, 111, 40, 39, 104, 116, 116, 112, 115, 58, 47, 47, 114, 101, 116, 114, 97, 99, 107, 46, 100, 101, 118, 47, 39, 41, 59, 32, 114, 101, 116, 117, 114, 110, 32, 114, 46, 104, 116, 109, 108, 40, 97, 119, 97, 105, 116, 32, 112, 46, 99, 111, 110, 116, 101, 110, 116, 40, 41, 41, 59, 32, 125, 0, 0],
                config: vec![1, 1, 2, 0, 0, 0],
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
                target: TrackerTarget::WebPage(WebPageTarget {
                    extractor: "export async function execute(p, r) { await p.goto('https://retrack.dev/'); return r.html(await p.content()); }".to_string(),
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
                created_at: OffsetDateTime::from_unix_timestamp(946720800)?,
                updated_at: OffsetDateTime::from_unix_timestamp(946720810)?,
                job_id: None,
            }
        );

        assert_eq!(
            Tracker::try_from(RawTracker {
                id: uuid!("00000000-0000-0000-0000-000000000001"),
                name: "tk".to_string(),
                target: vec![0, 111, 101, 120, 112, 111, 114, 116, 32, 97, 115, 121, 110, 99, 32, 102, 117, 110, 99, 116, 105, 111, 110, 32, 101, 120, 101, 99, 117, 116, 101, 40, 112, 44, 32, 114, 41, 32, 123, 32, 97, 119, 97, 105, 116, 32, 112, 46, 103, 111, 116, 111, 40, 39, 104, 116, 116, 112, 115, 58, 47, 47, 114, 101, 116, 114, 97, 99, 107, 46, 100, 101, 118, 47, 39, 41, 59, 32, 114, 101, 116, 117, 114, 110, 32, 114, 46, 104, 116, 109, 108, 40, 97, 119, 97, 105, 116, 32, 112, 46, 99, 111, 110, 116, 101, 110, 116, 40, 41, 41, 59, 32, 125, 1, 13, 82, 101, 116, 114, 97, 99, 107, 47, 49, 46, 48, 46, 48, 1, 1],
                config: vec![
                    1, 1, 2, 0, 1, 1, 6, 99, 111, 111, 107, 105, 101, 9, 109, 121, 45, 99, 111, 111, 107, 105, 101, 1, 9, 48, 32, 48, 32, 42, 32, 42, 32, 42, 1, 1, 1, 128, 157, 202, 111, 2, 120, 0, 5, 1, 1
                ],
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
                target: TrackerTarget::WebPage(WebPageTarget {
                    extractor: "export async function execute(p, r) { await p.goto('https://retrack.dev/'); return r.html(await p.content()); }".to_string(),
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
                        }),
                        notifications: Some(true)
                    }),
                },
                tags: vec!["tag".to_string()],
                created_at: OffsetDateTime::from_unix_timestamp(946720800)?,
                updated_at: OffsetDateTime::from_unix_timestamp(946720810)?,
                job_id: Some(uuid!("00000000-0000-0000-0000-000000000002")),
            }
        );

        Ok(())
    }

    #[test]
    fn can_convert_into_raw_tracker() -> anyhow::Result<()> {
        assert_eq!(
            RawTracker::try_from(&Tracker {
                id: uuid!("00000000-0000-0000-0000-000000000001"),
                name: "tk".to_string(),
                target: TrackerTarget::WebPage(WebPageTarget {
                    extractor: "export async function execute(p, r) { await p.goto('https://retrack.dev/'); return r.html(await p.content()); }".to_string(),
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
                created_at: OffsetDateTime::from_unix_timestamp(946720800)?,
                updated_at: OffsetDateTime::from_unix_timestamp(946720810)?,
                job_id: None,
            })?,
            RawTracker {
                id: uuid!("00000000-0000-0000-0000-000000000001"),
                name: "tk".to_string(),
                target: vec![0, 111, 101, 120, 112, 111, 114, 116, 32, 97, 115, 121, 110, 99, 32, 102, 117, 110, 99, 116, 105, 111, 110, 32, 101, 120, 101, 99, 117, 116, 101, 40, 112, 44, 32, 114, 41, 32, 123, 32, 97, 119, 97, 105, 116, 32, 112, 46, 103, 111, 116, 111, 40, 39, 104, 116, 116, 112, 115, 58, 47, 47, 114, 101, 116, 114, 97, 99, 107, 46, 100, 101, 118, 47, 39, 41, 59, 32, 114, 101, 116, 117, 114, 110, 32, 114, 46, 104, 116, 109, 108, 40, 97, 119, 97, 105, 116, 32, 112, 46, 99, 111, 110, 116, 101, 110, 116, 40, 41, 41, 59, 32, 125, 0, 0],
                config: vec![1, 1, 2, 0, 0, 0],
                tags: vec!["tag".to_string()],
                // January 1, 2000 10:00:00
                created_at: OffsetDateTime::from_unix_timestamp(946720800)?,
                // January 1, 2000 10:00:10
                updated_at: OffsetDateTime::from_unix_timestamp(946720810)?,
                job_id: None,
                job_needed: false,
            }
        );

        assert_eq!(
            RawTracker::try_from(&Tracker {
                id: uuid!("00000000-0000-0000-0000-000000000001"),
                name: "tk".to_string(),
                target: TrackerTarget::WebPage(WebPageTarget {
                    extractor: "export async function execute(p, r) { await p.goto('https://retrack.dev/'); return r.html(await p.content()); }".to_string(),
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
                        }),
                        notifications: Some(true)
                    }),
                },
                tags: vec!["tag".to_string()],
                created_at: OffsetDateTime::from_unix_timestamp(946720800)?,
                updated_at: OffsetDateTime::from_unix_timestamp(946720810)?,
                job_id: Some(uuid!("00000000-0000-0000-0000-000000000002")),
            })?,
            RawTracker {
                id: uuid!("00000000-0000-0000-0000-000000000001"),
                name: "tk".to_string(),
                target: vec![0, 111, 101, 120, 112, 111, 114, 116, 32, 97, 115, 121, 110, 99, 32, 102, 117, 110, 99, 116, 105, 111, 110, 32, 101, 120, 101, 99, 117, 116, 101, 40, 112, 44, 32, 114, 41, 32, 123, 32, 97, 119, 97, 105, 116, 32, 112, 46, 103, 111, 116, 111, 40, 39, 104, 116, 116, 112, 115, 58, 47, 47, 114, 101, 116, 114, 97, 99, 107, 46, 100, 101, 118, 47, 39, 41, 59, 32, 114, 101, 116, 117, 114, 110, 32, 114, 46, 104, 116, 109, 108, 40, 97, 119, 97, 105, 116, 32, 112, 46, 99, 111, 110, 116, 101, 110, 116, 40, 41, 41, 59, 32, 125, 1, 13, 82, 101, 116, 114, 97, 99, 107, 47, 49, 46, 48, 46, 48, 1, 1],
                config: vec![
                    1, 1, 2, 0, 1, 1, 6, 99, 111, 111, 107, 105, 101, 9, 109, 121, 45, 99, 111, 111, 107, 105, 101, 1, 9, 48, 32, 48, 32, 42, 32, 42, 32, 42, 1, 1, 1, 128, 157, 202, 111, 2, 120, 0, 5, 1, 1,
                ],
                tags: vec!["tag".to_string()],
                // January 1, 2000 10:00:00
                created_at: OffsetDateTime::from_unix_timestamp(946720800)?,
                // January 1, 2000 10:00:10
                updated_at: OffsetDateTime::from_unix_timestamp(946720810)?,
                job_id: Some(uuid!("00000000-0000-0000-0000-000000000002")),
                job_needed: true,
            }
        );

        Ok(())
    }
}
