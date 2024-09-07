mod email;
mod email_attachment;
mod email_attachment_disposition;
mod email_content;
mod email_template;

use serde::{Deserialize, Serialize};

pub use self::{
    email::Email, email_attachment::EmailAttachment,
    email_attachment_disposition::EmailAttachmentDisposition, email_content::EmailContent,
    email_template::EmailTemplate,
};

/// Describes the email task type.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct EmailTaskType {
    /// The email addresses to send the email to.
    pub to: Vec<String>,
    /// The content of the email.
    pub content: EmailContent,
}

#[cfg(test)]
mod tests {
    use super::EmailContent;
    use crate::tasks::{Email, EmailTaskType};

    #[test]
    fn serialization() -> anyhow::Result<()> {
        assert_eq!(
            postcard::to_stdvec(&EmailTaskType {
                to: vec!["one@retrack.dev".to_string(), "two@retrack.dev".to_string()],
                content: EmailContent::Custom(Email::text("subject", "text")),
            })?,
            vec![
                2, 15, 111, 110, 101, 64, 114, 101, 116, 114, 97, 99, 107, 46, 100, 101, 118, 15,
                116, 119, 111, 64, 114, 101, 116, 114, 97, 99, 107, 46, 100, 101, 118, 0, 7, 115,
                117, 98, 106, 101, 99, 116, 4, 116, 101, 120, 116, 0, 0
            ]
        );

        Ok(())
    }

    #[test]
    fn deserialization() -> anyhow::Result<()> {
        assert_eq!(
            postcard::from_bytes::<EmailTaskType>(&[
                2, 15, 111, 110, 101, 64, 114, 101, 116, 114, 97, 99, 107, 46, 100, 101, 118, 15,
                116, 119, 111, 64, 114, 101, 116, 114, 97, 99, 107, 46, 100, 101, 118, 0, 7, 115,
                117, 98, 106, 101, 99, 116, 4, 116, 101, 120, 116, 0, 0
            ])?,
            EmailTaskType {
                to: vec!["one@retrack.dev".to_string(), "two@retrack.dev".to_string()],
                content: EmailContent::Custom(Email::text("subject", "text")),
            }
        );

        Ok(())
    }
}
