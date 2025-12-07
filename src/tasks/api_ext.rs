use crate::{
    api::Api,
    config::TaskRetryStrategy,
    error::Error as RetrackError,
    network::DnsResolver,
    tasks::{
        EmailAttachmentDisposition, EmailContent, EmailTaskType, EmailTemplate, HttpTaskType, Task,
        TaskType,
    },
};
use anyhow::{Context, bail};
use futures::{StreamExt, pin_mut};
use lettre::{
    Message,
    message::{Attachment, MultiPart, SinglePart, header::ContentType},
};
use reqwest_middleware::ClientBuilder;
use reqwest_tracing::{SpanBackendWithUrl, TracingMiddleware};
use std::{cmp, ops::Add};
use time::OffsetDateTime;
use tracing::{debug, error, warn};
use uuid::Uuid;

/// Defines a maximum number of tasks that can be retrieved from the database at once.
const MAX_TASKS_PAGE_SIZE: usize = 100;

/// Describes the API to work with tasks.
pub struct TasksApi<'a, DR: DnsResolver> {
    api: &'a Api<DR>,
}

impl<'a, DR: DnsResolver> TasksApi<'a, DR> {
    /// Creates Tasks API.
    pub fn new(api: &'a Api<DR>) -> Self {
        Self { api }
    }

    /// Schedules a new task.
    pub async fn schedule_task(
        &self,
        task_type: TaskType,
        tags: Vec<String>,
        scheduled_at: OffsetDateTime,
    ) -> anyhow::Result<Task> {
        let task = Task {
            id: Uuid::now_v7(),
            task_type,
            tags,
            scheduled_at,
            retry_attempt: None,
        };

        self.api.db.insert_task(&task).await?;

        Ok(task)
    }

    /// Executes pending tasks limited by the `limit` parameter.
    pub async fn execute_pending_tasks(&self, limit: usize) -> anyhow::Result<usize> {
        let pending_tasks_ids = self.api.db.get_tasks_ids(
            OffsetDateTime::now_utc(),
            cmp::min(MAX_TASKS_PAGE_SIZE, limit),
        );
        pin_mut!(pending_tasks_ids);

        let mut executed_tasks = 0;
        let tasks = &self.api.db;
        while let Some(task_id) = pending_tasks_ids.next().await {
            let Some(task) = tasks.get_task(task_id?).await? else {
                continue;
            };

            let task_id = task.id;
            let task_type = task.task_type.type_tag();
            if let Err(err) = self.execute_task(&task).await {
                error!(task.id = %task_id, task.task_type = task_type, task.tags = ?task.tags, "Failed to execute task: {err:?}");

                // Check if there are still retries left.
                let retry_strategy = self.get_task_retry_strategy(&task.task_type);
                let retry_attempt = task.retry_attempt.unwrap_or_default();
                if retry_attempt >= retry_strategy.max_attempts() {
                    warn!(
                        task.id = %task_id, task.task_type = task_type, task.tags = ?task.tags,
                        "Retry limit reached ('{retry_attempt}') for a task."
                    );

                    // Remove the task and report the error.
                    self.report_error(&task, err).await;
                    tasks.remove_task(task_id).await?;
                } else {
                    let retry_in = retry_strategy.interval(retry_attempt + 1);
                    let next_at = OffsetDateTime::now_utc().add(retry_in);
                    warn!(
                        task.id = %task_id, task.task_type = task_type, task.tags = ?task.tags,
                        metrics.task_retries = retry_attempt + 1,
                        "Scheduled a task retry in {} ({next_at}).",
                        humantime::format_duration(retry_in)
                    );

                    // Re-schedule the task to retry.
                    tasks
                        .update_task(&Task {
                            retry_attempt: Some(retry_attempt + 1),
                            scheduled_at: next_at,
                            ..task
                        })
                        .await?;
                }
            } else {
                debug!(task.id = %task_id, task.task_type = task_type, task.tags = ?task.tags, "Successfully executed task.");
                tasks.remove_task(task_id).await?;
            }

            executed_tasks += 1;
            if executed_tasks >= limit {
                break;
            }
        }

        Ok(executed_tasks)
    }

    /// Executes the task and removes it from the database if it was executed successfully.
    async fn execute_task(&self, task: &Task) -> anyhow::Result<()> {
        match &task.task_type {
            TaskType::Email(email_task) => {
                debug!(task.id = %task.id, task.task_type = task.task_type.type_tag(), task.tags = ?task.tags, "Executing email task.");
                self.send_email(task.id, email_task, task.scheduled_at)
                    .await?;
            }
            TaskType::Http(http_task) => {
                debug!(task.id = %task.id, task.task_type = task.task_type.type_tag(), task.tags = ?task.tags, "Executing HTTP task.");
                self.send_http_request(http_task, task.scheduled_at).await?;
            }
        }

        Ok(())
    }

    /// Returns retry strategy for the specified task type.
    fn get_task_retry_strategy(&self, task_type: &TaskType) -> &TaskRetryStrategy {
        let tasks_config = &self.api.config.tasks;
        match task_type {
            TaskType::Email(_) => &tasks_config.email.retry_strategy,
            TaskType::Http(_) => &tasks_config.http.retry_strategy,
        }
    }

