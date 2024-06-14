use serde::{Deserialize, Serialize};

/// Defines a notification destination.
#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
pub enum NotificationDestination {
    /// Notification will be sent to the specified email.
    Email(String),
    /// Notification will be logged in the server log.
    ServerLog,
}

#[cfg(test)]
mod tests {
    use super::NotificationDestination;

    #[test]
    fn serialization() -> anyhow::Result<()> {
        assert_eq!(
            postcard::to_stdvec(&NotificationDestination::Email(
                "dev@retrack.dev".to_string()
            ))?,
            vec![0, 15, 100, 101, 118, 64, 114, 101, 116, 114, 97, 99, 107, 46, 100, 101, 118]
        );
        assert_eq!(
            postcard::to_stdvec(&NotificationDestination::ServerLog)?,
            vec![1]
        );
        Ok(())
    }

    #[test]
    fn deserialization() -> anyhow::Result<()> {
        assert_eq!(
            postcard::from_bytes::<NotificationDestination>(&[
                0, 15, 100, 101, 118, 64, 114, 101, 116, 114, 97, 99, 107, 46, 100, 101, 118
            ])?,
            NotificationDestination::Email("dev@retrack.dev".to_string())
        );
        assert_eq!(
            postcard::from_bytes::<NotificationDestination>(&[1])?,
            NotificationDestination::ServerLog
        );
        Ok(())
    }
}
