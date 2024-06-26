use crate::notifications::Notification;
use time::OffsetDateTime;

#[derive(Debug, Eq, PartialEq, Clone)]
pub(super) struct RawNotification {
    pub id: i32,
    pub destination: Vec<u8>,
    pub content: Vec<u8>,
    pub scheduled_at: OffsetDateTime,
}

impl TryFrom<RawNotification> for Notification {
    type Error = anyhow::Error;

    fn try_from(raw_notification: RawNotification) -> Result<Self, Self::Error> {
        Ok(Notification {
            id: raw_notification.id.try_into()?,
            destination: postcard::from_bytes(&raw_notification.destination)?,
            content: postcard::from_bytes(&raw_notification.content)?,
            scheduled_at: raw_notification.scheduled_at,
        })
    }
}

impl TryFrom<&Notification> for RawNotification {
    type Error = anyhow::Error;

    fn try_from(notification: &Notification) -> Result<Self, Self::Error> {
        Ok(RawNotification {
            id: *notification.id,
            destination: postcard::to_stdvec(&notification.destination)?,
            content: postcard::to_stdvec(&notification.content)?,
            scheduled_at: notification.scheduled_at,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::RawNotification;
    use crate::notifications::{Notification, NotificationContent, NotificationDestination};
    use time::OffsetDateTime;

    #[test]
    fn can_convert_to_notification() -> anyhow::Result<()> {
        assert_eq!(
            Notification::try_from(RawNotification {
                id: 1,
                destination: vec![
                    0, 15, 100, 101, 118, 64, 114, 101, 116, 114, 97, 99, 107, 46, 100, 101, 118
                ],
                content: vec![0, 3, 97, 98, 99],
                scheduled_at: OffsetDateTime::from_unix_timestamp(946720800)?,
            })?,
            Notification {
                id: 1.try_into()?,
                destination: NotificationDestination::Email("dev@retrack.dev".to_string()),
                content: NotificationContent::Text("abc".to_string()),
                scheduled_at: OffsetDateTime::from_unix_timestamp(946720800)?,
            }
        );

        Ok(())
    }

    #[test]
    fn can_convert_to_raw_notification() -> anyhow::Result<()> {
        assert_eq!(
            RawNotification::try_from(&Notification {
                id: 1.try_into()?,
                destination: NotificationDestination::Email("dev@retrack.dev".to_string()),
                content: NotificationContent::Text("abc".to_string()),
                scheduled_at: OffsetDateTime::from_unix_timestamp(946720800)?,
            })?,
            RawNotification {
                id: 1,
                destination: vec![
                    0, 15, 100, 101, 118, 64, 114, 101, 116, 114, 97, 99, 107, 46, 100, 101, 118
                ],
                content: vec![0, 3, 97, 98, 99],
                scheduled_at: OffsetDateTime::from_unix_timestamp(946720800)?,
            }
        );

        Ok(())
    }
}
