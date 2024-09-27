use crate::{
    api::Api,
    network::{DnsResolver, EmailTransport, EmailTransportError},
    tasks::{EmailAttachmentDisposition, EmailTaskType, HttpTaskType, Task, TaskType},
};
use anyhow::{bail, Context};
use futures::{pin_mut, StreamExt};
use lettre::{
    message::{header::ContentType, Attachment, MultiPart, SinglePart},
    Message,
};
use reqwest_middleware::ClientBuilder;
use reqwest_tracing::{SpanBackendWithUrl, TracingMiddleware};
use std::cmp;
use time::OffsetDateTime;
use tracing::{debug, error};
use uuid::Uuid;

/// Defines a maximum number of tasks that can be retrieved from the database at once.
const MAX_TASKS_PAGE_SIZE: usize = 100;

/// Describes the API to work with tasks.
pub struct TasksApi<'a, DR: DnsResolver, ET: EmailTransport> {
    api: &'a Api<DR, ET>,
}

impl<'a, DR: DnsResolver, ET: EmailTransport> TasksApi<'a, DR, ET>
where
    ET::Error: EmailTransportError,
{
    /// Creates Tasks API.
    pub fn new(api: &'a Api<DR, ET>) -> Self {
        Self { api }
    }

    /// Schedules a new task.
    pub async fn schedule_task(
        &self,
        task_type: TaskType,
        scheduled_at: OffsetDateTime,
    ) -> anyhow::Result<Task> {
        let task = Task {
            id: Uuid::now_v7(),
            task_type,
            scheduled_at,
        };

        self.api.db.insert_task(&task).await?;

        Ok(task)
    }

    /// Executes pending tasks. The max number to send is limited by `limit`.
    pub async fn execute_pending_tasks(&self, limit: usize) -> anyhow::Result<usize> {
        let pending_tasks_ids = self.api.db.get_tasks_ids(
            OffsetDateTime::now_utc(),
            cmp::min(MAX_TASKS_PAGE_SIZE, limit),
        );
        pin_mut!(pending_tasks_ids);

        let mut executed_tasks = 0;
        while let Some(task_id) = pending_tasks_ids.next().await {
            if let Some(task) = self.api.db.get_task(task_id?).await? {
                let task_id = task.id;
                if let Err(err) = self.execute_task(task).await {
                    error!(taask.id = %task_id, "Failed to execute task: {err:?}");
                } else {
                    debug!(taask.id = %task_id, "Successfully executed task.");
                    executed_tasks += 1;
                    self.api.db.remove_task(task_id).await?;
                }
            }

            if executed_tasks >= limit {
                break;
            }
        }

        Ok(executed_tasks)
    }

    /// Executes task and removes it from the database, if it was executed successfully.
    async fn execute_task(&self, task: Task) -> anyhow::Result<()> {
        match task.task_type {
            TaskType::Email(email_task) => {
                debug!(task.id = %task.id, "Executing email task.");
                self.send_email(email_task, task.scheduled_at).await?;
            }
            TaskType::Http(http_task) => {
                debug!(task.id = %task.id, "Executing HTTP task.");
                self.send_http_request(http_task, task.scheduled_at).await?;
            }
        }

        Ok(())
    }

    /// Send email using configured SMTP server.
    async fn send_email(
        &self,
        task: EmailTaskType,
        timestamp: OffsetDateTime,
    ) -> anyhow::Result<()> {
        let smtp_config = if let Some(ref smtp_config) = self.api.config.as_ref().smtp {
            smtp_config
        } else {
            bail!("SMTP is not configured.");
        };

        let email = task.content.into_email(self.api).await?;
        let catch_all_recipient = smtp_config.catch_all.as_ref().and_then(|catch_all| {
            // Checks if the email text matches the regular expression specified in `text_matcher`.
            if catch_all.text_matcher.is_match(&email.text) {
                Some(catch_all.recipient.as_str())
            } else {
                None
            }
        });

        let mut message_builder = Message::builder()
            .from(smtp_config.username.parse()?)
            .reply_to(smtp_config.username.parse()?)
            .subject(&email.subject)
            .date(timestamp.into());

        if let Some(catch_all_recipient) = catch_all_recipient {
            message_builder =
                message_builder.to(catch_all_recipient.parse().with_context(|| {
                    format!("Cannot parse catch-all TO address: {}", catch_all_recipient)
                })?)
        } else {
            for to in task.to {
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

        self.api.network.email_transport.send(message).await?;

        Ok(())
    }

    /// Send HTTP request with the specified parameters.
    async fn send_http_request(&self, task: HttpTaskType, _: OffsetDateTime) -> anyhow::Result<()> {
        // Start building request.
        let client = ClientBuilder::new(reqwest::Client::new())
            .with(TracingMiddleware::<SpanBackendWithUrl>::new())
            .build();
        let request_builder = client.request(task.method, task.url);

        // Add headers, if any.
        let request_builder = if let Some(headers) = task.headers {
            request_builder.headers(headers)
        } else {
            request_builder
        };

        // Add body, if any.
        let request_builder = if let Some(body) = task.body {
            request_builder.body(body)
        } else {
            request_builder
        };

        let response = client
            .execute(request_builder.build()?)
            .await?
            .error_for_status()?;

        let response_status = response.status().as_u16();
        let response_text = response.text().await?;
        debug!(
            http.status_code = response_status,
            http.response = response_text,
            "Successfully sent HTTP request."
        );

        Ok(())
    }
}

impl<DR: DnsResolver, ET: EmailTransport> Api<DR, ET>
where
    ET::Error: EmailTransportError,
{
    /// Returns an API to work with tasks.
    pub fn tasks(&self) -> TasksApi<'_, DR, ET> {
        TasksApi::new(self)
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        config::SmtpConfig,
        tasks::{
            Email, EmailAttachment, EmailContent, EmailTaskType, HttpTaskType, Task, TaskType,
        },
        tests::{mock_api, mock_api_with_config, mock_config, SmtpCatchAllConfig},
    };
    use http::{header::CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue, Method};
    use httpmock::MockServer;
    use insta::assert_debug_snapshot;
    use serde_json::json;
    use sqlx::PgPool;
    use time::OffsetDateTime;
    use uuid::uuid;

    #[sqlx::test]
    async fn properly_schedules_task(pool: PgPool) -> anyhow::Result<()> {
        let api = mock_api(pool).await?;

        assert!(api
            .db
            .get_task(uuid!("00000000-0000-0000-0000-000000000001"))
            .await?
            .is_none());

        let mut tasks = vec![
            Task {
                id: uuid!("00000000-0000-0000-0000-000000000001"),
                task_type: TaskType::Email(EmailTaskType {
                    to: vec!["dev@retrack.dev".to_string()],
                    content: EmailContent::Custom(Email::text(
                        "subj".to_string(),
                        "email text".to_string(),
                    )),
                }),
                scheduled_at: OffsetDateTime::from_unix_timestamp(946720800)?,
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
                scheduled_at: OffsetDateTime::from_unix_timestamp(946720800)?,
            },
        ];

        let tasks_api = api.tasks();
        for task in tasks.iter_mut() {
            task.id = tasks_api
                .schedule_task(task.task_type.clone(), task.scheduled_at)
                .await?
                .id;
        }

        let task = api.db.get_task(tasks[0].id).await?;
        assert_eq!(task.as_ref(), Some(&tasks[0]));

        let task = api.db.get_task(tasks[1].id).await?;
        assert_eq!(task.as_ref(), Some(&tasks[1]));

        assert!(api
            .db
            .get_task(uuid!("00000000-0000-0000-0000-000000000003"))
            .await?
            .is_none());

        Ok(())
    }

    #[sqlx::test]
    async fn properly_executes_email_tasks(pool: PgPool) -> anyhow::Result<()> {
        let api = mock_api(pool).await?;

        let mut tasks = vec![
            Task {
                id: uuid!("00000000-0000-0000-0000-000000000001"),
                task_type: TaskType::Email(EmailTaskType {
                    to: vec!["dev@retrack.dev".to_string()],
                    content: EmailContent::Custom(Email::text(
                        "subj".to_string(),
                        "email text".to_string(),
                    )),
                }),
                scheduled_at: OffsetDateTime::from_unix_timestamp(946720700)?,
            },
            Task {
                id: uuid!("00000000-0000-0000-0000-000000000002"),
                task_type: TaskType::Email(EmailTaskType {
                    to: vec!["some@retrack.dev".to_string()],
                    content: EmailContent::Custom(Email::html(
                        "subj #2".to_string(),
                        "email text #2".to_string(),
                        "html #2",
                    )),
                }),
                scheduled_at: OffsetDateTime::from_unix_timestamp(946720800)?,
            },
        ];

        let tasks_api = api.tasks();
        for task in tasks.iter_mut() {
            task.id = tasks_api
                .schedule_task(task.task_type.clone(), task.scheduled_at)
                .await?
                .id;
        }

        assert!(api.db.get_task(tasks[0].id).await?.is_some());
        assert!(api.db.get_task(tasks[1].id).await?.is_some());

        assert_eq!(api.tasks().execute_pending_tasks(3).await?, 2);

        assert!(api.db.get_task(tasks[0].id).await?.is_none());
        assert!(api.db.get_task(tasks[1].id).await?.is_none());

        let messages = api.network.email_transport.messages().await;
        assert_eq!(messages.len(), 2);

        let boundary_regex = regex::Regex::new(r#"boundary="(.+)""#)?;
        let messages = messages
            .into_iter()
            .map(|(envelope, content)| {
                let boundary = boundary_regex
                    .captures(&content)
                    .and_then(|captures| captures.get(1))
                    .map(|capture| capture.as_str());

                (
                    envelope,
                    if let Some(boundary) = boundary {
                        content.replace(boundary, "BOUNDARY")
                    } else {
                        content
                    },
                )
            })
            .collect::<Vec<_>>();

        assert_debug_snapshot!(messages, @r###"
        [
            (
                Envelope {
                    forward_path: [
                        Address {
                            serialized: "dev@retrack.dev",
                            at_start: 3,
                        },
                    ],
                    reverse_path: Some(
                        Address {
                            serialized: "dev@retrack.dev",
                            at_start: 3,
                        },
                    ),
                },
                "From: dev@retrack.dev\r\nReply-To: dev@retrack.dev\r\nSubject: subj\r\nDate: Sat, 01 Jan 2000 09:58:20 +0000\r\nTo: dev@retrack.dev\r\nContent-Transfer-Encoding: 7bit\r\n\r\nemail text",
            ),
            (
                Envelope {
                    forward_path: [
                        Address {
                            serialized: "some@retrack.dev",
                            at_start: 4,
                        },
                    ],
                    reverse_path: Some(
                        Address {
                            serialized: "dev@retrack.dev",
                            at_start: 3,
                        },
                    ),
                },
                "From: dev@retrack.dev\r\nReply-To: dev@retrack.dev\r\nSubject: subj #2\r\nDate: Sat, 01 Jan 2000 10:00:00 +0000\r\nTo: some@retrack.dev\r\nMIME-Version: 1.0\r\nContent-Type: multipart/alternative;\r\n boundary=\"BOUNDARY\"\r\n\r\n--BOUNDARY\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: 7bit\r\n\r\nemail text #2\r\n--BOUNDARY\r\nContent-Type: text/html; charset=utf-8\r\nContent-Transfer-Encoding: 7bit\r\n\r\nhtml #2\r\n--BOUNDARY--\r\n",
            ),
        ]
        "###);

        Ok(())
    }

    #[sqlx::test]
    async fn properly_executes_email_tasks_with_attachments(pool: PgPool) -> anyhow::Result<()> {
        let api = mock_api(pool).await?;

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
            scheduled_at: OffsetDateTime::from_unix_timestamp(946720800)?,
        }];

        let tasks_api = api.tasks();
        for task in tasks.iter_mut() {
            task.id = tasks_api
                .schedule_task(task.task_type.clone(), task.scheduled_at)
                .await?
                .id;
        }

        assert_eq!(api.tasks().execute_pending_tasks(3).await?, 1);
        assert!(api.db.get_task(tasks[0].id).await?.is_none());

        let messages = api.network.email_transport.messages().await;
        assert_eq!(messages.len(), 1);

        let boundary_regex = regex::Regex::new(r#"boundary="(.+)""#)?;
        let messages = messages
            .into_iter()
            .map(|(envelope, content)| {
                let mut patched_content = content.clone();
                for (index, capture) in boundary_regex
                    .captures_iter(&content)
                    .flat_map(|captures| captures.iter().skip(1).collect::<Vec<_>>())
                    .filter_map(|capture| Some(capture?.as_str()))
                    .enumerate()
                {
                    patched_content =
                        patched_content.replace(capture, &format!("BOUNDARY_{index}"));
                }

                (envelope, patched_content)
            })
            .collect::<Vec<_>>();

        assert_debug_snapshot!(messages, @r###"
        [
            (
                Envelope {
                    forward_path: [
                        Address {
                            serialized: "some@retrack.dev",
                            at_start: 4,
                        },
                    ],
                    reverse_path: Some(
                        Address {
                            serialized: "dev@retrack.dev",
                            at_start: 3,
                        },
                    ),
                },
                "From: dev@retrack.dev\r\nReply-To: dev@retrack.dev\r\nSubject: subject\r\nDate: Sat, 01 Jan 2000 10:00:00 +0000\r\nTo: some@retrack.dev\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed;\r\n boundary=\"BOUNDARY_0\"\r\n\r\n--BOUNDARY_0\r\nContent-Type: multipart/alternative;\r\n boundary=\"BOUNDARY_1\"\r\n\r\n--BOUNDARY_1\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: 7bit\r\n\r\ntext\r\n--BOUNDARY_1\r\nContent-Type: text/html; charset=utf-8\r\nContent-Transfer-Encoding: 7bit\r\n\r\n<img src='cid:logo' />\r\n--BOUNDARY_1--\r\n--BOUNDARY_0\r\nContent-ID: <logo>\r\nContent-Disposition: inline\r\nContent-Type: image/png\r\nContent-Transfer-Encoding: 7bit\r\n\r\n\u{1}\u{2}\u{3}\r\n--BOUNDARY_0--\r\n",
            ),
        ]
        "###);

        Ok(())
    }

    #[sqlx::test]
    async fn properly_executes_pending_tasks_in_batches(pool: PgPool) -> anyhow::Result<()> {
        let api = mock_api(pool).await?;

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
        let mut config = mock_config()?;
        let text_matcher = regex::Regex::new("(one text)|(two text)")?;
        config.smtp = config.smtp.map(|smtp| SmtpConfig {
            catch_all: Some(SmtpCatchAllConfig {
                recipient: "catch-all@retrack.dev".to_string(),
                text_matcher,
            }),
            ..smtp
        });

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

        let api = mock_api_with_config(pool, config).await?;
        for (task_type, scheduled_at) in tasks.into_iter() {
            api.tasks().schedule_task(task_type, scheduled_at).await?;
        }

        assert_eq!(api.tasks().execute_pending_tasks(4).await?, 3);

        let messages = api.network.email_transport.messages().await;
        assert_eq!(messages.len(), 3);

        let boundary_regex = regex::Regex::new(r#"boundary="(.+)""#)?;
        let messages = messages
            .into_iter()
            .map(|(envelope, content)| {
                let boundary = boundary_regex
                    .captures(&content)
                    .and_then(|captures| captures.get(1))
                    .map(|capture| capture.as_str());

                (
                    envelope,
                    if let Some(boundary) = boundary {
                        content.replace(boundary, "BOUNDARY")
                    } else {
                        content
                    },
                )
            })
            .collect::<Vec<_>>();

        assert_debug_snapshot!(messages, @r###"
        [
            (
                Envelope {
                    forward_path: [
                        Address {
                            serialized: "catch-all@retrack.dev",
                            at_start: 9,
                        },
                    ],
                    reverse_path: Some(
                        Address {
                            serialized: "dev@retrack.dev",
                            at_start: 3,
                        },
                    ),
                },
                "From: dev@retrack.dev\r\nReply-To: dev@retrack.dev\r\nSubject: subject\r\nDate: Sat, 01 Jan 2000 10:00:00 +0000\r\nTo: catch-all@retrack.dev\r\nMIME-Version: 1.0\r\nContent-Type: multipart/alternative;\r\n boundary=\"BOUNDARY\"\r\n\r\n--BOUNDARY\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: 7bit\r\n\r\nsome one text message\r\n--BOUNDARY\r\nContent-Type: text/html; charset=utf-8\r\nContent-Transfer-Encoding: 7bit\r\n\r\nhtml\r\n--BOUNDARY--\r\n",
            ),
            (
                Envelope {
                    forward_path: [
                        Address {
                            serialized: "catch-all@retrack.dev",
                            at_start: 9,
                        },
                    ],
                    reverse_path: Some(
                        Address {
                            serialized: "dev@retrack.dev",
                            at_start: 3,
                        },
                    ),
                },
                "From: dev@retrack.dev\r\nReply-To: dev@retrack.dev\r\nSubject: subject\r\nDate: Sat, 01 Jan 2000 10:00:00 +0000\r\nTo: catch-all@retrack.dev\r\nMIME-Version: 1.0\r\nContent-Type: multipart/alternative;\r\n boundary=\"BOUNDARY\"\r\n\r\n--BOUNDARY\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: 7bit\r\n\r\nsome two text message\r\n--BOUNDARY\r\nContent-Type: text/html; charset=utf-8\r\nContent-Transfer-Encoding: 7bit\r\n\r\nhtml\r\n--BOUNDARY--\r\n",
            ),
            (
                Envelope {
                    forward_path: [
                        Address {
                            serialized: "three@retrack.dev",
                            at_start: 5,
                        },
                    ],
                    reverse_path: Some(
                        Address {
                            serialized: "dev@retrack.dev",
                            at_start: 3,
                        },
                    ),
                },
                "From: dev@retrack.dev\r\nReply-To: dev@retrack.dev\r\nSubject: subject\r\nDate: Sat, 01 Jan 2000 10:00:00 +0000\r\nTo: three@retrack.dev\r\nMIME-Version: 1.0\r\nContent-Type: multipart/alternative;\r\n boundary=\"BOUNDARY\"\r\n\r\n--BOUNDARY\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: 7bit\r\n\r\nsome three text message\r\n--BOUNDARY\r\nContent-Type: text/html; charset=utf-8\r\nContent-Transfer-Encoding: 7bit\r\n\r\nhtml\r\n--BOUNDARY--\r\n",
            ),
        ]
        "###);

        Ok(())
    }

    #[sqlx::test]
    async fn sends_emails_respecting_wide_open_catch_all_filter(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        let mut config = mock_config()?;
        let text_matcher = regex::Regex::new(".*")?;
        config.smtp = config.smtp.map(|smtp| SmtpConfig {
            catch_all: Some(SmtpCatchAllConfig {
                recipient: "catch-all@retrack.dev".to_string(),
                text_matcher,
            }),
            ..smtp
        });

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

        let api = mock_api_with_config(pool, config).await?;
        for (task_type, scheduled_at) in tasks.into_iter() {
            api.tasks().schedule_task(task_type, scheduled_at).await?;
        }

        assert_eq!(api.tasks().execute_pending_tasks(4).await?, 3);

        let messages = api.network.email_transport.messages().await;
        assert_eq!(messages.len(), 3);

        let boundary_regex = regex::Regex::new(r#"boundary="(.+)""#)?;
        let messages = messages
            .into_iter()
            .map(|(envelope, content)| {
                let boundary = boundary_regex
                    .captures(&content)
                    .and_then(|captures| captures.get(1))
                    .map(|capture| capture.as_str());

                (
                    envelope,
                    if let Some(boundary) = boundary {
                        content.replace(boundary, "BOUNDARY")
                    } else {
                        content
                    },
                )
            })
            .collect::<Vec<_>>();

        assert_debug_snapshot!(messages, @r###"
        [
            (
                Envelope {
                    forward_path: [
                        Address {
                            serialized: "catch-all@retrack.dev",
                            at_start: 9,
                        },
                    ],
                    reverse_path: Some(
                        Address {
                            serialized: "dev@retrack.dev",
                            at_start: 3,
                        },
                    ),
                },
                "From: dev@retrack.dev\r\nReply-To: dev@retrack.dev\r\nSubject: subject\r\nDate: Sat, 01 Jan 2000 10:00:00 +0000\r\nTo: catch-all@retrack.dev\r\nMIME-Version: 1.0\r\nContent-Type: multipart/alternative;\r\n boundary=\"BOUNDARY\"\r\n\r\n--BOUNDARY\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: 7bit\r\n\r\nsome one text message\r\n--BOUNDARY\r\nContent-Type: text/html; charset=utf-8\r\nContent-Transfer-Encoding: 7bit\r\n\r\nhtml\r\n--BOUNDARY--\r\n",
            ),
            (
                Envelope {
                    forward_path: [
                        Address {
                            serialized: "catch-all@retrack.dev",
                            at_start: 9,
                        },
                    ],
                    reverse_path: Some(
                        Address {
                            serialized: "dev@retrack.dev",
                            at_start: 3,
                        },
                    ),
                },
                "From: dev@retrack.dev\r\nReply-To: dev@retrack.dev\r\nSubject: subject\r\nDate: Sat, 01 Jan 2000 10:00:00 +0000\r\nTo: catch-all@retrack.dev\r\nMIME-Version: 1.0\r\nContent-Type: multipart/alternative;\r\n boundary=\"BOUNDARY\"\r\n\r\n--BOUNDARY\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: 7bit\r\n\r\nsome two text message\r\n--BOUNDARY\r\nContent-Type: text/html; charset=utf-8\r\nContent-Transfer-Encoding: 7bit\r\n\r\nhtml\r\n--BOUNDARY--\r\n",
            ),
            (
                Envelope {
                    forward_path: [
                        Address {
                            serialized: "catch-all@retrack.dev",
                            at_start: 9,
                        },
                    ],
                    reverse_path: Some(
                        Address {
                            serialized: "dev@retrack.dev",
                            at_start: 3,
                        },
                    ),
                },
                "From: dev@retrack.dev\r\nReply-To: dev@retrack.dev\r\nSubject: subject\r\nDate: Sat, 01 Jan 2000 10:00:00 +0000\r\nTo: catch-all@retrack.dev\r\nMIME-Version: 1.0\r\nContent-Type: multipart/alternative;\r\n boundary=\"BOUNDARY\"\r\n\r\n--BOUNDARY\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: 7bit\r\n\r\nsome three text message\r\n--BOUNDARY\r\nContent-Type: text/html; charset=utf-8\r\nContent-Transfer-Encoding: 7bit\r\n\r\nhtml\r\n--BOUNDARY--\r\n",
            ),
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
                OffsetDateTime::from_unix_timestamp(946720800)?,
            )
            .await?;

        assert_eq!(tasks_api.execute_pending_tasks(3).await?, 1);
        assert!(api.db.get_task(task.id).await?.is_none());

        server_handler_mock.assert();

        Ok(())
    }

    #[sqlx::test]
    async fn keep_http_task_if_execution_fails(pool: PgPool) -> anyhow::Result<()> {
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
                OffsetDateTime::from_unix_timestamp(946720800)?,
            )
            .await?;

        assert_eq!(tasks_api.execute_pending_tasks(3).await?, 0);
        assert!(api.db.get_task(task.id).await?.is_some());

        server_handler_mock.assert();

        Ok(())
    }
}
