//! Projection consistency: log fold, change-log watermark.

mod common;

use common::SessionDataFixture;
use litecode::types::user_text;

#[test]
fn dirty_sessions_coalesce_successful_mutations() {
    let fixture = SessionDataFixture::new();
    let data = &fixture.data;
    let sid = data.create_session("/p", "default", None).unwrap();
    data.insert_items(&sid, &[user_text("one")]).unwrap();
    data.insert_items(&sid, &[user_text("two")]).unwrap();
    let latest = data.reader().latest_change_id_blocking().unwrap();
    assert!(latest >= 3, "create + 2 inserts, got {latest}");
    let dirty = data.reader().dirty_sessions_since_blocking(0).unwrap();
    assert_eq!(dirty.len(), 1, "one row per session, not one per mutation");
    assert_eq!(dirty[0].session_id, sid);
    assert_eq!(dirty[0].change_id, latest);
}

#[test]
fn hot_transcript_matches_committed_log() {
    let fixture = SessionDataFixture::new();
    let data = &fixture.data;
    let sid = data.create_session("/p", "default", None).unwrap();
    data.insert_items(&sid, &[user_text("a"), user_text("b"), user_text("c")])
        .unwrap();
    let transcript = data.transcript_blocking(&sid).unwrap();
    let events = data.events_blocking(&sid).unwrap();
    assert_eq!(transcript.len(), 3);
    assert_eq!(events.len(), 3);
}
