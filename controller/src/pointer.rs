pub type Area = [f64; 4];

pub const SLACK: f64 = 4.0;

pub fn over(areas: &[Area], pointer: (f64, f64), origin: (f64, f64), scale: f64) -> bool {
    let x = (pointer.0 - origin.0) / scale;
    let y = (pointer.1 - origin.1) / scale;
    areas.iter().any(|&[left, top, width, height]| {
        width > 0.0
            && height > 0.0
            && x >= left - SLACK
            && x <= left + width + SLACK
            && y >= top - SLACK
            && y <= top + height + SLACK
    })
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Seen {
    Hidden,
    Unknown,
    Pointer { clickable: bool, on_pill: bool },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Change {
    pub pass_through: bool,
    pub hover: Option<bool>,
}

#[derive(Debug, Default)]
pub struct Tracker {
    taking_clicks: Option<bool>,
    hover: Option<bool>,
}

impl Tracker {
    pub fn update(&mut self, seen: Seen) -> Option<Change> {
        let (take, hover) = match seen {
            Seen::Hidden => {
                *self = Self::default();
                return None;
            }
            Seen::Unknown => (true, None),
            Seen::Pointer { clickable, on_pill } => (clickable, Some(on_pill)),
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

    fn at(clickable: bool, on_pill: bool) -> Seen {
        Seen::Pointer { clickable, on_pill }
    }

    #[test]
    fn takes_clicks_only_on_what_is_drawn() {
        let at = |x, y| over(&[HANDLE], (x, y), (0.0, 0.0), 1.0);
        assert!(at(110.0, 39.0));
        assert!(at(78.0, 30.0));
        assert!(at(142.0, 48.0));
        assert!(!at(77.0, 39.0));
        assert!(!at(143.0, 39.0));
        assert!(!at(110.0, 29.0));
        assert!(!at(10.0, 5.0));
    }

    #[test]
    fn converts_screen_pixels_to_the_page() {
        let origin = (1000.0, 600.0);
        assert!(over(&[HANDLE], (1000.0 + 110.0 * 2.0, 600.0 + 39.0 * 2.0), origin, 2.0));
        assert!(!over(&[HANDLE], (1000.0 + 110.0, 600.0 + 39.0), origin, 2.0));
    }

    #[test]
    fn any_area_counts_and_none_takes_nothing() {
        let card = [12.0, 8.0, 416.0, 80.0];
        assert!(over(&[HANDLE, card], (20.0, 20.0), (0.0, 0.0), 1.0));
        assert!(!over(&[], (110.0, 39.0), (0.0, 0.0), 1.0));
        assert!(!over(&[[0.0, 0.0, 0.0, 0.0]], (1.0, 1.0), (0.0, 0.0), 1.0));
        assert!(!over(&[[5.0, 5.0, 10.0, 0.0]], (8.0, 5.0), (0.0, 0.0), 1.0));
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
        assert_eq!(tracker.update(at(false, false)), away);
        assert_eq!(tracker.update(at(false, false)), None);
        assert_eq!(tracker.update(at(true, true)), on);
        assert_eq!(tracker.update(at(true, true)), None);
        assert_eq!(tracker.update(at(false, false)), away);
    }

    #[test]
    fn hovers_only_on_the_pill_but_clicks_anywhere_drawn() {
        let mut tracker = Tracker::default();
        tracker.update(at(false, false));
        assert_eq!(
            tracker.update(at(true, false)),
            Some(Change {
                pass_through: false,
                hover: None
            })
        );
        assert_eq!(
            tracker.update(at(true, true)),
            Some(Change {
                pass_through: false,
                hover: Some(true)
            })
        );
        assert_eq!(
            tracker.update(at(true, false)),
            Some(Change {
                pass_through: false,
                hover: Some(false)
            })
        );
    }

    #[test]
    fn sets_a_reappearing_bar_up_again() {
        let mut tracker = Tracker::default();
        tracker.update(at(false, false));
        assert_eq!(tracker.update(Seen::Hidden), None);
        assert_eq!(
            tracker.update(at(false, false)),
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
        tracker.update(at(false, false));
        assert_eq!(tracker.update(Seen::Unknown), open);
    }

    #[test]
    fn tells_the_page_about_hover_after_an_unknown_pointer() {
        let on = Some(Change {
            pass_through: false,
            hover: Some(true),
        });
        let mut tracker = Tracker::default();
        tracker.update(at(false, false));
        tracker.update(Seen::Unknown);
        assert_eq!(tracker.update(at(true, true)), on);
        assert_eq!(tracker.update(at(true, true)), None);

        let mut tracker = Tracker::default();
        tracker.update(Seen::Hidden);
        tracker.update(Seen::Unknown);
        assert_eq!(tracker.update(at(true, true)), on);
    }
}
