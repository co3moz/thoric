//! The dependency list the manager image's build writes: that the route hands it over as the build
//! wrote it, that a manager without one says so, and that the image puts it where the route reads it.

use super::*;
use axum::body::to_bytes;

#[tokio::test]
async fn hands_over_the_list_the_build_wrote() {
    let dir = std::env::temp_dir().join(format!(
        "thoric-dependencies-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("dependencies.json");
    let list = br#"{"format":1,"version":"1.0.0","dependencies":[]}"#;
    std::fs::write(&path, list).unwrap();

    let response = read(&path).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
    let body = to_bytes(response.into_body(), 1 << 20).await.unwrap();
    assert_eq!(&body[..], list);

    std::fs::remove_dir_all(&dir).unwrap();
    let missing = read(&path).await.unwrap_err();
    assert_eq!(missing.0, StatusCode::NOT_FOUND);
    assert!(missing.1.contains("without a dependency list"));
}

#[test]
fn the_image_build_writes_the_list_where_the_route_reads_it() {
    let dockerfile = include_str!("../Dockerfile");
    assert!(
        dockerfile.contains(
            r#"python3 core/manager/dependencies.py --strict --platform "${TARGETPLATFORM:-linux/amd64}" --output /dependencies.json"#
        ),
        "the image build must write the list of the platform it builds for"
    );
    assert!(
        dockerfile.contains(&format!(
            "COPY --from=dependencies /dependencies.json {LIST}"
        )),
        "the image must keep the list at {LIST}"
    );
    assert!(
        include_str!("../../../docs/API.md").contains("| GET | `/api/dependencies` |"),
        "docs/API.md must list the route"
    );
}
