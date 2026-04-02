use crate::server::{SchedulerStatus, server_state::DatabaseStatus};
use serde::Serialize;
use utoipa::ToSchema;

/// Server status.
#[derive(Clone, Serialize, ToSchema)]
pub struct Status<'s> {
    /// Version of the server.
    pub version: &'s str,
    /// Status of the scheduler.
    pub scheduler: SchedulerStatus,
    /// Status of the database connection.
    pub db: DatabaseStatus,
}

impl Status<'_> {
    /// Checks if the API server and all its components are operational.
    pub fn is_operational(&self) -> bool {
        self.scheduler.operational && self.db.operational
    }
}

#[cfg(test)]
mod tests {
    use crate::server::{SchedulerStatus, Status, server_state::DatabaseStatus};
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
            scheduler,
            db: DatabaseStatus { operational: true },
        }, @r###"
        {
          "version": "1.0.0-alpha.4",
          "scheduler": {
            "operational": true,
            "timeTillNextJob": 10000
          },
          "db": {
            "operational": true
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
            db: DatabaseStatus { operational: true },
        };
        assert!(status.is_operational());

        let status = Status {
            version: "1.0.0-alpha.4",
            scheduler: SchedulerStatus {
                operational: false,
                time_till_next_job: None,
            },
            db: DatabaseStatus { operational: true },
        };
        assert!(!status.is_operational());

        let status = Status {
            version: "1.0.0-alpha.4",
            scheduler: SchedulerStatus {
                operational: true,
                time_till_next_job: Some(Duration::from_secs(10)),
            },
            db: DatabaseStatus { operational: false },
        };
        assert!(!status.is_operational());
    }
}
