//! Opt-in live contract test for Stremio's official Cinemeta addon.
//!
//! Run with `cargo test -p ankai-core --test stremio_live -- --ignored`.

use ankai_core::stremio::AddonClient;

#[tokio::test]
#[ignore = "calls the live public Cinemeta service"]
async fn cinemeta_manifest_catalog_and_meta_match_the_protocol() {
    let client = AddonClient::new("https://v3-cinemeta.strem.io/manifest.json").unwrap();
    let manifest = client.manifest().await.unwrap();
    assert_eq!(manifest.id, "com.linvo.cinemeta");
    assert!(manifest.catalogs.iter().any(|catalog| catalog.id == "top"));

    let catalog = client.catalog("movie", "top").await.unwrap();
    let first = catalog
        .first()
        .expect("Cinemeta's top movie catalog is empty");
    let meta = client.meta(&first.media_type, &first.id).await.unwrap();
    assert_eq!(meta.id, first.id);
}
