use crate::trackers::{ConfiguratorScriptRequest, TargetResponse};
use serde::Deserialize;

/// Result of the "configurator" script execution.
#[derive(Deserialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ConfiguratorScriptResult {
    /// Configurator script modifications for the request.
    Requests(Vec<ConfiguratorScriptRequest>),
    /// Configurator script modifications for the response. If responses are provided, the actual
    /// requests aren't sent and the responses are returned immediately.
    Responses(Vec<TargetResponse>),
}

#[cfg(test)]
mod tests {
    use crate::trackers::{ConfiguratorScriptRequest, ConfiguratorScriptResult, TargetResponse};
    use http::{HeaderValue, StatusCode, header::CONTENT_TYPE};
    use insta::assert_debug_snapshot;

    #[test]
    fn deserialization() -> anyhow::Result<()> {
        assert_eq!(
            serde_json::from_str::<ConfiguratorScriptResult>(
                r#"
{
    "requests": [{
        "url": "https://retrack.dev",
        "body": [1, 2 ,3],
        "headers": {
            "Content-Type": "text/plain"
        },
        "acceptStatuses": [200]
    }]
}
          "#
            )?,
            ConfiguratorScriptResult::Requests(vec![ConfiguratorScriptRequest {
                url: "https://retrack.dev".parse()?,
                method: None,
                headers: Some(
                    vec![(CONTENT_TYPE, HeaderValue::from_static("text/plain"))]
                        .into_iter()
                        .collect()
                ),
                media_type: None,
                body: Some(vec![1, 2, 3]),
                accept_statuses: Some([StatusCode::OK].into_iter().collect()),
                accept_invalid_certificates: None,
            }])
        );

        assert_eq!(
            serde_json::from_str::<ConfiguratorScriptResult>(
                r#"
{
    "requests": [{
        "url": "https://retrack.dev",
        "body": [1, 2 ,3],
        "headers": {
            "Content-Type": "text/plain"
        },
        "acceptStatuses": [200],
        "acceptInvalidCertificates": true
    }]
}
          "#
            )?,
            ConfiguratorScriptResult::Requests(vec![ConfiguratorScriptRequest {
                url: "https://retrack.dev".parse()?,
                method: None,
                headers: Some(
                    vec![(CONTENT_TYPE, HeaderValue::from_static("text/plain"))]
                        .into_iter()
                        .collect()
                ),
                media_type: None,
                body: Some(vec![1, 2, 3]),
                accept_statuses: Some([StatusCode::OK].into_iter().collect()),
                accept_invalid_certificates: Some(true),
            }])
        );

        assert_eq!(
            serde_json::from_str::<ConfiguratorScriptResult>(
                r#"
{
    "responses": [{
        "status": 200,
        "headers": {
            "Content-Type": "text/plain"
        },
        "body": [1, 2 ,3]
    }]
}
          "#
            )?,
            ConfiguratorScriptResult::Responses(vec![TargetResponse {
                status: StatusCode::OK,
                headers: vec![(CONTENT_TYPE, HeaderValue::from_static("text/plain"))]
                    .into_iter()
                    .collect(),
                body: vec![1, 2, 3],
            }])
        );

        assert_eq!(
            serde_json::from_str::<ConfiguratorScriptResult>(r#"{ "requests": [] }"#)?,
            ConfiguratorScriptResult::Requests(vec![])
        );

        assert_debug_snapshot!(serde_json::from_str::<ConfiguratorScriptResult>(
                r#"
{
    "requests": [{
        "url": "https://retrack.dev",
        "headers": {
            "Content-Type": "text/plain"
        }
    }],
    "responses": [{
        "status": 200,
        "headers": {
            "Content-Type": "text/plain"
        },
        "body": [1, 2 ,3]
    }]
}
          "#
            ).unwrap_err(), @r###"Error("expected value", line: 8, column: 6)"###);

        Ok(())
    }
}
