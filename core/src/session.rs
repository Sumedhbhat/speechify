//! The dictation state machine: hold, tap, double-tap lock, cancel, limits.

use crate::hotkey::Bind;
use serde::Serialize;

/// A press shorter than this is a tap, not a dictation.
pub const TAP_MS: u64 = 500;
/// The user is warned here and stopped a minute later.
pub const WARN_MS: u64 = 19 * 60_000;
pub const LIMIT_MS: u64 = 20 * 60_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Down(Bind),
    Up(Bind),
    /// All keys are up.
    Released,
    /// The bar was clicked at rest, or its stop button while recording.
    BarClick,
    BarCancel,
    /// Undo on the "cancelled" notice.
    Undo,
    /// Time passed.
    Tick,
    /// Transcription and pasting finished.
    Finished,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Notice {
    /// "Flow is still processing your last dictation."
    StillProcessing,
    LimitWarning,
    LimitReached,
    /// Cancelled, with an Undo offer.
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// Open the microphone.
    Start,
    /// The mode changed; the bar should show it.
    Mode {
        command: bool,
        locked: bool,
    },
    /// Close the microphone and transcribe.
    Stop {
        command: bool,
    },
    /// Close the microphone and drop the audio. `keep` holds it for Undo.
    Discard {
        keep: bool,
    },
    /// Transcribe the audio kept by the last cancel.
    Retry,
    PasteLast,
    CopyLast,
    Notice(Notice),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Idle,
    /// The shortcut `by` is held down.
    Held {
        since: u64,
        by: Bind,
        command: bool,
        warned: bool,
    },
    /// A tap happened; a second press before `until` locks hands-free.
    TapWait {
        started: u64,
        until: u64,
        command: bool,
    },
    Locked {
        started: u64,
        since: u64,
        command: bool,
        warned: bool,
    },
    Busy,
}

#[derive(Debug)]
pub struct Session {
    state: State,
    paste_pending: bool,
    undoable: bool,
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

impl Session {
    pub fn new() -> Self {
        Self {
            state: State::Idle,
            paste_pending: false,
            undoable: false,
        }
    }

    /// Lets Undo retry a recording the app kept after a failure.
    pub fn offer_undo(&mut self) {
        self.undoable = true;
    }

    pub fn is_recording(&self) -> bool {
        !matches!(self.state, State::Idle | State::Busy)
    }

    pub fn is_busy(&self) -> bool {
        self.state == State::Busy
    }

    /// Advances the machine. `now` is a monotonic clock in milliseconds.
    pub fn handle(&mut self, event: Event, now: u64) -> Vec<Effect> {
        use Bind::*;
        use Event::*;
        let starts = matches!(event, Down(PushToTalk) | Down(HandsFree) | Down(Command) | BarClick);
        match (self.state, event) {
            (_, Down(PasteLast)) => {
                // Pasting while the shortcut's own keys are down would send Alt+Shift+V.
                self.paste_pending = !self.is_recording();
                vec![]
            }
            (_, Released) if self.paste_pending => {
                self.paste_pending = false;
                vec![Effect::PasteLast]
            }
            (State::Idle | State::Busy, Down(CopyLast)) => vec![Effect::CopyLast],

            (State::Idle, Down(by @ (PushToTalk | Command))) => {
                let command = by == Command;
                self.state = State::Held {
                    since: now,
                    by,
                    command,
                    warned: false,
                };
                vec![Effect::Start, Effect::Mode { command, locked: false }]
            }
            (State::Idle, Down(HandsFree) | BarClick) => {
                self.state = State::Locked {
                    started: now,
                    since: now,
                    command: false,
                    warned: false,
                };
                vec![
                    Effect::Start,
                    Effect::Mode {
                        command: false,
                        locked: true,
                    },
                ]
            }
            (State::Idle, Undo) if self.undoable => {
                self.undoable = false;
                self.state = State::Busy;
                vec![Effect::Retry]
            }

            (State::Held { since, by, warned, .. }, Down(Command)) => {
                self.state = State::Held {
                    since,
                    by,
                    command: true,
                    warned,
                };
                vec![Effect::Mode {
                    command: true,
                    locked: false,
                }]
            }
            (
                State::Held {
                    since, command, warned, ..
                },
                Down(HandsFree),
            ) => {
                self.state = State::Locked {
                    started: since,
                    since: now,
                    command,
                    warned,
                };
                vec![Effect::Mode { command, locked: true }]
            }
            (State::Held { since, by, command, .. }, Up(released)) if released == by => {
                if now.saturating_sub(since) < TAP_MS {
                    self.state = State::TapWait {
                        started: since,
                        until: now + TAP_MS,
                        command,
                    };
                    vec![]
                } else {
                    self.state = State::Busy;
                    vec![Effect::Stop { command }]
                }
            }

            (State::TapWait { until, .. }, _) if starts && now >= until => {
                self.state = State::Idle;
                let mut effects = vec![Effect::Discard { keep: false }];
                effects.extend(self.handle(event, now));
                effects
            }
            (State::TapWait { started, command, .. }, _) if starts => {
                self.state = State::Locked {
                    started,
                    since: now,
                    command,
                    warned: false,
                };
                vec![Effect::Mode { command, locked: true }]
            }
            (State::TapWait { until, .. }, Tick) if now >= until => {
                // A stray tap: nothing was meant, nothing is kept.
                self.state = State::Idle;
                vec![Effect::Discard { keep: false }]
            }

            (State::Locked { since, command, .. }, _) if starts => {
                if now.saturating_sub(since) < TAP_MS {
                    self.cancel()
                } else {
                    self.state = State::Busy;
                    vec![Effect::Stop { command }]
                }
            }

            (State::Held { .. } | State::TapWait { .. } | State::Locked { .. }, Down(Cancel) | BarCancel) => {
                self.cancel()
            }

            (
                State::Held {
                    since: started,
                    command,
                    warned,
                    ..
                }
                | State::Locked {
                    started,
                    command,
                    warned,
                    ..
                },
                Tick,
            ) => {
                let elapsed = now.saturating_sub(started);
                if elapsed >= LIMIT_MS {
                    self.state = State::Busy;
                    vec![Effect::Notice(Notice::LimitReached), Effect::Stop { command }]
                } else if elapsed >= WARN_MS && !warned {
                    if let State::Held { warned, .. } | State::Locked { warned, .. } = &mut self.state {
                        *warned = true;
                    }
                    vec![Effect::Notice(Notice::LimitWarning)]
                } else {
                    vec![]
                }
            }

            (State::Busy, _) if starts => vec![Effect::Notice(Notice::StillProcessing)],
            (State::Busy, Finished) => {
                self.state = State::Idle;
                vec![]
            }
            _ => vec![],
        }
    }

