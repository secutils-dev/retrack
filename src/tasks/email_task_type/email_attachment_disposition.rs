use serde::{Deserialize, Serialize};

/// Describes the disposition of an email content attachment with an arbitrary ID.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum EmailAttachmentDisposition {
    /// Email attachment should be inlined.
    Inline(String),
}

#[cfg(test)]
mod tests {
    use super::EmailAttachmentDisposition;

    #[test]
    fn serialization() -> anyhow::Result<()> {
        assert_eq!(
            postcard::to_stdvec(&EmailAttachmentDisposition::Inline("abc".to_string()))?,
            vec![0, 3, 97, 98, 99]
        );

        Ok(())
    }

    #[test]
    fn deserialization() -> anyhow::Result<()> {
        assert_eq!(
            postcard::from_bytes::<EmailAttachmentDisposition>(&[0, 3, 97, 98, 99])?,
            EmailAttachmentDisposition::Inline("abc".to_string())
        );

        Ok(())
    }
}
