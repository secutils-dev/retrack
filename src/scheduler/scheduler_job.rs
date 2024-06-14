use serde::{Deserialize, Serialize};

/// Represents a job that can be scheduled.
#[derive(Serialize, Deserialize, Debug, Copy, Clone, Hash, PartialEq, Eq)]
pub enum SchedulerJob {
    TrackersTrigger,
    TrackersSchedule,
    TrackersFetch,
    NotificationsSend,
}

impl SchedulerJob {
    /// Indicates whether the job should be scheduled only once.
    pub fn is_unique(&self) -> bool {
        match self {
            Self::TrackersSchedule => true,
            Self::TrackersTrigger => false,
            Self::TrackersFetch => true,
            Self::NotificationsSend => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SchedulerJob;

    #[test]
    fn properly_determines_unique_jobs() -> anyhow::Result<()> {
        assert!(!SchedulerJob::TrackersTrigger.is_unique());
        assert!(SchedulerJob::TrackersSchedule.is_unique());
        assert!(SchedulerJob::TrackersFetch.is_unique());
        assert!(SchedulerJob::NotificationsSend.is_unique());

        Ok(())
    }
}
