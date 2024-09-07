use serde::{Deserialize, Serialize};

/// Represents a job that can be scheduled.
#[derive(Serialize, Deserialize, Debug, Copy, Clone, Hash, PartialEq, Eq)]
pub enum SchedulerJob {
    TrackersTrigger,
    TrackersSchedule,
    TrackersRun,
    TasksRun,
}

impl SchedulerJob {
    /// Indicates whether the job should be scheduled only once.
    pub fn is_unique(&self) -> bool {
        match self {
            Self::TrackersSchedule => true,
            Self::TrackersTrigger => false,
            Self::TrackersRun => true,
            Self::TasksRun => true,
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
        assert!(SchedulerJob::TrackersRun.is_unique());
        assert!(SchedulerJob::TasksRun.is_unique());

        Ok(())
    }
}
