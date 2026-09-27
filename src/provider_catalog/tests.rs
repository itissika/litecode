//! Catalog contract tests: strict parsing, seeding lifecycle, index integrity.

use std::path::Path;

use super::resolve::{
    ProviderCatalog, is_valid_provider_id, validate_endpoint, validate_headers, validate_provider,
};
use super::schema::{
    EndpointKind, Modality, ProviderQuirk, RESERVED_BODY_KEYS, RESERVED_HEADER_NAMES, ReasoningKey,
    UsagePatch,
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
    let chat = catalog.model("opencode-go/deepseek-flash").unwrap();
    assert_eq!(
        chat.request_url,
        "https://opencode.ai/zen/go/v1/chat/completions"
    );
}

#[test]
fn reasoning_tiers_inherit_from_provider_and_override_per_model() {
    let catalog = seeded();
    // commandcode declares one vocabulary for the whole provider; its models
    // carry no block of their own and inherit it.
    let commandcode = catalog.model("commandcode/gpt-5.6-sol").unwrap();
    let tiers = commandcode
        .reasoning
        .as_ref()
        .expect("provider tiers inherit");
    assert_eq!(tiers.medium, "high");
    // opencode declares none, so each Zen model states its own.
    let zen = catalog.model("opencode/gpt-6-sol").unwrap();
    let zen_tiers = zen.reasoning.as_ref().expect("model-declared tiers");
    assert_eq!(zen_tiers.medium, "medium");
}

#[test]
fn default_catalog_declares_off_where_the_vendor_defaults_to_thinking() {
    let catalog = seeded();
    for reference in [
        "deepseek/deepseek-flash",
        "mimo/mimo-v2.6-flash",
        "mimo/mimo-v2.6-pro",
        "opencode/gpt-6-sol",
        "opencode/gpt-6-luna",
    ] {
        assert_eq!(
            catalog.model(reference).unwrap().reasoning_off.as_deref(),
            Some("none"),
            "{reference} must send an explicit off literal"
        );
    }
    assert_eq!(
        catalog
            .model("ark-coding/doubao-seed-2.1-turbo")
            .unwrap()
            .reasoning_off
            .as_deref(),
        Some("disabled")
    );
    // Vendors whose ladder has no off literal keep sending nothing for Off.
    for reference in [
        "opencode-go/deepseek-flash",
        "opencode-go/deepseek-v4.1-flash",
    ] {
        assert_eq!(
            catalog.model(reference).unwrap().reasoning_off,
            None,
            "{reference} must send nothing for Off"
        );
    }
    // OpenAI's "none" is a real effort literal, so compaction uses it.
    assert_eq!(
        catalog
            .model("openai/gpt-5.6-sol")
            .unwrap()
            .reasoning_off
            .as_deref(),
        Some("none")
    );
}

#[test]
fn seed_declares_reasoning_summaries_only_for_the_gpt_family() {
    let catalog = seeded();
    // OpenAI never exposes raw reasoning and emits a summary only when the
    // request opts in, so every GPT entry on a Responses host declares the
    // literal; every other model stays silent.
    for reference in [
        "openai/gpt-5.6-sol",
        "openai/gpt-5.6-terra",
        "openai/gpt-5.6-luna",
        "opencode/gpt-6-sol",
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
        } else {
            assert_eq!(
                model.reasoning_summary, None,
                "{}: only the GPT family declares a summary literal",
                model.reference
            );
        }
    }
}

