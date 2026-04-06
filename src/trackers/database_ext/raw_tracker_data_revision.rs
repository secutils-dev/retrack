use flate2::{Compression, read::GzDecoder, write::GzEncoder};
use retrack_types::trackers::{TrackerDataRevision, TrackerDataValue};
use serde_json::Value as JsonValue;
use std::{
    collections::VecDeque,
    io::{Read, Write},
};
use time::OffsetDateTime;
use uuid::Uuid;

/// Minimum size (in bytes) of Postcard-encoded data before gzip compression is applied.
/// Values smaller than this typically don't benefit from compression.
const COMPRESSION_THRESHOLD: usize = 512;

/// Magic byte prefixed to gzip-compressed data blobs to distinguish them from legacy
/// uncompressed Postcard data. Uncompressed Postcard blobs never start with this byte
/// because the first Postcard byte is a varint-encoded Vec length (0x00 would mean an
/// empty vec, which is rejected on read anyway).
const GZIP_MAGIC: u8 = 0x00;

#[derive(Debug, Eq, PartialEq, Clone)]
pub(super) struct RawTrackerDataRevision {
    pub id: Uuid,
    pub tracker_id: Uuid,
    pub data: Vec<u8>,
    pub created_at: OffsetDateTime,
}

fn decompress_data(data: &[u8]) -> anyhow::Result<Vec<u8>> {
    if data.first() == Some(&GZIP_MAGIC) {
        let mut decoder = GzDecoder::new(&data[1..]);
        let mut decompressed = Vec::new();
        decoder.read_to_end(&mut decompressed)?;
        Ok(decompressed)
    } else {
        Ok(data.to_vec())
    }
}

fn compress_data(postcard_bytes: Vec<u8>) -> anyhow::Result<Vec<u8>> {
    if postcard_bytes.len() < COMPRESSION_THRESHOLD {
        return Ok(postcard_bytes);
    }

    let mut buf = Vec::with_capacity(postcard_bytes.len() / 2);
    buf.push(GZIP_MAGIC);
    let mut encoder = GzEncoder::new(&mut buf, Compression::default());
    encoder.write_all(&postcard_bytes)?;
    encoder.finish()?;

    if buf.len() < postcard_bytes.len() {
        Ok(buf)
    } else {
        Ok(postcard_bytes)
    }
}

impl TryFrom<RawTrackerDataRevision> for TrackerDataRevision {
    type Error = anyhow::Error;

    fn try_from(raw: RawTrackerDataRevision) -> Result<Self, Self::Error> {
        let postcard_bytes = decompress_data(&raw.data)?;
        let mut original_and_mods = postcard::from_bytes::<Vec<String>>(&postcard_bytes)?
            .into_iter()
            .map(|raw_value| serde_json::from_str(&raw_value))
            .collect::<Result<VecDeque<JsonValue>, _>>()?;

        let mut data = TrackerDataValue::new(original_and_mods.pop_front().ok_or_else(|| {
            anyhow::anyhow!("Tracker data revision must have at least one value.")
        })?);
        original_and_mods
            .into_iter()
            .for_each(|value| data.add_mod(value));

        Ok(Self {
            id: raw.id,
            tracker_id: raw.tracker_id,
            data,
            created_at: raw.created_at,
        })
    }
}

impl TryFrom<&TrackerDataRevision> for RawTrackerDataRevision {
    type Error = anyhow::Error;

