//! backend → dispatcher の内部 API（challenge 検証の委譲）。DB は使わない。
use backend::{apps::dispatcher_api, error::AppError};

async fn serve(token: &str) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = dispatcher_api::router(token);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

#[tokio::test]
async fn dispatcher_errors_reach_the_caller_with_the_right_class() {
    let base = serve("shared-token").await;
    // 非 https は dispatcher 側の検証で 400 になり、そのまま BadRequest として返る。
    let err = dispatcher_api::verify_via(&base, "shared-token", "http://example.com/hook", "sig")
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::BadRequest(_)), "{err:?}");
    // トークン不一致は設定ミスとして扱い、外には詳細を出さない。
    let err = dispatcher_api::verify_via(&base, "wrong", "https://example.com/hook", "sig")
        .await
        .unwrap_err();
    assert!(
        matches!(&err, AppError::ServiceUnavailable(m) if m.contains("misconfigured")),
        "{err:?}"
    );
}

#[tokio::test]
async fn unreachable_or_disabled_dispatcher_is_service_unavailable() {
    let err = dispatcher_api::verify_via(
        "http://127.0.0.1:1",
        "shared-token",
        "https://example.com/hook",
        "sig",
    )
    .await
    .unwrap_err();
    assert!(matches!(err, AppError::ServiceUnavailable(_)), "{err:?}");
    // DISPATCHER_TOKEN が空なら内部 API の route 自体が無い（404 → 利用不可扱い）。
    let base = serve("").await;
    let err = dispatcher_api::verify_via(&base, "", "https://example.com/hook", "sig")
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::ServiceUnavailable(_)), "{err:?}");
}
