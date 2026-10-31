//! Display-only session layout through the single writer: title override and
//! folder placement. These writes must never bump the transcript CAS revision
//! or the sidebar/GC clock.

mod common;

use common::SessionDataFixture;
use litecode::types::user_text;

fn create_with_preview(fixture: &SessionDataFixture, preview: &str) -> String {
    let sid = fixture.create("/proj", "default", None);
    fixture.insert_items(&sid, &[user_text(preview)]);
    sid
}

#[test]
fn title_is_revision_neutral_and_visible_in_meta_and_list() {
    let fixture = SessionDataFixture::new();
    let data = fixture.data.clone();
    let sid = create_with_preview(&fixture, "auto preview");

    let before = data.meta_blocking(&sid).unwrap();
    let revision = data.revision_blocking(&sid).unwrap();

    data.set_session_title_blocking(&sid, Some("  Release checklist  "))
        .unwrap();

    let after = data.meta_blocking(&sid).unwrap();
    assert_eq!(after.title.as_deref(), Some("Release checklist"));
    assert_eq!(
        after.preview, before.preview,
        "title must not overwrite preview"
    );
    assert_eq!(
        after.updated_at, before.updated_at,
        "title must not reorder the sidebar"
    );
    assert_eq!(
        data.revision_blocking(&sid).unwrap(),
        revision,
        "title must not invalidate a running turn's transcript CAS"
    );

    let row = data
        .list_sessions_blocking()
        .unwrap()
        .into_iter()
        .find(|row| row.id == sid)
        .expect("session listed");
    assert_eq!(row.title.as_deref(), Some("Release checklist"));
    assert_eq!(row.preview, before.preview);

    // Whitespace-only clears the override and falls back to the preview.
    data.set_session_title_blocking(&sid, Some("   ")).unwrap();
    assert_eq!(data.meta_blocking(&sid).unwrap().title, None);
}

#[test]
fn folder_crud_move_and_delete_returns_sessions_to_ungrouped() {
    let fixture = SessionDataFixture::new();
    let data = fixture.data.clone();
    let sid = create_with_preview(&fixture, "filed session");
    let revision = data.revision_blocking(&sid).unwrap();
    let updated_at = data.meta_blocking(&sid).unwrap().updated_at;

    let folder = data.create_session_folder_blocking("Work").unwrap();
    let folders = data.list_folders_blocking().unwrap();
    assert_eq!(folders.len(), 1);
    assert_eq!(folders[0].id, folder);
    assert_eq!(folders[0].name, "Work");

    data.set_session_folder_blocking(&sid, Some(&folder))
        .unwrap();
    let meta = data.meta_blocking(&sid).unwrap();
    assert_eq!(meta.folder_id.as_deref(), Some(folder.as_str()));
    assert_eq!(
        meta.updated_at, updated_at,
        "moving must not reorder the sidebar"
    );
    assert_eq!(
        data.revision_blocking(&sid).unwrap(),
        revision,
        "moving must not invalidate a running turn's transcript CAS"
    );
    let row = data
        .list_sessions_blocking()
        .unwrap()
        .into_iter()
        .find(|row| row.id == sid)
        .expect("session listed");
    assert_eq!(row.folder_id.as_deref(), Some(folder.as_str()));

    data.rename_session_folder_blocking(&folder, "Planning")
        .unwrap();
    assert_eq!(data.list_folders_blocking().unwrap()[0].name, "Planning");

    data.delete_session_folder_blocking(&folder).unwrap();
    assert!(data.list_folders_blocking().unwrap().is_empty());
    assert!(
        data.meta_blocking(&sid).is_ok(),
        "deleting a folder must never delete its sessions"
    );
    assert_eq!(data.meta_blocking(&sid).unwrap().folder_id, None);
}

#[test]
fn moving_to_an_unknown_folder_is_rejected_without_touching_the_session() {
    let fixture = SessionDataFixture::new();
    let data = fixture.data.clone();
    let sid = create_with_preview(&fixture, "unmoved");

    let error = data
        .set_session_folder_blocking(&sid, Some("missing-folder"))
        .expect_err("unknown folder must be rejected");
    assert!(
        error.to_string().contains("folder not found"),
        "unexpected error: {error}"
    );
    assert_eq!(data.meta_blocking(&sid).unwrap().folder_id, None);
}

#[test]
fn empty_folders_are_listed_and_new_folders_append_in_sort_order() {
    let fixture = SessionDataFixture::new();
    let data = fixture.data.clone();
    let first = data.create_session_folder_blocking("First").unwrap();
    let second = data.create_session_folder_blocking("Second").unwrap();

    let folders = data.list_folders_blocking().unwrap();
    assert_eq!(
        folders.iter().map(|f| f.id.as_str()).collect::<Vec<_>>(),
        vec![first.as_str(), second.as_str()]
    );
}

#[test]
fn root_list_caps_each_folder_bucket_separately() {
    let fixture = SessionDataFixture::new();
    let data = fixture.data.clone();
    let folder = data.create_session_folder_blocking("Archive").unwrap();
    let filed = create_with_preview(&fixture, "filed, older than the rest");
    data.set_session_folder_blocking(&filed, Some(&folder))
        .unwrap();

    // Make the filed root older than every ungrouped root. With a global
    // LIMIT 50 it would fall off the list; per-folder buckets must keep it.
    let conn = rusqlite::Connection::open(fixture.db_path()).unwrap();
    conn.execute("UPDATE sessions SET updated_at = 1 WHERE id = ?1", [&filed])
        .unwrap();
    for _ in 0..55 {
        fixture.create("/proj", "default", None);
    }

    let rows = data.list_sessions_blocking().unwrap();
    assert!(
        rows.iter().any(|row| row.id == filed),
        "a filed root must survive 55 newer ungrouped roots"
    );
    let ungrouped = rows
        .iter()
        .filter(|row| row.parent_session_id.is_none() && row.folder_id.is_none())
        .count();
    assert_eq!(ungrouped, 50, "each bucket is capped at 50 roots");
    let in_folder = rows
        .iter()
        .filter(|row| row.folder_id.as_deref() == Some(folder.as_str()))
        .count();
    assert_eq!(in_folder, 1);
}

#[test]
fn folder_names_and_titles_reject_empty_values() {
    let fixture = SessionDataFixture::new();
    let data = fixture.data.clone();
    let sid = create_with_preview(&fixture, "named");

    assert!(data.create_session_folder_blocking("   ").is_err());
    assert!(
        data.set_session_title_blocking(&sid, Some("   ")).is_ok(),
        "clearing a title is allowed; only folder names require content"
    );
}
