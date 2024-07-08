use crate::{error::Error as RetrackError, server::ServerState};
use actix_web::{get, web, HttpResponse};
use tracing::error;

/// Gets a list of active trackers.
#[utoipa::path(
    tags = ["trackers"],
    responses(
        (status = 200, description = "A list of currently active trackers.", body = [Tracker])
    )
)]
#[get("/api/trackers")]
pub async fn trackers_list(state: web::Data<ServerState>) -> Result<HttpResponse, RetrackError> {
    match state.api.trackers().get_trackers().await {
        Ok(trackers) => Ok(HttpResponse::Ok().json(trackers)),
        Err(err) => {
            error!("Failed to retrieve trackers: {err:?}");
            Err(err.into())
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        server::{handlers::trackers_list::trackers_list, server_state::tests::mock_server_state},
        trackers::{TrackerCreateParams, TrackerSettings},
    };
    use actix_web::{
        body::MessageBody,
        test::{call_service, init_service, TestRequest},
        web, App,
    };
    use sqlx::PgPool;
    use std::{str::from_utf8, time::Duration};
    use url::Url;

    #[sqlx::test]
    async fn can_list_tracker(pool: PgPool) -> anyhow::Result<()> {
        let server_state = web::Data::new(mock_server_state(pool).await?);
        let app = init_service(
            App::new()
                .app_data(server_state.clone())
                .service(trackers_list),
        )
        .await;

        let response = call_service(
            &app,
            TestRequest::with_uri("https://retrack.dev/api/trackers").to_request(),
        )
        .await;
        assert_eq!(response.status(), 200);
        assert_eq!(
            from_utf8(&response.into_body().try_into_bytes().unwrap())?,
            "[]"
        );

        // Create tracker.
        let tracker_1 = server_state
            .api
            .trackers()
            .create_tracker(TrackerCreateParams {
                name: "name_one".to_string(),
                url: Url::parse("http://localhost:1234/my/app?q=2")?,
                settings: TrackerSettings {
                    revisions: 3,
                    delay: Duration::from_millis(2000),
                    extractor: Default::default(),
                    headers: Default::default(),
                },
                job_config: None,
            })
            .await?;

        let response = call_service(
            &app,
            TestRequest::with_uri("https://retrack.dev/api/trackers").to_request(),
        )
        .await;
        assert_eq!(response.status(), 200);
        assert_eq!(
            serde_json::to_string(&[&tracker_1])?,
            from_utf8(&response.into_body().try_into_bytes().unwrap())?
        );

        // Create another tracker.
        let tracker_2 = server_state
            .api
            .trackers()
            .create_tracker(TrackerCreateParams {
                name: "name_two".to_string(),
                url: Url::parse("http://localhost:1234/my/app?q=2")?,
                settings: TrackerSettings {
                    revisions: 3,
                    delay: Duration::from_millis(2000),
                    extractor: Default::default(),
                    headers: Default::default(),
                },
                job_config: None,
            })
            .await?;

        let response = call_service(
            &app,
            TestRequest::with_uri("https://retrack.dev/api/trackers").to_request(),
        )
        .await;
        assert_eq!(response.status(), 200);
        assert_eq!(
            serde_json::to_string(&[tracker_1, tracker_2])?,
            from_utf8(&response.into_body().try_into_bytes().unwrap())?
        );

        Ok(())
    }
}
