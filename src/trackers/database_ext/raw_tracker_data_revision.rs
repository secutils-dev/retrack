use crate::trackers::TrackerDataRevision;
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
        Ok(Self {
            id: raw.id,
            tracker_id: raw.tracker_id,
            data: postcard::from_bytes(&raw.data)?,
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
            data: postcard::to_stdvec(&item.data)?,
            created_at: item.created_at,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::RawTrackerDataRevision;
    use crate::trackers::TrackerDataRevision;
    use time::OffsetDateTime;
    use uuid::uuid;

    #[test]
    fn can_convert_into_tracker_data_revision() -> anyhow::Result<()> {
        assert_eq!(
            TrackerDataRevision::try_from(RawTrackerDataRevision {
                id: uuid!("00000000-0000-0000-0000-000000000001"),
                tracker_id: uuid!("00000000-0000-0000-0000-000000000002"),
                data: vec![9, 115, 111, 109, 101, 45, 100, 97, 116, 97],
                // January 1, 2000 10:00:00
                created_at: OffsetDateTime::from_unix_timestamp(946720800)?,
            })?,
            TrackerDataRevision {
                id: uuid!("00000000-0000-0000-0000-000000000001"),
                tracker_id: uuid!("00000000-0000-0000-0000-000000000002"),
                data: "some-data".to_string(),
                created_at: OffsetDateTime::from_unix_timestamp(946720800)?,
            }
        );

        Ok(())
    }

    #[test]
    fn can_convert_into_raw_tracker_data_revision() -> anyhow::Result<()> {
        assert_eq!(
            RawTrackerDataRevision::try_from(&TrackerDataRevision {
                id: uuid!("00000000-0000-0000-0000-000000000001"),
                tracker_id: uuid!("00000000-0000-0000-0000-000000000002"),
                data: "some-data".to_string(),
                created_at: OffsetDateTime::from_unix_timestamp(946720800)?,
            })?,
            RawTrackerDataRevision {
                id: uuid!("00000000-0000-0000-0000-000000000001"),
                tracker_id: uuid!("00000000-0000-0000-0000-000000000002"),
                data: vec![9, 115, 111, 109, 101, 45, 100, 97, 116, 97],
                // January 1, 2000 10:00:00
                created_at: OffsetDateTime::from_unix_timestamp(946720800)?,
            }
        );

        Ok(())
    }
}
