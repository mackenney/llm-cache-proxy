use crate::common::{Cassette, MockUpstream};

#[tokio::test]
async fn loads_and_replays_dummy() {
    let c = Cassette::load("fixtures/anthropic/dummy.toml");
    assert!(!c.body_chunks.is_empty());
    let mock = MockUpstream::builder().cassette(&c).build().await;
    let resp = reqwest::get(format!("{}/test", mock.url())).await.unwrap();
    assert_eq!(resp.status(), c.status);
    let body = resp.bytes().await.unwrap();
    assert!(!body.is_empty());
}
