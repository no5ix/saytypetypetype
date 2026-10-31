//! Configuration checks for the production/development bundle split.
//!
//! The development build must be installable next to the production app:
//! distinct bundle identifier, distinct display name (and therefore a distinct
//! `.app` bundle name), no updater artifacts, and no change to the production
//! identity. These assertions read the real config files so a future edit cannot
//! silently re-merge the two identities.

use serde_json::Value;

const PRODUCTION_CONFIG: &str = include_str!("../../tauri.conf.json");
const DEVELOPMENT_CONFIG: &str = include_str!("../../tauri.dev.conf.json");

fn json(source: &str) -> Value {
    serde_json::from_str(source).expect("tauri config should be valid JSON")
}

fn production() -> Value {
    json(PRODUCTION_CONFIG)
}

fn development() -> Value {
    json(DEVELOPMENT_CONFIG)
}

#[test]
fn production_identity_is_unchanged() {
    let config = production();
    assert_eq!(config["identifier"], "com.yolanda.voicepaste");
    assert_eq!(config["productName"], "SayTypeTypeType");
}

#[test]
fn development_identity_uses_dot_dev_suffix() {
    let config = development();
    assert_eq!(config["identifier"], "com.yolanda.voicepaste.dev");
}

#[test]
fn bundle_identifiers_differ() {
    assert_ne!(
        production()["identifier"].as_str(),
        development()["identifier"].as_str(),
        "development builds must not share the production bundle identifier"
    );
}

#[test]
fn display_names_and_bundle_paths_differ() {
    let prod = production()["productName"].as_str().unwrap().to_string();
    let dev = development()["productName"].as_str().unwrap().to_string();

    assert_ne!(prod, dev);
    assert_eq!(dev, "SayTypeTypeType Dev");
    // The `.app` bundle name is derived from productName, so different names
    // guarantee separate output directories (…/SayTypeTypeType.app vs …/SayTypeTypeType Dev.app).
    assert!(dev.starts_with(&prod));
}

#[test]
fn development_build_does_not_create_updater_artifacts() {
    assert_eq!(
        development()["bundle"]["createUpdaterArtifacts"],
        false,
        "a development build must not publish/consume production updater metadata"
    );
}

#[test]
fn development_build_keeps_production_updater_endpoints_untouched() {
    // The development overlay must not rewrite production updater settings; the
    // updater is disabled at runtime through `build_identity` instead.
    assert!(development().get("plugins").is_none());
    assert_eq!(
        production()["plugins"]["updater"]["endpoints"][0],
        "https://github.com/no5ix/saytypetypetype/releases/latest/download/latest.json"
    );
}

#[test]
fn development_data_directory_differs_from_production() {
    // macOS `app_data_dir()` is `~/Library/Application Support/<identifier>`, so
    // distinct identifiers isolate config, history, logs and downloaded models.
    let prod_config = production();
    let dev_config = development();
    let prod = prod_config["identifier"].as_str().unwrap();
    let dev = dev_config["identifier"].as_str().unwrap();
    assert_ne!(prod, dev);
    assert_ne!(
        format!("/Library/Application Support/{prod}"),
        format!("/Library/Application Support/{dev}")
    );
}
