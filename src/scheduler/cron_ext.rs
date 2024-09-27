use croner::Cron;
use std::time::Duration;

pub trait CronExt {
    /// Returns the minimum interval between occurrences.
    fn min_interval(&self) -> anyhow::Result<Duration>;

    /// Converts string cron pattern to `Cron` expression while properly handling aliases (see
    /// https://github.com/Hexagon/croner-rust/issues/9).
    fn parse_pattern(pattern: impl AsRef<str>) -> anyhow::Result<Cron>;
}

impl CronExt for Cron {
    /// Returns the minimum interval between occurrences. To calculate it, we take the first 100
    /// upcoming occurrences and calculate the interval between each of them. Then we take the
    /// smallest interval.
    fn min_interval(&self) -> anyhow::Result<Duration> {
        let mut minimum_interval = Duration::MAX;
        let next_occurrences = self
            .iter_from(chrono::Utc::now())
            .take(100)
            .collect::<Vec<_>>();
        for (index, occurrence) in next_occurrences.iter().enumerate().skip(1) {
            let interval = (*occurrence - next_occurrences[index - 1]).to_std()?;
            if interval < minimum_interval {
                minimum_interval = interval;
            }
        }

        Ok(minimum_interval)
    }

    /// Converts string cron pattern to `Cron` expression while properly handling aliases (see
    /// https://github.com/Hexagon/croner-rust/issues/9).
    fn parse_pattern(pattern: impl AsRef<str>) -> anyhow::Result<Cron> {
        let pattern = pattern.as_ref().trim();

        // Closure that performs a case-insensitive comparison of two strings.
        let eq_ignore_case = |a: &str, b: &str| a.eq_ignore_ascii_case(b);

        let pattern = match pattern {
            p if eq_ignore_case(p, "@yearly") || eq_ignore_case(p, "@annually") => "0 0 0 1 1 *",
            p if eq_ignore_case(p, "@monthly") => "0 0 0 1 * *",
            p if eq_ignore_case(p, "@weekly") => "0 0 0 * * 0",
            p if eq_ignore_case(p, "@daily") => "0 0 0 * * *",
            p if eq_ignore_case(p, "@hourly") => "0 0 * * * *",
            _ => pattern,
        };

        Ok(Cron::new(pattern)
            .with_seconds_required()
            .with_dom_and_dow()
            .parse()?)
    }
}

#[cfg(test)]
mod tests {
    use super::CronExt;
    use croner::Cron;
    use std::time::Duration;

    #[test]
    fn can_calculate_min_interval() -> anyhow::Result<()> {
        let schedule = Cron::parse_pattern("0 * * * * *")?;
        assert_eq!(schedule.min_interval()?, Duration::from_secs(60));

        let schedule = Cron::parse_pattern("0 0 * * * *")?;
        assert_eq!(schedule.min_interval()?, Duration::from_secs(3600));
        let schedule = Cron::parse_pattern("@hourly")?;
        assert_eq!(schedule.min_interval()?, Duration::from_secs(3600));

        let schedule = Cron::parse_pattern("0 0 0 * * *")?;
        assert_eq!(schedule.min_interval()?, Duration::from_secs(24 * 3600));
        let schedule = Cron::parse_pattern("@daily")?;
        assert_eq!(schedule.min_interval()?, Duration::from_secs(24 * 3600));

        let schedule = Cron::parse_pattern("0 0 0 * * 1")?;
        assert_eq!(schedule.min_interval()?, Duration::from_secs(7 * 24 * 3600));
        let schedule = Cron::parse_pattern("@weekly")?;
        assert_eq!(schedule.min_interval()?, Duration::from_secs(7 * 24 * 3600));

        Ok(())
    }
}