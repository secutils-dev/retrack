use crate::{
    scheduler::{SchedulerJobConfig, SchedulerJobRetryStrategy},
    trackers::{Tracker, TrackerSettings},
};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, time::Duration};
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug, Eq, PartialEq, Clone)]
pub(super) struct RawTracker {
    pub id: Uuid,
    pub name: String,
    pub url: String,
    pub job_id: Option<Uuid>,
    pub job_config: Option<Vec<u8>>,
    pub data: Vec<u8>,
    pub created_at: OffsetDateTime,
}

#[derive(Serialize, Deserialize, Debug, Eq, PartialEq, Clone)]
pub(super) struct RawTrackerData {
    pub revisions: usize,
    pub delay: u64,
    pub scripts: Option<String>,
    pub headers: Option<HashMap<String, String>>,
}

#[derive(Serialize, Deserialize)]
struct RawSchedulerJobConfig(String, Option<RawSchedulerJobRetryStrategy>, bool);

#[derive(Serialize, Deserialize)]
enum RawSchedulerJobRetryStrategy {
    Constant(Duration, u32),
    Exponential(Duration, u32, Duration, u32),
    Linear(Duration, Duration, Duration, u32),
}

impl TryFrom<RawTracker> for Tracker {
    type Error = anyhow::Error;

    fn try_from(raw: RawTracker) -> Result<Self, Self::Error> {
        let raw_data = postcard::from_bytes::<RawTrackerData>(&raw.data)?;

        let job_config = if let Some(job_config) = raw.job_config {
            let RawSchedulerJobConfig(schedule, retry_strategy, notifications) =
                postcard::from_bytes(&job_config)?;
            Some(SchedulerJobConfig {
                schedule,
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
            job_id: raw.job_id,
            job_config,
            settings: TrackerSettings {
                revisions: raw_data.revisions,
                delay: Duration::from_millis(raw_data.delay),
                extractor: raw_data.scripts,
                headers: raw_data.headers,
            },
            created_at: raw.created_at,
        })
    }
}

impl TryFrom<&Tracker> for RawTracker {
    type Error = anyhow::Error;

    fn try_from(item: &Tracker) -> Result<Self, Self::Error> {
        let raw_data = RawTrackerData {
            revisions: item.settings.revisions,
            delay: item.settings.delay.as_millis() as u64,
            scripts: item.settings.extractor.clone(),
            headers: item.settings.headers.clone(),
        };

        let job_config = if let Some(SchedulerJobConfig {
            schedule,
            retry_strategy,
            notifications,
        }) = &item.job_config
        {
            Some(postcard::to_stdvec(&RawSchedulerJobConfig(
                schedule.to_string(),
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
            ))?)
        } else {
            None
        };

        Ok(RawTracker {
            id: item.id,
            name: item.name.clone(),
            url: item.url.to_string(),
            job_id: item.job_id,
            job_config,
            data: postcard::to_stdvec(&raw_data)?,
            created_at: item.created_at,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::RawTracker;
    use crate::{
        scheduler::{SchedulerJobConfig, SchedulerJobRetryStrategy},
        trackers::{Tracker, TrackerSettings},
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
                job_id: None,
                job_config: None,
                data: vec![1, 0, 0, 0],
                // January 1, 2000 10:00:00
                created_at: OffsetDateTime::from_unix_timestamp(946720800)?,
            })?,
            Tracker {
                id: uuid!("00000000-0000-0000-0000-000000000001"),
                name: "tk".to_string(),
                url: Url::parse("https://retrack.dev")?,
                job_id: None,
                job_config: None,
                settings: TrackerSettings {
                    revisions: 1,
                    delay: Default::default(),
                    extractor: Default::default(),
                    headers: Default::default()
                },
                created_at: OffsetDateTime::from_unix_timestamp(946720800)?
            }
        );

        assert_eq!(
            Tracker::try_from(RawTracker {
                id: uuid!("00000000-0000-0000-0000-000000000001"),
                name: "tk".to_string(),
                url: "https://retrack.dev".to_string(),
                job_id: Some(uuid!("00000000-0000-0000-0000-000000000002")),
                job_config: Some(vec![
                    9, 48, 32, 48, 32, 42, 32, 42, 32, 42, 1, 1, 1, 128, 157, 202, 111, 2, 120, 0,
                    5, 1
                ]),
                data: vec![
                    1, 208, 15, 1, 31, 114, 101, 116, 117, 114, 110, 32, 100, 111, 99, 117, 109,
                    101, 110, 116, 46, 98, 111, 100, 121, 46, 105, 110, 110, 101, 114, 72, 84, 77,
                    76, 59, 1, 1, 6, 99, 111, 111, 107, 105, 101, 9, 109, 121, 45, 99, 111, 111,
                    107, 105, 101
                ],
                // January 1, 2000 10:00:00
                created_at: OffsetDateTime::from_unix_timestamp(946720800)?,
            })?,
            Tracker {
                id: uuid!("00000000-0000-0000-0000-000000000001"),
                name: "tk".to_string(),
                url: Url::parse("https://retrack.dev")?,
                job_id: Some(uuid!("00000000-0000-0000-0000-000000000002")),
                job_config: Some(SchedulerJobConfig {
                    schedule: "0 0 * * *".to_string(),
                    retry_strategy: Some(SchedulerJobRetryStrategy::Exponential {
                        initial_interval: Duration::from_millis(1234),
                        multiplier: 2,
                        max_interval: Duration::from_secs(120),
                        max_attempts: 5,
                    }),
                    notifications: true
                }),
                settings: TrackerSettings {
                    revisions: 1,
                    delay: Duration::from_millis(2000),
                    extractor: Some("return document.body.innerHTML;".to_string()),
                    headers: Some(
                        [("cookie".to_string(), "my-cookie".to_string())]
                            .into_iter()
                            .collect()
                    )
                },
                created_at: OffsetDateTime::from_unix_timestamp(946720800)?
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
                job_id: None,
                job_config: None,
                settings: TrackerSettings {
                    revisions: 1,
                    delay: Default::default(),
                    extractor: Default::default(),
                    headers: Default::default(),
                },
                created_at: OffsetDateTime::from_unix_timestamp(946720800)?
            })?,
            RawTracker {
                id: uuid!("00000000-0000-0000-0000-000000000001"),
                name: "tk".to_string(),
                url: "https://retrack.dev/".to_string(),
                job_id: None,
                job_config: None,
                data: vec![1, 0, 0, 0],
                // January 1, 2000 10:00:00
                created_at: OffsetDateTime::from_unix_timestamp(946720800)?,
            }
        );

