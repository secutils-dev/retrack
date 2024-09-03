use crate::{
    error::Error as RetrackError, server::ServerState, trackers::TrackerListRevisionsParams,
};
use actix_web::{get, web, HttpResponse};
use tracing::error;
use uuid::Uuid;

/// Gets a list of revisions for a tracker with the specified ID.
#[utoipa::path(
    tags = ["trackers"],
    params(
        ("tracker_id" = Uuid, Path, description = "A unique tracker ID."),
        TrackerListRevisionsParams
    ),
    responses(
        (status = 200, description = "A list of currently active trackers.", body = [TrackerDataRevision]),
        (status = BAD_REQUEST, description = "Cannot list revisions for a tracker with the specified parameters.")
    )
)]
#[get("/api/trackers/{tracker_id}/revisions")]
pub async fn trackers_list_revisions(
    state: web::Data<ServerState>,
    tracker_id: web::Path<Uuid>,
    params: web::Query<TrackerListRevisionsParams>,
) -> Result<HttpResponse, RetrackError> {
    let trackers = state.api.trackers();
    match trackers
        .get_tracker_data(*tracker_id, params.into_inner())
        .await
    {
        Ok(revisions) => Ok(HttpResponse::Ok().json(revisions)),
        Err(err) => {
            error!("Failed to retrieve tracker data revisions: {err:?}");
            Err(err.into())
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        server::{
            handlers::trackers_list_revisions::trackers_list_revisions,
            server_state::tests::mock_server_state,
        },
        trackers::{
            TrackerConfig, TrackerCreateParams, TrackerDataRevision, TrackerTarget, WebPageTarget,
        },
    };
    use actix_web::{
        body::MessageBody,
        test::{call_service, init_service, TestRequest},
        web, App,
    };
    use insta::assert_debug_snapshot;
    use sqlx::PgPool;
    use std::{str::from_utf8, time::Duration};
    use time::OffsetDateTime;
    use uuid::uuid;

    #[sqlx::test]
    async fn can_list_tracker_data(pool: PgPool) -> anyhow::Result<()> {
        let server_state = web::Data::new(mock_server_state(pool).await?);

        // Create tracker.
        let trackers_api = server_state.api.trackers();
        let tracker = trackers_api
            .create_tracker(TrackerCreateParams {
                name: "name_one".to_string(),
                target: TrackerTarget::WebPage(WebPageTarget {
                    extractor: "export async function execute(p, r) { await p.goto('https://retrack.dev/'); return r.html(await p.content()); }".to_string(),
                    user_agent: Some("Retrack/1.0.0".to_string()),
                    ignore_https_errors: true,
                }),
                config: TrackerConfig {
                    revisions: 3,
                    timeout: Some(Duration::from_millis(2000)),
                    headers: Default::default(),
                    job: None,
                },
                tags: vec!["tag".to_string()],
            })
            .await?;

        let app = init_service(
            App::new()
                .app_data(server_state.clone())
                .service(trackers_list_revisions),
        )
        .await;

        // No data yet.
        let response = call_service(
            &app,
            TestRequest::with_uri(&format!(
                "https://retrack.dev/api/trackers/{}/revisions",
                tracker.id
            ))
            .to_request(),
        )
        .await;
        assert_eq!(response.status(), 200);
        assert_eq!(
            from_utf8(&response.into_body().try_into_bytes().unwrap())?,
            "[]"
        );

        // Add tracker data revision.
        let trackers_db = server_state.api.db.trackers();
        let data_revision_one = TrackerDataRevision {
            id: uuid!("00000000-0000-0000-0000-000000000001"),
            tracker_id: tracker.id,
            created_at: OffsetDateTime::from_unix_timestamp(946720800)?,
            data: "\"some-data\"".to_string(),
        };
        trackers_db
            .insert_tracker_data_revision(&data_revision_one)
            .await?;
        let response = call_service(
            &app,
            TestRequest::with_uri(&format!(
                "https://retrack.dev/api/trackers/{}/revisions",
                tracker.id
            ))
            .to_request(),
        )
        .await;
        assert_eq!(response.status(), 200);
        assert_debug_snapshot!(from_utf8(&response.into_body().try_into_bytes().unwrap())?, @r###""[{\"id\":\"00000000-0000-0000-0000-000000000001\",\"data\":\"\\\"some-data\\\"\",\"createdAt\":946720800}]""###);

        // Add another revision
        let data_revision_two = TrackerDataRevision {
            id: uuid!("00000000-0000-0000-0000-000000000002"),
            tracker_id: tracker.id,
            created_at: OffsetDateTime::from_unix_timestamp(946720900)?,
            data: "\"some-new-data\"".to_string(),
        };
        trackers_db
            .insert_tracker_data_revision(&data_revision_two)
            .await?;
        let response = call_service(
            &app,
            TestRequest::with_uri(&format!(
                "https://retrack.dev/api/trackers/{}/revisions",
                tracker.id
            ))
            .to_request(),
        )
        .await;
        assert_eq!(response.status(), 200);
        assert_debug_snapshot!(from_utf8(&response.into_body().try_into_bytes().unwrap())?, @r###""[{\"id\":\"00000000-0000-0000-0000-000000000001\",\"data\":\"\\\"some-data\\\"\",\"createdAt\":946720800},{\"id\":\"00000000-0000-0000-0000-000000000002\",\"data\":\"\\\"some-new-data\\\"\",\"createdAt\":946720900}]""###);

        let response = call_service(
            &app,
            TestRequest::with_uri(&format!(
                "https://retrack.dev/api/trackers/{}/revisions",
                tracker.id
            ))
            .to_request(),
        )
        .await;
        assert_eq!(response.status(), 200);
        assert_debug_snapshot!(from_utf8(&response.into_body().try_into_bytes().unwrap())?, @r###""[{\"id\":\"00000000-0000-0000-0000-000000000001\",\"data\":\"\\\"some-data\\\"\",\"createdAt\":946720800},{\"id\":\"00000000-0000-0000-0000-000000000002\",\"data\":\"\\\"some-new-data\\\"\",\"createdAt\":946720900}]""###);

        // Calculate the difference between the two revisions
        let response = call_service(
            &app,
            TestRequest::with_uri(&format!(
                "https://retrack.dev/api/trackers/{}/revisions?calculateDiff=true",
                tracker.id
            ))
            .to_request(),
        )
        .await;
        assert_eq!(response.status(), 200);
        assert_debug_snapshot!(from_utf8(&response.into_body().try_into_bytes().unwrap())?, @r###""[{\"id\":\"00000000-0000-0000-0000-000000000001\",\"data\":\"\\\"some-data\\\"\",\"createdAt\":946720800},{\"id\":\"00000000-0000-0000-0000-000000000002\",\"data\":\"@@ -1 +1 @@\\n-some-data\\n+some-new-data\\n\",\"createdAt\":946720900}]""###);

        Ok(())
    }

    #[sqlx::test]
    async fn fails_with_bad_request_for_unknown_trackers(pool: PgPool) -> anyhow::Result<()> {
        let server_state = web::Data::new(mock_server_state(pool).await?);
        let app = init_service(
            App::new()
                .app_data(server_state.clone())
                .service(trackers_list_revisions),
        )
        .await;

        let response = call_service(
            &app,
            TestRequest::with_uri(&format!(
                "https://retrack.dev/api/trackers/{}/revisions",
                uuid!("00000000-0000-0000-0000-000000000001")
            ))
            .to_request(),
        )
        .await;
        assert_eq!(response.status(), 400);
        assert_debug_snapshot!(from_utf8(&response.into_body().try_into_bytes().unwrap())?, @r###""{\"message\":\"Tracker ('00000000-0000-0000-0000-000000000001') is not found.\"}""###);

        Ok(())
    }
}
