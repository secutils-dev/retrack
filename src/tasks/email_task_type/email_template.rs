use crate::{
    api::Api,
    network::{DnsResolver, EmailTransport},
    tasks::{Email, EmailAttachment},
};
use serde::{Deserialize, Serialize};
use serde_json::json;

pub const RETRACK_LOGO_BYTES: &[u8] =
    include_bytes!("../../../assets/logo/retrack-logo-with-text.png");

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum EmailTemplate {
    TrackerChanges {
        tracker_name: String,
        content: Result<String, String>,
    },
}

impl EmailTemplate {
    /// Compiles tracker content changes template as an email.
    pub async fn compile_to_email<DR: DnsResolver, ET: EmailTransport>(
        &self,
        api: &Api<DR, ET>,
    ) -> anyhow::Result<Email> {
        match self {
            Self::TrackerChanges {
                tracker_name,
                content,
            } => Self::tracker_changes(api, tracker_name, content).await,
        }
    }
    /// Compiles tracker content changes template as an email.
    async fn tracker_changes<DR: DnsResolver, ET: EmailTransport>(
        api: &Api<DR, ET>,
        tracker_name: &str,
        content: &Result<String, String>,
    ) -> anyhow::Result<Email> {
        let back_link = format!("{}ws/web_scraping__content", api.config.public_url);
        let (subject, text, html) = match content {
            Ok(content) => (
                format!("[Retrack] Change detected: \"{}\"", tracker_name),
                format!(
                    "\"{}\" tracker detected content changes. Visit {} to learn more.",
                    tracker_name, back_link
                ),
                api.templates.render(
                    "tracker_changes_email",
                    &json!({
                    "tracker_name": tracker_name,
                    "content": content,
                    "back_link": back_link,
                    "home_link": api.config.public_url.as_str(),
                }),
                )?,
            ),
            Err(error_message) => (
                format!("[Retrack] Check failed: \"{}\"", tracker_name),
                format!(
                    "\"{}\" tracker failed to check for content changes due to the following error: {error_message}. Visit {} to learn more.",
                    tracker_name, back_link
                ),
                api.templates.render(
                    "tracker_changes_error_email",
                    &json!({
                "tracker_name": tracker_name,
                "error_message": error_message,
                "back_link": back_link,
                "home_link": api.config.public_url.as_str(),
            }),
                )?
            )
        };

        Ok(Email::html_with_attachments(
            subject,
            text,
            html,
            vec![EmailAttachment::inline(
                "retrack-logo",
                "image/png",
                RETRACK_LOGO_BYTES.to_vec(),
            )],
        ))
    }
}

#[cfg(test)]
mod tests {
    use crate::{tasks::EmailTemplate, tests::mock_api};
    use insta::assert_debug_snapshot;
    use itertools::Itertools;
    use sqlx::PgPool;

    #[sqlx::test]
    async fn can_compile_tracker_changes_template_to_email(pool: PgPool) -> anyhow::Result<()> {
        let api = mock_api(pool).await?;

        let mut template = EmailTemplate::TrackerChanges {
            tracker_name: "tracker".to_string(),
            content: Ok("content".to_string()),
        }
        .compile_to_email(&api)
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

    #[sqlx::test]
    async fn can_compile_tracker_changes_error_template_to_email(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        let api = mock_api(pool).await?;

        let mut template = EmailTemplate::TrackerChanges {
            tracker_name: "tracker".to_string(),
            content: Err("Something went wrong".to_string()),
        }
        .compile_to_email(&api)
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
            subject: "[Retrack] Check failed: \"tracker\"",
            text: "\"tracker\" tracker failed to check for content changes due to the following error: Something went wrong. Visit http://localhost:1234/ws/web_scraping__content to learn more.",
            html: Some(
                "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n    <title>\"tracker\" tracker failed to check for changes</title>\n    <meta charset=\"utf-8\">\n    <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n    <style>\n        body {\n            font-family: Arial, sans-serif;\n            background-color: #f1f1f1;\n            margin: 0;\n            padding: 0;\n        }\n    \n        .container {\n            max-width: 600px;\n            margin: 0 auto;\n            background-color: #fff;\n            padding: 20px;\n            border-radius: 5px;\n            box-shadow: 0 0 10px rgba(0, 0, 0, 0.1);\n        }\n    \n        h1 {\n            font-size: 24px;\n            margin-top: 0;\n        }\n    \n        p {\n            font-size: 16px;\n            line-height: 1.5;\n            margin-bottom: 20px;\n        }\n    \n        .navigate-link {\n            display: block;\n            width: 250px;\n            margin: auto;\n            padding: 10px 20px;\n            text-align: center;\n            text-decoration: none;\n            color: #5e1d3f;\n            background-color: #fed047;\n            border-radius: 5px;\n            font-weight: bold;\n        }\n    </style>\n</head>\n<body>\n<div class=\"container\">\n    <h1>\"tracker\" tracker failed to check for changes</h1>\n    <p>There was an error while checking content: <b>Something went wrong</b>.</p>\n    <p>To check the tracker configuration and re-try, visit the <b>Content trackers</b> page:</p>\n    <a class=\"navigate-link\" href=\"http://localhost:1234/ws/web_scraping__content\">Web Scraping → Content trackers</a>\n    <p>If the button above doesn't work, you can navigate to the following URL directly: </p>\n    <p>http://localhost:1234/ws/web_scraping__content</p>\n    <a href=\"http://localhost:1234/\"><img src=\"cid:retrack-logo\" alt=\"Retrack logo\" width=\"64\" height=\"16\"/></a>\n</div>\n</body>\n</html>\n",
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
