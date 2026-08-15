use crate::context_pipeline::Context;
use crate::types::Transcript;

use super::{HookOutput, HookPayload, HookRegistry, apply_hook_output};

/// Unified L1 entry for lifecycle hooks. Thin wrapper over [`HookRegistry`].
#[derive(Clone)]
pub struct HookDispatcher {
    registry: HookRegistry,
}

impl HookDispatcher {
    pub fn from_registry(registry: HookRegistry) -> Self {
        Self { registry }
    }

    pub fn registry(&self) -> &HookRegistry {
        &self.registry
    }

    pub fn set_registry(&mut self, registry: HookRegistry) {
        self.registry = registry;
    }

    /// Apply-inject rule table (v4 §5 Phase 2). PostToolUse apply is performed by
    /// `ToolPipeline` after Assistant push (design A), not via `fire_and_apply`.
    pub fn phase_applies_inject(&self, phase: &str) -> bool {
        unified_phase_applies_inject(phase)
    }

    pub async fn fire(&self, phase: &str, payload: &HookPayload, ctx: &Context) -> HookOutput {
        self.registry.run(phase, payload, ctx).await
    }

    /// Fire hooks and apply inject items when the phase apply table says so.
    pub async fn fire_and_apply(
        &self,
        phase: &str,
        payload: &HookPayload,
        ctx: &Context,
        transcript: &mut Transcript,
        ts: i64,
    ) -> HookOutput {
        let output = self.fire(phase, payload, ctx).await;
        if self.phase_applies_inject(phase) {
            apply_hook_output(transcript, output.clone(), ts);
        }
        output
    }
}

/// Unified apply rules (implementation source of truth).
fn unified_phase_applies_inject(phase: &str) -> bool {
    matches!(
        phase,
        "SessionStart" | "UserPromptSubmit" | "SessionEnd" | "PostToolUse"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hook::HookRegistryBuilder;

    fn empty_dispatcher() -> HookDispatcher {
        HookDispatcher::from_registry(HookRegistryBuilder::new().build())
    }

    #[test]
    fn unified_apply_table() {
        let d = empty_dispatcher();
        assert!(d.phase_applies_inject("SessionStart"));
        assert!(d.phase_applies_inject("UserPromptSubmit"));
        assert!(d.phase_applies_inject("SessionEnd"));
        assert!(!d.phase_applies_inject("Stop"));
        assert!(!d.phase_applies_inject("PreCompact"));
        assert!(!d.phase_applies_inject("PreToolUse"));
        assert!(!d.phase_applies_inject("PermissionRequest"));
        assert!(d.phase_applies_inject("PostToolUse"));
    }
}
