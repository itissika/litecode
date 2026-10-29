//! Session-scoped working memory.
//!
//! This is a small, agent-owned list of facts/preferences/decisions that must
//! survive compaction. It is deliberately not a second transcript and not a
//! long-term memory store: entries are bounded, mutable, and scoped to one
//! session row.

use serde::{Deserialize, Serialize};

/// Maximum entries kept in one session.
pub const MEMORY_MAX_ENTRIES: usize = 32;
/// Maximum characters in one entry text.
pub const MEMORY_MAX_TEXT_CHARS: usize = 200;
/// Maximum characters across all entry texts.
pub const MEMORY_MAX_TOTAL_CHARS: usize = 4_000;
/// Maximum entries accepted by one upsert/delete call.
pub const MEMORY_MAX_BATCH: usize = 32;
/// Maximum filter query length.
pub const MEMORY_MAX_QUERY_CHARS: usize = 100;

/// One session's memory document. Stored as JSON in `sessions.memory_json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionMemory {
    /// Next numeric id; ids are never reused. `m1`, `m2`, ...
    #[serde(default = "default_next_id")]
    pub next_id: u32,
    #[serde(default)]
    pub entries: Vec<MemoryEntry>,
}

fn default_next_id() -> u32 {
    1
}

impl Default for SessionMemory {
    fn default() -> Self {
        Self {
            next_id: 1,
            entries: Vec::new(),
        }
    }
}

/// One durable memory entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub id: String,
    pub text: String,
    #[serde(default)]
    pub kind: MemoryKind,
    #[serde(default)]
    pub updated_at: i64,
}

/// Small closed vocabulary. A free-form tag would drift into noise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MemoryKind {
    #[default]
    Fact,
    Preference,
    Decision,
    Constraint,
}

impl MemoryKind {
    pub const ALL: &'static [MemoryKind] = &[
        Self::Fact,
        Self::Preference,
        Self::Decision,
        Self::Constraint,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fact => "fact",
            Self::Preference => "preference",
            Self::Decision => "decision",
            Self::Constraint => "constraint",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "fact" => Some(Self::Fact),
            "preference" => Some(Self::Preference),
            "decision" => Some(Self::Decision),
            "constraint" => Some(Self::Constraint),
            _ => None,
        }
    }
}

/// One upsert item after the tool has read the JSON shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryUpsertRequest {
    pub id: Option<String>,
    pub text: String,
    pub kind: Option<String>,
}

/// One rejected batch item. Reasons are stable wire strings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryReject {
    pub index: usize,
    pub id: Option<String>,
    pub reason: &'static str,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemoryUpsertResult {
    pub added: Vec<String>,
    pub updated: Vec<String>,
    pub rejected: Vec<MemoryReject>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemoryDeleteResult {
    pub deleted: Vec<String>,
    pub rejected: Vec<MemoryReject>,
}

impl MemoryEntry {
    pub fn matches(&self, query: Option<&str>, kind: Option<MemoryKind>) -> bool {
        if let Some(kind) = kind {
            if self.kind != kind {
                return false;
            }
        }
        let Some(query) = query.map(str::trim).filter(|query| !query.is_empty()) else {
            return true;
        };
        let needle = query.to_lowercase();
        self.text.to_lowercase().contains(&needle)
    }
}

impl SessionMemory {
    pub fn total_chars(&self) -> usize {
        self.entries
            .iter()
            .map(|entry| entry.text.chars().count())
            .sum()
    }

    pub fn entry(&self, id: &str) -> Option<&MemoryEntry> {
        self.entries.iter().find(|entry| entry.id == id)
    }

    /// Repair ids after loading a document written before `next_id` existed.
    pub fn normalize(&mut self) {
        let max = self
            .entries
            .iter()
            .filter_map(|entry| entry.id.strip_prefix('m')?.parse::<u32>().ok())
            .max()
            .unwrap_or(0);
        self.next_id = self.next_id.max(max.saturating_add(1)).max(1);
    }

