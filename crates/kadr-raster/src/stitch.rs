//! The scrolling screenshot's picture: frames of an area that one scrolls,
//! stitched into one long picture by matching what they already showed. The
//! direction comes from the first real move: down (a page) or right (a
//! timeline, a wide table); the other way is worked as the picture turned on
//! its diagonal, so both go through the same matching.

use image::RgbImage;

/// The picture stops growing at this many rows (or columns).
const MAX_LENGTH: usize = 40_000;
/// The scrollbar and its fade, at the far edge across the scroll, change
/// while scrolling.
const IGNORE_EDGE: usize = 28;

/// Brightness samples per row: rows are compared by these, not exactly, since
/// with fractional scaling a row that scrolled lands on slightly different
/// pixels and never matches byte for byte while the page moves.
const BINS: usize = 32;
/// Rows that didn't move come out identical; this only absorbs rounding.
const SAME_ROW: f32 = 0.1;
/// The best shift has to line rows up at least this well...
const MATCH: f32 = 1.5;
/// ...and this much better than no shift at all: frames that moved the
/// other way line up a little under a shift of one by chance.
const BETTER: f32 = 0.5;

/// One row, as average brightness across `BINS` strips.
type Row = [f32; BINS];

/// Which way the picture grows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    Down,
    Right,
}

/// Stitches frames of a scrolling area into one long picture.
pub struct Stitcher {
    /// One strip per direction until the first move says which; then one.
    strips: Vec<(Axis, Strip)>,
}

impl Stitcher {
    pub fn new(first: RgbImage) -> Stitcher {
        let across = transpose(&first);
        Stitcher { strips: vec![(Axis::Down, Strip::new(first)), (Axis::Right, Strip::new(across))] }
    }

    /// The direction, once the area has moved.
    pub fn axis(&self) -> Option<Axis> {
        (self.strips.len() == 1).then(|| self.strips[0].0)
    }

    /// The picture's size so far.
    pub fn size(&self) -> (usize, usize) {
        match &self.strips[..] {
            [(Axis::Right, s)] => (s.height, s.width),
            [(_, s), ..] => (s.width, s.height),
            [] => (0, 0),
        }
    }

    /// Add a frame; true when it brought new rows.
    pub fn push(&mut self, frame: RgbImage) -> bool {
        if self.strips.len() == 1 {
            let (axis, strip) = &mut self.strips[0];
            let frame = if *axis == Axis::Right { transpose(&frame) } else { frame };
            let seen = strip.measure(&frame);
            return strip.push(&frame, seen);
        }
        let across = transpose(&frame);
        let down = self.strips[0].1.measure(&frame);
        let right = self.strips[1].1.measure(&across);
        let moved = |o: &Option<Seen>| matches!(o, Some(s) if s.shift > 0);
        // Whichever moved, or lined up better when both seem to.
        let pick = match (moved(&down), moved(&right)) {
            (false, false) => None,
            (true, false) => Some(0),
            (false, true) => Some(1),
            (true, true) => Some(if down.as_ref().map(|d| d.score) <= right.as_ref().map(|r| r.score) { 0 } else { 1 }),
        };
        match pick {
            None => {
                self.strips[0].1.push(&frame, down);
                self.strips[1].1.push(&across, right);
                false
            }
            Some(i) => {
                let (axis, mut strip) = self.strips.swap_remove(i);
                self.strips.clear();
                let grew = if i == 0 { strip.push(&frame, down) } else { strip.push(&across, right) };
                self.strips.push((axis, strip));
                grew
            }
        }
    }

    pub fn image(mut self) -> Option<RgbImage> {
        let (axis, strip) = self.strips.drain(..).next()?;
        let picture = RgbImage::from_raw(strip.width as u32, strip.height as u32, strip.pixels)?;
        Some(if axis == Axis::Right { transpose(&picture) } else { picture })
    }
}

