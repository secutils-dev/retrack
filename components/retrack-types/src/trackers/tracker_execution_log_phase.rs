use crate::trackers::TrackerExecutionLogStatus;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use utoipa::ToSchema;

/// A single step in a tracker execution timeline.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TrackerExecutionLogPhase {
    /// Phase identifier (e.g. "fetch_data", "extract", "compare", "persist", "action:webhook").
    pub phase: String,
    /// Duration of this phase in milliseconds.
    pub duration_ms: u64,
    /// Whether this phase succeeded or failed.
    pub status: TrackerExecutionLogStatus,
    /// Optional structured metadata specific to this phase.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<JsonValue>,
}

#[cfg(test)]
mod tests {
    use super::TrackerExecutionLogPhase;
    use crate::trackers::TrackerExecutionLogStatus;
    use insta::assert_json_snapshot;
    use serde_json::json;

    #[test]
    fn serialization() -> anyhow::Result<()> {
        assert_json_snapshot!(TrackerExecutionLogPhase {
            phase: "fetch_data".to_string(),
            duration_ms: 2340,
            status: TrackerExecutionLogStatus::Success,
            meta: Some(json!({"statusCode": 200, "bodySize": 4521})),
        }, @r###"
        {
          "phase": "fetch_data",
          "durationMs": 2340,
          "status": "success",
          "meta": {
            "statusCode": 200,
            "bodySize": 4521
          }
        }
        "###);

        assert_json_snapshot!(TrackerExecutionLogPhase {
            phase: "compare".to_string(),
            duration_ms: 2,
            status: TrackerExecutionLogStatus::Success,
            meta: None,
        }, @r###"
        {
          "phase": "compare",
          "durationMs": 2,
          "status": "success"
        }
        "###);

        Ok(())
    }

    #[test]
    fn deserialization() -> anyhow::Result<()> {
        let phase: TrackerExecutionLogPhase = serde_json::from_str(
            r#"{"phase":"extract","durationMs":120,"status":"failure","meta":{"error":"script timeout"}}"#,
        )?;
        assert_eq!(phase.phase, "extract");
        assert_eq!(phase.duration_ms, 120);
        assert_eq!(phase.status, TrackerExecutionLogStatus::Failure);
        assert_eq!(phase.meta, Some(json!({"error": "script timeout"})));
        Ok(())
    }
}