    /// Apply a batch. A bad item is reported, never silently dropped and never
    /// allowed to abort the rest of the batch.
    pub fn apply_upsert(
        &mut self,
        requests: &[MemoryUpsertRequest],
        now: i64,
    ) -> MemoryUpsertResult {
        self.normalize();
        let mut result = MemoryUpsertResult::default();
        for (index, request) in requests.iter().enumerate() {
            let text = request.text.trim();
            let text_len = text.chars().count();
            if text.is_empty() {
                result.rejected.push(MemoryReject {
                    index,
                    id: request.id.clone(),
                    reason: "empty_text",
                });
                continue;
            }
            if text_len > MEMORY_MAX_TEXT_CHARS {
                result.rejected.push(MemoryReject {
                    index,
                    id: request.id.clone(),
                    reason: "text_too_long",
                });
                continue;
            }
            let kind = match request.kind.as_deref() {
                None => None,
                Some(raw) => match MemoryKind::parse(raw) {
                    Some(kind) => Some(kind),
                    None => {
                        result.rejected.push(MemoryReject {
                            index,
                            id: request.id.clone(),
                            reason: "invalid_kind",
                        });
                        continue;
                    }
                },
            };

            let explicit_id = match request.id.as_deref() {
                None => None,
                Some(raw) => {
                    let id = raw.trim();
                    if id.is_empty() {
                        result.rejected.push(MemoryReject {
                            index,
                            id: None,
                            reason: "invalid_id",
                        });
                        continue;
                    }
                    Some(id)
                }
            };

            if let Some(id) = explicit_id {
                let Some(position) = self.entries.iter().position(|entry| entry.id == id) else {
                    result.rejected.push(MemoryReject {
                        index,
                        id: Some(id.to_string()),
                        reason: "unknown_id",
                    });
                    continue;
                };
                let new_total =
                    self.total_chars() - self.entries[position].text.chars().count() + text_len;
                if new_total > MEMORY_MAX_TOTAL_CHARS {
                    result.rejected.push(MemoryReject {
                        index,
                        id: Some(id.to_string()),
                        reason: "limit_exceeded",
                    });
                    continue;
                }
                let entry = &mut self.entries[position];
                entry.text = text.to_string();
                if let Some(kind) = kind {
                    entry.kind = kind;
                }
                entry.updated_at = now;
                result.updated.push(entry.id.clone());
                continue;
            }

            let duplicate = self
                .entries
                .iter()
                .position(|entry| entry.text.trim().to_lowercase() == text.to_lowercase());
            if let Some(position) = duplicate {
                let new_total =
                    self.total_chars() - self.entries[position].text.chars().count() + text_len;
                if new_total > MEMORY_MAX_TOTAL_CHARS {
                    result.rejected.push(MemoryReject {
                        index,
                        id: None,
                        reason: "limit_exceeded",
                    });
                    continue;
                }
                let entry = &mut self.entries[position];
                entry.text = text.to_string();
                if let Some(kind) = kind {
                    entry.kind = kind;
                }
                entry.updated_at = now;
                result.updated.push(entry.id.clone());
                continue;
            }

            if self.entries.len() >= MEMORY_MAX_ENTRIES
                || self.total_chars() + text_len > MEMORY_MAX_TOTAL_CHARS
            {
                result.rejected.push(MemoryReject {
                    index,
                    id: None,
                    reason: "limit_exceeded",
                });
                continue;
            }

            let id = format!("m{}", self.next_id);
            self.next_id = self.next_id.saturating_add(1);
            self.entries.push(MemoryEntry {
                id: id.clone(),
                text: text.to_string(),
                kind: kind.unwrap_or_default(),
                updated_at: now,
            });
            result.added.push(id);
        }
        self.normalize();
        result
    }

    /// Remove entries by id. Unknown ids are reported per item.
    pub fn apply_delete(&mut self, ids: &[String]) -> MemoryDeleteResult {
        let mut result = MemoryDeleteResult::default();
        for (index, raw) in ids.iter().enumerate() {
            let id = raw.trim();
            if id.is_empty() {
                result.rejected.push(MemoryReject {
                    index,
                    id: None,
                    reason: "invalid_id",
                });
                continue;
            }
            if let Some(position) = self.entries.iter().position(|entry| entry.id == id) {
                self.entries.remove(position);
                result.deleted.push(id.to_string());
            } else {
                result.rejected.push(MemoryReject {
                    index,
                    id: Some(id.to_string()),
                    reason: "unknown_id",
                });
            }
        }
        result
    }

