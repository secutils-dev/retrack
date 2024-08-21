use crate::server::SchedulerStatus;
use serde::Serialize;
use utoipa::ToSchema;

/// Server status.
#[derive(Clone, Serialize, ToSchema)]
pub struct Status<'s> {
    /// Version of the server.
    pub version: &'s str,
    /// Status of the scheduler.
    pub scheduler: SchedulerStatus,
}

impl<'s> Status<'s> {
    /// Checks if the API server and all its components are operational.
    pub fn is_operational(&self) -> bool {
        self.scheduler.operational
    }
}

#[cfg(test)]
mod tests {
    use crate::server::{SchedulerStatus, Status};
    use insta::assert_json_snapshot;
    use std::time::Duration;

    #[test]
    fn serialization() -> anyhow::Result<()> {
        let scheduler = SchedulerStatus {
            operational: true,
            time_till_next_job: Some(Duration::from_secs(10)),
        };
        assert_json_snapshot!(Status {
            version: "1.0.0-alpha.4",
            scheduler
        }, @r###"
        {
          "version": "1.0.0-alpha.4",
          "scheduler": {
            "operational": true,
            "timeTillNextJob": 10000
          }
        }
        "###);

        Ok(())
    }

    #[test]
    fn is_operational() {
        let status = Status {
            version: "1.0.0-alpha.4",
            scheduler: SchedulerStatus {
                operational: true,
                time_till_next_job: Some(Duration::from_secs(10)),
            },
        };
        assert!(status.is_operational());

        let status = Status {
            version: "1.0.0-alpha.4",
            scheduler: SchedulerStatus {
                operational: false,
                time_till_next_job: None,
            },
        };
        assert!(!status.is_operational());
    }
}
