use std::sync::{Arc, RwLock};
use std::time::Duration;

use crate::config::schema::WebSearchSettings;
use crate::optional::ToolEngine;
use crate::optional::exa_mcp;
use crate::types::{LitecodeError, Result};

pub struct WebsearchEngine {
    client: Arc<RwLock<Option<reqwest::blocking::Client>>>,
    endpoint: Arc<RwLock<Option<String>>>,
}

impl WebsearchEngine {
    pub fn new() -> Self {
        Self {
            client: Arc::new(RwLock::new(None)),
            endpoint: Arc::new(RwLock::new(None)),
        }
    }

    pub fn configure(&self, settings: &WebSearchSettings) {
        if let Ok(mut guard) = self.endpoint.write() {
            *guard = Some(exa_mcp::mcp_url(settings.api_key.as_deref()));
        }
    }

    pub fn client_handle(&self) -> Arc<RwLock<Option<reqwest::blocking::Client>>> {
        Arc::clone(&self.client)
    }

    pub fn endpoint_handle(&self) -> Arc<RwLock<Option<String>>> {
        Arc::clone(&self.endpoint)
    }
}

impl ToolEngine for WebsearchEngine {
    fn id(&self) -> &str {
        "websearch"
    }

    fn warmup(&self) -> Result<()> {
        self.endpoint
            .read()
            .map_err(|e| LitecodeError::Config(format!("websearch endpoint lock: {e}")))?
            .clone()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                LitecodeError::Config("websearch is not configured".into())
            })?;

        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()
            .map_err(|e| LitecodeError::Config(format!("websearch client build: {e}")))?;

        *self
            .client
            .write()
            .map_err(|e| LitecodeError::Config(format!("websearch client lock: {e}")))? =
            Some(client);
        Ok(())
    }

    fn stop(&self) {
        if let Ok(mut guard) = self.client.write() {
            *guard = None;
        }
    }
}
