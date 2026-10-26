//! One derivation of the model-visible working set.
//!
//! `WorkingView` replaced a single-shot replay with a fold that can also extend
//! an existing view. These tests pin that the new fold matches the algorithm it
//! replaced (kept here as a frozen oracle) and that extending a view event by
//! event matches folding the whole log at once.

mod common;

use std::collections::HashSet;

use common::session_data_fixture::SessionDataFixture;
use litecode::session::SurfaceOp;
use litecode::session::event::{EventType, SessionEvent, item_from_event, spine_working_payload};
use litecode::session::media::item_has_media;
use litecode::session::model::LogState;
use litecode::session::surface::{fold_surface, project_working_payloads};
use litecode::session::view::WorkingView;
use litecode::session::working::WorkingRow;
use litecode::types::{Item, LitecodeError, Result, assistant_text, user_message, user_text};

/// Frozen copy of the pre-refactor read-side derivation. Do not "fix" this to
/// match the new code: it is the reference the new code is measured against.
fn frozen_rows(events: &[SessionEvent]) -> Result<Vec<WorkingRow>> {
    let surface = fold_surface(events)?;
    let events_by_seq: std::collections::HashMap<_, _> =
        events.iter().map(|event| (event.seq, event)).collect();
    let replacement_media: std::collections::HashSet<_> = surface
        .nodes
        .iter()
        .filter_map(|seq| events_by_seq.get(seq).copied())
        .filter(|event| matches!(event.surface_op, Some(SurfaceOp::Replace { .. })))
        .filter(|event| {
            event.source_seqs.as_ref().is_some_and(|sources| {
                sources.iter().any(|seq| {
                    events_by_seq
                        .get(seq)
                        .and_then(|source| item_from_event(source).ok())
                        .is_some_and(|item| item_has_media(&item))
                })
            })
        })
        .map(|event| event.seq)
        .collect();
    let pairs = project_working_payloads(&surface, |seq| {
        events_by_seq
            .get(&seq)
            .ok_or_else(|| LitecodeError::InvalidSessionEvent(format!("surface seq {seq} missing")))
            .and_then(|event| spine_working_payload(event))
    })?;
    Ok(pairs
        .into_iter()
        .map(|(seq, payload)| WorkingRow {
            log_seq: Some(seq),
            payload,
            replacement_media: replacement_media.contains(&seq),
        })
        .collect())
}

fn event(
    seq: u64,
    event_type: EventType,
    item: Item,
    op: SurfaceOp,
    state: LogState,
    sources: Option<Vec<u64>>,
) -> SessionEvent {
    SessionEvent {
        seq,
        time: 0,
        event_type,
        data: serde_json::to_value(item).expect("item json"),
        result_data: None,
        surface_op: Some(op),
        source_seqs: sources,
        state,
        ignorable: false,
    }
}

fn user(seq: u64, text: &str) -> SessionEvent {
    event(
        seq,
        EventType::ItemUser,
        user_text(text),
        SurfaceOp::Append,
        LogState::Final,
        None,
    )
}

#[test]
fn cold_fold_matches_the_frozen_algorithm_on_a_real_log() {
    let fx = SessionDataFixture::new();
    let sid = fx.create("/p", "agent", None);
    let items: Vec<Item> = (0..12).map(|i| user_text(format!("turn {i}"))).collect();
    fx.insert_items(&sid, &items);

    let events = fx.data.events_blocking(&sid).expect("events");
    assert_eq!(events.len(), items.len());

    let view = WorkingView::from_events(&events)
        .expect("fold")
        .rows()
        .expect("rows");
    let frozen = frozen_rows(&events).expect("frozen");
    let from_db = fx.data.working_set_blocking(&sid).expect("db read");

    assert_eq!(view, frozen);
    assert_eq!(from_db, frozen);
    assert_eq!(view.len(), items.len());
}

