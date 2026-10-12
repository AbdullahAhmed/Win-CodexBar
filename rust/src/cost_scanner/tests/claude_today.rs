//! Claude "today" is the cost bucket zone's calendar day, and the chart
//! snapshot and the one-day scan must report the same totals for it.

use super::*;

const CHILD_MARKER: &str = "CODEXBAR_CLAUDE_TODAY_TEST_CHILD";
const CHILD_DONE: &str = "claude today fixtures verified";

fn line(timestamp: DateTime<Utc>, id: &str, model: &str) -> String {
    format!(
        r#"{{"type":"assistant","timestamp":"{}","requestId":"req_{id}","message":{{"id":"msg_{id}","model":"{model}","usage":{{"input_tokens":1000,"output_tokens":500}}}}}}"#,
        timestamp.to_rfc3339()
    )
}

fn write_transcript(path: &Path, lines: &[String]) {
    std::fs::write(path, format!("{}\n", lines.join("\n"))).unwrap();
}

fn child_body() {
    use crate::cost_reporting_period::{cost_bucket_zone, set_cost_bucket_zone};
    // UTC+9 with no DST: its midnight never coincides with UTC midnight.
    set_cost_bucket_zone("Asia/Tokyo");
    let config_dir = PathBuf::from(std::env::var_os("CLAUDE_CONFIG_DIR").unwrap());
    let project = config_dir.join("projects").join("fixture");
    std::fs::create_dir_all(&project).unwrap();
    let file = project.join("t.jsonl");

    let now = Utc::now();
    let zone = cost_bucket_zone();
    let today = zone.date(now);
    let midnight = zone.start_of_day_utc(today);
    let before = midnight - Duration::seconds(1);
    let after = midnight + Duration::seconds(1);
    let priced = "claude-sonnet-4-6";

    // A: one record just before local midnight, two just after.
    write_transcript(
        &file,
        &[
            line(before, "a1", priced),
            line(after, "a2", priced),
            line(after, "a3", priced),
        ],
    );
    let snapshot = CostScanner::new(30).scan_claude_chart_snapshot_with_cancel(None);
    assert_eq!(
        snapshot.today.tokens, 3_000,
        "yesterday's record is excluded"
    );
    let today_cost = snapshot.today.cost_usd.expect("priced day has a cost");
    assert!(today_cost > 0.0);
    let yesterday = super::super::today::day_key(today - Duration::days(1));
    assert_eq!(
        snapshot
            .daily_tokens
            .iter()
            .find(|(day, _)| *day == yesterday)
            .map(|(_, tokens)| *tokens),
        Some(1_500)
    );

    // The one-day scan behind the cached summary agrees with the card path.
    let one_day = CostScanner::new(1).scan_claude();
    assert_eq!(one_day.total_tokens_for_provider("claude"), 3_000);
    assert!((one_day.total_cost_usd - today_cost).abs() < 1e-9);
    assert_eq!(one_day.period_start, Some(today));
    assert_eq!(one_day.period_end, Some(today));

    // B: usage only yesterday is a known zero, not an unknown.
    write_transcript(&file, &[line(before, "b1", priced)]);
    let snapshot = CostScanner::new(30).scan_claude_chart_snapshot_with_cancel(None);
    assert_eq!(snapshot.today.cost_usd, Some(0.0));
    assert_eq!(snapshot.today.tokens, 0);
    assert_eq!(
        CostScanner::new(1)
            .scan_claude()
            .total_tokens_for_provider("claude"),
        0
    );

    // C: an unreadable line leaves coverage unknown, so a day without
    // priced usage is unknown rather than a known zero.
    std::fs::write(project.join("malformed.jsonl"), b"{malformed\n").unwrap();
    let snapshot = CostScanner::new(30).scan_claude_chart_snapshot_with_cancel(None);
    assert_eq!(snapshot.today.cost_usd, None);
    assert_eq!(snapshot.today.tokens, 0);
    println!("{CHILD_DONE}");
}

#[test]
fn claude_today_follows_the_pinned_zone_midnight_in_both_scan_paths() {
    if std::env::var_os(CHILD_MARKER).is_some() {
        child_body();
        return;
    }
    let config_dir = tempfile::tempdir().unwrap();
    let test_thread = std::thread::current();
    let test_name = test_thread.name().expect("test harness names this thread");
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", test_name, "--nocapture", "--test-threads=1"])
        .env(CHILD_MARKER, "1")
        .env("CLAUDE_CONFIG_DIR", config_dir.path())
        // Every home-relative root (claude-swap, Pi, OMP) resolves here.
        .env("CODEXBAR_TEST_HOME", config_dir.path())
        .output()
        .expect("spawn isolated exact-test child");
    assert!(
        output.status.success() && String::from_utf8_lossy(&output.stdout).contains(CHILD_DONE),
        "fixture child failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
