use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use uuid::Uuid;

/// The payload to send with the HTTP request while executing the tracker's webhook action.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WebhookActionPayload {
    /// ID of the tracker for which the webhook action is executed.
    pub tracker_id: Uuid,

    /// Name of the tracker for which the webhook action is executed.
    pub tracker_name: String,

    /// Result of the tracker execution.
    pub result: WebhookActionPayloadResult,
}

/// The payload to send with the HTTP request while executing the tracker's webhook action.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[serde(tag = "type", content = "value")]
pub enum WebhookActionPayloadResult {
    Success(JsonValue),
    Failure(String),
}

#[cfg(test)]
mod tests {
    use crate::trackers::{WebhookActionPayload, WebhookActionPayloadResult};
    use insta::assert_json_snapshot;
    use serde_json::json;
    use uuid::uuid;

    #[test]
    fn serialization() -> anyhow::Result<()> {
        let payload = WebhookActionPayload {
            tracker_id: uuid!("00000000-0000-0000-0000-000000000001"),
            tracker_name: "tracker1".to_string(),
            result: WebhookActionPayloadResult::Success(json!({ "prop":  100 })),
        };
        assert_json_snapshot!(payload, @r###"
        {
          "trackerId": "00000000-0000-0000-0000-000000000001",
          "trackerName": "tracker1",
          "result": {
            "type": "success",
            "value": {
              "prop": 100
            }
          }
        }
        "###);

        let payload = WebhookActionPayload {
            tracker_id: uuid!("00000000-0000-0000-0000-000000000001"),
            tracker_name: "tracker1".to_string(),
            result: WebhookActionPayloadResult::Failure("Uh oh".to_string()),
        };
        assert_json_snapshot!(payload, @r###"
        {
          "trackerId": "00000000-0000-0000-0000-000000000001",
          "trackerName": "tracker1",
          "result": {
            "type": "failure",
            "value": "Uh oh"
          }
        }
        "###);

        Ok(())
    }

    #[test]
    fn deserialization() -> anyhow::Result<()> {
        let payload = WebhookActionPayload {
            tracker_id: uuid!("00000000-0000-0000-0000-000000000001"),
            tracker_name: "tracker1".to_string(),
            result: WebhookActionPayloadResult::Success(json!({ "prop":  100 })),
        };
        assert_eq!(
            serde_json::from_str::<WebhookActionPayload>(
                &json!({
                    "trackerId": "00000000-0000-0000-0000-000000000001",
                    "trackerName": "tracker1",
                    "result": {
                        "type": "success",
                        "value": { "prop": 100 }
                    }
                })
                .to_string()
            )?,
            payload
        );

        let payload = WebhookActionPayload {
            tracker_id: uuid!("00000000-0000-0000-0000-000000000001"),
            tracker_name: "tracker1".to_string(),
            result: WebhookActionPayloadResult::Failure("Uh oh".to_string()),
        };
        assert_eq!(
            serde_json::from_str::<WebhookActionPayload>(
                &json!({
                    "trackerId": "00000000-0000-0000-0000-000000000001",
                    "trackerName": "tracker1",
                    "result": {
                        "type": "failure",
                        "value": "Uh oh"
                    }
                })
                .to_string()
            )?,
            payload
        );

        Ok(())
    }
}
