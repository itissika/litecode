/// L1 workspace domain event; no api:: or Wire* references.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceChange {
    pub paths: Vec<String>,
    pub kind: String, // "created" | "modified" | "deleted" | "renamed"
}