    /// Frozen text for the post-compaction reminder.
    pub fn render_reminder_text(&self) -> String {
        let mut out = String::from(
            "Session memory (survives compaction; update with memory.upsert, remove stale with memory.delete):",
        );
        for entry in &self.entries {
            out.push_str(&format!(
                "\n- [{}] {} ({})",
                entry.kind.as_str(),
                entry.text,
                entry.id
            ));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn upsert(text: &str) -> MemoryUpsertRequest {
        MemoryUpsertRequest {
            id: None,
            text: text.into(),
            kind: None,
        }
    }

    #[test]
    fn upsert_adds_updates_and_dedupes_by_text() {
        let mut memory = SessionMemory::default();
        let first = memory.apply_upsert(&[upsert("use pnpm")], 1);
        assert_eq!(first.added, ["m1"]);
        assert!(memory.entries[0].kind == MemoryKind::Fact);

        let second = memory.apply_upsert(&[upsert("  USE PNPM  ")], 2);
        assert!(second.added.is_empty());
        assert_eq!(second.updated, ["m1"]);
        assert_eq!(memory.entries.len(), 1);
        assert_eq!(memory.entries[0].text, "USE PNPM");
        assert_eq!(memory.entries[0].updated_at, 2);
    }

    #[test]
    fn explicit_id_updates_only_existing_entries() {
        let mut memory = SessionMemory {
            next_id: 2,
            entries: vec![MemoryEntry {
                id: "m1".into(),
                text: "old".into(),
                kind: MemoryKind::Fact,
                updated_at: 1,
            }],
        };
        let result = memory.apply_upsert(
            &[MemoryUpsertRequest {
                id: Some("m1".into()),
                text: "new".into(),
                kind: Some("decision".into()),
            }],
            2,
        );
        assert_eq!(result.updated, ["m1"]);
        assert_eq!(memory.entries[0].text, "new");
        assert_eq!(memory.entries[0].kind, MemoryKind::Decision);

        let missing = memory.apply_upsert(
            &[MemoryUpsertRequest {
                id: Some("m9".into()),
                text: "oops".into(),
                kind: None,
            }],
            3,
        );
        assert_eq!(missing.rejected[0].reason, "unknown_id");
        assert_eq!(memory.entries.len(), 1);
    }

    #[test]
    fn caps_are_enforced_per_item_not_by_silent_drop() {
        let mut memory = SessionMemory::default();
        let long = "x".repeat(MEMORY_MAX_TEXT_CHARS + 1);
        let result = memory.apply_upsert(&[upsert(""), upsert(&long)], 1);
        assert_eq!(result.rejected.len(), 2);
        assert_eq!(result.rejected[0].reason, "empty_text");
        assert_eq!(result.rejected[1].reason, "text_too_long");
        assert!(memory.entries.is_empty());

        let mut memory = SessionMemory::default();
        let batch: Vec<_> = (0..MEMORY_MAX_ENTRIES + 1)
            .map(|i| upsert(&format!("entry {i}")))
            .collect();
        let result = memory.apply_upsert(&batch, 1);
        assert_eq!(memory.entries.len(), MEMORY_MAX_ENTRIES);
        assert_eq!(result.added.len(), MEMORY_MAX_ENTRIES);
        assert_eq!(result.rejected.len(), 1);
        assert_eq!(result.rejected[0].reason, "limit_exceeded");
    }

    #[test]
    fn total_char_cap_rejects_add() {
        let mut memory = SessionMemory::default();
        let batch: Vec<_> = (0..21).map(|i| upsert(&format!("{i:0>200}"))).collect();
        // 21 * 200 = 4200 > 4000; the last item must be rejected.
        let result = memory.apply_upsert(&batch, 1);
        assert_eq!(memory.total_chars(), MEMORY_MAX_TOTAL_CHARS);
        assert_eq!(memory.entries.len(), 20);
        assert_eq!(result.rejected.len(), 1);
        assert_eq!(result.rejected[0].reason, "limit_exceeded");
    }

    #[test]
    fn delete_reports_unknown_ids_and_keeps_the_rest() {
        let mut memory = SessionMemory::default();
        memory.apply_upsert(&[upsert("a"), upsert("b")], 1);
        let result = memory.apply_delete(&["m1".into(), "m9".into(), " ".into()]);
        assert_eq!(result.deleted, ["m1"]);
        assert_eq!(result.rejected.len(), 2);
        assert_eq!(result.rejected[0].reason, "unknown_id");
        assert_eq!(result.rejected[1].reason, "invalid_id");
        assert_eq!(memory.entries.len(), 1);
        assert_eq!(memory.entries[0].id, "m2");
    }

    #[test]
    fn normalize_never_reuses_ids() {
        let mut memory = SessionMemory {
            next_id: 0,
            entries: vec![MemoryEntry {
                id: "m7".into(),
                text: "x".into(),
                kind: MemoryKind::Fact,
                updated_at: 1,
            }],
        };
        memory.normalize();
        assert_eq!(memory.next_id, 8);
        let result = memory.apply_upsert(&[upsert("y")], 2);
        assert_eq!(result.added, ["m8"]);
    }

    #[test]
    fn filter_matches_kind_and_case_insensitive_query() {
        let mut memory = SessionMemory::default();
        memory.apply_upsert(
            &[MemoryUpsertRequest {
                id: None,
                text: "Use pnpm for this repo".into(),
                kind: Some("preference".into()),
            }],
            1,
        );
        let entry = &memory.entries[0];
        assert!(entry.matches(Some("PNPM"), Some(MemoryKind::Preference)));
        assert!(!entry.matches(Some("npm i"), None));
        assert!(!entry.matches(None, Some(MemoryKind::Decision)));
    }

    #[test]
    fn reminder_text_lists_current_entries() {
        let mut memory = SessionMemory::default();
        memory.apply_upsert(
            &[MemoryUpsertRequest {
                id: None,
                text: "keep the API stable".into(),
                kind: Some("constraint".into()),
            }],
            1,
        );
        let text = memory.render_reminder_text();
        assert!(text.contains("Session memory"));
        assert!(text.contains("[constraint] keep the API stable (m1)"));
    }
}
