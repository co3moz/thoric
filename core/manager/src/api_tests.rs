//! Pins down what the settings route tells of the installation's features and their defaults,
//! and of the Docker host's architecture.
use crate::router;
use crate::test_support::app;
use axum::{body::Body, http::Request};
use std::sync::Arc;
use tower::ServiceExt;

#[tokio::test]
async fn the_settings_route_tells_the_features_and_their_defaults() {
    let mut app = app(false);
    {
        let config = &mut Arc::get_mut(&mut app).unwrap().config;
        config.wsl_gpu = false;
        config.cuda = false;
        config.sudo = false;
        config.architecture = "arm64".into();
    }
    let response = router(app)
        .oneshot(
            Request::builder()
                .uri("/api/settings")
                .header("host", "localhost:8080")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = axum::body::to_bytes(response.into_body(), 65536)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        (
            &body["gpu_enabled"],
            &body["sudo_enabled"],
            &body["internet_enabled"]
        ),
        (
            &serde_json::json!(false),
            &serde_json::json!(false),
            &serde_json::json!(true)
        )
    );
    // The configured defaults turn GPU access and sudo on, which the installation has off.
    assert_eq!(
        body["defaults"],
        serde_json::json!({"gpu": false, "internet": true, "sudo": false, "audio": false, "microphone": false, "gamepad": false, "vpn": false, "idle_stop_minutes": 0})
    );
    // DRI_GPU is the one GPU setting left on, and the manager cannot see this host.
    assert_eq!(
        body["gpu"],
        serde_json::json!({"routes": [], "unused": [{"route": "dri", "setting": "DRI_GPU", "reason": "unseen"}], "found": null})
    );
    // The templates page offers only the templates that build on the Docker host's architecture.
    assert_eq!(body["architecture"], "arm64");
}
