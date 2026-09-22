//! Projection consistency: log fold, change-log watermark.

mod common;

use common::SessionDataFixture;
use litecode::types::user_text;

#[test]
fn change_log_tracks_each_successful_mutation() {
    let fixture = SessionDataFixture::new();
    let data = &fixture.data;
    let sid = data.create_session("/p", "default", None).unwrap();
    data.insert_items(&sid, &[user_text("one")]).unwrap();
    data.insert_items(&sid, &[user_text("two")]).unwrap();
    let latest = data.reader().latest_change_id_blocking().unwrap();
    assert!(latest >= 3, "create + 2 inserts, got {latest}");
    let changes = data.reader().change_log_since_blocking(0).unwrap();
    assert!(changes.iter().any(|c| c.session_id == sid));
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
