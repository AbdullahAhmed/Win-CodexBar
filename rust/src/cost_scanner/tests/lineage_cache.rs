use super::*;

fn write_subagent(
    sessions_root: &Path,
    name: &str,
    session_id: &str,
    parent_id: &str,
    timestamp: DateTime<Utc>,
) -> PathBuf {
    let day = timestamp.with_timezone(&Local).date_naive();
    let day_dir = sessions_root
        .join(day.format("%Y").to_string())
        .join(day.format("%m").to_string())
        .join(day.format("%d").to_string());
    std::fs::create_dir_all(&day_dir).unwrap();
    let path = day_dir.join(name);
    let rows = [
        serde_json::json!({
            "type": "session_meta", "ordinal": 0, "timestamp": timestamp.to_rfc3339(),
            "payload": {
                "id": session_id,
                "forked_from_id": parent_id,
                "subagent_history_start_ordinal": 10,
                "thread_source": "subagent",
                "source": {"subagent": {"thread_spawn": {"parent_thread_id": parent_id}}}
            }
        }),
        lineage_token_row(timestamp, 2, 1_000, 0),
        serde_json::json!({
            "type": "turn_context", "ordinal": 10, "timestamp": timestamp.to_rfc3339(),
            "payload": {"model": "gpt-5.6-sol"}
        }),
        lineage_token_row(timestamp, 12, 1_000, 1_000),
        lineage_token_row(timestamp + Duration::seconds(1), 20, 1_050, 50),
    ];
    let body = rows
        .into_iter()
        .map(|row| row.to_string())
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    std::fs::write(&path, body).unwrap();
    path
}

fn lineage_token_row(
    timestamp: DateTime<Utc>,
    ordinal: i64,
    total_input: i64,
    last_input: i64,
) -> serde_json::Value {
    serde_json::json!({
        "type": "event_msg", "ordinal": ordinal, "timestamp": timestamp.to_rfc3339(),
        "payload": {"type": "token_count", "info": {
            "model": "gpt-5.6-sol",
            "total_token_usage": {
                "input_tokens": total_input, "cached_input_tokens": 0, "output_tokens": 5
            },
            "last_token_usage": {
                "input_tokens": last_input, "cached_input_tokens": 0, "output_tokens": 5
            }
        }}
    })
}

fn bounded_scanner(sessions: &Path, cache_root: &Path) -> CostScanner {
    let mut options = CostScanOptions::app_driven();
    options.codex_candidate_limit = 1;
    options.prefer_newest_codex_sessions_first = false;
    CostScanner::new(7)
        .with_options(options)
        .with_cache_root(cache_root)
        .with_sessions_dirs(vec![sessions.to_path_buf()])
}

fn assert_locally_inferred(cache: &CostUsageCache, path: &Path) {
    let usage = &cache.files[&path.to_string_lossy().to_string()];
    assert!(!usage.codex_unresolved_fork_parent);
    assert!(
        usage
            .codex_fork_accounting_state
            .as_ref()
            .is_some_and(|state| state.locally_resolved)
    );
}

fn assert_unresolved(cache: &CostUsageCache, path: &Path) {
    let usage = &cache.files[&path.to_string_lossy().to_string()];
    assert!(usage.codex_unresolved_fork_parent);
    assert!(usage.days.is_empty());
    assert!(usage.codex_fork_accounting_state.is_none());
}

