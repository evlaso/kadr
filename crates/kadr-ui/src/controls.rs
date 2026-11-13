//! Small controls' arithmetic, independent of the renderer. Tick marks are
//! visual only today; dragging does not snap to them.

/// A pointer on a track, clamped at its ends. Keep the one-pixel minimum
/// used by both editors, including before the track has its full width.
pub fn fraction(x: f32, start: f32, width: f32) -> f32 {
    ((x - start) / width.max(1.)).clamp(0., 1.)
}

/// The coordinate at a fraction of a track.
pub fn position(value: f32, start: f32, width: f32) -> f32 {
    start + width * value.clamp(0., 1.)
}

/// Round a value to the increments its label shows (the studio's zoom and
/// volume use twentieths). This does not change the track's tick marks.
pub fn stepped(value: f64, steps_per_unit: f64) -> f64 {
    (value * steps_per_unit).round() / steps_per_unit
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn track_mapping_keeps_endpoints_and_clamps() {
        assert_eq!(fraction(10., 10., 200.), 0.);
        assert_eq!(fraction(110., 10., 200.), 0.5);
        assert_eq!(fraction(210., 10., 200.), 1.);
        assert_eq!(fraction(-10., 10., 200.), 0.);
        assert_eq!(fraction(300., 10., 200.), 1.);
        assert_eq!(fraction(10.5, 10., 0.), 0.5);
        for v in [0., 0.13, 0.5, 1.] {
            assert!((fraction(position(v, 20., 160.), 20., 160.) - v).abs() < 1e-6);
        }
    }

    #[test]
    fn marks_do_not_snap_and_steps_match_labels() {
        assert_eq!(fraction(13., 0., 100.), 0.13);
        assert_eq!(stepped(1.224, 20.), 1.2);
        assert_eq!(stepped(1.226, 20.), 1.25);
        assert_eq!(stepped(0., 20.), 0.);
    }
}
