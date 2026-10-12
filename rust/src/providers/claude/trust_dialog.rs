//! Answers Claude Code's workspace-trust dialog during the trust preflight.
//!
//! Older releases focus "Yes, I trust this folder"; newer ones (2.1.29x)
//! focus "No, exit". Pressing Enter blindly would quit on the new layout and
//! never save the trust, so the focused option is read from the screen and
//! moved onto "Yes, I trust this folder" before Enter.

use super::cli_screen;
use crate::cli::tty_responder::{ScreenReading, ScreenResponder};

const ENTER: &str = "\r";
const DOWN: &str = "\x1b[B";
const UP: &str = "\x1b[A";

/// Responder for the preflight: accept trust, then end the session cleanly
/// once the dialog is gone instead of waiting for the idle kill.
pub(super) const TRUST_RESPONDER: ScreenResponder = ScreenResponder {
    read: read_trust_dialog,
    after_dialog: &["/exit", ENTER],
};

/// Looks at the dialog on the replayed screen. It is answerable once the
/// confirm hint is drawn and one option row has the focus marker.
fn read_trust_dialog(raw_screen: &str) -> ScreenReading {
    let screen = cli_screen::render(raw_screen, true).to_lowercase();
    let Some(question) = screen
        .lines()
        .position(|line| line.contains("quick safety check"))
    else {
        return ScreenReading::Absent;
    };
    // Only the dialog's own lines: after the question, up to the confirm hint.
    let mut rows = Vec::new();
    let mut complete = false;
    for line in screen.lines().skip(question + 1) {
        if line.contains("enter to confirm") {
            complete = true;
            break;
        }
        if !line.trim().is_empty() {
            rows.push(line);
        }
    }
    if !complete {
        return ScreenReading::Pending;
    }
    let trust = rows
        .iter()
        .position(|line| line.contains("yes, i trust this folder"));
    let focus = rows.iter().position(|line| has_focus_marker(line));
    let (Some(trust), Some(focus)) = (trust, focus) else {
        tracing::debug!(
            trust_row = trust.is_some(),
            focus_row = focus.is_some(),
            "Claude trust dialog is drawn but its trust or focus row was not found"
        );
        return ScreenReading::Pending;
    };
    let mut keys = if trust >= focus {
        vec![DOWN; trust - focus]
    } else {
        vec![UP; focus - trust]
    };
    keys.push(ENTER);
    ScreenReading::Answer(keys)
}

fn has_focus_marker(line: &str) -> bool {
    let line = line.trim_start();
    line.starts_with('❯') || line.starts_with('>')
}

#[cfg(test)]
mod tests {
    use super::*;

    const FOCUS_ON_NO: &str = "\x1b[2J\x1b[H Accessing workspace:\r\n\r\n C:/probe\r\n\r\n Quick safety check: Is this a project you created or one you trust?\r\n\r\n\x1b[36m \u{276f} No, exit\x1b[39m\r\n   Yes, I trust this folder\r\n\r\n Enter to confirm \u{b7} Esc to cancel\r\n";
    const FOCUS_ON_YES: &str = "Accessing workspace:\r\n\r\nC:/probe\r\n\r\nQuick safety check: Is this a project you created or one you trust?\r\n\r\n\u{276f} 1. Yes, I trust this folder\r\n  2. No, exit\r\n\r\nEnter to confirm \u{b7} Esc to cancel\r\n";
    // 2.1.293 skips blanks with absolute column jumps (CSI n G).
    const COLUMN_JUMPS: &str = "Quick\x1b[7Gsafety\x1b[14Gcheck: you trust this\r\n\x1b[1G\u{276f}\x1b[3GNo,\x1b[7Gexit\r\n\x1b[3GYes,\x1b[8GI\x1b[10Gtrust\x1b[16Gthis\x1b[21Gfolder\r\nEnter to confirm";
    const THREE_OPTIONS_FOCUS_FIRST: &str = "Quick safety check: Is this a project you trust?\r\n\r\n\u{276f} Open read-only\r\n  Yes, I trust this folder\r\n  No, exit\r\n\r\nEnter to confirm \u{b7} Esc to cancel\r\n";
    const THREE_OPTIONS_FOCUS_LAST: &str = "Quick safety check: Is this a project you trust?\r\n\r\n  Open read-only\r\n  Yes, I trust this folder\r\n\u{276f} No, exit\r\n\r\nEnter to confirm \u{b7} Esc to cancel\r\n";

    fn answer(screen: &str) -> Option<Vec<&'static str>> {
        match read_trust_dialog(screen) {
            ScreenReading::Answer(keys) => Some(keys),
            _ => None,
        }
    }

    #[test]
    fn focus_on_no_moves_down_before_enter() {
        assert_eq!(answer(FOCUS_ON_NO), Some(vec![DOWN, ENTER]));
    }

    #[test]
    fn focus_already_on_yes_just_presses_enter() {
        assert_eq!(answer(FOCUS_ON_YES), Some(vec![ENTER]));
    }

    #[test]
    fn three_options_count_the_rows_to_move() {
        assert_eq!(answer(THREE_OPTIONS_FOCUS_FIRST), Some(vec![DOWN, ENTER]));
        assert_eq!(answer(THREE_OPTIONS_FOCUS_LAST), Some(vec![UP, ENTER]));
        let two_down = THREE_OPTIONS_FOCUS_FIRST
            .replace("Yes, I trust this folder", "Skip")
            .replace("No, exit", "Yes, I trust this folder");
        assert_eq!(answer(&two_down), Some(vec![DOWN, DOWN, ENTER]));
    }

    #[test]
    fn column_jumps_do_not_glue_words_together() {
        assert_eq!(answer(COLUMN_JUMPS), Some(vec![DOWN, ENTER]));
    }

    #[test]
    fn stray_prompt_lines_outside_the_dialog_are_not_the_focus() {
        let before = format!("> earlier prompt\r\n{FOCUS_ON_NO}> later prompt\r\n");
        assert_eq!(answer(&before), Some(vec![DOWN, ENTER]));
        // No focus marker inside the dialog: a stray ">" after it must not count.
        let unfocused = "Quick safety check: trust?\r\n  No, exit\r\n  Yes, I trust this folder\r\nEnter to confirm\r\n> stray";
        assert_eq!(read_trust_dialog(unfocused), ScreenReading::Pending);
    }

    #[test]
    fn waits_until_the_dialog_is_fully_drawn() {
        let partial = FOCUS_ON_NO.split("Enter to confirm").next().unwrap();
        assert_eq!(read_trust_dialog(partial), ScreenReading::Pending);
        assert_eq!(
            read_trust_dialog("Welcome to Claude Code"),
            ScreenReading::Absent
        );
    }
}
