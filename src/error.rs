mod error_kind;

use actix_web::{HttpResponse, HttpResponseBuilder, ResponseError, http::StatusCode};
use anyhow::anyhow;
use std::fmt::{Debug, Display, Formatter};

pub use error_kind::ErrorKind;

/// Application-specific error type.
#[derive(thiserror::Error)]
pub struct Error {
    pub root_cause: anyhow::Error,
    kind: ErrorKind,
}

impl Error {
    /// Creates a Client error instance with the given root cause.
    pub fn client_with_root_cause(root_cause: anyhow::Error) -> Self {
        Self {
            root_cause,
            kind: ErrorKind::ClientError,
        }
    }

    /// Creates a Client error instance with the given message.
    pub fn client<M>(message: M) -> Self
    where
        M: Display + Debug + Send + Sync + 'static,
    {
        Self {
            root_cause: anyhow!(message),
            kind: ErrorKind::ClientError,
        }
    }
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        Debug::fmt(&self.root_cause, f)
    }
}

impl Debug for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        Debug::fmt(&self.root_cause, f)
    }
}

impl ResponseError for Error {
    fn status_code(&self) -> StatusCode {
        match self.kind {
            ErrorKind::ClientError => StatusCode::BAD_REQUEST,
            ErrorKind::Unknown => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn error_response(&self) -> HttpResponse {
        HttpResponseBuilder::new(self.status_code()).body(match self.kind {
            ErrorKind::ClientError => self.root_cause.to_string(),
            ErrorKind::Unknown => "Internal Server Error".to_string(),
        })
    }
}

impl From<anyhow::Error> for Error {
    fn from(err: anyhow::Error) -> Error {
        err.downcast::<Error>().unwrap_or_else(|root_cause| Error {
            root_cause,
            kind: ErrorKind::Unknown,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{Error, ErrorKind};
    use actix_web::{ResponseError, body::MessageBody, http::StatusCode};
    use anyhow::anyhow;
    use insta::assert_debug_snapshot;

    #[test]
    fn can_create_client_errors() -> anyhow::Result<()> {
        let error = Error::client("Uh oh.");

        assert_eq!(error.kind, ErrorKind::ClientError);
        assert_debug_snapshot!(error, @r###""Uh oh.""###);

        assert_eq!(error.status_code(), StatusCode::BAD_REQUEST);

        let error_response = error.error_response();
        assert_debug_snapshot!(error_response, @r###"
        HttpResponse {
            error: None,
            res: 
            Response HTTP/1.1 400 Bad Request
              headers:
              body: Sized(6)
            ,
        }
        "###);
        let body = error_response.into_body().try_into_bytes().unwrap();
        assert_eq!(body.as_ref(), b"Uh oh.");

        let error = Error::client_with_root_cause(anyhow!("Something sensitive").context("Uh oh."));

        assert_eq!(error.kind, ErrorKind::ClientError);
        assert_debug_snapshot!(error, @r###"
        Error {
            context: "Uh oh.",
            source: "Something sensitive",
        }
        "###);

        assert_eq!(error.status_code(), StatusCode::BAD_REQUEST);

        let error_response = error.error_response();
        assert_debug_snapshot!(error_response, @r###"
        HttpResponse {
            error: None,
            res: 
            Response HTTP/1.1 400 Bad Request
              headers:
              body: Sized(6)
            ,
        }
        "###);
        let body = error_response.into_body().try_into_bytes().unwrap();
        assert_eq!(body.as_ref(), b"Uh oh.");

        Ok(())
    }

    #[test]
    fn can_create_unknown_errors() -> anyhow::Result<()> {
        let error = Error::from(anyhow!("Something sensitive"));

        assert_eq!(error.kind, ErrorKind::Unknown);
        assert_debug_snapshot!(error, @r###""Something sensitive""###);

        assert_eq!(error.status_code(), StatusCode::INTERNAL_SERVER_ERROR);

        let error_response = error.error_response();
        assert_debug_snapshot!(error_response, @r###"
        HttpResponse {
            error: None,
            res: 
            Response HTTP/1.1 500 Internal Server Error
              headers:
              body: Sized(21)
            ,
        }
        "###);
        let body = error_response.into_body().try_into_bytes().unwrap();
        assert_eq!(body.as_ref(), b"Internal Server Error");

        Ok(())
    }

    #[test]
    fn can_recover_original_error() -> anyhow::Result<()> {
        let client_error =
            Error::client_with_root_cause(anyhow!("One").context("Two").context("Three"));
        let error = Error::from(anyhow!(client_error).context("Four"));

        assert_eq!(error.kind, ErrorKind::ClientError);
        assert_debug_snapshot!(error, @r###"
        Error {
            context: "Three",
            source: Error {
                context: "Two",
                source: "One",
            },
        }
        "###);

        assert_eq!(error.status_code(), StatusCode::BAD_REQUEST);

        let error_response = error.error_response();
        assert_debug_snapshot!(error_response, @r###"
        HttpResponse {
            error: None,
            res: 
            Response HTTP/1.1 400 Bad Request
              headers:
              body: Sized(5)
            ,
        }
        "###);
        let body = error_response.into_body().try_into_bytes().unwrap();
        assert_eq!(body.as_ref(), b"Three");

        Ok(())
    }
}
