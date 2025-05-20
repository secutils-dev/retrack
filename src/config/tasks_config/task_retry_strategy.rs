use serde::{Deserialize, Serialize};
use serde_with::{DurationMilliSeconds, serde_as};
use std::time::Duration;

/// Represents a retry strategy for a task.
#[serde_as]
#[derive(Serialize, Deserialize, Debug, Copy, Clone, Hash, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum TaskRetryStrategy {
    /// The task will be retried with a constant interval (1s -> 1s -> 1s).
    Constant {
        #[serde_as(as = "DurationMilliSeconds<u64>")]
        interval: Duration,
        max_attempts: u32,
    },
    /// The task will be retried with an exponential interval (1s -> 2s -> 4s -> 8s).
    Exponential {
        #[serde_as(as = "DurationMilliSeconds<u64>")]
        initial_interval: Duration,
        multiplier: u32,
        #[serde_as(as = "DurationMilliSeconds<u64>")]
        max_interval: Duration,
        max_attempts: u32,
    },
    /// The task will be retried with a linear interval (1s -> 2s -> 3s).
    Linear {
        #[serde_as(as = "DurationMilliSeconds<u64>")]
        initial_interval: Duration,
        #[serde_as(as = "DurationMilliSeconds<u64>")]
        increment: Duration,
        #[serde_as(as = "DurationMilliSeconds<u64>")]
        max_interval: Duration,
        max_attempts: u32,
    },
}

impl TaskRetryStrategy {
    /// Calculates the interval for the next retry attempt.
    pub fn interval(&self, attempt: u32) -> Duration {
        match self {
            Self::Constant { interval, .. } => *interval,
            Self::Exponential {
                initial_interval,
                multiplier,
                max_interval,
                ..
            } => multiplier
                .checked_pow(attempt)
                .and_then(|multiplier| initial_interval.checked_mul(multiplier))
                .map(|interval| interval.min(*max_interval))
                .unwrap_or_else(|| *max_interval),
            Self::Linear {
                initial_interval,
                increment,
                max_interval,
                ..
            } => increment
                .checked_mul(attempt)
                .and_then(|increment| initial_interval.checked_add(increment))
                .map(|interval| interval.min(*max_interval))
                .unwrap_or_else(|| *max_interval),
        }
    }

