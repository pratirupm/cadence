use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use cadence::auth::AuthService;
use cadence::tokens::Scope;
use cadence::workspaces::{ProjectVisibility, WorkspaceService};
use cadence::{AppState, app, db};
use http_body_util::BodyExt;
use tower::ServiceExt;

const PASSWORD: &str = "correct horse battery staple";

async fn fixture() -> (axum::Router, AppState, String, String, String) {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool).await.unwrap();
    let state = AppState::new(pool);
    let user = AuthService::new(state.pool.clone())
        .create_user("api@example.com", "API", PASSWORD, true)
        .await
        .unwrap()
        .id;
    let workspaces = WorkspaceService::new(state.pool.clone());
    let workspace = workspaces
        .create_workspace(&user, "API Home")
        .await
        .unwrap();
    let project = workspaces
        .create_project(&user, &workspace.id, "Inbox", ProjectVisibility::Workspace)
        .await
        .unwrap();
    let (token, _) = state
        .tokens
        .create(&user, "test", &[Scope::Read, Scope::Write], None)
        .await
        .unwrap();

    (app(state.clone()), state, token, workspace.id, project.id)
}

#[tokio::test]
async fn token_auth_lists_and_creates_tasks_idempotently() {
    let (router, _state, token, workspace, project) = fixture().await;

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/me")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let create = format!("/api/v1/workspaces/{workspace}/projects/{project}/tasks");
    let body = Body::from("{\"title\":\"buy milk\"}");
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&create)
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::CONTENT_TYPE, "application/json")
                .header("idempotency-key", "task-1")
                .body(body)
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let first = body_text(response).await;

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&create)
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::CONTENT_TYPE, "application/json")
                .header("idempotency-key", "task-1")
                .body(Body::from("{\"title\":\"buy milk\"}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(body_text(response).await, first);

    let response = router
        .oneshot(
            Request::builder()
                .uri(&create)
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let list = body_text(response).await;
    assert_eq!(list.matches("buy milk").count(), 1);
}

#[tokio::test]
async fn rejects_missing_and_scopeless_tokens() {
    let (router, state, _token, _, project) = fixture().await;
    let user = state
        .auth
        .find_by_email("api@example.com")
        .await
        .unwrap()
        .unwrap()
        .id;

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/me")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let (read_only, _) = state
        .tokens
        .create(&user, "reader", &[Scope::Read], None)
        .await
        .unwrap();
    let response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/api/v1/workspaces/{}/projects/{project}/tasks",
                    state.workspaces.list_workspaces(&user).await.unwrap()[0].id
                ))
                .header(header::AUTHORIZATION, format!("Bearer {read_only}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("{\"title\":\"nope\"}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn admin_tokens_can_create_and_delete_webhooks() {
    let (router, state, _token, workspace, _project) = fixture().await;
    let user = state
        .auth
        .find_by_email("api@example.com")
        .await
        .unwrap()
        .unwrap()
        .id;
    let (admin, _) = state
        .tokens
        .create(&user, "admin", &[Scope::Admin], None)
        .await
        .unwrap();

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/workspaces/{workspace}/webhooks"))
                .header(header::AUTHORIZATION, format!("Bearer {admin}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    "{\"url\":\"https://example.com/hook\",\"events\":[\"task.created\"]}",
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = body_text(response).await;
    let id = serde_json::from_str::<serde_json::Value>(&body).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/v1/workspaces/{workspace}/webhooks/{id}"))
                .header(header::AUTHORIZATION, format!("Bearer {admin}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    let response = router
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/workspaces/{workspace}/webhooks"))
                .header(header::AUTHORIZATION, format!("Bearer {admin}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(body_text(response).await, "[]");
}

async fn body_text(response: axum::response::Response) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}