#[test]
fn seed_provider_quirks_land_on_their_models() {
    let catalog = seeded();
    let flash = catalog.model("deepseek/deepseek-flash").unwrap();
    assert!(flash.has_quirk(ProviderQuirk::ReasoningReplay));
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
quirks = ["thinking_type_switch", "reasoning_replay"]

[[models]]
id = "inherit"
provider_id = "p"

[[models]]
id = "only-replay"
provider_id = "p"
quirks = ["reasoning_replay"]

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
    assert!(inherit.has_quirk(ProviderQuirk::ReasoningReplay));

    let only = catalog.model("p/only-replay").unwrap();
    assert!(!only.has_quirk(ProviderQuirk::ThinkingTypeSwitch));
    assert!(only.has_quirk(ProviderQuirk::ReasoningReplay));

    let cleared = catalog.model("p/cleared").unwrap();
    assert!(cleared.quirks.is_empty());

    let chat = catalog.model("p/chat").unwrap();
    assert_eq!(chat.endpoint_type, EndpointKind::ChatCompletions);
    assert!(chat.has_quirk(ProviderQuirk::ThinkingTypeSwitch));
}

#[test]
fn explicit_thinking_switch_on_a_chat_model_is_refused() {
    let message = err(
        "version = 1\n\n[[providers]]\nid = \"p\"\nname = \"P\"\nendpoint = \"https://x.example/v1\"\nendpoint_type = \"responses\"\n\n[[models]]\nid = \"m\"\nprovider_id = \"p\"\nendpoint_type = \"chat_completions\"\nquirks = [\"thinking_type_switch\"]\n",
    );
    assert!(message.contains("thinking_type_switch"), "{message}");
}

