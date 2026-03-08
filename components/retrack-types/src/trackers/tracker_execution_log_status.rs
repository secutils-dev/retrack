use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Status of a tracker execution.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum TrackerExecutionLogStatus {
    Success,
    Failure,
}

#[cfg(test)]
mod tests {
    use super::TrackerExecutionLogStatus;
    use insta::assert_json_snapshot;

    #[test]
    fn serialization() {
        assert_json_snapshot!(TrackerExecutionLogStatus::Success, @r###""success""###);
        assert_json_snapshot!(TrackerExecutionLogStatus::Failure, @r###""failure""###);
    }
}
