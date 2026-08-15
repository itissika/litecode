use std::sync::{Arc, RwLock};
use std::time::Duration;

use crate::optional::ToolEngine;
use crate::types::{LitecodeError, Result};

pub struct WebfetchEngine {
    client: Arc<RwLock<Option<reqwest::blocking::Client>>>,
}

impl WebfetchEngine {
    pub fn new() -> Self {
        Self {
            client: Arc::new(RwLock::new(None)),
        }
    }

    pub fn client_handle(&self) -> Arc<RwLock<Option<reqwest::blocking::Client>>> {
        Arc::clone(&self.client)
    }
}

impl ToolEngine for WebfetchEngine {
    fn id(&self) -> &str {
        "webfetch"
    }

    fn warmup(&self) -> Result<()> {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::limited(10))
            .build()
            .map_err(|e| LitecodeError::Config(format!("webfetch client build: {e}")))?;

        *self
            .client
            .write()
            .map_err(|e| LitecodeError::Config(format!("webfetch client lock: {e}")))? =
            Some(client);
        Ok(())
    }

    fn stop(&self) {
        if let Ok(mut guard) = self.client.write() {
            *guard = None;
        }
    }
}
