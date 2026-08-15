//! Shared golden Item JSON fixtures — Rust serde authority for DoD #6 partial alignment.
//! FE vitest (`r9-death-list.test.ts`) must accept the same files.

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use crate::types::Item;

    fn fixture(name: &str) -> String {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/items")
            .join(name);
        fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    #[test]
    fn golden_item_fixtures_deserialize() {
        for name in [
            "assistant_message.json",
            "function_call.json",
            "user_message.json",
        ] {
            let raw = fixture(name);
            let item: Item = serde_json::from_str(&raw).unwrap_or_else(|e| {
                panic!("{name}: deserialize failed: {e}\n{raw}");
            });
            let round = serde_json::to_value(&item).expect("serialize");
            assert_eq!(
                round["type"],
                serde_json::from_str::<serde_json::Value>(&raw).unwrap()["type"],
                "{name} type must round-trip"
            );
        }
    }
}