/// Rows become columns.
fn transpose(image: &RgbImage) -> RgbImage {
    let (w, h) = (image.width() as usize, image.height() as usize);
    let src = image.as_raw();
    let mut out = vec![0u8; src.len()];
    for y in 0..h {
        for x in 0..w {
            let (from, to) = ((y * w + x) * 3, (x * h + y) * 3);
            out[to..to + 3].copy_from_slice(&src[from..from + 3]);
        }
    }
    RgbImage::from_raw(h as u32, w as u32, out).expect("same size")
}

/// A frame as matched against the last one.
struct Seen {
    rows: Vec<Option<Row>>,
    /// How many rows it scrolled by.
    shift: usize,
    /// How well the rows lined up under that shift (lower is better).
    score: f32,
}

/// The picture growing downwards.
struct Strip {
    width: usize,
    height: usize,
    /// RGB rows of the whole picture so far.
    pixels: Vec<u8>,
    /// Rows of the last frame, None for flat (single-colour) rows, which
    /// match anything and can't tell scroll positions apart.
    last: Vec<Option<Row>>,
}

fn rows(frame: &RgbImage) -> Vec<Option<Row>> {
    let (w, h) = (frame.width() as usize, frame.height() as usize);
    let used = w.saturating_sub(IGNORE_EDGE).max(BINS).min(w);
    (0..h)
        .map(|y| {
            let line = &frame.as_raw()[y * w * 3..y * w * 3 + used * 3];
            let mut row = [0f32; BINS];
            for (i, bin) in row.iter_mut().enumerate() {
                let (a, b) = (i * used / BINS, ((i + 1) * used / BINS).max(i * used / BINS + 1));
                let sum: u32 = line[a * 3..b * 3].iter().map(|&v| v as u32).sum();
                *bin = sum as f32 / ((b - a) * 3) as f32;
            }
            let first = &line[..3];
            let flat = line.chunks_exact(3).all(|p| p.iter().zip(first).all(|(&a, &b)| a.abs_diff(b) <= 2));
            (!flat).then_some(row)
        })
        .collect()
}

fn distance(a: &Row, b: &Row) -> f32 {
    a.iter().zip(b).map(|(x, y)| (x - y).abs()).sum::<f32>() / BINS as f32
}

impl Strip {
    fn new(first: RgbImage) -> Strip {
        let last = rows(&first);
        Strip { width: first.width() as usize, height: first.height() as usize, pixels: first.into_raw(), last }
    }

    /// How far `frame` scrolled past the last frame; None when it doesn't
    /// line up (or the picture is full).
    fn measure(&self, frame: &RgbImage) -> Option<Seen> {
        if frame.width() as usize != self.width || self.height >= MAX_LENGTH {
            return None;
        }
        let rows = rows(frame);
        let (shift, score) = self.offset(&rows)?;
        Some(Seen { rows, shift, score })
    }

    /// How far `next` scrolled past the last frame, in rows: the shift under
    /// which their rows line up best, and its score. None when they don't
    /// line up.
    fn offset(&self, next: &[Option<Row>]) -> Option<(usize, f32)> {
        let h = next.len().min(self.last.len());
        // Rows that stayed put (a sticky header, a frozen column) say nothing
        // about the scroll; leave them out. Nothing moved at all: no scroll.
        let fixed: Vec<bool> = (0..h).map(|r| matches!((&next[r], &self.last[r]), (Some(a), Some(b)) if distance(a, b) < SAME_ROW)).collect();
        if (0..h).all(|r| fixed[r] || next[r].is_none()) {
            return Some((0, 0.));
        }
        // How far apart the rows are where they are: what a shift has to beat.
        let (mut total, mut seen) = (0f32, 0usize);
        for r in 0..h {
            if let (Some(a), Some(b)) = (&self.last[r], &next[r]) {
                seen += 1;
                total += distance(a, b);
            }
        }
        let unshifted = total / seen.max(1) as f32;
        let mut best: Option<(usize, f32)> = None;
        for dy in 1..h {
            let (mut total, mut seen) = (0f32, 0usize);
            for r in 0..h - dy {
                if fixed[r] {
                    continue;
                }
                if let (Some(a), Some(b)) = (&self.last[r + dy], &next[r]) {
                    seen += 1;
                    total += distance(a, b);
                }
            }
            // Enough of the overlap has to be real content: a few rows line up
            // with something by chance. Frames come often, so a real scroll
            // leaves plenty of overlap.
            if seen < 12 || (h - dy) * 3 < h {
                continue;
            }
            let score = total / seen as f32;
            if best.is_none_or(|(_, s)| score < s) {
                best = Some((dy, score));
            }
        }
        best.filter(|&(_, score)| score < MATCH && score < unshifted * BETTER)
    }

