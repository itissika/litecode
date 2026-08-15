use std::collections::HashMap;
use std::sync::Arc;

use super::external::ExternalHookAdapter;
use super::types::{HookOutput, HookPayload, LifecycleType};

#[derive(Default)]
struct HooksInner {
    hooks: HashMap<String, Vec<ExternalHookAdapter>>,
}

#[derive(Clone)]
pub struct HookRegistry {
    inner: Arc<HooksInner>,
}

pub struct HookRegistryBuilder {
    inner: HooksInner,
}

impl Default for HookRegistryBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl HookRegistryBuilder {
    pub fn new() -> Self {
        Self {
            inner: HooksInner::default(),
        }
    }

    /// Wire external hook commands into the registry. Production has no
    /// hook-config surface (2.13) — this API exists only so integration tests
    /// can exercise the (empty-by-default) dispatcher machinery.
    pub fn register_external(
        &mut self,
        global: &crate::config::HookConfig,
        agent: &crate::config::HookConfig,
    ) {
        for point in super::types::LIFECYCLE_POINTS_V2 {
            let mut all_cmds: Vec<_> = global.get(point).to_vec();
            all_cmds.extend_from_slice(agent.get(point));

            for cmd in all_cmds {
                let adapter = ExternalHookAdapter::new(point, cmd);
                let hooks = self.inner.hooks.entry(point.to_string()).or_default();
                hooks.push(adapter);
            }
        }
    }

    pub fn build(self) -> HookRegistry {
        HookRegistry {
            inner: Arc::new(self.inner),
        }
    }
}

impl HookRegistry {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(HooksInner::default()),
        }
    }

    pub async fn run(
        &self,
        point: &str,
        payload: &HookPayload,
        ctx: &crate::context_pipeline::Context,
    ) -> HookOutput {
        let mut combined = HookOutput::default();
        let metatype = LifecycleType::classify(point);

        if let Some(hooks) = self.inner.hooks.get(point) {
            for hook in hooks {
                let out = hook.run(payload, ctx).await;
                combined.merge(out);

                if metatype == LifecycleType::Gating
                    && combined.action == super::types::HookAction::Block
                {
                    return combined;
                }
            }
        }

        combined
    }
}

impl Default for HookRegistry {
    fn default() -> Self {
        Self::new()
    }
}
