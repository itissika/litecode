use std::sync::Mutex;

use crate::session::store::Session;

/// Exclusive gate around a Session's SQLite connection. Sync-safe; no unsafe.
pub struct SessionGate {
    inner: Mutex<Session>,
}

impl SessionGate {
    pub fn new(session: Session) -> Self {
        Self {
            inner: Mutex::new(session),
        }
    }

    pub fn with<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&Session) -> R,
    {
        let g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        f(&*g)
    }

    pub fn with_mut<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut Session) -> R,
    {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut *g)
    }

    pub fn id(&self) -> String {
        self.with(|s| s.id.clone())
    }
}
