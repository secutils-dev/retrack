use crate::{error::Error as RetrackError, server::ServerState};
use actix_web::{get, web, HttpResponse};
use anyhow::anyhow;
use std::ops::Deref;
use tracing::error;

/// Gets server status.
#[utoipa::path(
    tags = ["platform"],
    responses(
        (status = 200, body = Status)
    )
)]
#[get("/api/status")]
pub async fn status_get(state: web::Data<ServerState>) -> Result<HttpResponse, RetrackError> {
    state
        .status
        .read()
        .map(|status| HttpResponse::Ok().json(status.deref()))
        .map_err(|err| {
            error!("Failed to read status: {err}");
            RetrackError::from(anyhow!("Status is not available."))
        })
}

#[cfg(test)]
mod tests {
    use crate::server::{handlers::status_get::status_get, server_state::tests::mock_server_state};
    use actix_web::{
        body::MessageBody,
        test::{call_service, init_service, TestRequest},
        web, App,
    };
    use insta::assert_snapshot;
    use sqlx::PgPool;
    use std::str::from_utf8;

    #[sqlx::test]
    async fn can_return_status(pool: PgPool) -> anyhow::Result<()> {
        let app = init_service(
            App::new()
                .app_data(web::Data::new(mock_server_state(pool).await?))
                .service(status_get),
        )
        .await;

        let response = call_service(
            &app,
            TestRequest::with_uri("https://retrack.dev/api/status").to_request(),
        )
        .await;
        assert_eq!(response.status(), 200);

        let body = response.into_body().try_into_bytes().unwrap();
        assert_snapshot!(from_utf8(&body).unwrap(), @r###"{"version":"0.0.1"}"###);

        Ok(())
    }
}
