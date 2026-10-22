//! The camera picture's look in a recording: where it sits, how big and how
//! round, kept between recordings.

use kadr_core::dirs;

/// Where the camera can sit, as fractions across the free room: corners and
/// the middles of the edges, clockwise from the top left.
pub const ANCHORS: [(f64, f64); 8] = [(0., 0.), (0.5, 0.), (1., 0.), (1., 0.5), (1., 1.), (0.5, 1.), (0., 1.), (0., 0.5)];
/// Sizes, as a share of the area's short side.
pub const SIZES: [f64; 6] = [0.16, 0.21, 0.26, 0.32, 0.4, 0.5];
/// Rounding: a circle, rounded corners, square.
pub const ROUNDS: [f64; 3] = [1., 0.4, 0.];

/// The camera's look, kept between recordings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    pub anchor: usize,
    pub size: f64,
    /// 0 square … 1 circle.
    pub round: f64,
}

impl Default for Look {
    fn default() -> Self {
        Look { anchor: 4, size: 0.26, round: 1. }
    }
}

impl Look {
    fn path() -> std::path::PathBuf {
        dirs::config().join("camera.json")
    }

    pub fn load() -> Look {
        let v: serde_json::Value = std::fs::read_to_string(Self::path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        Look::from_json(&v)
    }

    pub fn from_json(v: &serde_json::Value) -> Look {
        let d = Look::default();
        Look {
            anchor: v["anchor"].as_u64().map_or(d.anchor, |a| (a as usize).min(ANCHORS.len() - 1)),
            size: v["size"].as_f64().unwrap_or(d.size).clamp(0.1, 0.6),
            round: v["round"].as_f64().unwrap_or(d.round).clamp(0., 1.),
        }
    }

    pub fn to_json(self) -> serde_json::Value {
        serde_json::json!({ "anchor": self.anchor, "size": self.size, "round": self.round })
    }

    pub fn save(self) {
        let _ = std::fs::create_dir_all(dirs::config());
        let _ = std::fs::write(Self::path(), self.to_json().to_string());
    }
}

/// The camera's box inside an area (x, y, w, h): left, top and side.
pub fn place(anchor: usize, size_share: f64, (x, y, w, h): (f64, f64, f64, f64)) -> (f64, f64, f64) {
    let short = w.min(h);
    let side = (size_share * short).max(8.);
    let margin = short * 0.04;
    let (fx, fy) = ANCHORS[anchor.min(ANCHORS.len() - 1)];
    (x + margin + fx * (w - side - 2. * margin).max(0.), y + margin + fy * (h - side - 2. * margin).max(0.), side)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_camera_snaps_inside_the_area_at_a_margin() {
        let area = (100., 50., 1000., 500.);
        // Bottom right: the margin is 4% of the short side.
        let (x, y, side) = place(4, 0.2, area);
        assert_eq!(side, 100.);
        assert!((x + side - (1100. - 20.)).abs() < 1e-9 && (y + side - (550. - 20.)).abs() < 1e-9);
        // The top edge's middle.
        let (x, y, _) = place(1, 0.2, area);
        assert!((x + 50. - 600.).abs() < 1e-9 && (y - 70.).abs() < 1e-9);
    }
}
