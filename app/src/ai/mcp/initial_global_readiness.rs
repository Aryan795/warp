use std::collections::HashSet;
use std::future::Future;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use uuid::Uuid;
use warp_util::sync::Condition;

use super::MCPProvider;

/// Late-subscriber-safe latch for the one-time initial global file-based MCP scan.
///
/// [`Condition`] is set exactly once when the scan settles. Waiters that subscribe after
/// that still observe completion immediately, with the frozen auto-start UUID list.
#[derive(Clone, Debug)]
pub struct InitialGlobalMcpReadiness {
    complete: Condition,
    result: Arc<Mutex<Option<Vec<Uuid>>>>,
}

impl InitialGlobalMcpReadiness {
    pub fn pending() -> Self {
        Self {
            complete: Condition::new(),
            result: Arc::new(Mutex::new(None)),
        }
    }

    pub fn complete_empty() -> Self {
        let latch = Self::pending();
        latch.complete(Vec::new());
        latch
    }

    /// Freeze the wait set and wake every waiter. Idempotent.
    pub fn complete(&self, wait_server_uuids: Vec<Uuid>) {
        let mut result = self.result.lock().unwrap_or_else(|err| err.into_inner());
        if result.is_some() {
            return;
        }
        *result = Some(wait_server_uuids);
        drop(result);
        self.complete.set();
    }

    pub fn result(&self) -> Option<Vec<Uuid>> {
        self.result
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
    }

    pub fn is_complete(&self) -> bool {
        self.complete.is_set()
    }

    pub fn wait(&self) -> impl Future<Output = Vec<Uuid>> + use<> {
        let complete = self.complete.clone();
        let result = Arc::clone(&self.result);
        async move {
            complete.wait().await;
            result
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .clone()
                .unwrap_or_default()
        }
    }
}

impl Default for InitialGlobalMcpReadiness {
    fn default() -> Self {
        Self::pending()
    }
}

/// Global home-config sources owed by the one-time startup scan, plus whether
/// completion has already been emitted.
///
/// Continuous filesystem watching is independent of this set: a source is
/// removed exactly once it produces a first terminal parse outcome.
#[derive(Clone, Debug, Default)]
pub struct InitialGlobalScanCohort {
    pending: HashSet<(PathBuf, MCPProvider)>,
    emitted: bool,
}

impl InitialGlobalScanCohort {
    pub fn from_pending(pending: HashSet<(PathBuf, MCPProvider)>) -> Self {
        Self {
            pending,
            emitted: false,
        }
    }

    pub fn insert(&mut self, source: (PathBuf, MCPProvider)) {
        self.pending.insert(source);
    }

    pub fn contains(&self, source: &(PathBuf, MCPProvider)) -> bool {
        self.pending.contains(source)
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    pub fn has_emitted(&self) -> bool {
        self.emitted
    }

    /// Remove `source` if it was still owed.
    pub fn remove(&mut self, source: &(PathBuf, MCPProvider)) -> bool {
        self.pending.remove(source)
    }

    /// Mark completion if every owed source has settled. Returns whether the caller
    /// should emit the completion event.
    pub fn try_complete(&mut self) -> bool {
        if self.emitted || !self.pending.is_empty() {
            return false;
        }
        self.emitted = true;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_is_idempotent_and_late_safe() {
        let latch = InitialGlobalMcpReadiness::pending();
        assert!(!latch.is_complete());
        assert_eq!(latch.result(), None);

        let first = vec![Uuid::nil()];
        latch.complete(first.clone());
        latch.complete(vec![Uuid::new_v4()]);

        assert!(latch.is_complete());
        assert_eq!(latch.result(), Some(first.clone()));
        assert_eq!(
            futures::executor::block_on(latch.wait()),
            first,
            "a waiter attached after completion must still see the frozen set"
        );
    }

    #[test]
    fn cohort_emits_once_when_the_last_source_settles() {
        let source = (PathBuf::from("/tmp/.mcp.json"), MCPProvider::Warp);
        let mut cohort = InitialGlobalScanCohort::from_pending(HashSet::from([source.clone()]));
        assert!(!cohort.try_complete());
        assert!(cohort.remove(&source));
        assert!(cohort.try_complete());
        assert!(cohort.has_emitted());
        assert!(!cohort.try_complete());
    }
}
