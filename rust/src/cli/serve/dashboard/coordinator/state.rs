//! Shared cache and in-flight state for the snapshot coordinator.

use std::sync::Arc;
use std::time::Instant;

use super::super::snapshot::SnapshotPayload;
use super::flight::Flight;

#[derive(Clone, Debug)]
pub(in crate::cli::serve::dashboard::coordinator) struct CachedSnapshot<S> {
    pub(in crate::cli::serve::dashboard::coordinator) payload: Arc<SnapshotPayload>,
    pub(in crate::cli::serve::dashboard::coordinator) sidecar: Option<Arc<S>>,
    pub(in crate::cli::serve::dashboard::coordinator) built_at: Instant,
}

/// Cache and in-flight generation are deliberately independent: an expired
/// successful snapshot remains available while its replacement is built.
#[derive(Debug)]
pub(in crate::cli::serve::dashboard::coordinator) struct CoordinatorState<S> {
    pub(in crate::cli::serve::dashboard::coordinator) cached: Option<CachedSnapshot<S>>,
    pub(in crate::cli::serve::dashboard::coordinator) flight: Option<Arc<Flight>>,
}

impl<S> Default for CoordinatorState<S> {
    fn default() -> Self {
        Self {
            cached: None,
            flight: None,
        }
    }
}