#[test]
fn extending_a_view_matches_folding_the_whole_log() {
    let fx = SessionDataFixture::new();
    let sid = fx.create("/p", "agent", None);
    let items: Vec<Item> = (0..11).map(|i| user_text(format!("turn {i}"))).collect();
    fx.insert_items(&sid, &items);
    let events = fx.data.events_blocking(&sid).expect("events");

    let cold = WorkingView::from_events(&events)
        .expect("cold")
        .rows()
        .expect("rows");

    let mut one_by_one = WorkingView::default();
    for event in &events {
        one_by_one.apply(event).expect("apply");
    }
    assert_eq!(one_by_one.rows().unwrap(), cold);

    let mut chunked = WorkingView::default();
    let mut end = 0;
    for chunk in events.chunks(4) {
        for event in chunk {
            chunked.apply(event).expect("apply");
        }
        end += chunk.len();
        let folded_so_far = WorkingView::from_events(&events[..end]).expect("prefix");
        assert_eq!(chunked.rows().unwrap(), folded_so_far.rows().unwrap());
    }
}

#[test]
fn a_seal_rewrites_a_row_without_appending_one() {
    let mut events = vec![
        user(0, "hello"),
        event(
            1,
            EventType::ItemAssistant,
            assistant_text("draft"),
            SurfaceOp::Append,
            LogState::InProgress,
            None,
        ),
    ];
    let mut view = WorkingView::from_events(&events).expect("fold");
    assert_eq!(view.max_seq(), Some(1));
    assert_eq!(view.in_progress_seqs(), &HashSet::from([1u64]));
    assert_eq!(view.rows().unwrap(), frozen_rows(&events).unwrap());

    // The seal keeps the seq and the op; only the body and the state move.
    events[1] = event(
        1,
        EventType::ItemAssistant,
        assistant_text("done"),
        SurfaceOp::Append,
        LogState::Final,
        None,
    );
    assert!(view.refresh(&events[1]).expect("refresh"));
    assert!(view.in_progress_seqs().is_empty());
    assert_eq!(view.rows().unwrap(), frozen_rows(&events).unwrap());
    assert_eq!(view.max_seq(), Some(1), "a seal must not move the high-water");
}

#[test]
fn a_replacement_reports_media_from_the_row_it_shadows() {
    let events = vec![
        event(
            0,
            EventType::ItemUser,
            user_message("image", &["litecode-media:abc.png".to_string()]),
            SurfaceOp::Append,
            LogState::Final,
            None,
        ),
        user(1, "follow up"),
        event(
            2,
            EventType::ItemAssistant,
            assistant_text("summary of two rows"),
            SurfaceOp::Replace { start: 0, end: 1 },
            LogState::Final,
            Some(vec![0, 1]),
        ),
    ];

    let view = WorkingView::from_events(&events).expect("fold");
    let rows = view.rows().expect("rows");
    assert_eq!(rows, frozen_rows(&events).unwrap());
    assert_eq!(rows.len(), 1, "both shadowed rows collapse into the replacement");
    assert!(rows[0].replacement_media, "source row 0 carried an image");

    // Without a media source the flag stays off in both implementations.
    let plain = vec![
        user(0, "a"),
        user(1, "b"),
        event(
            2,
            EventType::ItemAssistant,
            assistant_text("summary"),
            SurfaceOp::Replace { start: 0, end: 1 },
            LogState::Final,
            Some(vec![0, 1]),
        ),
    ];
    let plain_rows = WorkingView::from_events(&plain).unwrap().rows().unwrap();
    assert_eq!(plain_rows, frozen_rows(&plain).unwrap());
    assert!(!plain_rows[0].replacement_media);
}

#[test]
fn a_truncated_log_folds_to_the_shorter_view() {
    let events: Vec<SessionEvent> = (0..5).map(|i| user(i, "row")).collect();
    let full = WorkingView::from_events(&events).unwrap();
    assert_eq!(full.max_seq(), Some(4));

    let prefix = &events[..3];
    let short = WorkingView::from_events(prefix).unwrap();
    assert_eq!(short.max_seq(), Some(2));
    assert_eq!(short.rows().unwrap(), frozen_rows(prefix).unwrap());
    assert_ne!(short.rows().unwrap(), full.rows().unwrap());
}

#[test]
fn retroactive_ops_are_classified_exhaustively() {
    assert!(!SurfaceOp::Append.is_retroactive(None));
    assert!(!SurfaceOp::Append.is_retroactive(Some(9)));

    let replace = SurfaceOp::Replace { start: 3, end: 5 };
    assert!(!replace.is_retroactive(None), "empty view has nothing to rewrite");
    assert!(!replace.is_retroactive(Some(2)), "start sits above the held high-water");
    assert!(replace.is_retroactive(Some(3)), "start touches a folded row");
    assert!(replace.is_retroactive(Some(9)));
}