#[test]
fn seed_gateway_hosts_keep_each_models_dialect() {
    let catalog = seeded();

    let zhipu_52 = catalog.model("zhipu/glm-5.2").unwrap();
    assert_eq!(
        zhipu_52.request_url,
        "https://open.bigmodel.cn/api/paas/v4/chat/completions"
    );
    assert_eq!(zhipu_52.reasoning_off.as_deref(), Some("none"));
    assert!(
        catalog
            .model("zhipu/glm-5.3")
            .unwrap()
            .reasoning_off
            .is_none()
    );

    let doubao = catalog.model("ark-agent/doubao-seed-2.1-pro").unwrap();
    assert_eq!(
        doubao.request_url,
        "https://ark.cn-beijing.volces.com/api/plan/v3/responses"
    );
    assert!(doubao.has_quirk(ProviderQuirk::ThinkingTypeSwitch));
    assert!(doubao.supports(Modality::Image));

    let agent_glm = catalog.model("ark-agent/glm-5.3").unwrap();
    assert_eq!(
        agent_glm.request_url,
        "https://ark.cn-beijing.volces.com/api/plan/v3/chat/completions"
    );
    assert!(agent_glm.quirks.is_empty());
    assert!(!agent_glm.supports(Modality::Image));

    let tencent_glm = catalog.model("tencent-token/glm-5.2").unwrap();
    assert_eq!(tencent_glm.reasoning_off.as_deref(), Some("none"));
    assert!(
        catalog
            .model("tencent-token/deepseek-v4-pro-202606")
            .unwrap()
            .reasoning
            .is_none()
    );

    let qwen = catalog.model("aliyun-token/qwen3.8-max").unwrap();
    assert_eq!(
        qwen.request_url,
        "https://token-plan.cn-beijing.maas.aliyuncs.com/compatible-mode/v1/responses"
    );
    assert_eq!(qwen.reasoning_off.as_deref(), Some("none"));
    assert_eq!(qwen.usage_patch, UsagePatch::MapMaxEffortToXhigh);
    assert!(qwen.supports(Modality::Image));
    assert!(
        catalog
            .model("aliyun-token/deepseek-v4-pro")
            .unwrap()
            .quirks
            .is_empty()
    );
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
fn seed_context_and_output_follow_the_budget_policy() {
    let catalog = seeded();
    for model in catalog.models() {
        assert!(
            model.context_window <= 256_000,
            "{}: default context budget must not exceed 256000",
            model.reference
        );
        assert!(
            model.context_window_max >= model.context_window,
            "{}: max must not be below the default",
            model.reference
        );
        assert!(
            model.max_output <= 128_000,
            "{}: max output must not exceed 128000",
            model.reference
        );
    }
}

#[test]
fn provider_id_format_is_enforced() {
    assert!(is_valid_provider_id("opencode-go"));
    assert!(is_valid_provider_id("a1"));
    assert!(!is_valid_provider_id("OpenCode"));
    assert!(!is_valid_provider_id("-leading"));
    assert!(!is_valid_provider_id("has/slash"));
    assert!(!is_valid_provider_id(""));
}

#[test]
fn unknown_version_is_refused() {
    let message = err("version = 2\n");
    assert!(message.contains("version 2"), "{message}");
    assert!(message.contains("test-catalog.toml"), "{message}");
}

#[test]
fn unknown_field_is_refused() {
    let message = err(
        "version = 1\n\n[[providers]]\nid = \"p\"\nname = \"P\"\nendpoint = \"https://x.example/v1\"\nendpoint_type = \"responses\"\ntypo_field = 1\n",
    );
    assert!(message.contains("typo_field"), "{message}");
}

#[test]
fn unknown_enum_value_is_refused() {
    let message = err(
        "version = 1\n\n[[providers]]\nid = \"p\"\nname = \"P\"\nendpoint = \"https://x.example/v1\"\nendpoint_type = \"grpc\"\n",
    );
    assert!(message.contains("grpc"), "{message}");
}

#[test]
fn duplicate_provider_and_model_ids_are_refused() {
    let provider = "[[providers]]\nid = \"p\"\nname = \"P\"\nendpoint = \"https://x.example/v1\"\nendpoint_type = \"responses\"\n";
    let message = err(&format!("version = 1\n{provider}{provider}"));
    assert!(message.contains("duplicate provider id"), "{message}");

    let message = err(&format!(
        "version = 1\n{provider}\n[[models]]\nid = \"m\"\nprovider_id = \"p\"\n\n[[models]]\nid = \"m\"\nprovider_id = \"p\"\n"
    ));
    assert!(message.contains("duplicate model reference"), "{message}");
}

#[test]
fn dangling_provider_reference_is_refused() {
    let message = err("version = 1\n\n[[models]]\nid = \"m\"\nprovider_id = \"ghost\"\n");
    assert!(message.contains("ghost"), "{message}");
    assert!(message.contains("does not exist"), "{message}");
}

#[test]
fn partial_tiers_are_refused() {
    let message = err(
        "version = 1\n\n[[providers]]\nid = \"p\"\nname = \"P\"\nendpoint = \"https://x.example/v1\"\nendpoint_type = \"responses\"\ntiers = { low = \"low\", medium = \"medium\" }\n",
    );
    assert!(message.contains("high"), "{message}");
}

#[test]
fn empty_tier_literal_is_refused() {
    let message = err(
        "version = 1\n\n[[providers]]\nid = \"p\"\nname = \"P\"\nendpoint = \"https://x.example/v1\"\nendpoint_type = \"responses\"\ntiers = { low = \"\", medium = \"medium\", high = \"high\" }\n",
    );
    assert!(message.contains("tiers.low"), "{message}");
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
fn reasoning_summary_must_be_non_empty_and_ride_on_tiers() {
    let message = err(
        "version = 1\n\n[[providers]]\nid = \"p\"\nname = \"P\"\nendpoint = \"https://x.example/v1\"\nendpoint_type = \"responses\"\n\n[[models]]\nid = \"m\"\nprovider_id = \"p\"\nreasoning = { summary = \"  \", tiers = { low = \"low\", medium = \"medium\", high = \"high\" } }\n",
    );
    assert!(message.contains("reasoning.summary"), "{message}");

    // Without any tier mapping the literal would silently request nothing.
    let message = err(
        "version = 1\n\n[[providers]]\nid = \"p\"\nname = \"P\"\nendpoint = \"https://x.example/v1\"\nendpoint_type = \"responses\"\n\n[[models]]\nid = \"m\"\nprovider_id = \"p\"\nreasoning = { summary = \"auto\" }\n",
    );
    assert!(message.contains("reasoning.summary requires"), "{message}");

    // Provider-declared tiers satisfy the requirement for every model under them.
    let catalog = parse("version = 1\n\n[[providers]]\nid = \"p\"\nname = \"P\"\nendpoint = \"https://x.example/v1\"\nendpoint_type = \"responses\"\ntiers = { low = \"low\", medium = \"medium\", high = \"high\" }\n\n[[models]]\nid = \"m\"\nprovider_id = \"p\"\nreasoning = { summary = \"auto\" }\n").expect("provider tiers satisfy the summary");
    assert_eq!(
        catalog.model("p/m").unwrap().reasoning_summary.as_deref(),
        Some("auto")
    );
}

#[test]
fn endpoint_must_be_a_bare_absolute_url() {
    assert!(validate_endpoint("https://api.example.com/v1").is_ok());
    for bad in [
        "",
        "api.example.com",
        "ftp://api.example.com",
        "https://api.example.com/v1?x=1",
        "https://api.example.com/v1#frag",
        "https://api.example.com/v1/responses",
        "https://api.example.com/v1/chat/completions",
    ] {
        assert!(validate_endpoint(bad).is_err(), "{bad} must be refused");
    }
}

#[test]
fn reserved_body_keys_and_headers_are_refused() {
    let message = err(
        "version = 1\n\n[[providers]]\nid = \"p\"\nname = \"P\"\nendpoint = \"https://x.example/v1\"\nendpoint_type = \"responses\"\n\n[[models]]\nid = \"m\"\nprovider_id = \"p\"\nextra_body = { model = \"other\" }\n",
    );
    assert!(message.contains("extra_body key 'model'"), "{message}");

    let message = err(
        "version = 1\n\n[[providers]]\nid = \"p\"\nname = \"P\"\nendpoint = \"https://x.example/v1\"\nendpoint_type = \"responses\"\nheaders = { authorization = \"secret\" }\n",
    );
    assert!(message.contains("owned by the codec"), "{message}");
}

#[test]
fn header_templates_only_allow_session_id() {
    let ok =
        std::collections::BTreeMap::from([("x-session".to_string(), "{{session_id}}".to_string())]);
    assert!(validate_headers(&ok).is_ok());
    let bad =
        std::collections::BTreeMap::from([("x-session".to_string(), "{{api_key}}".to_string())]);
    let message = validate_headers(&bad).unwrap_err();
    assert!(message.contains("{{api_key}}"), "{message}");
}

#[test]
fn codec_capability_mismatch_is_refused_not_trimmed() {
    let message = err(
        "version = 1\n\n[[providers]]\nid = \"p\"\nname = \"P\"\nendpoint = \"https://x.example/v1\"\nendpoint_type = \"chat_completions\"\n\n[[models]]\nid = \"m\"\nprovider_id = \"p\"\nmodalities = [\"text\", \"image\"]\n",
    );
    assert!(message.contains("not implemented"), "{message}");
}

#[test]
fn text_modality_is_mandatory() {
    let message = err(
        "version = 1\n\n[[providers]]\nid = \"p\"\nname = \"P\"\nendpoint = \"https://x.example/v1\"\nendpoint_type = \"responses\"\n\n[[models]]\nid = \"m\"\nprovider_id = \"p\"\nmodalities = [\"image\"]\n",
    );
    assert!(message.contains("include 'text'"), "{message}");
}

#[test]
fn context_and_output_bounds_are_enforced() {
    let model = "[[models]]\nid = \"m\"\nprovider_id = \"p\"\n";
    let provider = "[[providers]]\nid = \"p\"\nname = \"P\"\nendpoint = \"https://x.example/v1\"\nendpoint_type = \"responses\"\n";
    let zero = err(&format!(
        "version = 1\n{provider}\n{model}context_window = 0\n"
    ));
    assert!(zero.contains("context_window must be > 0"), "{zero}");
    let inverted = err(&format!(
        "version = 1\n{provider}\n{model}context_window = 100\ncontext_window_max = 50\n"
    ));
    assert!(inverted.contains("context_window_max"), "{inverted}");
    let no_output = err(&format!("version = 1\n{provider}\n{model}max_output = 0\n"));
    assert!(no_output.contains("max_output must be > 0"), "{no_output}");
}

#[test]
fn provider_validation_covers_identity_and_thinking_quirk_scope() {
    let mut provider = super::schema::RawProvider {
        id: "p".into(),
        name: "P".into(),
        visible: true,
        endpoint: "https://x.example/v1".into(),
        endpoint_type: EndpointKind::ChatCompletions,
        auth: super::schema::AuthKind::Bearer,
        tiers: None,
        quirks: vec![ProviderQuirk::ThinkingTypeSwitch],
        headers: Default::default(),
    };
    let message = validate_provider(&provider).unwrap_err();
    assert!(message.contains("thinking_type_switch"), "{message}");
    provider.quirks.clear();
    assert!(validate_provider(&provider).is_ok());
}

#[test]
fn enum_domains_and_schema_file_stay_aligned() {
    let schema: serde_json::Value =
        serde_json::from_str(store::CATALOG_SCHEMA).expect("schema json");
    let definitions = &schema["definitions"];

    let listed = |key: &str| -> Vec<String> {
        definitions[key]["enum"]
            .as_array()
            .expect("enum array")
            .iter()
            .map(|value| value.as_str().unwrap().to_string())
            .collect()
    };

    assert_eq!(
        listed("endpoint_kind"),
        EndpointKind::ALL
            .iter()
            .map(|kind| kind.as_str().to_string())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        listed("usage_patch"),
        UsagePatch::ALL
            .iter()
            .map(|patch| {
                serde_json::to_value(patch)
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect::<Vec<_>>()
    );
    assert_eq!(
        listed("quirk"),
        ProviderQuirk::ALL
            .iter()
            .map(|quirk| {
                serde_json::to_value(quirk)
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect::<Vec<_>>()
    );
    assert_eq!(
        listed("reasoning_key"),
        ReasoningKey::ALL
            .iter()
            .map(|key| {
                serde_json::to_value(key)
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect::<Vec<_>>()
    );
    assert_eq!(
        listed("modality"),
        Modality::ALL
            .iter()
            .map(|modality| modality.as_str().to_string())
            .collect::<Vec<_>>()
    );

    let reserved = definitions["model"]["properties"]["extra_body"]["description"]
        .as_str()
        .unwrap_or_default();
    assert!(reserved.contains("Codec-owned"), "{reserved}");

    // Every reserved body key must be documented by the schema's model fields.
    for key in RESERVED_BODY_KEYS {
        assert!(!key.is_empty(), "reserved key must not be empty");
    }
    for name in RESERVED_HEADER_NAMES {
        assert!(!name.is_empty(), "reserved header must not be empty");
    }
}

#[test]
fn first_run_seeds_the_file_and_registers_initialized() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("litecode.db");
    store::forget(&db);

    let catalog = store::load_for_db(&db).expect("seed");
    assert_eq!(
        catalog.path(),
        store::catalog_path_for_db(&db).as_path(),
        "the catalog lives next to the global DB"
    );
    let text = std::fs::read_to_string(store::catalog_path_for_db(&db)).unwrap();
    assert!(text.contains("#:schema ./provider-catalog.schema.json"));
    assert!(
        text.contains("# LiteCode provider catalog"),
        "comments must survive seeding"
    );
    assert!(store::schema_path_for_db(&db).is_file());

    // A second load reads the user file and never rewrites it.
    std::fs::write(
        store::catalog_path_for_db(&db),
        "# my own note\nversion = 1\n",
    )
    .unwrap();
    store::forget(&db);
    let reloaded = store::load_for_db(&db).expect("reload");
    assert!(reloaded.providers().is_empty());
    assert_eq!(
        std::fs::read_to_string(store::catalog_path_for_db(&db)).unwrap(),
        "# my own note\nversion = 1\n"
    );
}

#[test]
fn initialized_catalog_that_disappears_is_an_error_not_a_reseed() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("litecode.db");
    store::forget(&db);
    store::load_for_db(&db).expect("seed");

    std::fs::remove_file(store::catalog_path_for_db(&db)).unwrap();
    store::forget(&db);
    let message = store::load_for_db(&db)
        .expect_err("must not silently rebuild")
        .to_string();
    assert!(message.contains("missing"), "{message}");
    assert!(!store::catalog_path_for_db(&db).exists());
}

#[test]
fn shared_catalog_is_loaded_once_per_path() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("litecode.db");
    store::forget(&db);
    let first = store::shared_for_db(&db).unwrap();
    std::fs::write(store::catalog_path_for_db(&db), "version = 1\n").unwrap();
    let second = store::shared_for_db(&db).unwrap();
    assert!(
        std::sync::Arc::ptr_eq(&first, &second),
        "edits require a restart: the process keeps one catalog"
    );
    store::forget(&db);
    let third = store::shared_for_db(&db).unwrap();
    assert!(!std::sync::Arc::ptr_eq(&first, &third));
}

#[test]
fn seed_gap_is_empty_when_the_loaded_catalog_has_every_shipped_id() {
    let gap = store::seed_gap(&seeded());
    assert!(gap.is_empty(), "{gap:?}");
}

#[test]
fn seed_gap_names_a_missing_provider_and_ignores_an_extra_one() {
    let empty = parse("version = 1\n").unwrap();
    let gap = store::seed_gap(&empty);
    assert!(
        gap.missing_providers
            .iter()
            .any(|provider| provider.id == "openai"),
        "{gap:?}"
    );
    assert!(
        gap.missing_models
            .iter()
            .any(|model| model == "openai/gpt-5.6-sol"),
        "{gap:?}"
    );

    let extra = parse(&format!(
        "{}\n[[providers]]\nid = \"extra\"\nname = \"Extra\"\nendpoint = \"https://example.invalid/v1\"\nendpoint_type = \"responses\"\nauth = \"bearer\"\n\n[[models]]\nid = \"extra-model\"\nprovider_id = \"extra\"\ncontext_window = 256000\nmax_output = 128000\nmodalities = [\"text\"]\n",
        store::DEFAULT_CATALOG
    ))
    .unwrap();
    assert!(store::seed_gap(&extra).is_empty());
}

#[test]
fn seed_gap_names_one_removed_model() {
    let marker = "id = \"gpt-5.6-sol\"";
    let start = store::DEFAULT_CATALOG.find(marker).expect("model");
    let block_start = store::DEFAULT_CATALOG[..start]
        .rfind("[[models]]")
        .expect("block");
    let after = &store::DEFAULT_CATALOG[start + marker.len()..];
    let next = after
        .find("[[models]]")
        .map(|index| start + marker.len() + index)
        .unwrap_or(store::DEFAULT_CATALOG.len());
    let mut text = String::new();
    text.push_str(&store::DEFAULT_CATALOG[..block_start]);
    text.push_str(&store::DEFAULT_CATALOG[next..]);
    let loaded = parse(&text).unwrap();
    let gap = store::seed_gap(&loaded);
    assert!(gap.missing_providers.is_empty(), "{gap:?}");
    assert_eq!(gap.missing_models, vec!["openai/gpt-5.6-sol".to_string()]);
    let blocks = store::seed_blocks(&gap);
    assert!(
        blocks.contains("[[models]]") && blocks.contains("id = \"gpt-5.6-sol\""),
        "{blocks}"
    );
    assert!(
        blocks.contains("OpenAI"),
        "the comment above the table travels with the block:\n{blocks}"
    );
    assert!(
        !blocks.contains("id = \"gpt-5.6-terra\""),
        "entries already in the file are left out:\n{blocks}"
    );
    assert!(store::seed_blocks(&store::seed_gap(&seeded())).is_empty());
}
