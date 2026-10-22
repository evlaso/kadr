/// A colour as plain 0..1 sRGB with straight (not premultiplied) alpha.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Rgba {
    pub const WHITE: Rgba = Rgba { r: 1., g: 1., b: 1., a: 1. };

    pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Rgba {
        Rgba { r, g, b, a }
    }

    /// `0xRRGGBB`, opaque.
    pub fn hex(hex: u32) -> Rgba {
        let c = |shift: u32| ((hex >> shift) & 0xFF) as f32 / 255.;
        Rgba { r: c(16), g: c(8), b: c(0), a: 1. }
    }

    /// Hue, saturation and lightness (all 0..1), the way GPUI turns its
    /// `Hsla` into RGB.
    pub fn hsl(h: f32, s: f32, l: f32, a: f32) -> Rgba {
        let c = (1. - (2. * l - 1.).abs()) * s;
        let h6 = (h * 6.).rem_euclid(6.);
        let x = c * (1. - (h6 % 2. - 1.).abs());
        let m = l - c / 2.;
        let (r, g, b) = match h6 as u32 {
            0 => (c, x, 0.),
            1 => (x, c, 0.),
            2 => (0., c, x),
            3 => (0., x, c),
            4 => (x, 0., c),
            _ => (c, 0., x),
        };
        Rgba { r: r + m, g: g + m, b: b + m, a }
    }

    /// HSL lightness: the mean of the brightest and darkest channel.
    pub fn lightness(self) -> f32 {
        (self.r.max(self.g).max(self.b) + self.r.min(self.g).min(self.b)) / 2.
    }

    pub fn with_alpha(self, a: f32) -> Rgba {
        Rgba { a, ..self }
    }
}