    /// Send email using the configured SMTP server.
    async fn send_email(
        &self,
        task_id: Uuid,
        task: &EmailTaskType,
        timestamp: OffsetDateTime,
    ) -> anyhow::Result<()> {
        let Some(ref smtp) = self.api.network.smtp else {
            error!(task.id = %task_id, "Email task cannot be executed since SMTP isn't configured.");
            bail!("SMTP is not configured.");
        };

        let email = task.content.clone().into_email(self.api).await?;
        let catch_all_recipient = smtp.config.catch_all.as_ref().and_then(|catch_all| {
            // Checks if the email text matches the regular expression specified in `text_matcher`.
            if catch_all.text_matcher.is_match(&email.text) {
                Some(catch_all.recipient.as_str())
            } else {
                None
            }
        });

        let mut message_builder = Message::builder()
            .from(smtp.config.username.parse()?)
            .reply_to(smtp.config.username.parse()?)
            .subject(&email.subject)
            .date(timestamp.into());

        if let Some(catch_all_recipient) = catch_all_recipient {
            message_builder =
                message_builder.to(catch_all_recipient.parse().with_context(|| {
                    format!("Cannot parse catch-all TO address: {catch_all_recipient}")
                })?)
        } else {
            for to in task.to.iter() {
                message_builder = message_builder.to(to
                    .parse()
                    .with_context(|| format!("Cannot parse TO address: {to}"))?);
            }
        };

        let message = match email.html {
            Some(html) => {
                let alternative_builder = MultiPart::alternative()
                    .singlepart(
                        SinglePart::builder()
                            .header(ContentType::TEXT_PLAIN)
                            .body(email.text),
                    )
                    .singlepart(
                        SinglePart::builder()
                            .header(ContentType::TEXT_HTML)
                            .body(html),
                    );
                message_builder.multipart(match email.attachments {
                    Some(attachments) if !attachments.is_empty() => {
                        let mut message_builder = MultiPart::mixed().multipart(alternative_builder);
                        for attachment in attachments {
                            let attachment_builder = match attachment.disposition {
                                EmailAttachmentDisposition::Inline(id) => {
                                    Attachment::new_inline(id)
                                }
                            };
                            message_builder = message_builder.singlepart(attachment_builder.body(
                                attachment.content,
                                ContentType::parse(&attachment.content_type)?,
                            ));
                        }
                        message_builder
                    }
                    _ => alternative_builder,
                })?
            }
            None => message_builder.body(email.text)?,
        };

        smtp.send(message).await?;

        Ok(())
    }

    /// Send an HTTP request with the specified parameters.
    async fn send_http_request(
        &self,
        task: &HttpTaskType,
        _: OffsetDateTime,
    ) -> anyhow::Result<()> {
        // Start building request.
        let client = ClientBuilder::new(reqwest::Client::new())
            .with(TracingMiddleware::<SpanBackendWithUrl>::new())
            .build();
        let request_builder = client.request(task.method.clone(), task.url.clone());

        // Add headers, if any.
        let request_builder = if let Some(ref headers) = task.headers {
            request_builder.headers(headers.clone())
        } else {
            request_builder
        };

        // Add body, if any.
        let request_builder = if let Some(ref body) = task.body {
            request_builder.body(body.clone())
        } else {
            request_builder
        };

        let response = client.execute(request_builder.build()?).await?;
        if response.status().is_client_error() || response.status().is_server_error() {
            bail!(RetrackError::client(format!(
                "Failed to execute HTTP task ({}): {}",
                response.status(),
                response.text().await?
            )));
        }

        let response_status = response.status().as_u16();
        let response_text = response.text().await?;
        debug!(
            http.status_code = response_status,
            http.response = response_text,
            "Successfully sent HTTP request."
        );

        Ok(())
    }

    /// Reports an error that occurred during task execution (after all retries).
    async fn report_error(&self, task: &Task, error: anyhow::Error) {
        let task_type = task.task_type.type_tag();
        let Some(ref smtp) = self.api.network.smtp else {
            warn!(
                task.id = %task.id, task.task_type = task_type, task.tags = ?task.tags,
                "Failed to report failed task: SMTP configuration is missing."
            );
            return;
        };

        let Some(ref catch_all_recipient) = smtp.config.catch_all else {
            warn!(
               task.id = %task.id, task.task_type = task_type, task.tags = ?task.tags,
                "Failed to report failed task: catch-all recipient is missing."
            );
            return;
        };

        // Don't report errors if the task is an error reporting task itself.
        if let TaskType::Email(EmailTaskType {
            content:
                EmailContent::Template(EmailTemplate::TaskFailed {
                    task_id,
                    task_type,
                    task_tags,
                    error_message,
                }),
            ..
        }) = &task.task_type
        {
            error!(
                task.id = %task_id, task.task_type = task_type, task.tags = ?task_tags,
                "Failed to report failed task ({error_message}): {error}."
            );
            return;
        }

        let email_task = TaskType::Email(EmailTaskType {
            to: vec![catch_all_recipient.recipient.clone()],
            content: EmailContent::Template(EmailTemplate::TaskFailed {
                task_id: task.id,
                task_type: task_type.to_string(),
                task_tags: task.tags.join(", "),
                error_message: error
                    .downcast::<RetrackError>()
                    .map(|err| format!("{err}"))
                    .unwrap_or_else(|_| "Unknown error".to_string()),
            }),
        });

        let tasks_schedule_result = self
            .schedule_task(email_task, task.tags.clone(), OffsetDateTime::now_utc())
            .await;
        if let Err(err) = tasks_schedule_result {
            error!(
                task.id = %task.id, task.task_type = task_type, task.tags = ?task.tags,
                "Failed to report failed task: {err:?}."
            );
        }
    }
}

