use super::*;

pub(super) fn cached_codex_file_is_fresh(
    cache: &CostUsageCache,
    entry: &CostUsageFileUsage,
    cache_covers_range: bool,
    mtime_unix_ms: i64,
    size: i64,
) -> bool {
    cache_covers_range
        && !entry.codex_unresolved_fork_parent
        && entry.mtime_unix_ms == mtime_unix_ms
        && entry.size == size
        && codex_scan_target_size(entry) == size
        && entry.parsed_bytes.unwrap_or(0) >= size
        && super::codex_fork_parent_is_safe(cache, entry)
}

pub(super) fn cached_codex_file_is_complete_for_range(
    cache: &CostUsageCache,
    path_key: &str,
    range: &CostUsageDayRange,
) -> bool {
    JsonlScanner::cache_covers_range(cache, range)
        && cache.files.get(path_key).is_some_and(|usage| {
            let Ok(metadata) = fs::metadata(path_key) else {
                return false;
            };
            let identity_matches = match (
                usage.codex_file_identity.as_ref(),
                JsonlScanner::codex_file_identity(Path::new(path_key), &metadata).as_ref(),
            ) {
                (Some(expected), Some(actual)) => expected == actual,
                (Some(_), None) => false,
                (None, _) => true,
            };
            #[allow(clippy::cast_possible_wrap, reason = "session file sizes fit i64")]
            let size = metadata.len().min(i64::MAX as u64) as i64;
            identity_matches
                && usage.mtime_unix_ms == system_time_to_unix_ms(metadata.modified().ok())
                && usage.size == size
                && codex_scan_target_size(usage) == size
                && usage.parsed_bytes.unwrap_or(0) >= size
                && !usage.codex_unresolved_fork_parent
                // Reconsider locally inferred children after this pass has
                // had a chance to discover and cache their parent.
                && !super::codex_fork_uses_local_inference(usage)
                && super::codex_fork_parent_is_safe(cache, usage)
        })
}

/// Process cached local-inference children after all other candidates. A
/// parent discovered in this pass must enter the cache before its unchanged
/// child can decide whether the inferred baseline is still authoritative.
pub(super) fn defer_codex_locally_inferred_candidates(
    candidates: &mut Vec<CodexScanCandidate>,
    cache: &CostUsageCache,
) {
    if candidates.len() < 2 {
        return;
    }

    let mut other = Vec::with_capacity(candidates.len());
    let mut locally_inferred = Vec::new();
    for candidate in candidates.drain(..) {
        let path_key = candidate.path.to_string_lossy();
        if cache
            .files
            .get(path_key.as_ref())
            .is_some_and(super::codex_fork_uses_local_inference)
        {
            locally_inferred.push(candidate);
        } else {
            other.push(candidate);
        }
    }
    other.extend(locally_inferred);
    candidates.extend(other);
}

