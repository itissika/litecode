//! Catalog contract tests: strict parsing, embedded seed, index integrity.

use std::path::Path;

use super::resolve::ProviderCatalog;
use super::schema::{
    EndpointKind, Modality, ProviderQuirk,
    ReasoningReplay, UsagePatch,
};
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
fn seed_declares_reasoning_summaries_only_for_the_gpt_family() {
    let catalog = seeded();
    // OpenAI never exposes raw reasoning and emits a summary only when the
    // request opts in, so every GPT entry on a Responses host declares the
    // literal; every other model stays silent.
    for reference in [
        "openai/gpt-5.6-sol",
        "opencode/gpt-6.1-sol",
        "opencode/gpt-6-luna",
    ] {
        assert_eq!(
            catalog
                .model(reference)
                .unwrap()
                .reasoning_summary
                .as_deref(),
            Some("auto"),
            "{reference} must opt in to reasoning summaries"
        );
    }
    for model in catalog.models() {
        let gpt_on_responses =
            model.id.starts_with("gpt-") && model.endpoint_type == EndpointKind::Responses;
        if gpt_on_responses {
            assert_eq!(
                model.reasoning_summary.as_deref(),
                Some("auto"),
                "{}: GPT models on Responses hosts declare the summary literal",
                model.reference
            );
        } else if model.endpoint_type == EndpointKind::Messages {
            if let Some(summary) = model.reasoning_summary.as_deref() {
                assert_eq!(
                    summary, "summarized",
                    "{}: Messages thinking display is the summarized literal",
                    model.reference
                );
            }
        } else {
            assert_eq!(
                model.reasoning_summary, None,
                "{}: only the GPT family and Messages models declare a summary literal",
                model.reference
            );
        }
    }
}

#[test]
fn seed_claude_messages_models_use_the_documented_routes() {
    let catalog = seeded();
    let expect = [
        (
            "opencode/claude-opus-5-5",
            "https://opencode.ai/inference/anthropic/v1/messages",
            None,
            true,
        ),
        (
            "opencode/claude-sonnet-5-5",
            "https://opencode.ai/inference/anthropic/v1/messages",
            Some("between_tools"),
            true,
        ),
        (
            "opencode/claude-haiku-5-5",
            "https://opencode.ai/inference/anthropic/v1/messages",
            Some("disabled"),
            true,
        ),
        (
            "opencode-go/claude-haiku-5-5",
            "https://opencode.ai/inference/go/anthropic/v1/messages",
            Some("disabled"),
            true,
        ),
        (
            "commandcode/claude-opus-5-5",
            "https://api.commandcode.ai/provider/v1/messages",
            None,
            false,
        ),
        (
            "commandcode/claude-sonnet-5-5",
            "https://api.commandcode.ai/provider/v1/messages",
            Some("between_tools"),
            false,
        ),
        (
            "commandcode/claude-haiku-5-5",
            "https://api.commandcode.ai/provider/v1/messages",
            Some("disabled"),
            false,
        ),
    ];
    for (reference, url, off, pdf) in expect {
        let model = catalog.model(reference).unwrap();
        assert_eq!(model.endpoint_type, EndpointKind::Messages, "{reference}");
        assert_eq!(model.request_url, url, "{reference}");
        assert_eq!(model.reasoning_off.as_deref(), off, "{reference}");
        assert_eq!(
            model.reasoning_summary.as_deref(),
            Some("summarized"),
            "{reference}"
        );
        assert_eq!(model.context_window, 256_000, "{reference}");
        assert_eq!(model.context_window_max, 1_000_000, "{reference}");
        assert_eq!(model.max_output, 128_000, "{reference}");
        assert!(model.supports(Modality::Text), "{reference}");
        assert!(model.supports(Modality::Image), "{reference}");
        assert_eq!(model.supports(Modality::Pdf), pdf, "{reference}");
        let tiers = model.reasoning.as_ref().unwrap();
        assert_eq!(tiers.low, "low");
        assert_eq!(tiers.medium, "high");
        assert_eq!(tiers.high, "max");
        let session = model
            .headers
            .iter()
            .any(|(name, value)| name == "x-opencode-session" && value == "{{session_id}}");
        assert_eq!(session, reference.starts_with("opencode"), "{reference}");
    }
}

#[test]
fn seed_provider_quirks_land_on_their_models() {
    let catalog = seeded();
    for reference in [
        "deepseek/deepseek-flash",
        "mimo/mimo-v2.6-pro",
        "mimo/mimo-v2.6-flash",
    ] {
        assert_eq!(
            catalog.model(reference).unwrap().reasoning_replay,
            ReasoningReplay::Text,
            "{reference}"
        );
    }
    let ark = catalog.model("ark-coding/doubao-seed-2.1-turbo").unwrap();
    assert!(ark.has_quirk(ProviderQuirk::ThinkingTypeSwitch));
    assert_eq!(
        ark.extra_body.get("store").and_then(|v| v.as_bool()),
        Some(false)
    );
    // Chat models on this host inherit the switch and do not declare their own
    // list. The chat codec never executes it; tightening that is a behavior change.
    let glm = catalog.model("ark-coding/glm-5.3").unwrap();
    assert_eq!(glm.endpoint_type, EndpointKind::ChatCompletions);
    assert!(glm.has_quirk(ProviderQuirk::ThinkingTypeSwitch));
}