impl<DR: DnsResolver> Api<DR> {
    /// Returns an API to work with tasks.
    pub fn tasks(&self) -> TasksApi<'_, DR> {
        TasksApi::new(self)
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        config::SmtpConfig,
        tasks::{
            Email, EmailAttachment, EmailContent, EmailTaskType, EmailTemplate, HttpTaskType, Task,
            TaskType,
        },
        tests::{
            MockSmtpServer, SmtpCatchAllConfig, mock_api, mock_api_with_network,
            mock_network_with_smtp, mock_smtp, mock_smtp_config,
        },
    };
    use futures::StreamExt;
    use http::{HeaderMap, HeaderName, HeaderValue, Method, header::CONTENT_TYPE};
    use httpmock::MockServer;
    use insta::assert_debug_snapshot;
    use regex::Regex;
    use serde_json::json;
    use sqlx::PgPool;
    use std::{ops::Add, str::from_utf8, sync::Arc};
    use time::{Duration, OffsetDateTime};
    use uuid::uuid;

    #[sqlx::test]
    async fn properly_schedules_task(pool: PgPool) -> anyhow::Result<()> {
        let api = mock_api(pool).await?;

        assert!(
            api.db
                .get_task(uuid!("00000000-0000-0000-0000-000000000001"))
                .await?
                .is_none()
        );

        let mut tasks = [
            Task {
                id: uuid!("00000000-0000-0000-0000-000000000001"),
                task_type: TaskType::Email(EmailTaskType {
                    to: vec!["dev@retrack.dev".to_string()],
                    content: EmailContent::Custom(Email::text(
                        "subj".to_string(),
                        "email text".to_string(),
                    )),
                }),
                tags: vec!["tag1".to_string(), "tag2".to_string()],
                scheduled_at: OffsetDateTime::from_unix_timestamp(946720800)?,
                retry_attempt: None,
            },
            Task {
                id: uuid!("00000000-0000-0000-0000-000000000002"),
                task_type: TaskType::Email(EmailTaskType {
                    to: vec!["dev@retrack.dev".to_string()],
                    content: EmailContent::Custom(Email::text(
                        "subj #2".to_string(),
                        "email text #2".to_string(),
                    )),
                }),
                tags: vec!["tag1".to_string(), "tag2".to_string()],
                scheduled_at: OffsetDateTime::from_unix_timestamp(946720800)?,
                retry_attempt: None,
            },
        ];

        let tasks_api = api.tasks();
        for task in tasks.iter_mut() {
            task.id = tasks_api
                .schedule_task(task.task_type.clone(), task.tags.clone(), task.scheduled_at)
                .await?
                .id;
        }

        let task = api.db.get_task(tasks[0].id).await?;
        assert_eq!(task.as_ref(), Some(&tasks[0]));

        let task = api.db.get_task(tasks[1].id).await?;
        assert_eq!(task.as_ref(), Some(&tasks[1]));

        assert!(
            api.db
                .get_task(uuid!("00000000-0000-0000-0000-000000000003"))
                .await?
                .is_none()
        );

        Ok(())
    }

    #[sqlx::test]
    async fn properly_executes_email_tasks(pool: PgPool) -> anyhow::Result<()> {
        let smtp_server = MockSmtpServer::new("smtp.retrack.dev");
        smtp_server.start();

        let api = mock_api_with_network(
            pool,
            mock_network_with_smtp(mock_smtp(mock_smtp_config(
                smtp_server.host.to_string(),
                smtp_server.port,
            ))),
        )
        .await?;

        let mut tasks = [
            Task {
                id: uuid!("00000000-0000-0000-0000-000000000001"),
                task_type: TaskType::Email(EmailTaskType {
                    to: vec!["some@retrack.dev".to_string()],
                    content: EmailContent::Custom(Email::text(
                        "subj".to_string(),
                        "email text".to_string(),
                    )),
                }),
                tags: vec!["tag1".to_string(), "tag2".to_string()],
                scheduled_at: OffsetDateTime::from_unix_timestamp(946720700)?,
                retry_attempt: None,
            },
            Task {
                id: uuid!("00000000-0000-0000-0000-000000000002"),
                task_type: TaskType::Email(EmailTaskType {
                    to: vec!["another@retrack.dev".to_string()],
                    content: EmailContent::Custom(Email::html(
                        "subj #2".to_string(),
                        "email text #2".to_string(),
                        "html #2",
                    )),
                }),
                tags: vec!["tag1".to_string(), "tag2".to_string()],
                scheduled_at: OffsetDateTime::from_unix_timestamp(946720800)?,
                retry_attempt: None,
            },
        ];

        let tasks_api = api.tasks();
        for task in tasks.iter_mut() {
            task.id = tasks_api
                .schedule_task(task.task_type.clone(), task.tags.clone(), task.scheduled_at)
                .await?
                .id;
        }

        assert!(api.db.get_task(tasks[0].id).await?.is_some());
        assert!(api.db.get_task(tasks[1].id).await?.is_some());

        assert_eq!(api.tasks().execute_pending_tasks(3).await?, 2);

        assert!(api.db.get_task(tasks[0].id).await?.is_none());
        assert!(api.db.get_task(tasks[1].id).await?.is_none());

        let mails = smtp_server.mails();
        assert_eq!(mails.len(), 2);

        let boundary_regex = Regex::new(r#"boundary="(.+)""#)?;
        let mails = mails
            .into_iter()
            .map(|mail| {
                let mail = from_utf8(&mail.content).unwrap();
                let boundary = boundary_regex
                    .captures(mail)
                    .and_then(|captures| captures.get(1))
                    .map(|capture| capture.as_str());
                if let Some(boundary) = boundary {
                    mail.replace(boundary, "BOUNDARY")
                } else {
                    mail.to_string()
                }
            })
            .collect::<Vec<_>>();

        assert_debug_snapshot!(mails, @r###"
        [
            "From: dev@retrack.dev\r\nReply-To: dev@retrack.dev\r\nSubject: subj\r\nDate: Sat, 01 Jan 2000 09:58:20 +0000\r\nTo: some@retrack.dev\r\nContent-Transfer-Encoding: 7bit\r\n\r\nemail text",
            "From: dev@retrack.dev\r\nReply-To: dev@retrack.dev\r\nSubject: subj #2\r\nDate: Sat, 01 Jan 2000 10:00:00 +0000\r\nTo: another@retrack.dev\r\nMIME-Version: 1.0\r\nContent-Type: multipart/alternative;\r\n boundary=\"BOUNDARY\"\r\n\r\n--BOUNDARY\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: 7bit\r\n\r\nemail text #2\r\n--BOUNDARY\r\nContent-Type: text/html; charset=utf-8\r\nContent-Transfer-Encoding: 7bit\r\n\r\nhtml #2\r\n--BOUNDARY--\r\n",
        ]
        "###);

        Ok(())
    }

    #[sqlx::test]
    async fn properly_executes_email_tasks_with_attachments(pool: PgPool) -> anyhow::Result<()> {
        let smtp_server = MockSmtpServer::new("smtp.retrack.dev");
        smtp_server.start();

        let api = mock_api_with_network(
            pool,
            mock_network_with_smtp(mock_smtp(mock_smtp_config(
                smtp_server.host.to_string(),
                smtp_server.port,
            ))),
        )
        .await?;

        let mut tasks = [Task {
            id: uuid!("00000000-0000-0000-0000-000000000002"),
            task_type: TaskType::Email(EmailTaskType {
                to: vec!["some@retrack.dev".to_string()],
                content: EmailContent::Custom(Email::html_with_attachments(
                    "subject".to_string(),
                    "text".to_string(),
                    "<img src='cid:logo' />",
                    vec![EmailAttachment::inline("logo", "image/png", vec![1, 2, 3])],
                )),
            }),
            tags: vec!["tag1".to_string(), "tag2".to_string()],
            scheduled_at: OffsetDateTime::from_unix_timestamp(946720800)?,
            retry_attempt: None,
        }];

        let tasks_api = api.tasks();
        for task in tasks.iter_mut() {
            task.id = tasks_api
                .schedule_task(task.task_type.clone(), task.tags.clone(), task.scheduled_at)
                .await?
                .id;
        }

        assert_eq!(api.tasks().execute_pending_tasks(3).await?, 1);
        assert!(api.db.get_task(tasks[0].id).await?.is_none());

        let mails = smtp_server.mails();
        let boundary_regex = Regex::new(r#"boundary="(.+)""#)?;
        let mails = mails
            .into_iter()
            .map(|mail| {
                let mail = from_utf8(&mail.content).unwrap();
                let mut patched_mail = mail.to_string();
                for (index, capture) in boundary_regex
                    .captures_iter(mail)
                    .flat_map(|captures| captures.iter().skip(1).collect::<Vec<_>>())
                    .filter_map(|capture| Some(capture?.as_str()))
                    .enumerate()
                {
                    patched_mail = patched_mail.replace(capture, &format!("BOUNDARY_{index}"));
                }

                patched_mail
            })
            .collect::<Vec<_>>();

        assert_debug_snapshot!(mails, @r###"
        [
            "From: dev@retrack.dev\r\nReply-To: dev@retrack.dev\r\nSubject: subject\r\nDate: Sat, 01 Jan 2000 10:00:00 +0000\r\nTo: some@retrack.dev\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed;\r\n boundary=\"BOUNDARY_0\"\r\n\r\n--BOUNDARY_0\r\nContent-Type: multipart/alternative;\r\n boundary=\"BOUNDARY_1\"\r\n\r\n--BOUNDARY_1\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: 7bit\r\n\r\ntext\r\n--BOUNDARY_1\r\nContent-Type: text/html; charset=utf-8\r\nContent-Transfer-Encoding: 7bit\r\n\r\n<img src='cid:logo' />\r\n--BOUNDARY_1--\r\n--BOUNDARY_0\r\nContent-ID: <logo>\r\nContent-Disposition: inline\r\nContent-Type: image/png\r\nContent-Transfer-Encoding: 7bit\r\n\r\n\u{1}\u{2}\u{3}\r\n--BOUNDARY_0--\r\n",
        ]
        "###);

        Ok(())
    }

    #[sqlx::test]
    async fn properly_executes_pending_tasks_in_batches(pool: PgPool) -> anyhow::Result<()> {
        let smtp_server = MockSmtpServer::new("smtp.retrack.dev");
        smtp_server.start();

        let api = mock_api_with_network(
            pool,
            mock_network_with_smtp(mock_smtp(mock_smtp_config(
                smtp_server.host.to_string(),
                smtp_server.port,
            ))),
        )
        .await?;

        let tasks_api = api.tasks();
        let mut task_ids = vec![];
        for n in 0..=9 {
            let task = tasks_api
                .schedule_task(
                    TaskType::Email(EmailTaskType {
                        to: vec!["dev@retrack.dev".to_string()],
                        content: EmailContent::Custom(Email::text(
                            format!("subj {n}"),
                            format!("email text {n}"),
                        )),
                    }),
                    vec!["tag1".to_string(), "tag2".to_string()],
                    OffsetDateTime::from_unix_timestamp(946720800 + n)?,
                )
                .await?;
            task_ids.push(task.id);
        }

        for task_id in task_ids.iter().take(10) {
            assert!(api.db.get_task(*task_id).await?.is_some());
        }

        assert_eq!(api.tasks().execute_pending_tasks(3).await?, 3);

        for (n, task_id) in task_ids.iter().take(10).enumerate() {
            assert_eq!(api.db.get_task(*task_id).await?.is_some(), n >= 3);
        }

        assert_eq!(api.tasks().execute_pending_tasks(3).await?, 3);

        for (n, task_id) in task_ids.iter().take(10).enumerate() {
            assert_eq!(api.db.get_task(*task_id).await?.is_some(), n >= 6);
        }

        assert_eq!(api.tasks().execute_pending_tasks(10).await?, 4);

        for task_id in task_ids.iter().take(10) {
            assert!(api.db.get_task(*task_id).await?.is_none());
        }

        Ok(())
    }

    #[sqlx::test]
    async fn sends_emails_respecting_catch_all_filter(pool: PgPool) -> anyhow::Result<()> {
        let smtp_server = MockSmtpServer::new("smtp.retrack.dev");
        smtp_server.start();

        let mut smtp_config = mock_smtp_config(smtp_server.host.to_string(), smtp_server.port);
        smtp_config.catch_all = Some(SmtpCatchAllConfig {
            recipient: "catch-all@retrack.dev".to_string(),
            text_matcher: Regex::new("(one text)|(two text)")?,
        });
        let api =
            mock_api_with_network(pool, mock_network_with_smtp(mock_smtp(smtp_config))).await?;

        let tasks = vec![
            (
                TaskType::Email(EmailTaskType {
                    to: vec!["one@retrack.dev".to_string()],
                    content: EmailContent::Custom(Email::html(
                        "subject".to_string(),
                        "some one text message".to_string(),
                        "html",
                    )),
                }),
                OffsetDateTime::from_unix_timestamp(946720800)?,
            ),
            (
                TaskType::Email(EmailTaskType {
                    to: vec!["two@retrack.dev".to_string()],
                    content: EmailContent::Custom(Email::html(
                        "subject".to_string(),
                        "some two text message".to_string(),
                        "html",
                    )),
                }),
                OffsetDateTime::from_unix_timestamp(946720800)?,
            ),
            (
                TaskType::Email(EmailTaskType {
                    to: vec!["three@retrack.dev".to_string()],
                    content: EmailContent::Custom(Email::html(
                        "subject".to_string(),
                        "some three text message".to_string(),
                        "html",
                    )),
                }),
                OffsetDateTime::from_unix_timestamp(946720800)?,
            ),
        ];

        for (task_type, scheduled_at) in tasks.into_iter() {
            api.tasks()
                .schedule_task(task_type, vec![], scheduled_at)
                .await?;
        }

        assert_eq!(api.tasks().execute_pending_tasks(4).await?, 3);

        let mails = smtp_server.mails();
        assert_eq!(mails.len(), 3);

        let boundary_regex = Regex::new(r#"boundary="(.+)""#)?;
        let mails = mails
            .into_iter()
            .map(|mail| {
                let mail = from_utf8(&mail.content).unwrap();
                let boundary = boundary_regex
                    .captures(mail)
                    .and_then(|captures| captures.get(1))
                    .map(|capture| capture.as_str());
                if let Some(boundary) = boundary {
                    mail.replace(boundary, "BOUNDARY")
                } else {
                    mail.to_string()
                }
            })
            .collect::<Vec<_>>();

        assert_debug_snapshot!(mails, @r###"
        [
            "From: dev@retrack.dev\r\nReply-To: dev@retrack.dev\r\nSubject: subject\r\nDate: Sat, 01 Jan 2000 10:00:00 +0000\r\nTo: catch-all@retrack.dev\r\nMIME-Version: 1.0\r\nContent-Type: multipart/alternative;\r\n boundary=\"BOUNDARY\"\r\n\r\n--BOUNDARY\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: 7bit\r\n\r\nsome one text message\r\n--BOUNDARY\r\nContent-Type: text/html; charset=utf-8\r\nContent-Transfer-Encoding: 7bit\r\n\r\nhtml\r\n--BOUNDARY--\r\n",
            "From: dev@retrack.dev\r\nReply-To: dev@retrack.dev\r\nSubject: subject\r\nDate: Sat, 01 Jan 2000 10:00:00 +0000\r\nTo: catch-all@retrack.dev\r\nMIME-Version: 1.0\r\nContent-Type: multipart/alternative;\r\n boundary=\"BOUNDARY\"\r\n\r\n--BOUNDARY\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: 7bit\r\n\r\nsome two text message\r\n--BOUNDARY\r\nContent-Type: text/html; charset=utf-8\r\nContent-Transfer-Encoding: 7bit\r\n\r\nhtml\r\n--BOUNDARY--\r\n",
            "From: dev@retrack.dev\r\nReply-To: dev@retrack.dev\r\nSubject: subject\r\nDate: Sat, 01 Jan 2000 10:00:00 +0000\r\nTo: three@retrack.dev\r\nMIME-Version: 1.0\r\nContent-Type: multipart/alternative;\r\n boundary=\"BOUNDARY\"\r\n\r\n--BOUNDARY\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: 7bit\r\n\r\nsome three text message\r\n--BOUNDARY\r\nContent-Type: text/html; charset=utf-8\r\nContent-Transfer-Encoding: 7bit\r\n\r\nhtml\r\n--BOUNDARY--\r\n",
        ]
        "###);

        Ok(())
    }

    #[sqlx::test]
    async fn sends_emails_respecting_wide_open_catch_all_filter(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        let smtp_server = MockSmtpServer::new("smtp.retrack.dev");
        smtp_server.start();

        let mut smtp_config = mock_smtp_config(smtp_server.host.to_string(), smtp_server.port);
        smtp_config.catch_all = Some(SmtpCatchAllConfig {
            recipient: "catch-all@retrack.dev".to_string(),
            text_matcher: Regex::new(".*")?,
        });
        let api =
            mock_api_with_network(pool, mock_network_with_smtp(mock_smtp(smtp_config))).await?;
        let tasks = vec![
            (
                TaskType::Email(EmailTaskType {
                    to: vec!["one@retrack.dev".to_string()],
                    content: EmailContent::Custom(Email::html(
                        "subject".to_string(),
                        "some one text message".to_string(),
                        "html",
                    )),
                }),
                OffsetDateTime::from_unix_timestamp(946720800)?,
            ),
            (
                TaskType::Email(EmailTaskType {
                    to: vec!["two@retrack.dev".to_string()],
                    content: EmailContent::Custom(Email::html(
                        "subject".to_string(),
                        "some two text message".to_string(),
                        "html",
                    )),
                }),
                OffsetDateTime::from_unix_timestamp(946720800)?,
            ),
            (
                TaskType::Email(EmailTaskType {
                    to: vec!["three@retrack.dev".to_string()],
                    content: EmailContent::Custom(Email::html(
                        "subject".to_string(),
                        "some three text message".to_string(),
                        "html",
                    )),
                }),
                OffsetDateTime::from_unix_timestamp(946720800)?,
            ),
        ];

        for (task_type, scheduled_at) in tasks.into_iter() {
            api.tasks()
                .schedule_task(task_type, vec![], scheduled_at)
                .await?;
        }

        assert_eq!(api.tasks().execute_pending_tasks(4).await?, 3);

        let mails = smtp_server.mails();
        assert_eq!(mails.len(), 3);

        let boundary_regex = Regex::new(r#"boundary="(.+)""#)?;
        let mails = mails
            .into_iter()
            .map(|mail| {
                let mail = from_utf8(&mail.content).unwrap();
                let boundary = boundary_regex
                    .captures(mail)
                    .and_then(|captures| captures.get(1))
                    .map(|capture| capture.as_str());
                if let Some(boundary) = boundary {
                    mail.replace(boundary, "BOUNDARY")
                } else {
                    mail.to_string()
                }
            })
            .collect::<Vec<_>>();

        assert_debug_snapshot!(mails, @r###"
        [
            "From: dev@retrack.dev\r\nReply-To: dev@retrack.dev\r\nSubject: subject\r\nDate: Sat, 01 Jan 2000 10:00:00 +0000\r\nTo: catch-all@retrack.dev\r\nMIME-Version: 1.0\r\nContent-Type: multipart/alternative;\r\n boundary=\"BOUNDARY\"\r\n\r\n--BOUNDARY\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: 7bit\r\n\r\nsome one text message\r\n--BOUNDARY\r\nContent-Type: text/html; charset=utf-8\r\nContent-Transfer-Encoding: 7bit\r\n\r\nhtml\r\n--BOUNDARY--\r\n",
            "From: dev@retrack.dev\r\nReply-To: dev@retrack.dev\r\nSubject: subject\r\nDate: Sat, 01 Jan 2000 10:00:00 +0000\r\nTo: catch-all@retrack.dev\r\nMIME-Version: 1.0\r\nContent-Type: multipart/alternative;\r\n boundary=\"BOUNDARY\"\r\n\r\n--BOUNDARY\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: 7bit\r\n\r\nsome two text message\r\n--BOUNDARY\r\nContent-Type: text/html; charset=utf-8\r\nContent-Transfer-Encoding: 7bit\r\n\r\nhtml\r\n--BOUNDARY--\r\n",
            "From: dev@retrack.dev\r\nReply-To: dev@retrack.dev\r\nSubject: subject\r\nDate: Sat, 01 Jan 2000 10:00:00 +0000\r\nTo: catch-all@retrack.dev\r\nMIME-Version: 1.0\r\nContent-Type: multipart/alternative;\r\n boundary=\"BOUNDARY\"\r\n\r\n--BOUNDARY\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: 7bit\r\n\r\nsome three text message\r\n--BOUNDARY\r\nContent-Type: text/html; charset=utf-8\r\nContent-Transfer-Encoding: 7bit\r\n\r\nhtml\r\n--BOUNDARY--\r\n",
        ]
        "###);

        Ok(())
    }

    #[sqlx::test]
    async fn properly_executes_http_tasks(pool: PgPool) -> anyhow::Result<()> {
        let api = mock_api(pool).await?;
        let tasks_api = api.tasks();

        let server = MockServer::start();
        let mut server_handler_mock = server.mock(|when, then| {
            when.method(httpmock::Method::POST)
                .path("/api/some/execute");
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body_obj(&json!({ "ok": true }));
        });

        let task = tasks_api
            .schedule_task(
                TaskType::Http(HttpTaskType {
                    url: format!("{}/api/some/execute", server.base_url()).parse()?,
                    method: Method::POST,
                    headers: None,
                    body: None,
                }),
                vec![],
                OffsetDateTime::from_unix_timestamp(946720800)?,
            )
            .await?;

        assert_eq!(tasks_api.execute_pending_tasks(3).await?, 1);
        assert!(api.db.get_task(task.id).await?.is_none());

        server_handler_mock.assert();
        server_handler_mock.delete();

        let mut server_handler_mock = server.mock(|when, then| {
            when.method(httpmock::Method::POST)
                .path("/api/some/execute")
                .json_body(serde_json::to_value(vec![1, 2, 3]).unwrap());
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body_obj(&json!({ "ok": true }));
        });

        let task = tasks_api
            .schedule_task(
                TaskType::Http(HttpTaskType {
                    url: format!("{}/api/some/execute", server.base_url()).parse()?,
                    method: Method::POST,
                    headers: None,
                    body: Some(serde_json::to_vec(&vec![1, 2, 3])?),
                }),
                vec![],
                OffsetDateTime::from_unix_timestamp(946720800)?,
            )
            .await?;

        assert_eq!(tasks_api.execute_pending_tasks(3).await?, 1);
        assert!(api.db.get_task(task.id).await?.is_none());

        server_handler_mock.assert();
        server_handler_mock.delete();

        let server_handler_mock = server.mock(|when, then| {
            when.method(httpmock::Method::POST)
                .path("/api/some/execute")
                .header("Content-Type", "text/plain")
                .header("x-custom-header", "x-custom-value")
                .json_body(serde_json::to_value(vec![1, 2, 3]).unwrap());
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body_obj(&json!({ "ok": true }));
        });

        let task = tasks_api
            .schedule_task(
                TaskType::Http(HttpTaskType {
                    url: format!("{}/api/some/execute", server.base_url()).parse()?,
                    method: Method::POST,
                    headers: Some(HeaderMap::from_iter([
                        (CONTENT_TYPE, HeaderValue::from_static("text/plain")),
                        (
                            HeaderName::from_static("x-custom-header"),
                            HeaderValue::from_static("x-custom-value"),
                        ),
                    ])),
                    body: Some(serde_json::to_vec(&vec![1, 2, 3])?),
                }),
                vec![],
                OffsetDateTime::from_unix_timestamp(946720800)?,
            )
            .await?;

        assert_eq!(tasks_api.execute_pending_tasks(3).await?, 1);
        assert!(api.db.get_task(task.id).await?.is_none());

        server_handler_mock.assert();

        Ok(())
    }

    #[sqlx::test]
    async fn schedules_retry_if_http_task_execution_fails(pool: PgPool) -> anyhow::Result<()> {
        let api = mock_api(pool).await?;
        let tasks_api = api.tasks();

        let server = MockServer::start();
        let server_handler_mock = server.mock(|when, then| {
            when.method(httpmock::Method::POST)
                .path("/api/some/execute");
            then.status(400)
                .header("Content-Type", "application/json")
                .json_body_obj(&json!({ "ok": false }));
        });

        let task = tasks_api
            .schedule_task(
                TaskType::Http(HttpTaskType {
                    url: format!("{}/api/some/execute", server.base_url()).parse()?,
                    method: Method::POST,
                    headers: None,
                    body: None,
                }),
                vec![],
                OffsetDateTime::from_unix_timestamp(946720800)?,
            )
            .await?;

        assert_eq!(tasks_api.execute_pending_tasks(3).await?, 1);

        let rescheduled_task = api.db.get_task(task.id).await?.unwrap();
        assert_eq!(rescheduled_task.task_type, task.task_type);
        assert_eq!(rescheduled_task.retry_attempt, Some(1));
        assert!(
            (OffsetDateTime::now_utc().add(Duration::seconds(30)) - rescheduled_task.scheduled_at)
                < Duration::seconds(5)
        );
        server_handler_mock.assert();

        api.db
            .update_task(&Task {
                scheduled_at: OffsetDateTime::from_unix_timestamp(946720800)?,
                ..rescheduled_task.clone()
            })
            .await?;
        assert_eq!(tasks_api.execute_pending_tasks(3).await?, 1);

        let rescheduled_task = api.db.get_task(task.id).await?.unwrap();
        assert_eq!(rescheduled_task.task_type, task.task_type);
        assert_eq!(rescheduled_task.retry_attempt, Some(2));
        assert!(
            (OffsetDateTime::now_utc().add(Duration::seconds(30)) - rescheduled_task.scheduled_at)
                < Duration::seconds(5)
        );
        server_handler_mock.assert_calls(2);

        Ok(())
    }

    #[sqlx::test]
    async fn properly_executes_task_after_retry(pool: PgPool) -> anyhow::Result<()> {
        let api = mock_api(pool).await?;
        let tasks_api = api.tasks();

        let server = MockServer::start();
        let server_handler_mock = server.mock(|when, then| {
            when.method(httpmock::Method::POST)
                .path("/api/some/execute");
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body_obj(&json!({ "ok": true }));
        });

        let task = tasks_api
            .schedule_task(
                TaskType::Http(HttpTaskType {
                    url: format!("{}/api/some/execute", server.base_url()).parse()?,
                    method: Method::POST,
                    headers: None,
                    body: None,
                }),
                vec![],
                OffsetDateTime::from_unix_timestamp(946720800)?,
            )
            .await?;
        api.db
            .update_task(&Task {
                retry_attempt: Some(1),
                ..task.clone()
            })
            .await?;

        assert_eq!(tasks_api.execute_pending_tasks(3).await?, 1);
        assert!(api.db.get_task(task.id).await?.is_none());

        server_handler_mock.assert();

        Ok(())
    }

    #[sqlx::test]
    async fn removes_task_and_reports_error_if_all_retries_exhausted(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        let smtp_server = MockSmtpServer::new("smtp.retrack.dev");
        smtp_server.start();

        let smtp_config = SmtpConfig {
            catch_all: Some(SmtpCatchAllConfig {
                recipient: "dev@retrack.dev".to_string(),
                text_matcher: Regex::new(r"alpha")?,
            }),
            ..mock_smtp_config(smtp_server.host.to_string(), smtp_server.port)
        };
        let api = Arc::new(
            mock_api_with_network(pool, mock_network_with_smtp(mock_smtp(smtp_config))).await?,
        );
        let tasks_api = api.tasks();

        let server = MockServer::start();
        let server_handler_mock = server.mock(|when, then| {
            when.method(httpmock::Method::POST)
                .path("/api/some/execute");
            then.status(400)
                .header("Content-Type", "application/json")
                .body("Uh oh (failed retry)!");
        });

        let task = tasks_api
            .schedule_task(
                TaskType::Http(HttpTaskType {
                    url: format!("{}/api/some/execute", server.base_url()).parse()?,
                    method: Method::POST,
                    headers: None,
                    body: None,
                }),
                vec!["tag1".to_string(), "tag2".to_string()],
                OffsetDateTime::from_unix_timestamp(946720800)?,
            )
            .await?;
        api.db
            .update_task(&Task {
                retry_attempt: Some(3),
                ..task.clone()
            })
            .await?;

        assert_eq!(tasks_api.execute_pending_tasks(3).await?, 1);
        assert!(api.db.get_task(task.id).await?.is_none());
        server_handler_mock.assert();

        let mut tasks_ids = api
            .db
            .get_tasks_ids(
                OffsetDateTime::now_utc().add(std::time::Duration::from_secs(3600 * 24 * 365)),
                10,
            )
            .collect::<Vec<_>>()
            .await;
        assert_eq!(tasks_ids.len(), 1);

        let report_error_task = api.db.get_task(tasks_ids.remove(0)?).await?.unwrap();
        assert_eq!(
            report_error_task.task_type,
            TaskType::Email(EmailTaskType {
                to: vec!["dev@retrack.dev".to_string()],
                content: EmailContent::Template(EmailTemplate::TaskFailed {
                    task_id: task.id,
                    task_type: task.task_type.type_tag().to_string(),
                    task_tags: "tag1, tag2".to_string(),
                    error_message:
                        "Failed to execute HTTP task (400 Bad Request): Uh oh (failed retry)!"
                            .to_string(),
                })
            })
        );

        Ok(())
    }
}
