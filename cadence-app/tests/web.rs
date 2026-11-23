use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use cadence::{AppState, app, db};
use http_body_util::BodyExt;
use tower::ServiceExt;

async fn test_app() -> axum::Router {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool).await.unwrap();
    app(AppState::new(pool))
}

fn form(fields: &[(&str, &str)]) -> Body {
    Body::from(serde_urlencoded::to_string(fields).unwrap())
}

async fn body_text(response: axum::response::Response) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

fn session_cookie(response: &axum::response::Response) -> String {
    response
        .headers()
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned()
}

fn location(response: &axum::response::Response) -> String {
    response
        .headers()
        .get(header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned()
}

fn csrf(html: &str) -> String {
    let marker = "name=\"csrf\" value=\"";
    let start = html.find(marker).unwrap() + marker.len();
    html[start..].split('"').next().unwrap().to_owned()
}

#[tokio::test]
async fn signup_login_and_task_flow() {
    let router = test_app().await;

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/signup")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(form(&[
                    ("name", "Ada"),
                    ("email", "ada@example.com"),
                    ("password", "correct horse battery staple"),
                ]))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let cookie = session_cookie(&response);

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/app")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(html.contains("Ada"));
    assert!(html.contains("New workspace"));
    let token = csrf(&html);

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/workspaces")
                .header(header::COOKIE, &cookie)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(form(&[("csrf", &token), ("name", "Home")]))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let workspace_url = location(&response);
    assert!(workspace_url.starts_with("/app?workspace="));

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(&workspace_url)
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let html = body_text(response).await;
    assert!(html.contains("Home"));
    let workspace_id = workspace_url.trim_start_matches("/app?workspace=");
    let token = csrf(&html);

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/projects")
                .header(header::COOKIE, &cookie)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(form(&[
                    ("csrf", &token),
                    ("workspace_id", workspace_id),
                    ("name", "Chores"),
                ]))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let project_url = location(&response);
    assert!(project_url.contains("project="));

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(&project_url)
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let html = body_text(response).await;
    assert!(html.contains("Chores"));
    let token = csrf(&html);
    let project_id = project_url.split("project=").nth(1).unwrap();

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/tasks")
                .header(header::COOKIE, &cookie)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(form(&[
                    ("csrf", &token),
                    ("workspace_id", workspace_id),
                    ("project_id", project_id),
                    ("title", "buy milk"),
                ]))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(&project_url)
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let html = body_text(response).await;
    assert!(html.contains("buy milk"));
}

#[tokio::test]
async fn logout_requires_csrf_and_clears_the_session() {
    let router = test_app().await;

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/signup")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(form(&[
                    ("name", "Ada"),
                    ("email", "ada@example.com"),
                    ("password", "correct horse battery staple"),
                ]))
                .unwrap(),
        )
        .await
        .unwrap();
    let cookie = session_cookie(&response);

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/app")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let html = body_text(response).await;
    let token = csrf(&html);

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/logout")
                .header(header::COOKIE, &cookie)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(form(&[("csrf", "wrong")]))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/logout")
                .header(header::COOKIE, &cookie)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(form(&[("csrf", &token)]))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert!(
        response
            .headers()
            .get(header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap()
            .contains("Max-Age=0")
    );

    let response = router
        .oneshot(
            Request::builder()
                .uri("/app")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(location(&response), "/login");
}

#[tokio::test]
async fn login_rejects_a_bad_password() {
    let router = test_app().await;

    let response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/login")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(form(&[
                    ("email", "nobody@example.com"),
                    ("password", "wrong password"),
                ]))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(html.contains("Invalid email or password."));
}