    fn cancel(&mut self) -> Vec<Effect> {
        self.state = State::Idle;
        self.undoable = true;
        vec![Effect::Discard { keep: true }, Effect::Notice(Notice::Cancelled)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Bind::*;
    use Event::*;

    const START: [Effect; 2] = [
        Effect::Start,
        Effect::Mode {
            command: false,
            locked: false,
        },
    ];

    #[test]
    fn hold_to_dictate() {
        let mut s = Session::default();
        assert_eq!(s.handle(Down(PushToTalk), 0), START);
        assert!(s.is_recording());
        assert_eq!(s.handle(Up(HandsFree), 100), vec![]);
        assert_eq!(s.handle(Up(PushToTalk), 2000), vec![Effect::Stop { command: false }]);
        assert!(s.is_busy() && !s.is_recording());
        assert_eq!(
            s.handle(Down(PushToTalk), 2100),
            vec![Effect::Notice(Notice::StillProcessing)]
        );
        assert_eq!(s.handle(Finished, 3000), vec![]);
        assert_eq!(s.handle(Down(PushToTalk), 3100), START);
    }

    #[test]
    fn a_stray_tap_is_discarded() {
        let mut s = Session::new();
        s.handle(Down(PushToTalk), 0);
        assert_eq!(s.handle(Up(PushToTalk), 200), vec![]);
        assert_eq!(s.handle(Tick, 400), vec![]);
        assert_eq!(s.handle(Tick, 700), vec![Effect::Discard { keep: false }]);
        assert_eq!(s.handle(Undo, 800), vec![]);
    }

    #[test]
    fn double_tap_locks_and_a_later_press_stops() {
        let mut s = Session::new();
        s.handle(Down(PushToTalk), 0);
        s.handle(Up(PushToTalk), 150);
        assert_eq!(
            s.handle(Down(PushToTalk), 300),
            vec![Effect::Mode {
                command: false,
                locked: true
            }]
        );
        assert_eq!(s.handle(Up(PushToTalk), 400), vec![]);
        assert_eq!(s.handle(Tick, 5000), vec![]);
        assert_eq!(s.handle(Down(PushToTalk), 9000), vec![Effect::Stop { command: false }]);
    }

    #[test]
    fn a_third_quick_press_cancels_with_undo() {
        let mut s = Session::new();
        s.handle(Down(PushToTalk), 0);
        s.handle(Up(PushToTalk), 100);
        s.handle(Down(PushToTalk), 200);
        s.handle(Up(PushToTalk), 300);
        assert_eq!(
            s.handle(Down(PushToTalk), 400),
            vec![Effect::Discard { keep: true }, Effect::Notice(Notice::Cancelled)]
        );
        assert_eq!(s.handle(Undo, 500), vec![Effect::Retry]);
        assert!(s.is_busy());
        assert_eq!(s.handle(Undo, 600), vec![]);
    }

    #[test]
    fn hands_free_from_shortcut_bar_and_mid_hold() {
        let mut s = Session::new();
        assert_eq!(
            s.handle(Down(HandsFree), 0),
            vec![
                Effect::Start,
                Effect::Mode {
                    command: false,
                    locked: true
                }
            ]
        );
        assert_eq!(s.handle(BarClick, 3000), vec![Effect::Stop { command: false }]);
        s.handle(Finished, 3500);
        assert_eq!(
            s.handle(BarClick, 4000),
            vec![
                Effect::Start,
                Effect::Mode {
                    command: false,
                    locked: true
                }
            ]
        );
        assert_eq!(
            s.handle(BarCancel, 6000),
            vec![Effect::Discard { keep: true }, Effect::Notice(Notice::Cancelled)]
        );
        s.handle(Down(PushToTalk), 7000);
        assert_eq!(
            s.handle(Down(HandsFree), 7300),
            vec![Effect::Mode {
                command: false,
                locked: true
            }]
        );
        assert_eq!(s.handle(Up(PushToTalk), 7400), vec![]);
        assert_eq!(s.handle(Down(HandsFree), 9000), vec![Effect::Stop { command: false }]);
    }

    #[test]
    fn command_mode() {
        let mut s = Session::new();
        assert_eq!(
            s.handle(Down(Command), 0),
            vec![
                Effect::Start,
                Effect::Mode {
                    command: true,
                    locked: false
                }
            ]
        );
        assert_eq!(s.handle(Up(PushToTalk), 900), vec![]);
        assert_eq!(s.handle(Up(Command), 1000), vec![Effect::Stop { command: true }]);
        s.handle(Finished, 1100);
        // Ctrl+Meta registers first, then Alt upgrades the dictation to a command.
        s.handle(Down(PushToTalk), 2000);
        assert_eq!(
            s.handle(Down(Command), 2050),
            vec![Effect::Mode {
                command: true,
                locked: false
            }]
        );
        assert_eq!(s.handle(Up(Command), 3000), vec![]);
        assert_eq!(s.handle(Up(PushToTalk), 3100), vec![Effect::Stop { command: true }]);
    }

    #[test]
    fn escape_cancels_every_recording_state() {
        let mut s = Session::new();
        assert_eq!(s.handle(Down(Cancel), 0), vec![]);
        s.handle(Down(PushToTalk), 0);
        assert_eq!(s.handle(Down(Cancel), 100).len(), 2);
        s.handle(Down(PushToTalk), 1000);
        s.handle(Up(PushToTalk), 1100);
        assert_eq!(s.handle(Down(Cancel), 1200).len(), 2);
        assert!(!s.is_recording());
    }

    #[test]
    fn long_sessions_warn_then_stop() {
        let mut s = Session::new();
        s.handle(Down(HandsFree), 0);
        assert_eq!(s.handle(Tick, WARN_MS), vec![Effect::Notice(Notice::LimitWarning)]);
        assert_eq!(s.handle(Tick, WARN_MS + 1000), vec![]);
        assert_eq!(
            s.handle(Tick, LIMIT_MS),
            vec![Effect::Notice(Notice::LimitReached), Effect::Stop { command: false }]
        );
        s.handle(Finished, LIMIT_MS + 1);
        s.handle(Down(PushToTalk), 0);
        assert_eq!(s.handle(Tick, WARN_MS), vec![Effect::Notice(Notice::LimitWarning)]);
        assert_eq!(
            s.handle(Tick, LIMIT_MS),
            vec![Effect::Notice(Notice::LimitReached), Effect::Stop { command: false }]
        );
    }

    #[test]
    fn paste_last_waits_for_keys_to_be_released() {
        let mut s = Session::new();
        assert_eq!(s.handle(Released, 0), vec![]);
        assert_eq!(s.handle(Down(PasteLast), 10), vec![]);
        assert_eq!(s.handle(Released, 50), vec![Effect::PasteLast]);
        assert_eq!(s.handle(Released, 60), vec![]);
        assert_eq!(s.handle(Down(CopyLast), 70), vec![Effect::CopyLast]);
        s.handle(Down(PushToTalk), 100);
        assert_eq!(s.handle(Down(PasteLast), 200), vec![]);
        assert_eq!(s.handle(Down(CopyLast), 210), vec![]);
        assert_eq!(s.handle(Released, 300), vec![]);
    }

    #[test]
    fn expired_tap_starts_a_fresh_recording_without_a_tick() {
        for event in [Down(PushToTalk), Down(Command), Down(HandsFree), BarClick] {
            let mut s = Session::new();
            s.handle(Down(PushToTalk), 0);
            s.handle(Up(PushToTalk), 100);
            let mut fresh = Session::new();
            let mut expected = vec![Effect::Discard { keep: false }];
            expected.extend(fresh.handle(event, 600));
            assert_eq!(s.handle(event, 600), expected);
            assert!(s.is_recording());
        }
        let mut s = Session::new();
        s.handle(Down(PushToTalk), 0);
        s.handle(Up(PushToTalk), 100);
        assert_eq!(
            s.handle(Down(PushToTalk), 800),
            vec![
                Effect::Discard { keep: false },
                Effect::Start,
                Effect::Mode {
                    command: false,
                    locked: false
                }
            ]
        );
        assert_eq!(s.handle(Up(PushToTalk), 1400), vec![Effect::Stop { command: false }]);
    }
}
