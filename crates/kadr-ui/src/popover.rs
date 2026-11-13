//! Popover geometry and dismissal, without a window-system dependency.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side { Below, Above }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align { Start, End, Stretch }

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect { pub x: f32, pub y: f32, pub w: f32, pub h: f32 }

/// The offset is from the anchor's origin (or its far edge for `edge`).
/// Origin offsets preserve the existing menus' fixed 30/34/38/40 px tops.
#[derive(Clone, Copy, Debug)]
pub struct Placement {
    pub side: Side,
    pub align: Align,
    pub offset: f32,
    pub edge: bool,
}

impl Placement {
    pub fn below(offset: f32, align: Align) -> Self { Self { side: Side::Below, align, offset, edge: false } }
    pub fn above(offset: f32, align: Align) -> Self { Self { side: Side::Above, align, offset, edge: false } }

    /// A gap beyond the anchor, rather than a distance from its origin.
    pub fn edge(mut self) -> Self { self.edge = true; self }

    pub fn position(self, anchor: Rect, size: (f32, f32), window: (f32, f32), constrain: bool) -> (f32, f32) {
        let x = match self.align { Align::Start | Align::Stretch => anchor.x, Align::End => anchor.x + anchor.w - size.0 };
        let y = match self.side {
            Side::Below => anchor.y + self.offset + if self.edge { anchor.h } else { 0. },
            Side::Above => anchor.y - size.1 - self.offset + if self.edge { 0. } else { anchor.h },
        };
        if constrain { (x.clamp(0., (window.0 - size.0).max(0.)), y.clamp(0., (window.1 - size.1).max(0.))) } else { (x, y) }
    }
}

/// Whether a menu fits below a recording bar. Its strict edge check is
/// intentional: equality used to put that menu above the bar.
pub fn fits_below(bottom: f32, height: f32, margin: f32, window_height: f32) -> bool {
    bottom + height + margin < window_height
}

#[derive(Clone, Copy, Debug, Default)]
pub enum Outside {
    #[default]
    AnchorAndMenu,
    /// The compact editor's old menu closes even on deferred children.
    AnchorOnly,
}

pub fn dismiss_outside(open: bool, in_menu: bool, policy: Outside) -> bool {
    open && (matches!(policy, Outside::AnchorOnly) || !in_menu)
}

#[cfg(test)]
mod tests {
    use super::*;
    const ANCHOR: Rect = Rect { x: 100., y: 80., w: 60., h: 32. };

    #[test]
    fn origin_offsets_and_alignment_keep_the_old_positions() {
        assert_eq!(Placement::below(38., Align::Start).position(ANCHOR, (150., 100.), (500., 400.), false), (100., 118.));
        assert_eq!(Placement::above(38., Align::End).position(ANCHOR, (150., 100.), (500., 400.), false), (10., -26.));
        assert_eq!(Placement::below(6., Align::Stretch).edge().position(ANCHOR, (60., 100.), (500., 400.), false), (100., 118.));
    }

    #[test]
    fn constrained_placement_handles_edges_and_oversized_menus() {
        assert_eq!(Placement::above(38., Align::End).position(ANCHOR, (150., 100.), (500., 400.), true), (10., 0.));
        assert_eq!(Placement::below(38., Align::Start).position(ANCHOR, (600., 500.), (500., 400.), true), (0., 0.));
        assert_eq!(Placement::below(38., Align::End).position(ANCHOR, (150., 100.), (120., 150.), true), (0., 50.));
    }

    #[test]
    fn outside_policy_does_not_hide_host_state() {
        assert!(!dismiss_outside(false, false, Outside::AnchorOnly));
        assert!(!dismiss_outside(true, true, Outside::AnchorAndMenu));
        assert!(dismiss_outside(true, false, Outside::AnchorAndMenu));
        assert!(dismiss_outside(true, true, Outside::AnchorOnly));
    }

    #[test]
    fn recording_menu_flips_at_its_original_boundary() {
        assert!(fits_below(100., 200., 8., 309.));
        assert!(!fits_below(100., 200., 8., 308.));
    }
}
