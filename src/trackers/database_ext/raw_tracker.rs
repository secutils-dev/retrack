use crate::{
    scheduler::{SchedulerJobConfig, SchedulerJobRetryStrategy},
    trackers::{
        JsonApiTarget, Tracker, TrackerConfig, TrackerTarget, WebPageTarget, WebPageWaitFor,
        WebPageWaitForState,
    },
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
    pub url: String,
    pub config: Vec<u8>,
    pub target: Vec<u8>,
    pub created_at: OffsetDateTime,
    pub job_id: Option<Uuid>,
    pub job_needed: bool,
}

#[serde_as]
#[derive(Serialize, Deserialize, Debug, Eq, PartialEq, Clone)]
pub struct RawTrackerConfig<'s> {
    revisions: usize,
    extractor: Option<Cow<'s, str>>,
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
    JsonApi(JsonApiTarget),
}

#[derive(Serialize, Deserialize, Debug, Eq, PartialEq, Clone)]
struct RawWebPageTarget<'s> {
    delay: Option<Duration>,
    wait_for: Option<(Cow<'s, str>, Option<WebPageWaitForState>, Option<Duration>)>,
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
            url: raw.url.parse()?,
            target: match postcard::from_bytes::<RawTrackerTarget>(&raw.target)? {
                RawTrackerTarget::WebPage(target) => TrackerTarget::WebPage(WebPageTarget {
                    delay: target.delay,
                    wait_for: target
                        .wait_for
                        .map(|(selector, state, timeout)| WebPageWaitFor {
                            selector: selector.into_owned(),
                            state,
                            timeout,
                        }),
                }),
                RawTrackerTarget::JsonApi(target) => TrackerTarget::JsonApi(target),
            },
            job_id: raw.job_id,
            config: TrackerConfig {
                revisions: raw_config.revisions,
                extractor: raw_config.extractor.map(Cow::into_owned),
                headers: raw_config.headers.map(Cow::into_owned),
                job: job_config,
            },
            created_at: raw.created_at,
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
            url: item.url.to_string(),
            target: postcard::to_stdvec(
                &(match &item.target {
                    TrackerTarget::WebPage(target) => RawTrackerTarget::WebPage(RawWebPageTarget {
                        delay: target.delay,
                        wait_for: target.wait_for.as_ref().map(|wait_for| {
                            (
                                Cow::Borrowed(wait_for.selector.as_str()),
                                wait_for.state,
                                wait_for.timeout,
                            )
                        }),
                    }),
                    TrackerTarget::JsonApi(target) => RawTrackerTarget::JsonApi(*target),
                }),
            )?,
            config: postcard::to_stdvec(&RawTrackerConfig {
                revisions: item.config.revisions,
                extractor: item
                    .config
                    .extractor
                    .as_ref()
                    .map(|extractor| Cow::Borrowed(extractor.as_ref())),
                headers: item.config.headers.as_ref().map(Cow::Borrowed),
                job: job_config,
            })?,
            created_at: item.created_at,
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
        trackers::{
            Tracker, TrackerConfig, TrackerTarget, WebPageTarget, WebPageWaitFor,
            WebPageWaitForState,
        },
    };
    use std::time::Duration;
    use time::OffsetDateTime;
    use url::Url;
    use uuid::uuid;

    #[test]
    fn can_convert_into_tracker() -> anyhow::Result<()> {
        assert_eq!(
            Tracker::try_from(RawTracker {
                id: uuid!("00000000-0000-0000-0000-000000000001"),
                name: "tk".to_string(),
                url: "https://retrack.dev".to_string(),
                target: vec![0, 0, 0],
                config: vec![1, 0, 0, 0],
                // January 1, 2000 10:00:00
                created_at: OffsetDateTime::from_unix_timestamp(946720800)?,
                job_id: None,
                job_needed: false,
            })?,
            Tracker {
                id: uuid!("00000000-0000-0000-0000-000000000001"),
                name: "tk".to_string(),
                url: Url::parse("https://retrack.dev")?,
                target: Default::default(),
                config: TrackerConfig {
                    revisions: 1,
                    extractor: Default::default(),
                    headers: Default::default(),
                    job: None,
                },
                created_at: OffsetDateTime::from_unix_timestamp(946720800)?,
                job_id: None,
            }
        );

        assert_eq!(
            Tracker::try_from(RawTracker {
                id: uuid!("00000000-0000-0000-0000-000000000001"),
                name: "tk".to_string(),
                url: "https://retrack.dev".to_string(),
                target: vec![0, 1, 2, 0, 1, 3, 100, 105, 118, 1, 0, 1, 5, 0],
                config: vec![
                    1, 1, 31, 114, 101, 116, 117, 114, 110, 32, 100, 111, 99, 117, 109, 101, 110,
                    116, 46, 98, 111, 100, 121, 46, 105, 110, 110, 101, 114, 72, 84, 77, 76, 59, 1,
                    1, 6, 99, 111, 111, 107, 105, 101, 9, 109, 121, 45, 99, 111, 111, 107, 105,
                    101, 1, 9, 48, 32, 48, 32, 42, 32, 42, 32, 42, 1, 1, 1, 128, 157, 202, 111, 2,
                    120, 0, 5, 1, 1
                ],
                // January 1, 2000 10:00:00
                created_at: OffsetDateTime::from_unix_timestamp(946720800)?,
                job_id: Some(uuid!("00000000-0000-0000-0000-000000000002")),
                job_needed: true,
            })?,
            Tracker {
                id: uuid!("00000000-0000-0000-0000-000000000001"),
                name: "tk".to_string(),
                url: Url::parse("https://retrack.dev")?,
                target: TrackerTarget::WebPage(WebPageTarget {
                    delay: Some(Duration::from_millis(2000)),
                    wait_for: Some(WebPageWaitFor {
                        selector: "div".to_string(),
                        state: Some(WebPageWaitForState::Attached),
                        timeout: Some(Duration::from_secs(5)),
                    }),
                }),
                config: TrackerConfig {
                    revisions: 1,
                    extractor: Some("return document.body.innerHTML;".to_string()),
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
                created_at: OffsetDateTime::from_unix_timestamp(946720800)?,
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
                url: Url::parse("https://retrack.dev")?,
                target: Default::default(),
                config: TrackerConfig {
                    revisions: 1,
                    extractor: Default::default(),
                    headers: Default::default(),
                    job: None,
                },
                created_at: OffsetDateTime::from_unix_timestamp(946720800)?,
                job_id: None,
            })?,
            RawTracker {
                id: uuid!("00000000-0000-0000-0000-000000000001"),
                name: "tk".to_string(),
                url: "https://retrack.dev/".to_string(),
                target: vec![0, 0, 0],
                config: vec![1, 0, 0, 0],
                // January 1, 2000 10:00:00
                created_at: OffsetDateTime::from_unix_timestamp(946720800)?,
                job_id: None,
                job_needed: false,
            }
        );

        assert_eq!(
            RawTracker::try_from(&Tracker {
                id: uuid!("00000000-0000-0000-0000-000000000001"),
                name: "tk".to_string(),
                url: Url::parse("https://retrack.dev")?,
                target: TrackerTarget::WebPage(WebPageTarget {
                    delay: Some(Duration::from_millis(2000)),
                    wait_for: Some(WebPageWaitFor {
                        selector: "div".to_string(),
                        state: Some(WebPageWaitForState::Attached),
                        timeout: Some(Duration::from_secs(5)),
                    }),
                }),
                config: TrackerConfig {
                    revisions: 1,
                    extractor: Some("return document.body.innerHTML;".to_string()),
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
                created_at: OffsetDateTime::from_unix_timestamp(946720800)?,
                job_id: Some(uuid!("00000000-0000-0000-0000-000000000002")),
            })?,
            RawTracker {
                id: uuid!("00000000-0000-0000-0000-000000000001"),
                name: "tk".to_string(),
                url: "https://retrack.dev/".to_string(),
                target: vec![0, 1, 2, 0, 1, 3, 100, 105, 118, 1, 0, 1, 5, 0],
                config: vec![
                    1, 1, 31, 114, 101, 116, 117, 114, 110, 32, 100, 111, 99, 117, 109, 101, 110,
                    116, 46, 98, 111, 100, 121, 46, 105, 110, 110, 101, 114, 72, 84, 77, 76, 59, 1,
                    1, 6, 99, 111, 111, 107, 105, 101, 9, 109, 121, 45, 99, 111, 111, 107, 105,
                    101, 1, 9, 48, 32, 48, 32, 42, 32, 42, 32, 42, 1, 1, 1, 128, 157, 202, 111, 2,
                    120, 0, 5, 1, 1
                ],
                // January 1, 2000 10:00:00
                created_at: OffsetDateTime::from_unix_timestamp(946720800)?,
                job_id: Some(uuid!("00000000-0000-0000-0000-000000000002")),
                job_needed: true,
            }
        );

        Ok(())
    }
}
