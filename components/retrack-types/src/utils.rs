use byte_unit::Byte;
use http::StatusCode;
use http_serde::status_code;
use serde::{Deserialize, Deserializer, Serializer};
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

pub fn serialize_opt_byte_as_u64<S: Serializer>(
    value: &Option<Byte>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match value {
        Some(b) => serializer.serialize_u64(b.as_u64()),
        None => serializer.serialize_none(),
    }
}

pub fn deserialize_opt_byte_from_u64<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Byte>, D::Error> {
    Option::<u64>::deserialize(deserializer).map(|opt| opt.map(Byte::from_u64))
}