    /// Take a frame as `measure` saw it; true when it brought new rows.
    fn push(&mut self, frame: &RgbImage, seen: Option<Seen>) -> bool {
        let Some(Seen { rows, shift: dy, .. }) = seen else { return false };
        self.last = rows;
        if dy == 0 {
            return false;
        }
        // The last `dy` rows of the new frame are new.
        let h = frame.height() as usize;
        self.pixels.extend_from_slice(&frame.as_raw()[(h - dy) * self.width * 3..]);
        self.height += dy;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tall page with distinct rows, seen through a window that scrolls.
    fn page(height: u32) -> RgbImage {
        RgbImage::from_fn(80, height, |x, y| image::Rgb([(y % 251) as u8, (y / 251) as u8 * 40 + (x % 7) as u8, (x * 3) as u8]))
    }

    /// Smooth like rendered content, so a blend of two rows sits between
    /// them, and never the same twice.
    fn smooth(height: u32) -> RgbImage {
        RgbImage::from_fn(240, height, |x, y| {
            let (y, x) = (y as f32, x as f32);
            let v = 128. + 60. * (y * 0.071).sin() + 60. * (y * 0.0173 + x * 0.05).sin();
            image::Rgb([v as u8, (v * 0.5) as u8, (x as u32 % 256) as u8])
        })
    }

    fn view(page: &RgbImage, top: u32, h: u32) -> RgbImage {
        image::imageops::crop_imm(page, 0, top, page.width(), h).to_image()
    }

    #[test]
    fn stitches_a_scrolled_page_back_together() {
        let full = page(900);
        let mut s = Stitcher::new(view(&full, 0, 300));
        assert!(!s.push(view(&full, 0, 300)), "no scroll, nothing new");
        assert_eq!(s.axis(), None);
        for top in [70, 190, 350, 520, 600] {
            assert!(s.push(view(&full, top, 300)));
        }
        assert_eq!(s.axis(), Some(Axis::Down));
        let out = s.image().unwrap();
        assert_eq!(out.height(), 900);
        assert_eq!(out, full);
    }

    #[test]
    fn stitches_a_page_scrolled_sideways() {
        let full = transpose(&page(900));
        let side = |left: u32| image::imageops::crop_imm(&full, left, 0, 300, full.height()).to_image();
        let mut s = Stitcher::new(side(0));
        for left in [0, 55, 200, 310, 480, 600] {
            s.push(side(left));
        }
        assert_eq!(s.axis(), Some(Axis::Right));
        assert_eq!(s.size(), (900, 80));
        assert_eq!(s.image().unwrap(), full);
    }

    /// A page scrolled down further between two grabs than they overlap
    /// (a slow grab) doesn't line up, and isn't taken for a move to the
    /// right either; scrolled back to what was taken, it goes on.
    #[test]
    fn a_long_jump_waits_for_the_page_to_come_back() {
        let full = smooth(1200);
        let mut s = Stitcher::new(view(&full, 0, 300));
        assert!(!s.push(view(&full, 250, 300)), "too far to line up");
        assert_eq!(s.axis(), None);
        for top in [120, 300, 480, 660] {
            assert!(s.push(view(&full, top, 300)), "at {top}");
        }
        assert_eq!(s.axis(), Some(Axis::Down));
        assert_eq!(s.image().unwrap(), view(&full, 0, 960));
    }

    /// A small move down, caught half way (between pixels), leaves every
    /// column a little changed, and neighbouring columns alike: a shift of
    /// one column mustn't pass for a move to the right.
    #[test]
    fn a_small_move_down_is_not_taken_for_sideways() {
        let full = smooth(600);
        for dy in 1..6 {
            let (a, b) = (view(&full, dy, 300), view(&full, dy + 1, 300));
            let half = RgbImage::from_fn(a.width(), a.height(), |x, y| {
                let (p, q) = (a.get_pixel(x, y).0, b.get_pixel(x, y).0);
                image::Rgb([0, 1, 2].map(|i| ((p[i] as u16 + q[i] as u16) / 2) as u8))
            });
            let mut s = Stitcher::new(view(&full, 0, 300));
            s.push(half);
            assert_ne!(s.axis(), Some(Axis::Right), "{dy}.5 rows down taken for sideways");
        }
    }

    /// With fractional scaling a page that scrolls lands between pixels: each
    /// frame is a blend of two neighbouring rows, never an exact copy.
    #[test]
    fn stitches_frames_that_land_between_pixels() {
        let full = smooth(1200);
        let half = |top: u32| {
            let (a, b) = (view(&full, top, 300), view(&full, top + 1, 300));
            RgbImage::from_fn(a.width(), a.height(), |x, y| {
                let (p, q) = (a.get_pixel(x, y).0, b.get_pixel(x, y).0);
                image::Rgb([0, 1, 2].map(|i| ((p[i] as u16 + q[i] as u16) / 2) as u8))
            })
        };
        let mut s = Stitcher::new(view(&full, 0, 300));
        let mut tops = Vec::new();
        for top in [40, 95, 170, 260, 330, 420, 515, 600, 700, 820] {
            if top % 2 == 0 { s.push(half(top)) } else { s.push(view(&full, top, 300)) };
            tops.push(top);
        }
        let end = *tops.last().unwrap() as i64 + 300;
        assert!((s.size().1 as i64 - end).abs() <= 2, "height {} vs {end}", s.size().1);
    }

    /// KADR_TEST_PAGE=<tall png>: a real page with a sticky header and a
    /// scrollbar that moves, scrolled by uneven steps.
    #[test]
    fn stitches_a_real_page() {
        let Ok(path) = std::env::var("KADR_TEST_PAGE") else { return };
        let full = image::open(path).unwrap().to_rgb8();
        let (w, h) = (full.width(), 700);
        let header = image::imageops::crop_imm(&full, 0, 0, w, 90).to_image();
        let shot = |top: u32| {
            let mut v = view(&full, top, h);
            image::imageops::replace(&mut v, &header, 0, 0);
            // The scrollbar thumb moves with the page.
            for y in 0..h {
                for x in w - 12..w {
                    let on = y >= top * h / full.height() && y < top * h / full.height() + 60;
                    v.put_pixel(x, y, if on { image::Rgb([200, 200, 200]) } else { image::Rgb([40, 40, 40]) });
                }
            }
            v
        };
        let mut s = Stitcher::new(shot(0));
        let mut top = 0;
        for step in [37, 120, 260, 95, 300, 180, 410] {
            top = (top + step).min(full.height() - h);
            s.push(shot(top));
        }
        let out = s.image().unwrap();
        assert_eq!(out.height(), top + h, "stitched to the bottom of the page");
        let a = image::imageops::crop_imm(&out, 0, 300, w - 30, 600).to_image();
        let b = image::imageops::crop_imm(&full, 0, 300, w - 30, 600).to_image();
        assert!(a == b, "content matches the page");
    }
}

