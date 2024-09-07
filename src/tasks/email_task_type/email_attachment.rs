use crate::tasks::EmailAttachmentDisposition;
use serde::{Deserialize, Serialize};

/// Describes the content of the email attachment.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct EmailAttachment {
    /// Email attachment disposition.
    pub disposition: EmailAttachmentDisposition,
    /// Email attachment content type (e.g. image/png).
    pub content_type: String,
    /// Email attachment content.
    pub content: Vec<u8>,
}

impl EmailAttachment {
    /// Create an inline HTML email attachment.
    pub fn inline<I: Into<String>, T: Into<String>, C: Into<Vec<u8>>>(
        id: I,
        content_type: T,
        content: C,
    ) -> Self {
        Self {
            disposition: EmailAttachmentDisposition::Inline(id.into()),
            content_type: content_type.into(),
            content: content.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::EmailAttachment;
    use crate::tasks::EmailAttachmentDisposition;

    #[test]
    fn serialization() -> anyhow::Result<()> {
        assert_eq!(
            postcard::to_stdvec(&EmailAttachment::inline(
                "my-id",
                "text/plain",
                vec![1, 2, 3]
            ))?,
            vec![
                0, 5, 109, 121, 45, 105, 100, 10, 116, 101, 120, 116, 47, 112, 108, 97, 105, 110,
                3, 1, 2, 3
            ]
        );

        Ok(())
    }

    #[test]
    fn deserialization() -> anyhow::Result<()> {
        assert_eq!(
            postcard::from_bytes::<EmailAttachment>(&[
                0, 5, 109, 121, 45, 105, 100, 10, 116, 101, 120, 116, 47, 112, 108, 97, 105, 110,
                3, 1, 2, 3
            ])?,
            EmailAttachment::inline("my-id", "text/plain", vec![1, 2, 3])
        );

        Ok(())
    }

    #[test]
    fn create_inline() -> anyhow::Result<()> {
        assert_eq!(
            EmailAttachment::inline("my-id", "text/plain", vec![1, 2, 3]),
            EmailAttachment {
                disposition: EmailAttachmentDisposition::Inline("my-id".to_string()),
                content_type: "text/plain".to_string(),
                content: vec![1, 2, 3],
            }
        );

        Ok(())
    }
}