/// Order one bounded work set so every uniquely identified parent is parsed
/// before its children. Duplicate identities, cycles, and every dependent
/// candidate are marked unsafe so parsing cannot accept or infer a baseline
/// from ambiguous lineage.
pub(super) fn order_codex_candidates_by_lineage(candidates: &mut Vec<CodexPreparedCandidate>) {
    if candidates.is_empty() {
        return;
    }

    let mut session_owners = HashMap::<String, Vec<usize>>::new();
    for (index, candidate) in candidates.iter().enumerate() {
        let Some(session_id) = candidate.session_metadata.session_id.as_ref() else {
            continue;
        };
        session_owners
            .entry(session_id.clone())
            .or_default()
            .push(index);
    }
    let mut unsafe_lineage = vec![false; candidates.len()];
    for owners in session_owners.values().filter(|owners| owners.len() > 1) {
        for &index in owners {
            unsafe_lineage[index] = true;
        }
    }
    let mut parent_indices = vec![None; candidates.len()];
    for (index, candidate) in candidates.iter().enumerate() {
        let Some(parent_id) = candidate.session_metadata.forked_from_id.as_ref() else {
            continue;
        };
        match session_owners.get(parent_id).map(Vec::as_slice) {
            Some([parent_index]) => parent_indices[index] = Some(*parent_index),
            Some([]) | None => {}
            Some(_) => unsafe_lineage[index] = true,
        }
    }

    let mut remaining = candidates.drain(..).map(Some).collect::<Vec<_>>();
    let mut ordered = Vec::with_capacity(remaining.len());
    let mut completed = vec![false; remaining.len()];

    loop {
        let mut progressed = false;
        for index in 0..remaining.len() {
            if remaining[index].is_none() || unsafe_lineage[index] {
                continue;
            }
            let parent_is_ready = parent_indices[index].is_none_or(|parent_index| {
                completed[parent_index] && !unsafe_lineage[parent_index]
            });
            if parent_is_ready {
                let mut candidate = remaining[index].take().expect("candidate checked above");
                candidate.lineage_disposition = CodexLineageDisposition::Ready;
                ordered.push(candidate);
                completed[index] = true;
                progressed = true;
            }
        }
        if !progressed {
            break;
        }
    }

    for mut candidate in remaining.into_iter().flatten() {
        candidate.lineage_disposition = CodexLineageDisposition::AmbiguousOrCyclic;
        ordered.push(candidate);
    }
    candidates.extend(ordered);
}

/// Give paths already in the durable queue their saved turn before newly
/// discovered dirty paths. The scanner appends unfinished paths after this
/// pass, making the queue a round-robin cursor instead of a newest-first loop.
pub(super) fn prioritize_codex_pending_candidates(
    candidates: &mut Vec<CodexScanCandidate>,
    pending_paths: &[String],
) {
    if pending_paths.is_empty() || candidates.len() < 2 {
        return;
    }

    let mut pending = Vec::with_capacity(candidates.len());
    let mut fresh = Vec::with_capacity(candidates.len());
    for candidate in candidates.drain(..) {
        let key = candidate.path.to_string_lossy();
        if pending_paths
            .iter()
            .any(|path| path.as_str() == key.as_ref())
        {
            pending.push(candidate);
        } else {
            fresh.push(candidate);
        }
    }
    pending.extend(fresh);
    candidates.extend(pending);
}

/// Return the persisted logical end of a Codex parse. Older cache entries did
/// not have a frozen target, so their physical size remains the safe fallback.
pub(super) fn codex_scan_target_size(usage: &CostUsageFileUsage) -> i64 {
    usage.codex_scan_target_size.unwrap_or(usage.size).max(0)
}

/// A cached prefix is resumable toward its original target when the target is
/// still present in the current file. The caller separately validates the
/// byte-boundary/parser-state invariants before using the cursor.
pub(super) fn codex_resumable_scan_target_size(
    metadata_size: i64,
    usage: &CostUsageFileUsage,
) -> Option<i64> {
    let parsed_bytes = usage.parsed_bytes.unwrap_or(usage.size).max(0);
    let target_size = codex_scan_target_size(usage);
    (parsed_bytes < target_size && target_size <= metadata_size).then_some(target_size)
}

/// Whether a logically complete prefix still has physical bytes that must be
/// revisited. This is the catch-up cursor for a growing rollout or a retained
/// incomplete tail.
pub(super) fn codex_logical_target_has_unconsumed_tail(
    metadata_size: i64,
    usage: &CostUsageFileUsage,
) -> bool {
    let parsed_bytes = usage.parsed_bytes.unwrap_or(usage.size).max(0);
    let target_size = codex_scan_target_size(usage);
    parsed_bytes < metadata_size || target_size < metadata_size
}

/// Whether a completed empty fragment has no parser state worth resuming.
pub(super) fn codex_cached_entry_is_complete_empty_fragment(usage: &CostUsageFileUsage) -> bool {
    usage.days.is_empty()
        && usage.parsed_bytes == Some(usage.size)
        && usage.codex_scan_target_size == Some(usage.size)
        && usage.last_model.is_none()
        && usage.last_totals.is_none()
        && usage.codex_last_token_timestamp.is_none()
        && usage.codex_token_timestamps_monotonic != Some(false)
}
