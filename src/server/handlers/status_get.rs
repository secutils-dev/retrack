use crate::{
    error::Error as RetrackError,
    server::{GetStatusParams, ServerState, Status},
};
use actix_web::{HttpResponse, get, web};
use tracing::error;

/// Gets server status.
#[utoipa::path(
    tags = ["platform"],
    params(GetStatusParams),
    responses(
        (status = 200, body = Status, description = "server status retrieved successfully"),
        (status = 500, description = "internal server error, server might not be operational"),
    )
)]
#[get("/api/status")]
pub async fn status_get(
    state: web::Data<ServerState>,
    params: web::Query<GetStatusParams>,
) -> Result<HttpResponse, RetrackError> {
    let status = state.status().await?;

    if !status.is_operational() {
        error!(
            status.scheduler.operational = status.scheduler.operational,
            "Server is not fully operational."
        );

        if params.fail_if_not_operational {
            return Ok(HttpResponse::InternalServerError().finish());
        }
    }

    Ok(HttpResponse::Ok().json(status))
}

#[cfg(test)]
mod tests {
    use crate::server::{handlers::status_get::status_get, server_state::tests::mock_server_state};
    use actix_web::{
        App,
        body::MessageBody,
        test::{TestRequest, call_service, init_service},
        web,
    };
    use insta::assert_snapshot;
    use sqlx::PgPool;
    use std::str::from_utf8;

    #[sqlx::test]
    async fn can_return_status(pool: PgPool) -> anyhow::Result<()> {
        let app = init_service(
            App::new()
                .app_data(web::Data::new(mock_server_state(pool.clone()).await?))
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
        assert_snapshot!(from_utf8(&body)?, @r###"{"version":"0.0.1","scheduler":{"operational":true}}"###);

        Ok(())
    }
}
