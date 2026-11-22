//! Which clicks the floating bar takes.
//!
//! The bar is a transparent window a little larger than what it shows. A
//! transparent window still takes every click inside its frame, and on a Mac
//! each one brings Speechify to the front. So the shell watches the pointer
//! and lets the window take clicks only while the pointer is over something
//! the bar draws: the pill, a card or a notice.

/// An area of the bar page that takes clicks: `[x, y, width, height]` in
/// logical pixels from the window's top-left corner.
pub type Area = [f64; 4];

/// Extra room around each area so the thin idle handle is not fiddly to hit.
pub const SLACK: f64 = 4.0;

/// Whether the pointer is over one of `areas`. `pointer` and `origin` (the
/// window's top-left corner) are physical screen pixels; `scale` is the
/// window's pixels per logical pixel.
pub fn over(areas: &[Area], pointer: (f64, f64), origin: (f64, f64), scale: f64) -> bool {
    let x = (pointer.0 - origin.0) / scale;
    let y = (pointer.1 - origin.1) / scale;
    areas.iter().any(|&[left, top, width, height]| {
        x >= left - SLACK && x <= left + width + SLACK && y >= top - SLACK && y <= top + height + SLACK
    })
}

/// What the shell saw on one look at the pointer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Seen {
    /// The bar is not on screen.
    Hidden,
    /// The system would not say where the pointer is.
    Unknown,
    /// The pointer is over (true) or away from (false) a clickable area.
    Pointer(bool),
}

/// What the shell should change on the bar window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Change {
    /// Let clicks through to the apps underneath.
    pub pass_through: bool,
    /// Tell the page whether the pointer is on it; `None` leaves it to CSS.
    pub hover: Option<bool>,
}

/// Turns looks at the pointer into window changes, only when something
/// changed.
#[derive(Debug, Default)]
pub struct Tracker {
    /// `None` until the window's state is known, and again once it hides,
    /// so a bar that reappears is set up afresh.
    taking_clicks: Option<bool>,
    /// The hover last told to the page, kept apart from `taking_clicks`
    /// because an unknown pointer changes one but not the other.
    hover: Option<bool>,
}

impl Tracker {
    pub fn update(&mut self, seen: Seen) -> Option<Change> {
        let (take, hover) = match seen {
            Seen::Hidden => {
                *self = Self::default();
                return None;
            }
            // Without the pointer, keep the bar clickable rather than dead.
            Seen::Unknown => (true, None),
            Seen::Pointer(inside) => (inside, Some(inside)),
        };
        let hover = hover.filter(|&hover| self.hover != Some(hover));
        if self.taking_clicks == Some(take) && hover.is_none() {
            return None;
        }
        self.taking_clicks = Some(take);
        if hover.is_some() {
            self.hover = hover;
        }
        Some(Change {
            pass_through: !take,
            hover,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HANDLE: Area = [82.0, 34.0, 56.0, 10.0];

    #[test]
    fn takes_clicks_only_on_what_is_drawn() {
        let at = |x, y| over(&[HANDLE], (x, y), (0.0, 0.0), 1.0);
        assert!(at(110.0, 39.0));
        // Within the slack around the handle.
        assert!(at(78.0, 30.0));
        assert!(at(142.0, 48.0));
        // Beside and above the handle, still inside the window.
        assert!(!at(77.0, 39.0));
        assert!(!at(143.0, 39.0));
        assert!(!at(110.0, 29.0));
        assert!(!at(10.0, 5.0));
    }

    #[test]
    fn converts_screen_pixels_to_the_page() {
        // A window at (1000, 600) physical on a 2x screen.
        let origin = (1000.0, 600.0);
        assert!(over(&[HANDLE], (1000.0 + 110.0 * 2.0, 600.0 + 39.0 * 2.0), origin, 2.0));
        assert!(!over(&[HANDLE], (1000.0 + 110.0, 600.0 + 39.0), origin, 2.0));
    }

    #[test]
    fn any_area_counts_and_none_takes_nothing() {
        let card = [12.0, 8.0, 416.0, 80.0];
        assert!(over(&[HANDLE, card], (20.0, 20.0), (0.0, 0.0), 1.0));
        assert!(!over(&[], (110.0, 39.0), (0.0, 0.0), 1.0));
    }

    #[test]
    fn changes_the_window_only_when_the_pointer_crosses() {
        let mut tracker = Tracker::default();
        let away = Some(Change {
            pass_through: true,
            hover: Some(false),
        });
        let on = Some(Change {
            pass_through: false,
            hover: Some(true),
        });
        assert_eq!(tracker.update(Seen::Pointer(false)), away);
        assert_eq!(tracker.update(Seen::Pointer(false)), None);
        assert_eq!(tracker.update(Seen::Pointer(true)), on);
        assert_eq!(tracker.update(Seen::Pointer(true)), None);
        assert_eq!(tracker.update(Seen::Pointer(false)), away);
    }

    #[test]
    fn sets_a_reappearing_bar_up_again() {
        let mut tracker = Tracker::default();
        tracker.update(Seen::Pointer(false));
        assert_eq!(tracker.update(Seen::Hidden), None);
        assert_eq!(
            tracker.update(Seen::Pointer(false)),
            Some(Change {
                pass_through: true,
                hover: Some(false)
            })
        );
    }

    #[test]
    fn stays_clickable_when_the_pointer_is_unknown() {
        let mut tracker = Tracker::default();
        let open = Some(Change {
            pass_through: false,
            hover: None,
        });
        assert_eq!(tracker.update(Seen::Unknown), open);
        assert_eq!(tracker.update(Seen::Unknown), None);
        tracker.update(Seen::Pointer(false));
        assert_eq!(tracker.update(Seen::Unknown), open);
    }

    #[test]
    fn tells_the_page_about_hover_after_an_unknown_pointer() {
        let on = Some(Change {
            pass_through: false,
            hover: Some(true),
        });
        let mut tracker = Tracker::default();
        tracker.update(Seen::Pointer(false));
        tracker.update(Seen::Unknown);
        // Already taking clicks, but the page has not heard of the hover.
        assert_eq!(tracker.update(Seen::Pointer(true)), on);
        assert_eq!(tracker.update(Seen::Pointer(true)), None);

        let mut tracker = Tracker::default();
        tracker.update(Seen::Hidden);
        tracker.update(Seen::Unknown);
        assert_eq!(tracker.update(Seen::Pointer(true)), on);
    }
}