#[test]
fn bounded_refresh_detects_duplicate_parent_owners_across_cache_and_candidate() {
    let root = tempfile::tempdir().unwrap();
    let sessions = root.path().join("sessions");
    let cache_root = root.path().join("cache");
    let base = Utc::now() - Duration::hours(1);
    let child = write_subagent(&sessions, "child.jsonl", "child-id", "parent-id", base);
    let scanner = bounded_scanner(&sessions, &cache_root);

    let (_, first_stats, first_cache) = scanner.scan_codex_detailed_with_cache(None);
    assert_eq!(first_stats.codex_read_receipt.metadata_reads, 1);
    assert_eq!(first_stats.codex_read_receipt.history_reads, 1);
    assert_locally_inferred(&first_cache, &child);

    let first_parent = write_codex_fork_session_fixture(
        &sessions,
        "parent-a.jsonl",
        "parent-id",
        None,
        base - Duration::seconds(2),
        base - Duration::seconds(2),
        &[1_000],
    );
    let (_, second_stats, second_cache) = scanner.scan_codex_detailed_with_cache(None);
    assert_eq!(second_stats.codex_read_receipt.metadata_reads, 1);
    assert_eq!(second_stats.codex_read_receipt.history_reads, 1);
    assert_locally_inferred(&second_cache, &child);

    let second_parent = write_codex_fork_session_fixture(
        &sessions,
        "parent-b.jsonl",
        "parent-id",
        None,
        base - Duration::seconds(1),
        base - Duration::seconds(1),
        &[2_000],
    );
    let (summary, third_stats, cache) = scanner.scan_codex_detailed_with_cache(None);

    assert_eq!(third_stats.codex_read_receipt.metadata_reads, 1);
    assert_eq!(third_stats.codex_read_receipt.history_reads, 0);
    assert_eq!(summary.sessions_count, 0);
    assert_unresolved(&cache, &first_parent);
    assert_unresolved(&cache, &second_parent);
    assert_unresolved(&cache, &child);
}

#[test]
fn bounded_refresh_detects_equal_timestamp_two_node_cycle() {
    let root = tempfile::tempdir().unwrap();
    let sessions = root.path().join("sessions");
    let cache_root = root.path().join("cache");
    let base = Utc::now() - Duration::hours(1);
    let first = write_subagent(&sessions, "first.jsonl", "first-id", "second-id", base);
    let scanner = bounded_scanner(&sessions, &cache_root);
    let (_, _, first_cache) = scanner.scan_codex_detailed_with_cache(None);
    assert_locally_inferred(&first_cache, &first);

    let second = write_subagent(&sessions, "second.jsonl", "second-id", "first-id", base);
    let (summary, stats, cache) = scanner.scan_codex_detailed_with_cache(None);

    assert_eq!(stats.codex_read_receipt.metadata_reads, 1);
    assert_eq!(stats.codex_read_receipt.history_reads, 0);
    assert_eq!(summary.sessions_count, 0);
    assert_unresolved(&cache, &first);
    assert_unresolved(&cache, &second);
}

#[test]
fn bounded_refresh_rejects_self_cycle_migration() {
    let root = tempfile::tempdir().unwrap();
    let sessions = root.path().join("sessions");
    let cache_root = root.path().join("cache");
    let base = Utc::now() - Duration::hours(1);
    let session = write_subagent(&sessions, "self.jsonl", "self-id", "missing-id", base);
    let scanner = bounded_scanner(&sessions, &cache_root);
    let (_, _, first_cache) = scanner.scan_codex_detailed_with_cache(None);
    assert_locally_inferred(&first_cache, &session);

    write_subagent(
        &sessions,
        "self.jsonl",
        "self-id",
        "self-id",
        base + Duration::seconds(1),
    );
    let (summary, stats, cache) = scanner.scan_codex_detailed_with_cache(None);

    assert_eq!(stats.codex_read_receipt.metadata_reads, 1);
    assert_eq!(stats.codex_read_receipt.history_reads, 0);
    assert_eq!(summary.sessions_count, 0);
    assert_unresolved(&cache, &session);
}

#[test]
fn bounded_refresh_rejects_dependent_of_locally_inferred_parent() {
    let root = tempfile::tempdir().unwrap();
    let sessions = root.path().join("sessions");
    let cache_root = root.path().join("cache");
    let base = Utc::now() - Duration::hours(1);
    let parent = write_subagent(&sessions, "parent.jsonl", "parent-id", "missing-id", base);
    let scanner = bounded_scanner(&sessions, &cache_root);
    let (_, _, first_cache) = scanner.scan_codex_detailed_with_cache(None);
    assert_locally_inferred(&first_cache, &parent);

    let dependent = write_subagent(
        &sessions,
        "dependent.jsonl",
        "dependent-id",
        "parent-id",
        base + Duration::seconds(1),
    );
    let (_, stats, cache) = scanner.scan_codex_detailed_with_cache(None);

    assert_eq!(stats.codex_read_receipt.metadata_reads, 1);
    assert_eq!(stats.codex_read_receipt.history_reads, 0);
    assert_locally_inferred(&cache, &parent);
    assert_unresolved(&cache, &dependent);
}
