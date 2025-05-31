use http::StatusCode;
use http_serde::status_code;
use serde::{Deserializer, Serializer};
use serde_with::{DeserializeAs, SerializeAs};

/// Utility-wrapper around `StatusCode` to use with `serde_with` crate macros.
pub struct StatusCodeLocal;
impl SerializeAs<StatusCode> for StatusCodeLocal {
    fn serialize_as<S>(source: &StatusCode, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        status_code::serialize(source, serializer)
    }
}

impl<'de> DeserializeAs<'de, StatusCode> for StatusCodeLocal {
    fn deserialize_as<D>(deserializer: D) -> Result<StatusCode, D::Error>
    where
        D: Deserializer<'de>,
    {
        status_code::deserialize(deserializer)
    }
}
