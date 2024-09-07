mod email_action;
mod webhook_action;

pub use self::{email_action::EmailAction, webhook_action::WebhookAction};

use serde_derive::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Tracker's action (sending emails or HTTP request, logging, or transforming the tracker data).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "camelCase")]
#[serde(tag = "type")]
pub enum TrackerAction {
    /// Sends an email with the extracted data.
    Email(EmailAction),
    /// Sends an HTTP request with the extracted data as a JSON.
    Webhook(WebhookAction),
    /// Records extracted data in a server log.
    #[serde(rename = "log")]
    ServerLog,
}

#[cfg(test)]
mod tests {
    use super::TrackerAction;
    use crate::trackers::{EmailAction, WebhookAction};
    use http::{header::CONTENT_TYPE, Method};
    use insta::assert_json_snapshot;
    use serde_json::json;
    use std::collections::HashMap;

    #[test]
    fn serialization() -> anyhow::Result<()> {
        let action = TrackerAction::Email(EmailAction {
            to: vec!["dev@retrack.dev".to_string()],
        });
        assert_json_snapshot!(action, @r###"
        {
          "type": "email",
          "to": [
            "dev@retrack.dev"
          ]
        }
        "###);

        let action = TrackerAction::Webhook(WebhookAction {
            url: "https://retrack.dev".parse()?,
            method: Some(Method::PUT),
            headers: Some(
                (&[(CONTENT_TYPE, "application/json".to_string())]
                    .into_iter()
                    .collect::<HashMap<_, _>>())
                    .try_into()?,
            ),
        });
        assert_json_snapshot!(action, @r###"
        {
          "type": "webhook",
          "url": "https://retrack.dev/",
          "method": "PUT",
          "headers": {
            "content-type": "application/json"
          }
        }
        "###);

        let action = TrackerAction::Webhook(WebhookAction {
            url: "https://retrack.dev".parse()?,
            method: None,
            headers: None,
        });
        assert_json_snapshot!(action, @r###"
        {
          "type": "webhook",
          "url": "https://retrack.dev/"
        }
        "###);

        let action = TrackerAction::ServerLog;
        assert_json_snapshot!(action, @r###"
        {
          "type": "log"
        }
        "###);

        Ok(())
    }

    #[test]
    fn deserialization() -> anyhow::Result<()> {
        let action = TrackerAction::Email(EmailAction {
            to: vec!["dev@retrack.dev".to_string()],
        });
        assert_eq!(
            serde_json::from_str::<TrackerAction>(
                &json!({ "type": "email", "to": ["dev@retrack.dev"] }).to_string()
            )?,
            action
        );

        let action = TrackerAction::Webhook(WebhookAction {
            url: "https://retrack.dev".parse()?,
            method: Some(Method::PUT),
            headers: Some(
                (&[(CONTENT_TYPE, "application/json".to_string())]
                    .into_iter()
                    .collect::<HashMap<_, _>>())
                    .try_into()?,
            ),
        });
        assert_eq!(
            serde_json::from_str::<TrackerAction>(
                &json!({
                    "type": "webhook",
                    "url": "https://retrack.dev",
                    "method": "PUT",
                    "headers": { "content-type": "application/json" }
                })
                .to_string()
            )?,
            action
        );

        let action = TrackerAction::Webhook(WebhookAction {
            url: "https://retrack.dev".parse()?,
            method: None,
            headers: None,
        });
        assert_eq!(
            serde_json::from_str::<TrackerAction>(
                &json!({ "type": "webhook", "url": "https://retrack.dev" }).to_string()
            )?,
            action
        );

        let action = TrackerAction::ServerLog;
        assert_eq!(
            serde_json::from_str::<TrackerAction>(&json!({ "type": "log" }).to_string())?,
            action
        );

        Ok(())
    }
}
