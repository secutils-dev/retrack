use crate::{error::Error as RetrackError, server::ServerState, trackers::TrackersListParams};
use actix_web::{get, web, HttpResponse};
use actix_web_lab::extract::Query;
use retrack_types::trackers::Tracker;
use tracing::error;

/// Gets a list of active trackers.
#[utoipa::path(
    tags = ["trackers"],
    params(TrackersListParams),
    responses(
        (status = 200, description = "A list of currently active trackers, optionally filtered by the specified tags.", body = [Tracker])
    )
)]
#[get("/api/trackers")]
pub async fn trackers_list(
    state: web::Data<ServerState>,
    params: Query<TrackersListParams>,
) -> Result<HttpResponse, RetrackError> {
    match state.api.trackers().get_trackers(params.into_inner()).await {
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
        trackers::TrackerCreateParams,
    };
    use actix_web::{
        body::MessageBody,
        test::{call_service, init_service, TestRequest},
        web, App,
    };
    use retrack_types::trackers::{EmailAction, TrackerAction};
    use sqlx::PgPool;
    use std::str::from_utf8;

    #[sqlx::test]
    async fn can_list_trackers(pool: PgPool) -> anyhow::Result<()> {
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
            .create_tracker(TrackerCreateParams::new("name_one"))
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
            .create_tracker(
                TrackerCreateParams::new("name_two")
                    .with_tags(vec!["tag_two".to_string()])
                    .with_actions(vec![TrackerAction::Email(EmailAction {
                        to: vec!["dev@retrack.dev".to_string()],
                    })]),
            )
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

    #[sqlx::test]
    async fn can_list_trackers_with_tags(pool: PgPool) -> anyhow::Result<()> {
        let server_state = web::Data::new(mock_server_state(pool).await?);
        let app = init_service(
            App::new()
                .app_data(server_state.clone())
                .service(trackers_list),
        )
        .await;

        let response = call_service(
            &app,
            TestRequest::with_uri("https://retrack.dev/api/trackers?tag=app:retrack").to_request(),
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
            .create_tracker(
                TrackerCreateParams::new("name_one")
                    .with_tags(vec!["app:retrack".to_string(), "User:1".to_string()]),
            )
            .await?;

        // Create another tracker.
        let tracker_2 = server_state
            .api
            .trackers()
            .create_tracker(
                TrackerCreateParams::new("name_two")
                    .with_tags(vec!["app:retrack".to_string(), "User:2".to_string()]),
            )
            .await?;

        let response = call_service(
            &app,
            TestRequest::with_uri("https://retrack.dev/api/trackers?tag=user:1").to_request(),
        )
        .await;
        assert_eq!(response.status(), 200);
        assert_eq!(
            serde_json::to_string(&[&tracker_1])?,
            from_utf8(&response.into_body().try_into_bytes().unwrap())?
        );

        let response = call_service(
            &app,
            TestRequest::with_uri("https://retrack.dev/api/trackers?tag=user:1&tag=app:retrack")
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), 200);
        assert_eq!(
            serde_json::to_string(&[&tracker_1])?,
            from_utf8(&response.into_body().try_into_bytes().unwrap())?
        );

        let response = call_service(
            &app,
            TestRequest::with_uri("https://retrack.dev/api/trackers?tag=USER:2").to_request(),
        )
        .await;
        assert_eq!(response.status(), 200);
        assert_eq!(
            serde_json::to_string(&[&tracker_2])?,
            from_utf8(&response.into_body().try_into_bytes().unwrap())?
        );

        let response = call_service(
            &app,
            TestRequest::with_uri("https://retrack.dev/api/trackers?tag=user:2&tag=app:retrack")
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), 200);
        assert_eq!(
            serde_json::to_string(&[&tracker_2])?,
            from_utf8(&response.into_body().try_into_bytes().unwrap())?
        );

        let response = call_service(
            &app,
            TestRequest::with_uri("https://retrack.dev/api/trackers?tag=App:retrack").to_request(),
        )
        .await;
        assert_eq!(response.status(), 200);
        assert_eq!(
            serde_json::to_string(&[&tracker_1, &tracker_2])?,
            from_utf8(&response.into_body().try_into_bytes().unwrap())?
        );

        let response = call_service(
            &app,
            TestRequest::with_uri("https://retrack.dev/api/trackers?tag=app:retrack").to_request(),
        )
        .await;
        assert_eq!(response.status(), 200);
        assert_eq!(
            serde_json::to_string(&[tracker_1, tracker_2])?,
            from_utf8(&response.into_body().try_into_bytes().unwrap())?
        );

        let response = call_service(
            &app,
            TestRequest::with_uri("https://retrack.dev/api/trackers?tag=app:unknown").to_request(),
        )
        .await;
        assert_eq!(response.status(), 200);
        assert_eq!(
            from_utf8(&response.into_body().try_into_bytes().unwrap())?,
            "[]"
        );

        Ok(())
    }
}