#[test]
fn model_quirks_replace_the_provider_list_when_declared() {
    let catalog = parse(
        r#"
version = 1

[[providers]]
id = "p"
name = "P"
endpoint = "https://x.example/v1"
endpoint_type = "responses"
quirks = ["thinking_type_switch"]

[[models]]
id = "inherit"
provider_id = "p"

[[models]]
id = "cleared"
provider_id = "p"
quirks = []

[[models]]
id = "chat"
provider_id = "p"
endpoint_type = "chat_completions"
"#,
    )
    .unwrap();

    let inherit = catalog.model("p/inherit").unwrap();
    assert!(inherit.has_quirk(ProviderQuirk::ThinkingTypeSwitch));

    let cleared = catalog.model("p/cleared").unwrap();
    assert!(cleared.quirks.is_empty());

    let chat = catalog.model("p/chat").unwrap();
    assert_eq!(chat.endpoint_type, EndpointKind::ChatCompletions);
    assert!(chat.has_quirk(ProviderQuirk::ThinkingTypeSwitch));
}

#[test]
fn seed_responses_models_that_send_max_declare_the_effort_patch() {
    let catalog = seeded();
    for model in catalog.models() {
        if model.endpoint_type != EndpointKind::Responses {
            continue;
        }
        let Some(tiers) = &model.reasoning else {
            continue;
        };
        if ![&tiers.low, &tiers.medium, &tiers.high]
            .iter()
            .any(|literal| literal.as_str() == "max")
        {
            continue;
        }
        assert_eq!(
            model.usage_patch,
            UsagePatch::MapMaxEffortToXhigh,
            "{}: sends the `max` effort literal, so its response echo must be normalized",
            model.reference
        );
    }
}

#[test]
fn empty_off_literal_is_refused_and_off_inherits_from_the_provider() {
    let message = err(
        "version = 1\n\n[[providers]]\nid = \"p\"\nname = \"P\"\nendpoint = \"https://x.example/v1\"\nendpoint_type = \"responses\"\ntiers = { off = \"  \", low = \"low\", medium = \"medium\", high = \"high\" }\n",
    );
    assert!(message.contains("tiers.off"), "{message}");

    let text = "version = 1\n\n[[providers]]\nid = \"p\"\nname = \"P\"\nendpoint = \"https://x.example/v1\"\nendpoint_type = \"responses\"\ntiers = { off = \"none\", low = \"low\", medium = \"medium\", high = \"high\" }\n\n[[models]]\nid = \"m\"\nprovider_id = \"p\"\n";
    let catalog = parse(text).expect("valid catalog");
    assert_eq!(
        catalog.model("p/m").unwrap().reasoning_off.as_deref(),
        Some("none"),
        "a model without its own tiers inherits the provider's off literal"
    );

    let text = "version = 1\n\n[[providers]]\nid = \"p\"\nname = \"P\"\nendpoint = \"https://x.example/v1\"\nendpoint_type = \"responses\"\ntiers = { off = \"none\", low = \"low\", medium = \"medium\", high = \"high\" }\n\n[[models]]\nid = \"m\"\nprovider_id = \"p\"\nreasoning = { tiers = { low = \"high\", medium = \"high\", high = \"max\" } }\n";
    let catalog = parse(text).expect("valid catalog");
    let model = catalog.model("p/m").unwrap();
    assert_eq!(
        model.reasoning_off, None,
        "model tiers replace the provider's vocabulary, including off"
    );
    assert_eq!(model.reasoning.as_ref().unwrap().low, "high");
}

#[test]
fn bailian_models_replay_their_own_summary() {
    let catalog = seeded();
    for id in [
        "qwen3.8-max",
        "qwen3.8-flash",
        "deepseek-v4.1-flash",
        "glm-5.3",
    ] {
        let model = catalog
            .model(&format!("aliyun-token/{id}"))
            .unwrap_or_else(|| panic!("missing {id}"));
        assert_eq!(model.reasoning_replay, ReasoningReplay::Summary, "{id}");
        assert_eq!(model.endpoint_type, EndpointKind::Responses, "{id}");
    }
    assert_eq!(
        catalog
            .model("aliyun-token/qwen3.8-flash")
            .unwrap()
            .max_output,
        128000
    );
    assert_eq!(
        catalog
            .model("openai/gpt-5.6-sol")
            .unwrap()
            .reasoning_replay,
        ReasoningReplay::Ciphertext
    );
    assert_eq!(
        catalog
            .model("deepseek/deepseek-flash")
            .unwrap()
            .reasoning_replay,
        ReasoningReplay::Text
    );
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
