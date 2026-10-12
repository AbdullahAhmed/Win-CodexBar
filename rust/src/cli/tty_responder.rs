//! Keys the TTY runner sends in reaction to what the child has drawn.

use std::io::Write;
use std::time::Duration;

/// What a [`ScreenResponder`] sees on the current screen.
#[derive(Debug, PartialEq, Eq)]
pub enum ScreenReading {
    /// The dialog the responder handles is not on screen.
    Absent,
    /// The dialog is on screen but cannot be answered (yet).
    Pending,
    /// The dialog is on screen; send these key presses to answer it.
    Answer(Vec<&'static str>),
}

/// Reacts to the whole output captured so far (not a fragment), so a
/// differential redraw cannot hide part of a dialog.
#[derive(Debug, Clone, Copy)]
pub struct ScreenResponder {
    /// Classifies the output captured so far.
    pub read: fn(&str) -> ScreenReading,
    /// Keys sent once after an answered dialog has left the screen
    /// (for example `/exit` to end the session cleanly).
    pub after_dialog: &'static [&'static str],
}

/// Upper bound on dialog answers per run, so a screen that keeps redrawing
/// the dialog cannot make the runner type forever.
const MAX_SCREEN_RESPONSES: usize = 6;
/// Pause between the key presses of one responder answer.
const SCREEN_RESPONSE_KEY_GAP: Duration = Duration::from_millis(150);

#[derive(Default, PartialEq, Eq)]
enum Phase {
    /// Waiting for a dialog to answer.
    #[default]
    Watching,
    /// Keys were sent; the redraw our own keys cause is ignored until the
    /// dialog has left the screen.
    Answered,
}

/// Tracks whether the responder has answered the dialog now on screen.
#[derive(Default)]
pub(super) struct ResponderState {
    phase: Phase,
    answers: usize,
    after_dialog_sent: bool,
    cap_logged: bool,
}

impl ResponderState {
    /// Read the screen and send the keys the responder asks for. A dialog is
    /// answered once per appearance: after answering, nothing more is sent
    /// until the dialog disappears.
    pub(super) fn answer(
        &mut self,
        responder: &ScreenResponder,
        buffer: &str,
        writer: &mut impl Write,
    ) {
        match self.phase {
            Phase::Watching => {
                if self.answers >= MAX_SCREEN_RESPONSES {
                    if !self.cap_logged {
                        self.cap_logged = true;
                        tracing::debug!(
                            limit = MAX_SCREEN_RESPONSES,
                            "tty screen responder: answer limit reached"
                        );
                    }
                    return;
                }
                if let ScreenReading::Answer(keys) = (responder.read)(buffer) {
                    send_keys(&keys, writer);
                    self.answers += 1;
                    self.phase = Phase::Answered;
                }
            }
            Phase::Answered => {
                if (responder.read)(buffer) == ScreenReading::Absent {
                    self.phase = Phase::Watching;
                    if !self.after_dialog_sent {
                        self.after_dialog_sent = true;
                        send_keys(responder.after_dialog, writer);
                    }
                }
            }
        }
    }
}

fn send_keys(keys: &[&str], writer: &mut impl Write) {
    for (index, key) in keys.iter().enumerate() {
        if index > 0 {
            std::thread::sleep(SCREEN_RESPONSE_KEY_GAP);
        }
        // Best-effort key press; a closed PTY drops the write.
        let _key_written = writer.write_all(key.as_bytes());
        // Best-effort flush after a responder key press.
        let _key_flushed = writer.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_dialog(screen: &str) -> ScreenReading {
        if !screen.contains("DIALOG") {
            ScreenReading::Absent
        } else if screen.contains("READY") {
            ScreenReading::Answer(vec!["a", "b"])
        } else {
            ScreenReading::Pending
        }
    }

    const RESPONDER: ScreenResponder = ScreenResponder {
        read: read_dialog,
        after_dialog: &["/exit", "\r"],
    };

    #[test]
    fn answers_once_and_ignores_our_own_redraw() {
        let mut state = ResponderState::default();
        let mut sent = Vec::new();
        state.answer(&RESPONDER, "DIALOG", &mut sent);
        assert!(sent.is_empty());
        state.answer(&RESPONDER, "DIALOG READY", &mut sent);
        assert_eq!(sent, b"ab");
        // Redraw caused by our keys still shows the dialog: no second answer.
        state.answer(&RESPONDER, "DIALOG READY redrawn READY", &mut sent);
        assert_eq!(sent, b"ab");
    }

    #[test]
    fn sends_exit_once_after_the_dialog_is_gone() {
        let mut state = ResponderState::default();
        let mut sent = Vec::new();
        state.answer(&RESPONDER, "DIALOG READY", &mut sent);
        assert_eq!(sent, b"ab");
        state.answer(&RESPONDER, "DIALOG READY more", &mut sent);
        assert_eq!(sent, b"ab");
        state.answer(&RESPONDER, "welcome screen", &mut sent);
        assert_eq!(sent, b"ab/exit\r");
        state.answer(&RESPONDER, "welcome screen again", &mut sent);
        assert_eq!(sent, b"ab/exit\r");
    }

    #[test]
    fn answers_a_dialog_drawn_anew() {
        let mut state = ResponderState::default();
        let mut sent = Vec::new();
        state.answer(&RESPONDER, "DIALOG READY", &mut sent);
        state.answer(&RESPONDER, "gone", &mut sent);
        state.answer(&RESPONDER, "gone DIALOG READY", &mut sent);
        assert_eq!(sent, b"ab/exit\rab");
    }

    #[test]
    fn answer_cap_holds_when_the_dialog_keeps_reappearing() {
        let mut state = ResponderState::default();
        let mut sent = Vec::new();
        for _ in 0..(MAX_SCREEN_RESPONSES * 2) {
            state.answer(&RESPONDER, "DIALOG READY", &mut sent);
            state.answer(&RESPONDER, "gone", &mut sent);
        }
        let answers = sent.windows(2).filter(|pair| *pair == b"ab").count();
        assert_eq!(answers, MAX_SCREEN_RESPONSES);
    }
}
