use crate::trackers::{TrackerDataRevision, TrackerDataValue};
use serde_json::Value as JsonValue;
use std::collections::VecDeque;
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug, Eq, PartialEq, Clone)]
pub(super) struct RawTrackerDataRevision {
    pub id: Uuid,
    pub tracker_id: Uuid,
    pub data: Vec<u8>,
    pub created_at: OffsetDateTime,
}

impl TryFrom<RawTrackerDataRevision> for TrackerDataRevision {
    type Error = anyhow::Error;

    fn try_from(raw: RawTrackerDataRevision) -> Result<Self, Self::Error> {
        let mut original_and_mods = postcard::from_bytes::<Vec<String>>(&raw.data)?
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
        Ok(Self {
            id: item.id,
            tracker_id: item.tracker_id,
            data: postcard::to_stdvec(
                &(&item.data)
                    .into_iter()
                    .map(|value| value.to_string())
                    .collect::<Vec<_>>(),
            )?,
            created_at: item.created_at,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::RawTrackerDataRevision;
    use crate::trackers::{TrackerDataRevision, TrackerDataValue};
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
}
