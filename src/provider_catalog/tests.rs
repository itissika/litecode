//! Catalog contract tests: strict parsing, embedded seed, index integrity.

use std::path::Path;

use super::resolve::ProviderCatalog;
use super::store;

fn parse(text: &str) -> crate::types::Result<ProviderCatalog> {
    ProviderCatalog::parse(text, Path::new("test-catalog.toml"))
}

fn seeded() -> ProviderCatalog {
    parse(store::DEFAULT_CATALOG).expect("the embedded catalog must always be valid")
}

fn err(text: &str) -> String {
    parse(text).expect_err("must be refused").to_string()
}

#[test]
fn embedded_catalog_is_valid_and_complete() {
    let catalog = seeded();
    assert!(
        catalog.providers().len() >= 6,
        "the seeded catalog must cover every shipped provider"
    );
    assert!(!catalog.models().is_empty());
    for provider in catalog.providers() {
        assert!(
            !catalog.models_of(&provider.id).is_empty(),
            "provider '{}' has no models",
            provider.id
        );
    }
}

#[test]
fn references_are_provider_slash_model_and_lookup_splits_once() {
    let catalog = seeded();
    for model in catalog.models() {
        assert_eq!(
            model.reference,
            format!("{}/{}", model.provider_id, model.id)
        );
        assert_eq!(
            catalog.model(&model.reference).map(|m| m.id.as_str()),
            Some(model.id.as_str())
        );
    }
    // Model ids may contain '/', provider ids may not: only the first split counts.
    assert_eq!(
        ProviderCatalog::split_reference("commandcode/deepseek/deepseek-v4-flash"),
        Some(("commandcode", "deepseek/deepseek-v4-flash"))
    );
    assert_eq!(ProviderCatalog::split_reference("nolash"), None);
}

#[test]
fn request_url_comes_from_endpoint_type() {
    let catalog = seeded();
    let responses = catalog.model("deepseek/deepseek-flash").unwrap();
    assert_eq!(responses.request_url, "https://api.deepseek.com/responses");
    let chat = catalog.model("opencode-go/deepseek-v4.1-flash").unwrap();
    assert_eq!(
        chat.request_url,
        "https://opencode.ai/inference/go/v1/chat/completions"
    );
}

#[test]
fn unknown_field_is_refused() {
    let message = err(
        "version = 1\n\n[[providers]]\nid = \"p\"\nname = \"P\"\nendpoint = \"https://x.example/v1\"\nendpoint_type = \"responses\"\ntypo_field = 1\n",
    );
    assert!(message.contains("typo_field"), "{message}");
}

#[test]
fn codec_capability_mismatch_is_refused_not_trimmed() {
    let message = err(
        "version = 1\n\n[[providers]]\nid = \"p\"\nname = \"P\"\nendpoint = \"https://x.example/v1\"\nendpoint_type = \"chat_completions\"\n\n[[models]]\nid = \"m\"\nprovider_id = \"p\"\nmodalities = [\"text\", \"image\"]\n",
    );
    assert!(message.contains("not implemented"), "{message}");
}

#[test]
fn shared_catalog_is_the_embedded_seed_and_does_not_read_a_file() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("litecode.db");
    store::forget(&db);
    std::fs::write(dir.path().join("provider-catalog.toml"), "version = 1\n").unwrap();

    let catalog = store::shared_for_db(&db).expect("embedded");
    assert!(catalog.provider("openai").is_some());
    assert!(std::sync::Arc::ptr_eq(&catalog, &store::embedded()));
    assert_eq!(
        std::fs::read_to_string(dir.path().join("provider-catalog.toml")).unwrap(),
        "version = 1\n",
        "a file beside the database is not the catalog"
    );

    let fixture = parse(
        "version = 1\n\n[[providers]]\nid = \"extra\"\nname = \"Extra\"\nendpoint = \"https://extra.example/v1\"\nendpoint_type = \"responses\"\nauth = \"bearer\"\n\n[[models]]\nid = \"mine\"\nprovider_id = \"extra\"\nlabel = \"Mine\"\ncontext_window = 256000\nmax_output = 128000\nmodalities = [\"text\"]\n",
    )
    .unwrap();
    store::pin(&db, std::sync::Arc::new(fixture));
    let pinned = store::shared_for_db(&db).unwrap();
    assert_eq!(
        pinned.provider("extra").unwrap().endpoint,
        "https://extra.example/v1"
    );
    assert!(pinned.provider("openai").is_none());
    store::forget(&db);
    assert!(
        store::shared_for_db(&db)
            .unwrap()
            .provider("openai")
            .is_some()
    );
}
