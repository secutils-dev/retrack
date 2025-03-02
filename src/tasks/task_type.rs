use crate::tasks::{EmailTaskType, HttpTaskType};
use serde::{Deserialize, Serialize};

/// Defines a task type.
#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
pub enum TaskType {
    /// Task for sending email.
    Email(EmailTaskType),
    /// Task for sending HTTP request.
    Http(HttpTaskType),
}

impl TaskType {
    /// Returns the type tag of the task.
    pub fn type_tag(&self) -> &'static str {
        match self {
            TaskType::Email(_) => "email",
            TaskType::Http(_) => "http",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::TaskType;
    use crate::tasks::{Email, EmailContent, EmailTaskType, HttpTaskType};
    use http::{HeaderMap, HeaderValue, Method, header};

    #[test]
    fn serialization() -> anyhow::Result<()> {
        assert_eq!(
            postcard::to_stdvec(&TaskType::Email(EmailTaskType {
                to: vec!["two@retrack.dev".to_string()],
                content: EmailContent::Custom(Email::text(
                    "subject".to_string(),
                    "some text message".to_string()
                )),
            }))?,
            vec![
                0, 1, 15, 116, 119, 111, 64, 114, 101, 116, 114, 97, 99, 107, 46, 100, 101, 118, 0,
                7, 115, 117, 98, 106, 101, 99, 116, 17, 115, 111, 109, 101, 32, 116, 101, 120, 116,
                32, 109, 101, 115, 115, 97, 103, 101, 0, 0
            ]
        );

        assert_eq!(
            postcard::to_stdvec(&TaskType::Http(HttpTaskType {
                method: Method::PUT,
                url: "https://retrack.dev/some-path".parse()?,
                headers: Some(HeaderMap::from_iter([(
                    header::CONTENT_TYPE,
                    HeaderValue::from_static("text/plain")
                )])),
                body: Some(vec![1, 2, 3]),
            }))?,
            vec![
                1, 29, 104, 116, 116, 112, 115, 58, 47, 47, 114, 101, 116, 114, 97, 99, 107, 46,
                100, 101, 118, 47, 115, 111, 109, 101, 45, 112, 97, 116, 104, 3, 80, 85, 84, 1, 1,
                12, 99, 111, 110, 116, 101, 110, 116, 45, 116, 121, 112, 101, 1, 10, 116, 101, 120,
                116, 47, 112, 108, 97, 105, 110, 1, 3, 1, 2, 3
            ]
        );

        Ok(())
    }

    #[test]
    fn deserialization() -> anyhow::Result<()> {
        assert_eq!(
            postcard::from_bytes::<TaskType>(&[
                0, 1, 15, 116, 119, 111, 64, 114, 101, 116, 114, 97, 99, 107, 46, 100, 101, 118, 0,
                7, 115, 117, 98, 106, 101, 99, 116, 17, 115, 111, 109, 101, 32, 116, 101, 120, 116,
                32, 109, 101, 115, 115, 97, 103, 101, 0, 0
            ])?,
            TaskType::Email(EmailTaskType {
                to: vec!["two@retrack.dev".to_string()],
                content: EmailContent::Custom(Email::text(
                    "subject".to_string(),
                    "some text message".to_string()
                )),
            })
        );

        assert_eq!(
            postcard::from_bytes::<TaskType>(&[
                1, 29, 104, 116, 116, 112, 115, 58, 47, 47, 114, 101, 116, 114, 97, 99, 107, 46,
                100, 101, 118, 47, 115, 111, 109, 101, 45, 112, 97, 116, 104, 3, 80, 85, 84, 1, 1,
                12, 99, 111, 110, 116, 101, 110, 116, 45, 116, 121, 112, 101, 1, 10, 116, 101, 120,
                116, 47, 112, 108, 97, 105, 110, 1, 3, 1, 2, 3
            ])?,
            TaskType::Http(HttpTaskType {
                method: Method::PUT,
                url: "https://retrack.dev/some-path".parse()?,
                headers: Some(HeaderMap::from_iter([(
                    header::CONTENT_TYPE,
                    HeaderValue::from_static("text/plain")
                )])),
                body: Some(vec![1, 2, 3]),
            })
        );

        Ok(())
    }

    #[test]
    fn can_return_type_tag() {
        assert_eq!(
            TaskType::Email(EmailTaskType {
                to: vec!["two@retrack.dev".to_string()],
                content: EmailContent::Custom(Email::text(
                    "subject".to_string(),
                    "some text message".to_string()
                )),
            })
            .type_tag(),
            "email"
        );
        assert_eq!(
            TaskType::Http(HttpTaskType {
                method: Method::PUT,
                url: "https://retrack.dev/some-path".parse().unwrap(),
                headers: Some(HeaderMap::from_iter([(
                    header::CONTENT_TYPE,
                    HeaderValue::from_static("text/plain")
                )])),
                body: Some(vec![1, 2, 3]),
            })
            .type_tag(),
            "http"
        );
    }
}
