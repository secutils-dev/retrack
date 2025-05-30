use crate::scheduler::SchedulerJob;
use serde::{Deserialize, Serialize};

/// Application-specific metadata of the scheduler job.
#[derive(Serialize, Deserialize, Debug, Copy, Clone, Hash, PartialEq, Eq)]
pub struct SchedulerJobMetadata {
    /// The type of the job.
    pub job_type: SchedulerJob,
    /// Indicates whether the job is currently running.
    pub is_running: bool,
    /// If the job is being re-tried, contains the current retry attempt.
    pub retry_attempt: u32,
}

impl SchedulerJobMetadata {
    /// Creates a new job state without a retry state.
    pub fn new(job_type: SchedulerJob) -> Self {
        Self {
            job_type,
            is_running: false,
            retry_attempt: 0,
        }
    }

    /// Sets the `is_running` flag for the job.
    pub fn set_running(&mut self) -> &mut Self {
        self.is_running = true;
        self
    }

    /// Sets the retry state for the job.
    pub fn set_retry_attempt(&mut self, attempt: u32) -> &mut Self {
        self.retry_attempt = attempt;
        self
    }
}

impl TryFrom<&[u8]> for SchedulerJobMetadata {
    type Error = anyhow::Error;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        Ok(postcard::from_bytes(value)?)
    }
}

impl<'a> TryFrom<&'a SchedulerJobMetadata> for Vec<u8> {
    type Error = anyhow::Error;

    fn try_from(value: &'a SchedulerJobMetadata) -> Result<Self, Self::Error> {
        Ok(postcard::to_stdvec(value)?)
    }
}

#[cfg(test)]
mod tests {
    use super::SchedulerJob;
    use crate::scheduler::SchedulerJobMetadata;
    use insta::assert_debug_snapshot;

    #[test]
    fn properly_creates_metadata() -> anyhow::Result<()> {
        for job in &[
            SchedulerJob::TasksRun,
            SchedulerJob::TrackersSchedule,
            SchedulerJob::TrackersRun,
        ] {
            assert_eq!(
                SchedulerJobMetadata::new(*job),
                SchedulerJobMetadata {
                    job_type: *job,
                    is_running: false,
                    retry_attempt: 0,
                }
            );
        }

        assert_eq!(
            SchedulerJobMetadata::new(SchedulerJob::TrackersRun).set_running(),
            &SchedulerJobMetadata {
                job_type: SchedulerJob::TrackersRun,
                is_running: true,
                retry_attempt: 0,
            }
        );

        assert_eq!(
            SchedulerJobMetadata::new(SchedulerJob::TrackersRun).set_retry_attempt(10),
            &SchedulerJobMetadata {
                job_type: SchedulerJob::TrackersRun,
                is_running: false,
                retry_attempt: 10,
            }
        );

        assert_eq!(
            SchedulerJobMetadata::new(SchedulerJob::TrackersRun)
                .set_running()
                .set_retry_attempt(10),
            &SchedulerJobMetadata {
                job_type: SchedulerJob::TrackersRun,
                is_running: true,
                retry_attempt: 10,
            }
        );

        Ok(())
    }

    #[test]
    fn serialize() -> anyhow::Result<()> {
        assert_eq!(
            Vec::try_from(&SchedulerJobMetadata::new(SchedulerJob::TasksRun))?,
            vec![0, 0, 0]
        );
        assert_eq!(
            Vec::try_from(&SchedulerJobMetadata::new(SchedulerJob::TrackersSchedule))?,
            vec![1, 0, 0]
        );
        assert_eq!(
            Vec::try_from(&SchedulerJobMetadata {
                job_type: SchedulerJob::TrackersRun,
                retry_attempt: 10,
                is_running: true
            })?,
            vec![2, 1, 10]
        );

        Ok(())
    }

    #[test]
    fn deserialize() -> anyhow::Result<()> {
        assert_eq!(
            SchedulerJobMetadata::try_from([0, 0, 0].as_ref())?,
            SchedulerJobMetadata::new(SchedulerJob::TasksRun)
        );

        assert_eq!(
            SchedulerJobMetadata::try_from([1, 0, 0].as_ref())?,
            SchedulerJobMetadata::new(SchedulerJob::TrackersSchedule)
        );

        assert_eq!(
            SchedulerJobMetadata::try_from([2, 1, 10].as_ref())?,
            SchedulerJobMetadata {
                job_type: SchedulerJob::TrackersRun,
                retry_attempt: 10,
                is_running: true
            }
        );

        assert_eq!(
            SchedulerJobMetadata::try_from([2, 1, 0].as_ref())?,
            SchedulerJobMetadata {
                job_type: SchedulerJob::TrackersRun,
                retry_attempt: 0,
                is_running: true
            }
        );

        assert_debug_snapshot!(SchedulerJobMetadata::try_from([3].as_ref()), @r###"
        Err(
            SerdeDeCustom,
        )
        "###);

        Ok(())
    }
}
