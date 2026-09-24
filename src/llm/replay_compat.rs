//! Replay compatibility — where a session's provider-minted item identities meet
//! a target endpoint that never issued them.
//!
//! Providers mint opaque item ids (`rs_…`, `msg_…`, `fc_…`) and may keep the item
//! server-side. A session is long-lived and its model is switchable, so a replayed
//! transcript can carry identities the target cannot resolve. Two observed
//! failures, both HTTP 400 on the next turn after a switch:
//!
//! * A foreign or locally generated id is rejected by prefix — `Invalid
//!   'input[50].id': '6010…'. Expected an ID that begins with 'rs'.`
//! * A stored reference is unresolved on a stateless endpoint — `Item with id
//!   'rs_…' not found. Items are not persisted when store is set to false.`
//!
//! This module owns that projection, so no codec has to guess provider intent and
//! the catalog stays declaration-only: providers remain data, policy remains code.
//!
//! Two rules the design rests on:
//!
//! * The session log is never rewritten. Projection runs on a per-request copy.
//! * An id is only replayed to the issuer that minted it. Without recorded
//!   provenance the legal-for-every-issuer shape goes out (see [`ItemOrigin`]).
//!
//! Not a Catalog concern: nothing here is a provider declaration. `store` is the
//! one wire fact a model declares, and it is read, not re-declared.

use crate::authority::responses::{Item, MessageItem};
use crate::provider_catalog::ResolvedModel;

use std::collections::{HashMap, HashSet};

/// What the target endpoint does with a replayed item identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreMode {
    /// The endpoint persisted the item, so a reference can still resolve.
    Stored,
    /// The endpoint keeps nothing; every stored reference is unresolvable.
    Stateless,
}

impl StoreMode {
    /// The mode a model's declared wire extras imply. `store = false` is the only
    /// declaration that means "keeps nothing"; anything else is the protocol's own
    /// default, which stores.
    pub fn of_model(model: &ResolvedModel) -> Self {
        match model.extra_body.get("store").and_then(|value| value.as_bool()) {
            Some(false) => Self::Stateless,
            _ => Self::Stored,
        }
    }
}

/// One durable request boundary. `request_key` is unique for one turn/step;
/// provider item IDs are explicitly not used to recover this source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestOrigin {
    pub seq: u64,
    pub issuer: String,
    pub request_key: String,
}

/// Who minted one item's provider identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemOrigin {
    /// LiteCode built the item: user turn, reminder, or synthesized assistant text.
    Host,
    /// The endpoint and request that minted it, found through its durable seq.
    Issuer { issuer: String, request_key: String },
    /// Same ID appears at different request boundaries in active history.
    CrossCall,
    /// Written before provenance was recorded: ownership cannot be proven.
    Unknown,
}

/// A stable, non-secret name for the service that mints identities.
///
/// The catalog provider plus the normalized transport endpoint (including base
/// path). Two catalog entries that share a host but hold different credentials
/// are different services, so the provider id stays in the name; the API key,
/// query string, and fragment never do.
pub fn issuer_of_model(model: &ResolvedModel) -> String {
    format!("{}@{}", model.provider_id, transport_origin(&model.request_url))
}

/// Resolve an item to the newest recorded request boundary at or before its
/// durable seq. Provider item IDs never participate in this lookup.
pub fn origin_for_seq(known: &[RequestOrigin], seq: Option<u64>) -> ItemOrigin {
    let Some(seq) = seq else {
        return ItemOrigin::Host;
    };
    known
        .iter()
        .filter(|record| record.seq <= seq)
        .max_by_key(|record| record.seq)
        .map(|record| ItemOrigin::Issuer {
            issuer: record.issuer.clone(),
            request_key: record.request_key.clone(),
        })
        .unwrap_or(ItemOrigin::Unknown)
}