        assert_eq!(
            RawTracker::try_from(&Tracker {
                id: uuid!("00000000-0000-0000-0000-000000000001"),
                name: "tk".to_string(),
                url: Url::parse("https://retrack.dev")?,
                job_id: Some(uuid!("00000000-0000-0000-0000-000000000002")),
                job_config: Some(SchedulerJobConfig {
                    schedule: "0 0 * * *".to_string(),
                    retry_strategy: Some(SchedulerJobRetryStrategy::Exponential {
                        initial_interval: Duration::from_millis(1234),
                        multiplier: 2,
                        max_interval: Duration::from_secs(120),
                        max_attempts: 5,
                    }),
                    notifications: true
                }),
                settings: TrackerSettings {
                    revisions: 1,
                    delay: Duration::from_millis(2000),
                    extractor: Some("return document.body.innerHTML;".to_string()),
                    headers: Some(
                        [("cookie".to_string(), "my-cookie".to_string())]
                            .into_iter()
                            .collect()
                    ),
                },
                created_at: OffsetDateTime::from_unix_timestamp(946720800)?
            })?,
            RawTracker {
                id: uuid!("00000000-0000-0000-0000-000000000001"),
                name: "tk".to_string(),
                url: "https://retrack.dev/".to_string(),
                job_id: Some(uuid!("00000000-0000-0000-0000-000000000002")),
                job_config: Some(vec![
                    9, 48, 32, 48, 32, 42, 32, 42, 32, 42, 1, 1, 1, 128, 157, 202, 111, 2, 120, 0,
                    5, 1
                ]),
                data: vec![
                    1, 208, 15, 1, 31, 114, 101, 116, 117, 114, 110, 32, 100, 111, 99, 117, 109,
                    101, 110, 116, 46, 98, 111, 100, 121, 46, 105, 110, 110, 101, 114, 72, 84, 77,
                    76, 59, 1, 1, 6, 99, 111, 111, 107, 105, 101, 9, 109, 121, 45, 99, 111, 111,
                    107, 105, 101
                ],
                // January 1, 2000 10:00:00
                created_at: OffsetDateTime::from_unix_timestamp(946720800)?,
            }
        );

        Ok(())
    }
}
