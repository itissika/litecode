use crate::optional::ToolEngine;
use crate::types::Result;
use crate::types::error::LitecodeError;

/// P0 stub engine: immediate noop warmup/stop until Stage 2+ implementations land.
pub struct StubEngine {
    id: &'static str,
}

impl StubEngine {
    pub const fn new(id: &'static str) -> Self {
        Self { id }
    }
}

impl ToolEngine for StubEngine {
    fn id(&self) -> &str {
        self.id
    }

    fn warmup(&self) -> Result<()> {
        Err(LitecodeError::Config(format!(
            "engine '{}' not implemented (stub)",
            self.id
        )))
    }

    fn stop(&self) {}
}
