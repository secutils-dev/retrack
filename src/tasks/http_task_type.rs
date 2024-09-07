use http::{HeaderMap, Method};
use serde::{Deserialize, Serialize};
use url::Url;

/// Describes the HTTP task type.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct HttpTaskType {
    /// The URI to send the request to (must be absolute).
    pub url: Url,
    /// The HTTP method to use to send request to.
    #[serde(with = "http_serde::method")]
    pub method: Method,
    /// Optional headers to include in the request.
    #[serde(with = "http_serde::option::header_map")]
    pub headers: Option<HeaderMap>,
    /// Optional body to include in the request.
    pub body: Option<Vec<u8>>,
}

#[cfg(test)]
mod tests {
    use crate::tasks::HttpTaskType;
    use http::{header, HeaderMap, HeaderValue, Method};

    #[test]
    fn serialization() -> anyhow::Result<()> {
        assert_eq!(
            postcard::to_stdvec(&HttpTaskType {
                method: Method::PUT,
                url: "https://retrack.dev/some-path".parse()?,
                headers: Some(HeaderMap::from_iter([(
                    header::CONTENT_TYPE,
                    HeaderValue::from_static("text/plain")
                )])),
                body: Some(vec![1, 2, 3]),
            })?,
            vec![
                29, 104, 116, 116, 112, 115, 58, 47, 47, 114, 101, 116, 114, 97, 99, 107, 46, 100,
                101, 118, 47, 115, 111, 109, 101, 45, 112, 97, 116, 104, 3, 80, 85, 84, 1, 1, 12,
                99, 111, 110, 116, 101, 110, 116, 45, 116, 121, 112, 101, 1, 10, 116, 101, 120,
                116, 47, 112, 108, 97, 105, 110, 1, 3, 1, 2, 3
            ]
        );

        Ok(())
    }

    #[test]
    fn deserialization() -> anyhow::Result<()> {
        assert_eq!(
            postcard::from_bytes::<HttpTaskType>(&[
                29, 104, 116, 116, 112, 115, 58, 47, 47, 114, 101, 116, 114, 97, 99, 107, 46, 100,
                101, 118, 47, 115, 111, 109, 101, 45, 112, 97, 116, 104, 3, 80, 85, 84, 1, 1, 12,
                99, 111, 110, 116, 101, 110, 116, 45, 116, 121, 112, 101, 1, 10, 116, 101, 120,
                116, 47, 112, 108, 97, 105, 110, 1, 3, 1, 2, 3
            ])?,
            HttpTaskType {
                method: Method::PUT,
                url: "https://retrack.dev/some-path".parse()?,
                headers: Some(HeaderMap::from_iter([(
                    header::CONTENT_TYPE,
                    HeaderValue::from_static("text/plain")
                )])),
                body: Some(vec![1, 2, 3]),
            }
        );

        Ok(())
    }
}
