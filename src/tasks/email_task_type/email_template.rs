use crate::{
    api::Api,
    network::DnsResolver,
    tasks::{Email, EmailAttachment},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

pub const RETRACK_LOGO_BYTES: &[u8] =
    include_bytes!("../../../assets/logo/retrack-logo-with-text.png");

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum EmailTemplate {
    TrackerCheckResult {
        tracker_id: Uuid,
        tracker_name: String,
        result: Result<String, String>,
    },
    TaskFailed {
        task_id: Uuid,
        task_type: String,
        task_tags: String,
        error_message: String,
    },
}

impl EmailTemplate {
    /// Compiles tracker check result template as an email.
    pub async fn compile_to_email<DR: DnsResolver>(&self, api: &Api<DR>) -> anyhow::Result<Email> {
        match self {
            Self::TrackerCheckResult {
                tracker_id,
                tracker_name,
                result: content,
            } => Self::tracker_check_result(api, *tracker_id, tracker_name, content).await,
            Self::TaskFailed {
                task_id,
                task_type,
                task_tags,
                error_message,
            } => Self::task_failed(api, *task_id, task_type, task_tags, error_message).await,
        }
    }

    /// Compiles tracker check result template as an email.
    async fn tracker_check_result<DR: DnsResolver>(
        api: &Api<DR>,
        tracker_id: Uuid,
        tracker_name: &str,
        content: &Result<String, String>,
    ) -> anyhow::Result<Email> {
        let (subject, text, html) = match content {
            Ok(content) => (
                format!("[Retrack] Change detected: \"{tracker_name}\""),
                format!("\"{tracker_name}\" tracker detected content changes."),
                api.templates.render(
                    "tracker_check_result_success_email",
                    &json!({
                        "tracker_id": tracker_id,
                        "tracker_name": tracker_name,
                        "content": content,
                        "home_link": api.config.public_url.as_str(),
                    }),
                )?,
            ),
            Err(error_message) => (
                format!("[Retrack] Check failed: \"{tracker_name}\""),
                format!(
                    "\"{tracker_name}\" tracker failed to check for content changes due to the following error: {error_message}.",
                ),
                api.templates.render(
                    "tracker_check_result_failure_email",
                    &json!({
                        "tracker_id": tracker_id,
                        "tracker_name": tracker_name,
                        "error_message": error_message,
                        "home_link": api.config.public_url.as_str(),
                    }),
                )?,
            ),
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

    /// Compiles failed task template as an email.
    async fn task_failed<DR: DnsResolver>(
        api: &Api<DR>,
        task_id: Uuid,
        task_type: &str,
        task_tags: &str,
        error_message: &str,
    ) -> anyhow::Result<Email> {
        let task_type = task_type.to_uppercase();
        Ok(Email::html_with_attachments(
            format!("[Retrack] {task_type} task failed"),
            format!(
                "{task_type} task failed to execute due to the following error: {error_message}.",
            ),
            api.templates.render(
                "task_failure_email",
                &json!({
                    "task_id": task_id,
                    "task_type": task_type,
                    "task_tags": task_tags,
                    "error_message": error_message,
                    "home_link": api.config.public_url.as_str(),
                }),
            )?,
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
    use uuid::uuid;

    #[sqlx::test]
    async fn can_compile_tracker_changes_template_to_email(pool: PgPool) -> anyhow::Result<()> {
        let api = mock_api(pool).await?;

        let mut template = EmailTemplate::TrackerCheckResult {
            tracker_id: uuid!("00000000-0000-0000-0000-000000000001"),
            tracker_name: "tracker".to_string(),
            result: Ok("content".to_string()),
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
            text: "\"tracker\" tracker detected content changes.",
            html: Some(
                "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n  <title>\"tracker\" tracker detected changes</title>\n  <meta charset=\"utf-8\">\n  <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n  <style>\n      body {\n          font-family: Arial, sans-serif;\n          background-color: #f1f1f1;\n          margin: 0;\n          padding: 0;\n      }\n  \n      .container {\n          max-width: 600px;\n          margin: 0 auto;\n          background-color: #fff;\n          padding: 20px;\n          border-radius: 5px;\n          box-shadow: 0 0 10px rgba(0, 0, 0, 0.1);\n      }\n  \n      h1 {\n          font-size: 24px;\n          margin-top: 0;\n      }\n  \n      p {\n          font-size: 16px;\n          line-height: 1.5;\n          margin-bottom: 20px;\n      }\n  \n      .navigate-link {\n          display: block;\n          width: 250px;\n          margin: auto;\n          padding: 10px 20px;\n          text-align: center;\n          text-decoration: none;\n          color: #5e1d3f;\n          background-color: #fed047;\n          border-radius: 5px;\n          font-weight: bold;\n      }\n  \n    hr {\n      border: 1px dashed #e5e7eb;\n    }\n  </style>\n</head>\n<body>\n<div class=\"container\">\n  <p>Retrack tracker detected the change:</p>\n  <hr />\n  <p><b>Tracker:</b> tracker <small><i>00000000-0000-0000-0000-000000000001</i></small></p>\n  <hr />\n  content\n  <hr />\n  <a href=\"http://localhost:1234/\"><img src=\"cid:retrack-logo\" alt=\"Retrack logo\" width=\"64\" height=\"16\"/></a>\n</div>\n</body>\n</html>\n",
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

        let mut template = EmailTemplate::TrackerCheckResult {
            tracker_id: uuid!("00000000-0000-0000-0000-000000000001"),
            tracker_name: "tracker".to_string(),
            result: Err("Something went wrong".to_string()),
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
            text: "\"tracker\" tracker failed to check for content changes due to the following error: Something went wrong.",
            html: Some(
                "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n  <title>\"tracker\" tracker failed to check for changes</title>\n  <meta charset=\"utf-8\">\n  <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n  <style>\n      body {\n          font-family: Arial, sans-serif;\n          background-color: #f1f1f1;\n          margin: 0;\n          padding: 0;\n      }\n  \n      .container {\n          max-width: 600px;\n          margin: 0 auto;\n          background-color: #fff;\n          padding: 20px;\n          border-radius: 5px;\n          box-shadow: 0 0 10px rgba(0, 0, 0, 0.1);\n      }\n  \n      h1 {\n          font-size: 24px;\n          margin-top: 0;\n      }\n  \n      p {\n          font-size: 16px;\n          line-height: 1.5;\n          margin-bottom: 20px;\n      }\n  \n      .navigate-link {\n          display: block;\n          width: 250px;\n          margin: auto;\n          padding: 10px 20px;\n          text-align: center;\n          text-decoration: none;\n          color: #5e1d3f;\n          background-color: #fed047;\n          border-radius: 5px;\n          font-weight: bold;\n      }\n  \n    hr {\n      border: 1px dashed #e5e7eb;\n    }\n  </style>\n</head>\n<body>\n<div class=\"container\">\n  <p>There was an error while running Retrack tracker:</p>\n  <hr />\n  <p><b>Tracker:</b> tracker <small><i>00000000-0000-0000-0000-000000000001</i></small></p>\n  <hr />\n  <p><b>Something went wrong</b></p>\n  <hr />\n  <p>Please check tracker configuration and make sure tracker target is available.</p>\n  <a href=\"http://localhost:1234/\"><img src=\"cid:retrack-logo\" alt=\"Retrack logo\" width=\"64\" height=\"16\"/></a>\n</div>\n</body>\n</html>\n",
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
    async fn can_compile_task_failed_template_to_email(pool: PgPool) -> anyhow::Result<()> {
        let api = mock_api(pool).await?;

        let mut template = EmailTemplate::TaskFailed {
            task_id: uuid!("00000000-0000-0000-0000-000000000001"),
            task_type: "http".to_string(),
            task_tags: "tag1, tag2".to_string(),
            error_message: "Something went wrong".to_string(),
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
            subject: "[Retrack] HTTP task failed",
            text: "HTTP task failed to execute due to the following error: Something went wrong.",
            html: Some(
                "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n  <title>HTTP task failed</title>\n  <meta charset=\"utf-8\">\n  <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n  <style>\n      body {\n          font-family: Arial, sans-serif;\n          background-color: #f1f1f1;\n          margin: 0;\n          padding: 0;\n      }\n  \n      .container {\n          max-width: 600px;\n          margin: 0 auto;\n          background-color: #fff;\n          padding: 20px;\n          border-radius: 5px;\n          box-shadow: 0 0 10px rgba(0, 0, 0, 0.1);\n      }\n  \n      h1 {\n          font-size: 24px;\n          margin-top: 0;\n      }\n  \n      p {\n          font-size: 16px;\n          line-height: 1.5;\n          margin-bottom: 20px;\n      }\n  \n      .navigate-link {\n          display: block;\n          width: 250px;\n          margin: auto;\n          padding: 10px 20px;\n          text-align: center;\n          text-decoration: none;\n          color: #5e1d3f;\n          background-color: #fed047;\n          border-radius: 5px;\n          font-weight: bold;\n      }\n  \n    hr {\n      border: 1px dashed #e5e7eb;\n    }\n  </style>\n</head>\n<body>\n<div class=\"container\">\n  <p>There was an error while running Retrack task:</p>\n  <hr />\n  <p><b>Task (HTTP):</b> 00000000-0000-0000-0000-000000000001 <small><i>tag1, tag2</i></small></p>\n  <hr />\n  <p><b>Something went wrong</b></p>\n  <hr />\n  <p>Please check tracker action configuration that triggered the task.</p>\n  <a href=\"http://localhost:1234/\"><img src=\"cid:retrack-logo\" alt=\"Retrack logo\" width=\"64\" height=\"16\"/></a>\n</div>\n</body>\n</html>\n",
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