/// Resolve request-item origins directly from their durable seq sidecar.
/// A missing seq is positive proof that the item was host-synthesized.
pub fn origins_for_seqs(seqs: &[Option<u64>], known: &[RequestOrigin]) -> Vec<ItemOrigin> {
    seqs.iter().map(|seq| origin_for_seq(known, *seq)).collect()
}

/// IDs are grouping hints only. If an ID belongs to distinct request boundaries,
/// mark occurrences in this view ambiguous so the per-request copy drops the ID.
pub fn mark_cross_call_reuse(
    view: &[Item],
    view_origins: &mut [ItemOrigin],
    active_rows: &[(Item, ItemOrigin)],
) {
    let mut requests_by_id: HashMap<String, HashSet<String>> = HashMap::new();
    for (item, origin) in active_rows {
        let ItemOrigin::Issuer { request_key, .. } = origin else { continue };
        if let Some(key) = provider_id_key(item) {
            requests_by_id.entry(key).or_default().insert(request_key.clone());
        }
    }
    for (item, origin) in view.iter().zip(view_origins.iter_mut()) {
        if let Some(key) = provider_id_key(item)
            && requests_by_id.get(&key).is_some_and(|keys| keys.len() > 1)
        {
            *origin = ItemOrigin::CrossCall;
        }
    }
}

fn provider_id_key(item: &Item) -> Option<String> {
    let id = item_id(item)?;
    let kind = match WireKind::of(item) {
        WireKind::Reasoning => "reasoning",
        WireKind::OutputMessage => "message",
        WireKind::FunctionCall => "function_call",
        WireKind::FunctionCallOutput => "function_call_output",
        WireKind::Other => return None,
    };
    Some(format!("{kind}:{id}"))
}

/// `scheme://host[:port]/base/path` of a request URL, without credentials,
/// query, or fragment.
fn transport_origin(raw: &str) -> String {
    let Ok(url) = url::Url::parse(raw) else {
        // Catalog validation already checks request URLs; retain a stable opaque
        // key if a future caller constructs a model without passing that gate.
        return raw.to_string();
    };
    let Some(host) = url.host_str() else {
        return raw.to_string();
    };
    let port = url.port().map(|port| format!(":{port}")).unwrap_or_default();
    format!("{}://{host}{port}{}", url.scheme(), url.path())
}

/// The wire identity each item kind carries, when it carries one./// The wire identity each item kind carries, when it carries one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireKind {
    Reasoning,
    OutputMessage,
    FunctionCall,
    FunctionCallOutput,
    /// User/developer messages and non-call items: never a provider identity.
    Other,
}

impl WireKind {
    pub fn of(item: &Item) -> Self {
        match item {
            Item::Reasoning(_) => Self::Reasoning,
            Item::FunctionCall(_) => Self::FunctionCall,
            Item::FunctionCallOutput(_) => Self::FunctionCallOutput,
            Item::Message(MessageItem::Output(_)) => Self::OutputMessage,
            _ => Self::Other,
        }
    }

    /// The prefix this kind's identity carries in the Responses dialect. Kinds
    /// whose identity is a provider-local convention have none here, and their ids
    /// are treated as non-portable.
    fn prefix(self) -> Option<&'static str> {
        match self {
            Self::Reasoning => Some("rs_"),
            Self::OutputMessage => Some("msg_"),
            Self::FunctionCall => Some("fc_"),
            Self::FunctionCallOutput => None,
            Self::Other => None,
        }
    }
}

/// What projection decided, for the caller's log line. Counts only — never the
/// identities themselves, which stay out of logs.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ProjectionReport {
    pub identities_kept: usize,
    pub identities_stripped: usize,
    pub items_dropped: usize,
}

impl ProjectionReport {
    pub fn is_empty(&self) -> bool {
        self.identities_kept == 0 && self.identities_stripped == 0 && self.items_dropped == 0
    }

    fn merge(&mut self, other: Self) {
        self.identities_kept += other.identities_kept;
        self.identities_stripped += other.identities_stripped;
        self.items_dropped += other.items_dropped;
    }
}

