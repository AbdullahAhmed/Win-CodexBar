use super::*;

#[test]
fn session_meta_paginated_v2_subagents_have_independent_usage_identity() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("subagent.jsonl");
    for source in [
        serde_json::json!({"thread_source": "subagent"}),
        serde_json::json!({"source": {"subagent": {"thread_spawn": {
            "parent_thread_id": "parent-id", "depth": 1
        }}}}),
    ] {
        let mut payload = serde_json::json!({
            "id": "child-id",
            "session_id": "parent-id",
            "forked_from_id": "parent-id",
            "parent_thread_id": "parent-id",
            "history_mode": "paginated",
            "subagent_history_start_ordinal": 42,
            "multi_agent_version": "v2"
        });
        payload
            .as_object_mut()
            .unwrap()
            .extend(source.as_object().unwrap().clone());
        let row = serde_json::json!({
            "type": "session_meta",
            "timestamp": "2026-09-17T10:00:00Z",
            "payload": payload
        });
        std::fs::write(&path, format!("{row}\n")).unwrap();

        let metadata = JsonlScanner::read_codex_session_metadata(&path).unwrap();
        assert_eq!(metadata.session_id.as_deref(), Some("child-id"));
        assert_eq!(metadata.forked_from_id, None, "source: {source}");
        assert_eq!(
            metadata.fork_timestamp.as_deref(),
            Some("2026-09-17T10:00:00Z")
        );
    }
}

#[test]
fn session_meta_incomplete_v2_markers_preserve_inherited_fork_accounting() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("fork.jsonl");
    for markers in [
        serde_json::json!({"history_mode": "paginated", "multi_agent_version": "v2"}),
        serde_json::json!({
            "history_mode": "paginated", "multi_agent_version": "v2", "thread_source": "cli"
        }),
        serde_json::json!({"history_mode": "paginated", "thread_source": "subagent"}),
        serde_json::json!({"multi_agent_version": "v2", "thread_source": "subagent"}),
        serde_json::json!({
            "history_mode": "paginated", "multi_agent_version": "v1", "thread_source": "subagent"
        }),
        serde_json::json!({
            "history_mode": "full", "multi_agent_version": "v2", "thread_source": "subagent"
        }),
        serde_json::json!({
            "history_mode": "paginated", "multi_agent_version": "v2",
            "source": {"subagent": {"thread_spawn": null}}
        }),
    ] {
        let mut payload = serde_json::json!({
            "id": "child-id",
            "session_id": "parent-id",
            "forked_from_id": "parent-id"
        });
        payload
            .as_object_mut()
            .unwrap()
            .extend(markers.as_object().unwrap().clone());
        let row = serde_json::json!({"type": "session_meta", "payload": payload});
        std::fs::write(&path, format!("{row}\n")).unwrap();

        let metadata = JsonlScanner::read_codex_session_metadata(&path).unwrap();
        assert_eq!(metadata.session_id.as_deref(), Some("child-id"));
        assert_eq!(
            metadata.forked_from_id.as_deref(),
            Some("parent-id"),
            "incomplete markers must not opt into independent counters: {markers}"
        );
    }
}