    fn try_from(item: &TrackerDataRevision) -> Result<Self, Self::Error> {
        let postcard_bytes = postcard::to_stdvec(
            &(&item.data)
                .into_iter()
                .map(|value| value.to_string())
                .collect::<Vec<_>>(),
        )?;
        Ok(Self {
            id: item.id,
            tracker_id: item.tracker_id,
            data: compress_data(postcard_bytes)?,
            created_at: item.created_at,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::RawTrackerDataRevision;
    use retrack_types::trackers::{TrackerDataRevision, TrackerDataValue};
    use serde_json::json;
    use time::OffsetDateTime;
    use uuid::uuid;

    #[test]
    fn can_convert_into_and_from_raw_tracker_data_revision() -> anyhow::Result<()> {
        let data_revision = TrackerDataRevision {
            id: uuid!("00000000-0000-0000-0000-000000000001"),
            tracker_id: uuid!("00000000-0000-0000-0000-000000000002"),
            data: TrackerDataValue::new(json!("some-data")),
            created_at: OffsetDateTime::from_unix_timestamp(946720800)?,
        };
        assert_eq!(
            TrackerDataRevision::try_from(RawTrackerDataRevision::try_from(&data_revision)?)?,
            data_revision
        );

        let mut data = TrackerDataValue::new(json!("some-data"));
        data.add_mod(json!("some-other-data"));
        data.add_mod(json!("some-other-other-data"));
        let data_revision = TrackerDataRevision {
            data,
            ..data_revision
        };
        assert_eq!(
            TrackerDataRevision::try_from(RawTrackerDataRevision::try_from(&data_revision)?)?,
            data_revision
        );

        Ok(())
    }

    #[test]
    fn small_data_is_not_compressed() -> anyhow::Result<()> {
        let data_revision = TrackerDataRevision {
            id: uuid!("00000000-0000-0000-0000-000000000001"),
            tracker_id: uuid!("00000000-0000-0000-0000-000000000002"),
            data: TrackerDataValue::new(json!("small")),
            created_at: OffsetDateTime::from_unix_timestamp(946720800)?,
        };

        let raw = RawTrackerDataRevision::try_from(&data_revision)?;
        assert_ne!(raw.data.first(), Some(&super::GZIP_MAGIC));
        assert_eq!(TrackerDataRevision::try_from(raw)?, data_revision);

        Ok(())
    }

    #[test]
    fn large_data_is_compressed_and_decompressed() -> anyhow::Result<()> {
        let large_json = json!({
            "content": "x".repeat(2000),
            "nested": { "array": (0..100).collect::<Vec<_>>() }
        });
        let data_revision = TrackerDataRevision {
            id: uuid!("00000000-0000-0000-0000-000000000001"),
            tracker_id: uuid!("00000000-0000-0000-0000-000000000002"),
            data: TrackerDataValue::new(large_json),
            created_at: OffsetDateTime::from_unix_timestamp(946720800)?,
        };

        let raw = RawTrackerDataRevision::try_from(&data_revision)?;
        assert_eq!(raw.data.first(), Some(&super::GZIP_MAGIC));

        let postcard_bytes = postcard::to_stdvec(
            &(&data_revision.data)
                .into_iter()
                .map(|v| v.to_string())
                .collect::<Vec<_>>(),
        )?;
        assert!(
            raw.data.len() < postcard_bytes.len(),
            "compressed ({}) should be smaller than uncompressed ({})",
            raw.data.len(),
            postcard_bytes.len()
        );

        assert_eq!(TrackerDataRevision::try_from(raw)?, data_revision);

        Ok(())
    }

    #[test]
    fn can_read_legacy_uncompressed_data() -> anyhow::Result<()> {
        let data_revision = TrackerDataRevision {
            id: uuid!("00000000-0000-0000-0000-000000000001"),
            tracker_id: uuid!("00000000-0000-0000-0000-000000000002"),
            data: TrackerDataValue::new(json!({"key": "value".repeat(200)})),
            created_at: OffsetDateTime::from_unix_timestamp(946720800)?,
        };

        let postcard_bytes = postcard::to_stdvec(
            &(&data_revision.data)
                .into_iter()
                .map(|v| v.to_string())
                .collect::<Vec<_>>(),
        )?;

        let raw = RawTrackerDataRevision {
            id: data_revision.id,
            tracker_id: data_revision.tracker_id,
            data: postcard_bytes,
            created_at: data_revision.created_at,
        };

        assert_eq!(TrackerDataRevision::try_from(raw)?, data_revision);

        Ok(())
    }
}
