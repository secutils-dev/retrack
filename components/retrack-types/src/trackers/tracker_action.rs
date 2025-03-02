mod email_action;
mod formatter_script_args;
mod formatter_script_result;
mod server_log_action;
mod webhook_action;

pub use self::{
    email_action::EmailAction, formatter_script_args::FormatterScriptArgs,
    formatter_script_result::FormatterScriptResult, server_log_action::ServerLogAction,
    webhook_action::WebhookAction,
};
use serde::{Deserialize, Serialize};

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
    ServerLog(ServerLogAction),
}

impl TrackerAction {
    /// Returns the type tag of the action.
    pub fn type_tag(&self) -> &'static str {
        match self {
            TrackerAction::Email(_) => "email",
            TrackerAction::Webhook(_) => "webhook",
            TrackerAction::ServerLog(_) => "log",
        }
    }
}

impl TrackerAction {
    /// Returns the formatter script for the action, if specified.
    pub fn formatter(&self) -> Option<&str> {
        match self {
            TrackerAction::Email(EmailAction { formatter, .. })
            | TrackerAction::Webhook(WebhookAction { formatter, .. })
            | TrackerAction::ServerLog(ServerLogAction { formatter }) => formatter.as_deref(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::TrackerAction;
    use crate::trackers::{EmailAction, ServerLogAction, WebhookAction};
    use http::{Method, header::CONTENT_TYPE};
    use insta::assert_json_snapshot;
    use serde_json::json;
    use std::collections::HashMap;

    #[test]
    fn serialization() -> anyhow::Result<()> {
        let action = TrackerAction::Email(EmailAction {
            to: vec!["dev@retrack.dev".to_string()],
            formatter: Some(
                "(async () => Deno.core.encode(JSON.stringify({ key: 'value' })))();".to_string(),
            ),
        });
        assert_json_snapshot!(action, @r###"
        {
          "type": "email",
          "to": [
            "dev@retrack.dev"
          ],
          "formatter": "(async () => Deno.core.encode(JSON.stringify({ key: 'value' })))();"
        }
        "###);

        let action = TrackerAction::Email(EmailAction {
            to: vec!["dev@retrack.dev".to_string()],
            formatter: None,
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
            formatter: Some(
                "(async () => Deno.core.encode(JSON.stringify({ key: 'value' })))();".to_string(),
            ),
        });
        assert_json_snapshot!(action, @r###"
        {
          "type": "webhook",
          "url": "https://retrack.dev/",
          "method": "PUT",
          "headers": {
            "content-type": "application/json"
          },
          "formatter": "(async () => Deno.core.encode(JSON.stringify({ key: 'value' })))();"
        }
        "###);

        let action = TrackerAction::Webhook(WebhookAction {
            url: "https://retrack.dev".parse()?,
            method: None,
            headers: None,
            formatter: None,
        });
        assert_json_snapshot!(action, @r###"
        {
          "type": "webhook",
          "url": "https://retrack.dev/"
        }
        "###);

        let action = TrackerAction::ServerLog(ServerLogAction {
            formatter: Some(
                "(async () => Deno.core.encode(JSON.stringify({ key: 'value' })))();".to_string(),
            ),
        });
        assert_json_snapshot!(action, @r###"
        {
          "type": "log",
          "formatter": "(async () => Deno.core.encode(JSON.stringify({ key: 'value' })))();"
        }
        "###);

        let action = TrackerAction::ServerLog(Default::default());
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
            formatter: Some(
                "(async () => Deno.core.encode(JSON.stringify({ key: 'value' })))();".to_string(),
            ),
        });
        assert_eq!(
            serde_json::from_str::<TrackerAction>(
                &json!({
                    "type": "email",
                    "to": ["dev@retrack.dev"],
                    "formatter": "(async () => Deno.core.encode(JSON.stringify({ key: 'value' })))();"
                }).to_string()
            )?,
            action
        );

        let action = TrackerAction::Email(EmailAction {
            to: vec!["dev@retrack.dev".to_string()],
            formatter: None,
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
            formatter: Some(
                "(async () => Deno.core.encode(JSON.stringify({ key: 'value' })))();".to_string(),
            ),
        });
        assert_eq!(
            serde_json::from_str::<TrackerAction>(
                &json!({
                    "type": "webhook",
                    "url": "https://retrack.dev",
                    "method": "PUT",
                    "headers": { "content-type": "application/json" },
                    "formatter": "(async () => Deno.core.encode(JSON.stringify({ key: 'value' })))();"
                })
                    .to_string()
            )?,
            action
        );

        let action = TrackerAction::Webhook(WebhookAction {
            url: "https://retrack.dev".parse()?,
            method: None,
            headers: None,
            formatter: None,
        });
        assert_eq!(
            serde_json::from_str::<TrackerAction>(
                &json!({ "type": "webhook", "url": "https://retrack.dev" }).to_string()
            )?,
            action
        );

        let action = TrackerAction::ServerLog(ServerLogAction {
            formatter: Some(
                "(async () => Deno.core.encode(JSON.stringify({ key: 'value' })))();".to_string(),
            ),
        });
        assert_eq!(
            serde_json::from_str::<TrackerAction>(&json!({
                "type": "log",
                "formatter": "(async () => Deno.core.encode(JSON.stringify({ key: 'value' })))();"
            }).to_string())?,
            action
        );

        let action = TrackerAction::ServerLog(Default::default());
        assert_eq!(
            serde_json::from_str::<TrackerAction>(&json!({ "type": "log" }).to_string())?,
            action
        );

        Ok(())
    }

    #[test]
    fn can_return_formatter() {
        let actions_with_formatter = vec![
            TrackerAction::Email(EmailAction {
                to: vec!["dev@retrack.dev".to_string()],
                formatter: Some(
                    "(async () => Deno.core.encode(JSON.stringify({ key: 'value' })))();"
                        .to_string(),
                ),
            }),
            TrackerAction::Webhook(WebhookAction {
                url: "https://retrack.dev".parse().unwrap(),
                method: None,
                headers: None,
                formatter: Some(
                    "(async () => Deno.core.encode(JSON.stringify({ key: 'value' })))();"
                        .to_string(),
                ),
            }),
            TrackerAction::ServerLog(ServerLogAction {
                formatter: Some(
                    "(async () => Deno.core.encode(JSON.stringify({ key: 'value' })))();"
                        .to_string(),
                ),
            }),
        ];
        for action in actions_with_formatter {
            assert_eq!(
                action.formatter(),
                Some("(async () => Deno.core.encode(JSON.stringify({ key: 'value' })))();")
            );
        }

        let actions_without_formatter = vec![
            TrackerAction::Email(Default::default()),
            TrackerAction::Webhook(WebhookAction {
                url: "https://retrack.dev".parse().unwrap(),
                method: None,
                headers: None,
                formatter: None,
            }),
            TrackerAction::ServerLog(Default::default()),
        ];
        for action in actions_without_formatter {
            assert_eq!(action.formatter(), None);
        }
    }

    #[test]
    fn can_return_type_tag() {
        assert_eq!(TrackerAction::Email(Default::default()).type_tag(), "email");
        assert_eq!(
            TrackerAction::Webhook(WebhookAction {
                url: "https://retrack.dev".parse().unwrap(),
                method: None,
                headers: None,
                formatter: None,
            })
            .type_tag(),
            "webhook"
        );
        assert_eq!(
            TrackerAction::ServerLog(Default::default()).type_tag(),
            "log"
        );
    }
}
