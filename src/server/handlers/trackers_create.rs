use crate::{error::Error as RetrackError, server::ServerState, trackers::TrackerCreateParams};
use actix_web::{post, web, HttpResponse};
use tracing::error;

/// Creates a new tracker with the specified parameters.
#[utoipa::path(
    tags = ["trackers"],
    request_body = TrackerCreateParams,
    responses(
        (status = 200, description = "Tracker was successfully created.", body = Tracker),
        (status = BAD_REQUEST, description = "Cannot create a tracker with the specified properties.")
    )
)]
#[post("/api/trackers")]
pub async fn trackers_create(
    state: web::Data<ServerState>,
    params: web::Json<TrackerCreateParams>,
) -> Result<HttpResponse, RetrackError> {
    let trackers = state.api.trackers();
    match trackers.create_tracker(params.into_inner()).await {
        Ok(tracker) => Ok(HttpResponse::Ok().json(tracker)),
        Err(err) => {
            error!("Failed to create tracker: {err:?}");
            Err(err.into())
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        config::{Config, TrackersConfig},
        scheduler::{SchedulerJobConfig, SchedulerJobRetryStrategy},
        server::{
            handlers::trackers_create::trackers_create,
            server_state::tests::{mock_server_state, mock_server_state_with_config},
        },
        tests::mock_config,
        trackers::{TrackerTarget, WebPageTarget},
    };
    use actix_web::{
        body::MessageBody,
        http::Method,
        test::{call_service, init_service, TestRequest},
        web, App,
    };
    use serde_json::json;
    use sqlx::PgPool;
    use std::{str::from_utf8, time::Duration};

    #[sqlx::test]
    async fn can_create_tracker(pool: PgPool) -> anyhow::Result<()> {
        let server_state = web::Data::new(mock_server_state(pool).await?);
        let app = init_service(
            App::new()
                .app_data(server_state.clone())
                .service(trackers_create),
        )
        .await;

        let response = call_service(
            &app,
            TestRequest::with_uri("https://retrack.dev/api/trackers")
                .method(Method::POST)
                .set_json(json!({ "name": "my-minimal-tracker".to_string(), "url": "https://retrack.dev/app"}))
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), 200);

        let trackers = server_state.api.trackers().get_trackers().await?;
        assert_eq!(trackers.len(), 1);
        assert_eq!(trackers[0].name, "my-minimal-tracker");
        assert_eq!(trackers[0].url, "https://retrack.dev/app".parse()?);
        assert_eq!(trackers[0].config.revisions, 3);
        assert_eq!(
            trackers[0].target,
            TrackerTarget::WebPage(Default::default())
        );

        assert_eq!(
            serde_json::to_string(&trackers[0])?,
            from_utf8(&response.into_body().try_into_bytes().unwrap())?
        );

        Ok(())
    }

    #[sqlx::test]
    async fn can_create_tracker_with_optional_params(pool: PgPool) -> anyhow::Result<()> {
        let server_state = web::Data::new(mock_server_state(pool).await?);
        let app = init_service(
            App::new()
                .app_data(server_state.clone())
                .service(trackers_create),
        )
        .await;

        let response = call_service(
            &app,
            TestRequest::with_uri("https://retrack.dev/api/trackers")
                .method(Method::POST)
                .set_json(json!({
                    "name": "my-minimal-tracker".to_string(),
                    "url": "https://retrack.dev/app",
                    "target": {
                        "type": "web:page",
                        "delay": 5000,
                        "waitFor": "div"
                    },
                    "config": {
                        "revisions": 5,
                        "extractor": "return document.body.innerHTML;",
                        "headers": {
                            "cookie": "my-cookie"
                        },
                        "job": {
                            "schedule": "@daily",
                            "retryStrategy": {
                                "type": "constant",
                                "interval": 500000,
                                "maxAttempts": 5
                            },
                            "notifications": true
                        }
                    }
                }))
                .to_request(),
        )
        .await;

        let status = response.status();
        let body = response.into_body().try_into_bytes().unwrap();

        assert_eq!(status, 200);

        let trackers = server_state.api.trackers().get_trackers().await?;
        assert_eq!(trackers.len(), 1);
        assert_eq!(trackers[0].name, "my-minimal-tracker");
        assert_eq!(trackers[0].url, "https://retrack.dev/app".parse()?);
        assert_eq!(trackers[0].config.revisions, 5);
        assert_eq!(
            trackers[0].target,
            TrackerTarget::WebPage(WebPageTarget {
                delay: Some(Duration::from_millis(5000)),
                wait_for: Some("div".parse()?),
            })
        );
        assert_eq!(
            trackers[0].config.extractor,
            Some("return document.body.innerHTML;".to_string())
        );
        assert_eq!(
            trackers[0].config.headers,
            Some(
                [("cookie".to_string(), "my-cookie".to_string())]
                    .iter()
                    .cloned()
                    .collect()
            )
        );
        assert_eq!(
            trackers[0].config.job,
            Some(SchedulerJobConfig {
                schedule: "@daily".to_string(),
                retry_strategy: Some(SchedulerJobRetryStrategy::Constant {
                    interval: Duration::from_secs(500),
                    max_attempts: 5
                }),
                notifications: Some(true),
            })
        );
        assert_eq!(serde_json::to_string(&trackers[0])?, from_utf8(&body)?);

        Ok(())
    }

    #[sqlx::test]
    async fn fails_with_bad_request_for_invalid_params(pool: PgPool) -> anyhow::Result<()> {
        let server_state = web::Data::new(
            mock_server_state_with_config(
                pool,
                Config {
                    trackers: TrackersConfig {
                        restrict_to_public_urls: true,
                        ..Default::default()
                    },
                    ..mock_config()?
                },
            )
            .await?,
        );
        let app = init_service(
            App::new()
                .app_data(server_state.clone())
                .service(trackers_create),
        )
        .await;

        let response = call_service(
            &app,
            TestRequest::with_uri("https://retrack.dev/api/trackers")
                .method(Method::POST)
                .set_json(json!({
                    "name": "my-minimal-tracker".to_string(),
                    "url": "https://127.0.0.1/app",
                    "config": {
                        "revisions": 5,
                        "delay": 5000

                    }
                }))
                .to_request(),
        )
        .await;

        assert_eq!(response.status(), 400);
        assert_eq!(
            from_utf8(&response.into_body().try_into_bytes().unwrap()).unwrap(),
            r###"{"message":"Tracker URL must be either `http` or `https` and have a valid public reachable domain name, but received https://127.0.0.1/app."}"###
        );
        assert!(server_state.api.trackers().get_trackers().await?.is_empty());

        Ok(())
    }
}