/// A projected replay view plus what it changed. The view is a per-request value;
/// nothing here is ever written back to the session.
#[derive(Debug)]
pub struct Projected {
    pub items: Vec<Item>,
    pub report: ProjectionReport,
}

/// Project a replayed transcript onto the target endpoint's identity rules.
///
/// `origin_of` answers where each item's identity came from; a session with no
/// recorded provenance answers [`ItemOrigin::Unknown`] for every item.
///
/// Rules:
///
/// * Provider-minted ids are kept only for the issuer that minted them **and**
///   only when the endpoint still stores what the id refers to.
/// * A kind whose id does not carry the prefix its dialect requires loses the id,
///   never the item: text and `call_id` pairs survive.
/// * A reasoning item that would replay as a bare reference to storage that is
///   not there is dropped rather than sent as a dangling lookup. Ciphertext is
///   portable only back to its own issuer.
pub fn project_for_target(
    items: &[Item],
    origins: &[ItemOrigin],
    issuer: &str,
    store: StoreMode,
) -> Projected {
    let mut out = Vec::with_capacity(items.len());
    let mut report = ProjectionReport::default();
    for (index, item) in items.iter().enumerate() {
        let origin = origins.get(index).cloned().unwrap_or(ItemOrigin::Unknown);
        let decision = project_item(item, &origin, issuer, store);
        report.merge(decision.report);
        out.extend(decision.items);
    }
    Projected { items: out, report }
}

struct Decision {
    items: Vec<Item>,
    report: ProjectionReport,
}

impl Decision {
    fn keep(item: Item, kept: usize, stripped: usize, dropped: usize) -> Self {
        Self {
            items: vec![item],
            report: ProjectionReport {
                identities_kept: kept,
                identities_stripped: stripped,
                items_dropped: dropped,
            },
        }
    }

    fn drop_item() -> Self {
        Self {
            items: Vec::new(),
            report: ProjectionReport {
                items_dropped: 1,
                ..ProjectionReport::default()
            },
        }
    }
}

fn project_item(item: &Item, origin: &ItemOrigin, issuer: &str, store: StoreMode) -> Decision {
    let kind = WireKind::of(item);
    if matches!(kind, WireKind::Other) {
        // User/developer turns and host structure carry no provider identity.
        return Decision::keep(item.clone(), 0, 0, 0);
    }

    let same_issuer = matches!(origin, ItemOrigin::Issuer { issuer: id, .. } if id == issuer);
    let id_ok = kind.prefix().is_some_and(|prefix| {
        item_id(item).is_some_and(|id| id.starts_with(prefix))
    });

    if matches!(origin, ItemOrigin::Host) {
        // Host-built items must never claim a provider identity.
        return match item_id(item) {
            Some(_) => Decision::keep(without_id(item), 0, 1, 0),
            None => Decision::keep(item.clone(), 0, 0, 0),
        };
    }

    match kind {
        WireKind::Reasoning => project_reasoning(item, origin, issuer, store),
        // A stored reference is only usable by its own issuer on a storing
        // endpoint; anything else replays as plain content.
        WireKind::OutputMessage | WireKind::FunctionCall | WireKind::FunctionCallOutput => {
            if same_issuer && store == StoreMode::Stored && id_ok {
                Decision::keep(item.clone(), 1, 0, 0)
            } else if item_id(item).is_some() {
                Decision::keep(without_id(item), 0, 1, 0)
            } else {
                Decision::keep(item.clone(), 0, 0, 0)
            }
        }
        WireKind::Other => Decision::keep(item.clone(), 0, 0, 0),
    }
}



