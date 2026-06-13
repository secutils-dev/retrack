use crate::{error::Error as RetrackError, server::ServerState};
use actix_web::{HttpResponse, get, web};
use actix_web_lab::extract::Query;
use retrack_types::trackers::{Page, Tracker, TrackersListParams};
use tracing::error;

/// Gets a list of active trackers.
#[utoipa::path(
    tags = ["trackers"],
    params(TrackersListParams),
    responses(
        (status = 200, description = "A page of currently active trackers, optionally filtered by the specified tags.", body = Page<Tracker>)
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
        tests::TrackerCreateParamsBuilder,
    };
    use actix_web::{
        App,
        body::MessageBody,
        test::{TestRequest, call_service, init_service},
        web,
    };
    use retrack_types::trackers::{EmailAction, Page, Tracker, TrackerAction};
    use sqlx::PgPool;

    fn response_body(body: impl MessageBody) -> anyhow::Result<serde_json::Value> {
        let bytes = body
            .try_into_bytes()
            .map_err(|_| anyhow::anyhow!("Failed to read response body."))?;
        Ok(serde_json::from_slice(&bytes)?)
    }

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
            response_body(response.into_body())?,
            serde_json::to_value(Page::<Tracker>::new(vec![], 0))?
        );

        // Create tracker.
        let tracker_1 = server_state
            .api
            .trackers()
            .create_tracker(TrackerCreateParamsBuilder::new("name_one").build())
            .await?;

        let response = call_service(
            &app,
            TestRequest::with_uri("https://retrack.dev/api/trackers").to_request(),
        )
        .await;
        assert_eq!(response.status(), 200);
        assert_eq!(
            response_body(response.into_body())?,
            serde_json::to_value(Page::new(vec![tracker_1.clone()], 1))?
        );

        // Create another tracker.
        let tracker_2 = server_state
            .api
            .trackers()
            .create_tracker(
                TrackerCreateParamsBuilder::new("name_two")
                    .with_tags(vec!["tag_two".to_string()])
                    .with_actions(vec![TrackerAction::Email(EmailAction {
                        to: vec!["dev@retrack.dev".to_string()],
                        formatter: None,
                    })])
                    .build(),
            )
            .await?;

        let response = call_service(
            &app,
            TestRequest::with_uri("https://retrack.dev/api/trackers").to_request(),
        )
        .await;
        assert_eq!(response.status(), 200);
        assert_eq!(
            response_body(response.into_body())?,
            serde_json::to_value(Page::new(vec![tracker_1, tracker_2], 2))?
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
            response_body(response.into_body())?,
            serde_json::to_value(Page::<Tracker>::new(vec![], 0))?
        );

        // Create tracker.
        let tracker_1 = server_state
            .api
            .trackers()
            .create_tracker(
                TrackerCreateParamsBuilder::new("name_one")
                    .with_tags(vec!["app:retrack".to_string(), "User:1".to_string()])
                    .build(),
            )
            .await?;

        // Create another tracker.
        let tracker_2 = server_state
            .api
            .trackers()
            .create_tracker(
                TrackerCreateParamsBuilder::new("name_two")
                    .with_tags(vec!["app:retrack".to_string(), "User:2".to_string()])
                    .build(),
            )
            .await?;

        let response = call_service(
            &app,
            TestRequest::with_uri("https://retrack.dev/api/trackers?tag=user:1").to_request(),
        )
        .await;
        assert_eq!(response.status(), 200);
        assert_eq!(
            response_body(response.into_body())?,
            serde_json::to_value(Page::new(vec![tracker_1.clone()], 1))?
        );

        let response = call_service(
            &app,
            TestRequest::with_uri("https://retrack.dev/api/trackers?tag=user:1&tag=app:retrack")
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), 200);
        assert_eq!(
            response_body(response.into_body())?,
            serde_json::to_value(Page::new(vec![tracker_1.clone()], 1))?
        );

        let response = call_service(
            &app,
            TestRequest::with_uri("https://retrack.dev/api/trackers?tag=USER:2").to_request(),
        )
        .await;
        assert_eq!(response.status(), 200);
        assert_eq!(
            response_body(response.into_body())?,
            serde_json::to_value(Page::new(vec![tracker_2.clone()], 1))?
        );

        let response = call_service(
            &app,
            TestRequest::with_uri("https://retrack.dev/api/trackers?tag=user:2&tag=app:retrack")
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), 200);
        assert_eq!(
            response_body(response.into_body())?,
            serde_json::to_value(Page::new(vec![tracker_2.clone()], 1))?
        );

        let response = call_service(
            &app,
            TestRequest::with_uri("https://retrack.dev/api/trackers?tag=App:retrack").to_request(),
        )
        .await;
        assert_eq!(response.status(), 200);
        assert_eq!(
            response_body(response.into_body())?,
            serde_json::to_value(Page::new(vec![tracker_1.clone(), tracker_2.clone()], 2))?
        );

        let response = call_service(
            &app,
            TestRequest::with_uri("https://retrack.dev/api/trackers?tag=app:retrack").to_request(),
        )
        .await;
        assert_eq!(response.status(), 200);
        assert_eq!(
            response_body(response.into_body())?,
            serde_json::to_value(Page::new(vec![tracker_1, tracker_2], 2))?
        );

        let response = call_service(
            &app,
            TestRequest::with_uri("https://retrack.dev/api/trackers?tag=app:unknown").to_request(),
        )
        .await;
        assert_eq!(response.status(), 200);
        assert_eq!(
            response_body(response.into_body())?,
            serde_json::to_value(Page::<Tracker>::new(vec![], 0))?
        );

        Ok(())
    }

    #[sqlx::test]
    async fn can_list_trackers_with_pagination_params(pool: PgPool) -> anyhow::Result<()> {
        let server_state = web::Data::new(mock_server_state(pool).await?);
        let app = init_service(
            App::new()
                .app_data(server_state.clone())
                .service(trackers_list),
        )
        .await;

        let tracker_1 = server_state
            .api
            .trackers()
            .create_tracker(TrackerCreateParamsBuilder::new("name_one").build())
            .await?;
        let tracker_2 = server_state
            .api
            .trackers()
            .create_tracker(TrackerCreateParamsBuilder::new("name_two").build())
            .await?;

        let response = call_service(
            &app,
            TestRequest::with_uri(
                "https://retrack.dev/api/trackers?page=1&pageSize=1&sort=name&order=asc",
            )
            .to_request(),
        )
        .await;
        assert_eq!(response.status(), 200);
        assert_eq!(
            response_body(response.into_body())?,
            serde_json::to_value(Page::new(vec![tracker_2], 2))?
        );

        let response = call_service(
            &app,
            TestRequest::with_uri("https://retrack.dev/api/trackers?q=name_one").to_request(),
        )
        .await;
        assert_eq!(response.status(), 200);
        assert_eq!(
            response_body(response.into_body())?,
            serde_json::to_value(Page::new(vec![tracker_1], 1))?
        );

        let response = call_service(
            &app,
            TestRequest::with_uri("https://retrack.dev/api/trackers?sort=notAField").to_request(),
        )
        .await;
        assert_eq!(response.status(), 422);

        Ok(())
    }
}