    /// Returns the maximum number of attempts.
    pub fn max_attempts(&self) -> u32 {
        match self {
            Self::Constant { max_attempts, .. } => *max_attempts,
            Self::Exponential { max_attempts, .. } => *max_attempts,
            Self::Linear { max_attempts, .. } => *max_attempts,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::TaskRetryStrategy;
    use insta::assert_toml_snapshot;
    use std::time::Duration;

    #[test]
    fn serialization() {
        let strategy = TaskRetryStrategy::Constant {
            interval: Duration::from_secs(1),
            max_attempts: 10,
        };
        assert_toml_snapshot!(strategy, @r###"
        type = 'constant'
        interval = 1000
        max_attempts = 10
        "###);

        let strategy = TaskRetryStrategy::Exponential {
            initial_interval: Duration::from_secs(1),
            multiplier: 2,
            max_interval: Duration::from_secs(10),
            max_attempts: 15,
        };
        assert_toml_snapshot!(strategy, @r###"
        type = 'exponential'
        initial_interval = 1000
        multiplier = 2
        max_interval = 10000
        max_attempts = 15
        "###);

        let strategy = TaskRetryStrategy::Linear {
            initial_interval: Duration::from_secs(1),
            increment: Duration::from_secs(1),
            max_interval: Duration::from_secs(10),
            max_attempts: 20,
        };
        assert_toml_snapshot!(strategy, @r###"
        type = 'linear'
        initial_interval = 1000
        increment = 1000
        max_interval = 10000
        max_attempts = 20
        "###);
    }

    #[test]
    fn deserialization() {
        let strategy: TaskRetryStrategy = toml::from_str(
            r#"
        type = 'constant'
        interval = 1000
        max_attempts = 10
    "#,
        )
        .unwrap();
        assert_eq!(
            strategy,
            TaskRetryStrategy::Constant {
                interval: Duration::from_secs(1),
                max_attempts: 10,
            }
        );

        let strategy: TaskRetryStrategy = toml::from_str(
            r#"
        type = 'exponential'
        initial_interval = 1000
        multiplier = 2
        max_interval = 10000
        max_attempts = 15
    "#,
        )
        .unwrap();
        assert_eq!(
            strategy,
            TaskRetryStrategy::Exponential {
                initial_interval: Duration::from_secs(1),
                multiplier: 2,
                max_interval: Duration::from_secs(10),
                max_attempts: 15,
            }
        );

        let strategy: TaskRetryStrategy = toml::from_str(
            r#"
        type = 'linear'
        initial_interval = 1000
        increment = 1000
        max_interval = 10000
        max_attempts = 20
    "#,
        )
        .unwrap();
        assert_eq!(
            strategy,
            TaskRetryStrategy::Linear {
                initial_interval: Duration::from_secs(1),
                increment: Duration::from_secs(1),
                max_interval: Duration::from_secs(10),
                max_attempts: 20,
            }
        );
    }

    #[test]
    fn properly_detects_max_number_of_attempts() {
        assert_eq!(
            TaskRetryStrategy::Constant {
                interval: Duration::from_secs(1),
                max_attempts: 10,
            }
            .max_attempts(),
            10
        );
        assert_eq!(
            TaskRetryStrategy::Exponential {
                initial_interval: Duration::from_secs(1),
                multiplier: 2,
                max_interval: Duration::from_secs(10),
                max_attempts: 15,
            }
            .max_attempts(),
            15
        );
        assert_eq!(
            TaskRetryStrategy::Linear {
                initial_interval: Duration::from_secs(1),
                increment: Duration::from_secs(1),
                max_interval: Duration::from_secs(10),
                max_attempts: 20,
            }
            .max_attempts(),
            20
        );
    }

    #[test]
    fn properly_calculates_constant_interval() {
        let retry_strategy = TaskRetryStrategy::Constant {
            interval: Duration::from_secs(1),
            max_attempts: 10,
        };
        assert_eq!(retry_strategy.interval(0), Duration::from_secs(1));
        assert_eq!(retry_strategy.interval(1), Duration::from_secs(1));
        assert_eq!(retry_strategy.interval(2), Duration::from_secs(1));
        assert_eq!(retry_strategy.interval(u32::MAX), Duration::from_secs(1));
    }

    #[test]
    fn properly_calculates_linear_interval() {
        let retry_strategy = TaskRetryStrategy::Linear {
            initial_interval: Duration::from_secs(1),
            increment: Duration::from_secs(1),
            max_interval: Duration::from_secs(5),
            max_attempts: 10,
        };
        assert_eq!(retry_strategy.interval(0), Duration::from_secs(1));
        assert_eq!(retry_strategy.interval(1), Duration::from_secs(2));
        assert_eq!(retry_strategy.interval(2), Duration::from_secs(3));
        assert_eq!(retry_strategy.interval(3), Duration::from_secs(4));
        assert_eq!(retry_strategy.interval(4), Duration::from_secs(5));
        assert_eq!(retry_strategy.interval(5), Duration::from_secs(5));
        assert_eq!(retry_strategy.interval(6), Duration::from_secs(5));
        assert_eq!(retry_strategy.interval(100), Duration::from_secs(5));
        assert_eq!(retry_strategy.interval(u32::MAX), Duration::from_secs(5));
    }

    #[test]
    fn properly_calculates_exponential_interval() {
        let retry_strategy = TaskRetryStrategy::Exponential {
            initial_interval: Duration::from_secs(1),
            multiplier: 2,
            max_interval: Duration::from_secs(100),
            max_attempts: 10,
        };
        assert_eq!(retry_strategy.interval(0), Duration::from_secs(1));
        assert_eq!(retry_strategy.interval(1), Duration::from_secs(2));
        assert_eq!(retry_strategy.interval(2), Duration::from_secs(4));
        assert_eq!(retry_strategy.interval(3), Duration::from_secs(8));
        assert_eq!(retry_strategy.interval(4), Duration::from_secs(16));
        assert_eq!(retry_strategy.interval(5), Duration::from_secs(32));
        assert_eq!(retry_strategy.interval(6), Duration::from_secs(64));
        assert_eq!(retry_strategy.interval(7), Duration::from_secs(100));
        assert_eq!(retry_strategy.interval(100), Duration::from_secs(100));
        assert_eq!(retry_strategy.interval(u32::MAX), Duration::from_secs(100));
    }
}