fn project_reasoning(
    item: &Item,
    origin: &ItemOrigin,
    issuer: &str,
    store: StoreMode,
) -> Decision {
    let Item::Reasoning(reasoning) = item else {
        return Decision::keep(item.clone(), 0, 0, 0);
    };
    let same_issuer = matches!(origin, ItemOrigin::Issuer { issuer: id, .. } if id == issuer);
    if !same_issuer {
        // Foreign or unproven: neither the id nor the ciphertext is this
        // service's to resolve. Visible summary text still carries meaning.
        return match visible_reasoning(item) {
            Some(visible) => Decision::keep(visible, 0, 1, 0),
            None => Decision::drop_item(),
        };
    }
    let id_ok = reasoning
        .id
        .as_deref()
        .is_some_and(|id| id.starts_with("rs_"));
    let encrypted = reasoning.encrypted_content.is_some();
    match store {
        // Its own service, which still stores what the id refers to.
        StoreMode::Stored if id_ok => Decision::keep(item.clone(), 1, 0, 0),
        // Nothing stored: ciphertext carries the state, the id does not.
        _ if encrypted => Decision::keep(without_id(item), 0, 1, 0),
        // A bare lookup that cannot resolve.
        _ => Decision::drop_item(),
    }
}

/// Visible reasoning summary a target can carry as ordinary assistant context.
///
/// The id and encrypted payload are provider state. Raw reasoning `content` is
/// not a public summary and is not transferred across issuers.
fn visible_reasoning(item: &Item) -> Option<Item> {
    let Item::Reasoning(reasoning) = item else {
        return None;
    };
    let summary = reasoning
        .summary
        .iter()
        .map(|part| match part {
            crate::authority::responses::SummaryPart::SummaryText(text) => text.text.as_str(),
        })
        .filter(|text| !text.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    (!summary.is_empty()).then(|| crate::types::assistant_text(summary))
}

fn item_id(item: &Item) -> Option<&str> {
    let id = match item {
        Item::Reasoning(reasoning) => reasoning.id.as_deref(),
        Item::Message(MessageItem::Output(message)) => Some(message.id.as_str()),
        Item::FunctionCall(call) => call.id.as_deref(),
        Item::FunctionCallOutput(output) => output.id.as_deref(),
        _ => None,
    };
    id.map(str::trim).filter(|id| !id.is_empty())
}

/// The same item with its provider identity removed. Content, `call_id` pairs,
/// summary, and ciphertext are untouched.
fn without_id(item: &Item) -> Item {
    let mut item = item.clone();
    match &mut item {
        Item::Reasoning(reasoning) => reasoning.id = None,
        Item::Message(MessageItem::Output(message)) => message.id = String::new(),
        Item::FunctionCall(call) => call.id = None,
        Item::FunctionCallOutput(output) => output.id = None,
        _ => {}
    }
    item
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authority::responses::{
        AssistantRole, FunctionCallOutputItemParam, FunctionToolCall, OutputMessage,
        OutputMessageContent, OutputStatus, OutputTextContent, ReasoningItem, ReasoningItemContent,
        ReasoningTextContent, SummaryPart, SummaryTextContent,
    };
    use crate::types::user_text;

    /// All-`Unknown` provenance, the shape a session written before origins were
    /// recorded projects with.
    fn unproven(count: usize) -> Vec<ItemOrigin> {
        vec![ItemOrigin::Unknown; count]
    }

    fn all(items: &[Item], origin: ItemOrigin) -> Vec<ItemOrigin> {
        vec![origin; items.len()]
    }

    fn issuer(name: &str) -> ItemOrigin {
        ItemOrigin::Issuer {
            issuer: name.into(),
            request_key: "turn:step".into(),
        }
    }

    fn reasoning(id: Option<&str>, encrypted: Option<&str>) -> Item {
        Item::Reasoning(ReasoningItem {
            id: id.map(str::to_string),
            summary: vec![],
            content: Some(vec![ReasoningItemContent::ReasoningText(
                ReasoningTextContent {
                    text: "think".into(),
                },
            )]),
            encrypted_content: encrypted.map(str::to_string),
            status: None,
        })
    }

    fn assistant(id: &str, text: &str) -> Item {
        Item::Message(MessageItem::Output(OutputMessage {
            id: id.into(),
            role: AssistantRole::Assistant,
            status: OutputStatus::Completed,
            phase: None,
            content: vec![OutputMessageContent::OutputText(OutputTextContent {
                text: text.into(),
                annotations: vec![],
                logprobs: None,
            })],
        }))
    }

    fn reasoning_summary(id: &str, text: &str) -> Item {
        Item::Reasoning(ReasoningItem {
            id: Some(id.into()),
            summary: vec![SummaryPart::SummaryText(SummaryTextContent {
                text: text.into(),
            })],
            content: None,
            encrypted_content: Some("gAAAA".into()),
            status: None,
        })
    }

    fn call(id: Option<&str>, call_id: &str) -> Item {
        Item::FunctionCall(FunctionToolCall {
            id: id.map(str::to_string),
            call_id: call_id.into(),
            name: "read".into(),
            arguments: "{}".into(),
            status: None,
            namespace: None,
        })
    }

    fn output(id: Option<&str>, call_id: &str) -> Item {
        Item::FunctionCallOutput(FunctionCallOutputItemParam {
            id: id.map(str::to_string),
            call_id: call_id.into(),
            output: crate::authority::responses::FunctionCallOutput::Text("ok".into()),
            status: None,
        })
    }

    /// The reported failure: a locally generated UUID replayed as a reasoning id.
    /// Without a public summary the opaque reasoning is dropped; the id never
    /// reaches the wire.
    #[test]
    fn foreign_reasoning_uuid_never_reaches_the_wire() {
        let items = vec![
            user_text("hi"),
            reasoning(Some("60102752-7590-49a2-92bf-cd3e631b96bf"), None),
            assistant("msg_1", "done"),
        ];
        let projected = project_for_target(
            &items,
            &unproven(items.len()),
            "opencode@https://x/responses",
            StoreMode::Stored,
        );
        assert_eq!(projected.items.len(), 2, "{:?}", projected.items);
        assert_eq!(projected.report.items_dropped, 1);
        assert!(matches!(projected.items[0], Item::Message(MessageItem::Input(_))));
        assert!(matches!(projected.items[1], Item::Message(MessageItem::Output(_))));
    }

    /// A stored reference is only usable by its own issuer; content survives.
    #[test]
    fn non_portable_ids_are_stripped_not_rewritten() {
        let items = vec![
            user_text("hi"),
            assistant("foreign-uuid", "text stays"),
            call(Some("tooluse_x"), "call_1"),
            output(Some("chatcmpl-1"), "call_1"),
        ];
        let projected = project_for_target(&items, &unproven(items.len()), "t@https://x", StoreMode::Stored);
        assert_eq!(projected.items.len(), 4);
        assert_eq!(projected.report.identities_stripped, 3);
        let call_id = match &projected.items[2] {
            Item::FunctionCall(call) => call.call_id.clone(),
            other => panic!("expected call, got {other:?}"),
        };
        let out_id = match &projected.items[3] {
            Item::FunctionCallOutput(out) => out.call_id.clone(),
            other => panic!("expected output, got {other:?}"),
        };
        assert_eq!(call_id, out_id, "pairing must survive id removal");
        assert!(item_id(&projected.items[1]).is_none());
        // The session's own copy is untouched.
        assert_eq!(item_id(&items[1]), Some("foreign-uuid"));
    }

    /// The issuer that minted the identity may keep replaying it while the
    /// endpoint still stores the item.
    #[test]
    fn same_issuer_keeps_stored_identities() {
        let items = vec![reasoning(Some("rs_1"), None), assistant("msg_1", "hi")];
        
        let projected = project_for_target(&items, &all(&items, issuer("me@https://x/responses")), "me@https://x/responses", StoreMode::Stored);
        assert_eq!(projected.report.identities_kept, 2);
        assert_eq!(projected.report.items_dropped, 0);
        assert_eq!(item_id(&projected.items[0]), Some("rs_1"));
    }

    /// `store = false` keeps nothing, so a bare stored reference is a dangling
    /// lookup; ciphertext carries the same state without it.
    #[test]
    fn stateless_target_drops_bare_references_and_keeps_ciphertext() {
        let items = vec![
            reasoning(Some("rs_bare"), None),
            reasoning(Some("rs_cipher"), Some("gAAAA")),
            assistant("msg_1", "hi"),
            call(Some("fc_1"), "call_1"),
        ];
        
        let projected = project_for_target(&items, &all(&items, issuer("me@https://x/responses")), "me@https://x/responses", StoreMode::Stateless);
        assert_eq!(projected.items.len(), 3, "{:?}", projected.items);
        assert_eq!(projected.report.items_dropped, 1);
        let Item::Reasoning(kept) = &projected.items[0] else {
            panic!("expected reasoning");
        };
        assert_eq!(kept.id, None, "ciphertext item survives without its id");
        assert_eq!(kept.encrypted_content.as_deref(), Some("gAAAA"));
        assert!(item_id(&projected.items[1]).is_none(), "message keeps text only");
        assert!(item_id(&projected.items[2]).is_none(), "call keeps call_id");
    }

    /// Another service's ciphertext is not portable. Without a public summary the
    /// reasoning is dropped; with one, only that summary becomes normal context.
    #[test]
    fn foreign_ciphertext_is_not_replayed() {
        let raw = vec![reasoning(Some("rs_1"), Some("gAAAA"))];
        let foreign = all(&raw, issuer("other@https://y/responses"));
        let projected = project_for_target(&raw, &foreign, "me@https://x/responses", StoreMode::Stored);
        assert!(projected.items.is_empty(), "private reasoning is not portable");
        assert_eq!(projected.report.items_dropped, 1);

        let summary = vec![reasoning_summary("rs_2", "brief public summary")];
        let foreign = all(&summary, issuer("other@https://y/responses"));
        let projected = project_for_target(&summary, &foreign, "me@https://x/responses", StoreMode::Stored);
        assert_eq!(projected.items.len(), 1);
        let Item::Message(MessageItem::Output(message)) = &projected.items[0] else {
            panic!("a foreign summary is ordinary assistant context");
        };
        assert!(message.id.is_empty(), "the provider id is not carried over");
        assert_eq!(crate::types::item_text_preview(&projected.items[0]), "brief public summary");
        assert_eq!(projected.report.identities_stripped, 1);

        // With ciphertext but no public text there is no portable fallback.
        let opaque = vec![Item::Reasoning(ReasoningItem {
            id: Some("rs_1".into()),
            summary: vec![],
            content: None,
            encrypted_content: Some("gAAAA".into()),
            status: None,
        })];
        let foreign = all(&opaque, issuer("other@https://y/responses"));
        let projected = project_for_target(&opaque, &foreign, "me@https://x/responses", StoreMode::Stored);
        assert!(projected.items.is_empty());
        assert_eq!(projected.report.items_dropped, 1);
    }

    #[test]
    fn host_items_never_claim_an_identity() {
        let mut host = assistant("", "synthesized");
        if let Item::Message(MessageItem::Output(message)) = &mut host {
            message.id = "uuid-looking".into();
        }
        
        let origins = vec![ItemOrigin::Host];
        let projected = project_for_target(&[host], &origins, "me@https://x/responses", StoreMode::Stored);
        assert!(item_id(&projected.items[0]).is_none());
        assert_eq!(projected.report.identities_stripped, 1);
    }

    /// Source resolution is seq -> latest request header, independent of item IDs.
    #[test]
    fn provenance_uses_durable_seq_boundaries() {
        let known = vec![
            RequestOrigin { seq: 5, issuer: "one".into(), request_key: "t:1".into() },
            RequestOrigin { seq: 12, issuer: "two".into(), request_key: "t:2".into() },
        ];
        let origins = origins_for_seqs(&[None, Some(10), Some(14)], &known);
        assert_eq!(origins[0], ItemOrigin::Host);
        assert_eq!(origins[1], ItemOrigin::Issuer {
            issuer: "one".into(),
            request_key: "t:1".into(),
        });
        assert_eq!(origins[2], ItemOrigin::Issuer {
            issuer: "two".into(), request_key: "t:2".into(),
        });
    }

    /// Reuse of an otherwise valid provider ID in different calls is removed only
    /// from this outbound copy; both semantic messages remain.
    #[test]
    fn ids_reused_across_calls_are_deidentified_not_collapsed() {
        let first = assistant("msg_reused", "first text");
        let second = assistant("msg_reused", "second text");
        let view = vec![first.clone(), second.clone()];
        let mut origins = vec![
            ItemOrigin::Issuer { issuer: "one".into(), request_key: "t:1".into() },
            ItemOrigin::Issuer { issuer: "one".into(), request_key: "t:2".into() },
        ];
        let rows = vec![(first, origins[0].clone()), (second, origins[1].clone())];
        mark_cross_call_reuse(&view, &mut origins, &rows);
        assert_eq!(origins, vec![ItemOrigin::CrossCall, ItemOrigin::CrossCall]);
        let projected = project_for_target(&view, &origins, "one", StoreMode::Stored);
        assert_eq!(projected.items.len(), 2, "do not merge history items");
        assert_eq!(crate::types::item_text_preview(&projected.items[0]), "first text");
        assert_eq!(crate::types::item_text_preview(&projected.items[1]), "second text");
        assert!(projected.items.iter().all(|item| item_id(item).is_none()));
    }

    /// A single call reusing an item id is not reclassified as a cross-call
    /// conflict; StreamProjection owns that current-call invariant.
    #[test]
    fn same_call_duplicate_is_not_a_cross_call_collision() {
        let a = assistant("msg_reused", "a");
        let b = assistant("msg_reused", "b");
        let view = vec![a.clone(), b.clone()];
        let origin = ItemOrigin::Issuer { issuer: "one".into(), request_key: "t:1".into() };
        let mut origins = vec![origin.clone(), origin.clone()];
        mark_cross_call_reuse(&view, &mut origins, &[(a, origin.clone()), (b, origin)]);
        assert!(origins.iter().all(|origin| matches!(origin, ItemOrigin::Issuer { .. })));
    }

    #[test]
    fn store_mode_reads_the_declared_extra() {
        use std::path::Path;
        use crate::provider_catalog::ProviderCatalog;
        let text = r#"
version = 1
[[providers]]
id = "p"
name = "P"
endpoint = "https://x.example/v1"
endpoint_type = "responses"

[[models]]
id = "stateless"
provider_id = "p"
extra_body = { store = false }

[[models]]
id = "default"
provider_id = "p"
"#;
        let catalog = ProviderCatalog::parse(text, Path::new("t.toml")).unwrap();
        let stateless = catalog.model("p/stateless").unwrap();
        let stored = catalog.model("p/default").unwrap();
        assert_eq!(StoreMode::of_model(stateless), StoreMode::Stateless);
        assert_eq!(StoreMode::of_model(stored), StoreMode::Stored);
    }

    #[test]
    fn issuer_names_the_service_and_normalized_endpoint_path() {
        use std::path::Path;
        use crate::provider_catalog::ProviderCatalog;
        let text = r#"
version = 1
[[providers]]
id = "opencode"
name = "OpenCode Zen"
endpoint = "https://opencode.ai/zen/v1"
endpoint_type = "responses"

[[models]]
id = "m"
provider_id = "opencode"
"#;
        let catalog = ProviderCatalog::parse(text, Path::new("t.toml")).unwrap();
        let model = catalog.model("opencode/m").unwrap();
        assert_eq!(issuer_of_model(model), "opencode@https://opencode.ai/zen/v1/responses");
    }
}
