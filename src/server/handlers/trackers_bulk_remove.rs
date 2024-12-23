use crate::{error::Error as RetrackError, server::ServerState};
use actix_web::{delete, web, HttpResponse};
use actix_web_lab::extract::Query;
use retrack_types::trackers::TrackersListParams;
use tracing::error;

/// Removes a list of trackers.
#[utoipa::path(
    tags = ["trackers"],
    params(TrackersListParams),
    responses(
        (status = OK, description = "A number of removed trackers.", body = u64),
    )
)]
#[delete("/api/trackers")]
pub async fn trackers_bulk_remove(
    state: web::Data<ServerState>,
    params: Query<TrackersListParams>,
) -> Result<HttpResponse, RetrackError> {
    let trackers = state.api.trackers();
    match trackers.remove_trackers(params.into_inner()).await {
        Ok(trackers_removed) => Ok(HttpResponse::Ok().json(trackers_removed)),
        Err(err) => {
            error!("Failed to remove trackers: {err:?}");
            Err(err.into())
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        server::{
            handlers::trackers_bulk_remove::trackers_bulk_remove,
            server_state::tests::mock_server_state,
        },
        tests::TrackerCreateParamsBuilder,
    };
    use actix_web::{
        body::MessageBody,
        http::Method,
        test::{call_service, init_service, TestRequest},
        web, App,
    };
    use retrack_types::trackers::{EmailAction, TrackerAction};
    use sqlx::PgPool;
    use std::str::from_utf8;

    #[sqlx::test]
    async fn can_remove_all_trackers(pool: PgPool) -> anyhow::Result<()> {
        let server_state = web::Data::new(mock_server_state(pool).await?);
        let app = init_service(
            App::new()
                .app_data(server_state.clone())
                .service(trackers_bulk_remove),
        )
        .await;

        let response = call_service(
            &app,
            TestRequest::with_uri("https://retrack.dev/api/trackers")
                .method(Method::DELETE)
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), 200);
        assert_eq!(
            from_utf8(&response.into_body().try_into_bytes().unwrap())?,
            "0"
        );

        // Create tracker.
        server_state
            .api
            .trackers()
            .create_tracker(TrackerCreateParamsBuilder::new("name_one").build())
            .await?;

        // Create another tracker.
        server_state
            .api
            .trackers()
            .create_tracker(
                TrackerCreateParamsBuilder::new("name_two")
                    .with_tags(vec!["tag_two".to_string()])
                    .with_actions(vec![TrackerAction::Email(EmailAction {
                        to: vec!["dev@retrack.dev".to_string()],
                    })])
                    .build(),
            )
            .await?;

        let trackers = server_state
            .api
            .trackers()
            .get_trackers(Default::default())
            .await?;
        assert_eq!(trackers.len(), 2);

        let response = call_service(
            &app,
            TestRequest::with_uri("https://retrack.dev/api/trackers")
                .method(Method::DELETE)
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), 200);
        assert_eq!(
            from_utf8(&response.into_body().try_into_bytes().unwrap())?,
            "2"
        );

        let trackers = server_state
            .api
            .trackers()
            .get_trackers(Default::default())
            .await?;
        assert!(trackers.is_empty());

        Ok(())
    }

    #[sqlx::test]
    async fn can_remove_trackers_with_tags(pool: PgPool) -> anyhow::Result<()> {
        let server_state = web::Data::new(mock_server_state(pool).await?);
        let app = init_service(
            App::new()
                .app_data(server_state.clone())
                .service(trackers_bulk_remove),
        )
        .await;

        let response = call_service(
            &app,
            TestRequest::with_uri("https://retrack.dev/api/trackers?tag=app:retrack")
                .method(Method::DELETE)
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), 200);
        assert_eq!(
            from_utf8(&response.into_body().try_into_bytes().unwrap())?,
            "0"
        );

        // Create tracker.
        server_state
            .api
            .trackers()
            .create_tracker(
                TrackerCreateParamsBuilder::new("name_one")
                    .with_tags(vec!["app:retrack".to_string(), "User:1".to_string()])
                    .build(),
            )
            .await?;

        // Create another tracker.
        server_state
            .api
            .trackers()
            .create_tracker(
                TrackerCreateParamsBuilder::new("name_two")
                    .with_tags(vec!["app:retrack".to_string(), "User:2".to_string()])
                    .build(),
            )
            .await?;

        let trackers = server_state
            .api
            .trackers()
            .get_trackers(Default::default())
            .await?;
        assert_eq!(trackers.len(), 2);

        let response = call_service(
            &app,
            TestRequest::with_uri("https://retrack.dev/api/trackers?tag=user:1")
                .method(Method::DELETE)
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), 200);
        assert_eq!(
            from_utf8(&response.into_body().try_into_bytes().unwrap())?,
            "1"
        );

        let response = call_service(
            &app,
            TestRequest::with_uri("https://retrack.dev/api/trackers?tag=user:1&tag=app:retrack")
                .method(Method::DELETE)
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), 200);
        assert_eq!(
            from_utf8(&response.into_body().try_into_bytes().unwrap())?,
            "0"
        );

        let trackers = server_state
            .api
            .trackers()
            .get_trackers(Default::default())
            .await?;
        assert_eq!(trackers.len(), 1);

        let response = call_service(
            &app,
            TestRequest::with_uri("https://retrack.dev/api/trackers?tag=USER:2")
                .method(Method::DELETE)
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), 200);
        assert_eq!(
            from_utf8(&response.into_body().try_into_bytes().unwrap())?,
            "1"
        );

        let trackers = server_state
            .api
            .trackers()
            .get_trackers(Default::default())
            .await?;
        assert!(trackers.is_empty());
        assert!(trackers.is_empty());

        Ok(())
    }
}
