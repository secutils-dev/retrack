use crate::{
    api::Api,
    network::{DnsResolver, EmailTransport},
    tasks::{Email, EmailTemplate},
};
use serde::{Deserialize, Serialize};

/// Describes the content of an email.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum EmailContent {
    /// Email content is represented as a custom email.
    Custom(Email),
    /// Email content is represented as a template.
    Template(EmailTemplate),
}

impl EmailContent {
    /// Consumes email content and returns its email representation if supported.
    pub async fn into_email<DR: DnsResolver, ET: EmailTransport>(
        self,
        api: &Api<DR, ET>,
    ) -> anyhow::Result<Email> {
        Ok(match self {
            EmailContent::Custom(email) => email,
            EmailContent::Template(template) => template.compile_to_email(api).await?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::EmailContent;
    use crate::{
        tasks::{Email, EmailAttachment, EmailTemplate},
        tests::mock_api,
    };
    use insta::assert_debug_snapshot;
    use itertools::Itertools;
    use sqlx::PgPool;

    #[test]
    fn serialization() -> anyhow::Result<()> {
        assert_eq!(
            postcard::to_stdvec(&EmailContent::Custom(Email::text("abc", "def")))?,
            vec![0, 3, 97, 98, 99, 3, 100, 101, 102, 0, 0]
        );

        assert_eq!(
            postcard::to_stdvec(&EmailContent::Template(EmailTemplate::TrackerChanges {
                tracker_name: "tracker_name_1".to_string(),
                content: Ok("email content".to_string())
            }))?,
            vec![
                1, 0, 14, 116, 114, 97, 99, 107, 101, 114, 95, 110, 97, 109, 101, 95, 49, 0, 13,
                101, 109, 97, 105, 108, 32, 99, 111, 110, 116, 101, 110, 116
            ]
        );
        Ok(())
    }

    #[test]
    fn deserialization() -> anyhow::Result<()> {
        assert_eq!(
            postcard::from_bytes::<EmailContent>(&[0, 3, 97, 98, 99, 3, 100, 101, 102, 0, 0])?,
            EmailContent::Custom(Email::text("abc", "def"))
        );

        assert_eq!(
            postcard::from_bytes::<EmailContent>(&[
                1, 0, 14, 116, 114, 97, 99, 107, 101, 114, 95, 110, 97, 109, 101, 95, 49, 0, 13,
                101, 109, 97, 105, 108, 32, 99, 111, 110, 116, 101, 110, 116
            ])?,
            EmailContent::Template(EmailTemplate::TrackerChanges {
                tracker_name: "tracker_name_1".to_string(),
                content: Ok("email content".to_string())
            })
        );

        Ok(())
    }

    #[sqlx::test]
    async fn convert_email_content_to_email(pool: PgPool) -> anyhow::Result<()> {
        let api = mock_api(pool).await?;

        assert_eq!(
            EmailContent::Custom(Email::text("subject", "text"))
                .into_email(&api)
                .await?,
            Email {
                subject: "subject".to_string(),
                text: "text".to_string(),
                html: None,
                attachments: None,
            }
        );

        assert_eq!(
            EmailContent::Custom(Email::html("subject", "text", "html"))
                .into_email(&api)
                .await?,
            Email {
                subject: "subject".to_string(),
                text: "text".to_string(),
                html: Some("html".to_string()),
                attachments: None,
            }
        );

        assert_eq!(
            EmailContent::Custom(Email::html_with_attachments(
                "subject",
                "text",
                "html",
                vec![EmailAttachment::inline("cid", "text/plain", vec![1, 2, 3])]
            ))
            .into_email(&api)
            .await?,
            Email {
                subject: "subject".to_string(),
                text: "text".to_string(),
                html: Some("html".to_string()),
                attachments: Some(vec![EmailAttachment::inline(
                    "cid",
                    "text/plain",
                    vec![1, 2, 3]
                )]),
            }
        );

        Ok(())
    }

    #[sqlx::test]
    async fn convert_template_content_to_email(pool: PgPool) -> anyhow::Result<()> {
        let api = mock_api(pool).await?;
        let mut template = EmailContent::Template(EmailTemplate::TrackerChanges {
            tracker_name: "tracker".to_string(),
            content: Ok("content".to_string()),
        })
        .into_email(&api)
        .await?;
        template
            .attachments
            .as_mut()
            .unwrap()
            .iter_mut()
            .for_each(|a| {
                a.content = a.content.len().to_be_bytes().iter().cloned().collect_vec();
            });

        assert_debug_snapshot!(template, @r###"
        Email {
            subject: "[Retrack] Change detected: \"tracker\"",
            text: "\"tracker\" tracker detected content changes. Visit http://localhost:1234/ws/web_scraping__content to learn more.",
            html: Some(
                "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n    <title>\"tracker\" tracker detected changes</title>\n    <meta charset=\"utf-8\">\n    <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n    <style>\n        body {\n            font-family: Arial, sans-serif;\n            background-color: #f1f1f1;\n            margin: 0;\n            padding: 0;\n        }\n    \n        .container {\n            max-width: 600px;\n            margin: 0 auto;\n            background-color: #fff;\n            padding: 20px;\n            border-radius: 5px;\n            box-shadow: 0 0 10px rgba(0, 0, 0, 0.1);\n        }\n    \n        h1 {\n            font-size: 24px;\n            margin-top: 0;\n        }\n    \n        p {\n            font-size: 16px;\n            line-height: 1.5;\n            margin-bottom: 20px;\n        }\n    \n        .navigate-link {\n            display: block;\n            width: 250px;\n            margin: auto;\n            padding: 10px 20px;\n            text-align: center;\n            text-decoration: none;\n            color: #5e1d3f;\n            background-color: #fed047;\n            border-radius: 5px;\n            font-weight: bold;\n        }\n    </style>\n</head>\n<body>\n<div class=\"container\">\n    <h1>\"tracker\" tracker detected changes</h1>\n    <p>Current content: content</p>\n    <p>To learn more, visit the <b>Content trackers</b> page:</p>\n    <a class=\"navigate-link\" href=\"http://localhost:1234/ws/web_scraping__content\">Web Scraping → Content trackers</a>\n    <p>If the button above doesn't work, you can navigate to the following URL directly: </p>\n    <p>http://localhost:1234/ws/web_scraping__content</p>\n    <a href=\"http://localhost:1234/\"><img src=\"cid:retrack-logo\" alt=\"Retrack logo\" width=\"64\" height=\"16\"/></a>\n</div>\n</body>\n</html>\n",
            ),
            attachments: Some(
                [
                    EmailAttachment {
                        disposition: Inline(
                            "retrack-logo",
                        ),
                        content_type: "image/png",
                        content: [
                            0,
                            0,
                            0,
                            0,
                            0,
                            0,
                            20,
                            157,
                        ],
                    },
                ],
            ),
        }
        "###
        );

        Ok(())
    }
}
