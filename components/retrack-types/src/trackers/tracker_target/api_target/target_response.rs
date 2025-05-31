use http::{HeaderMap, StatusCode};
use serde::{Deserialize, Serialize};

/// Response structure for the API target.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TargetResponse {
    /// Status code returned from the API.
    #[serde(with = "http_serde::status_code")]
    pub status: StatusCode,

    /// Headers returned from the API.
    #[serde(with = "http_serde::header_map")]
    pub headers: HeaderMap,

    /// Body returned from the API (in bytes).
    #[serde(with = "serde_bytes")]
    pub body: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use crate::trackers::TargetResponse;
    use http::{
        StatusCode,
        header::{AUTHORIZATION, CONTENT_TYPE},
    };
    use serde_json::json;
    use std::collections::HashMap;

    #[test]
    fn can_serialize_and_deserialize() -> anyhow::Result<()> {
        let response = TargetResponse {
            status: StatusCode::OK,
            headers: (&[
                (CONTENT_TYPE, "application/json".to_string()),
                (AUTHORIZATION, "Bearer token".to_string()),
            ]
            .into_iter()
            .collect::<HashMap<_, _>>())
                .try_into()?,
            body: serde_json::to_vec(&json!({ "key": "value" }))?,
        };
        let request_json = json!({
            "status": 200,
            "headers": {
                "content-type": "application/json",
                "authorization": "Bearer token"
            },
            "body": [123, 34, 107, 101, 121, 34, 58, 34, 118, 97, 108, 117, 101, 34, 125]
        });
        assert_eq!(serde_json::to_value(&response)?, request_json);
        assert_eq!(
            serde_json::from_value::<TargetResponse>(request_json)?,
            response
        );

        Ok(())
    }
}
