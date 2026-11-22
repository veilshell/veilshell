//! Color primitives, generic theme palette, and WCAG contrast validation.

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Rgba {
    #[must_use]
    pub const fn from_rgb_u8(r: u8, g: u8, b: u8) -> Self {
        Self {
            r: r as f32 / 255.0,
            g: g as f32 / 255.0,
            b: b as f32 / 255.0,
            a: 1.0,
        }
    }

    #[must_use]
    pub fn from_hex(hex: &str) -> Option<Self> {
        let s = hex.trim().trim_start_matches('#');
        if s.len() != 6 {
            return None;
        }
        let r = u8::from_str_radix(&s[0..2], 16).ok()?;
        let g = u8::from_str_radix(&s[2..4], 16).ok()?;
        let b = u8::from_str_radix(&s[4..6], 16).ok()?;
        Some(Self::from_rgb_u8(r, g, b))
    }

    #[must_use]
    pub const fn with_alpha(self, a: f32) -> Self {
        Self { a, ..self }
    }

    #[must_use]
    pub fn relative_luminance(self) -> f32 {
        fn linear(c: f32) -> f32 {
            if c <= 0.03928 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        }
        0.2126 * linear(self.r) + 0.7152 * linear(self.g) + 0.0722 * linear(self.b)
    }

    #[must_use]
    pub fn contrast_ratio(self, other: Self) -> f32 {
        let l1 = self.relative_luminance();
        let l2 = other.relative_luminance();
        let (hi, lo) = if l1 >= l2 { (l1, l2) } else { (l2, l1) };
        (hi + 0.05) / (lo + 0.05)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ThemePalette {
    pub name: String,
    pub background: Rgba,
    pub surface: Rgba,
    pub foreground: Rgba,
    pub muted: Rgba,
    pub accent: Rgba,
    pub error: Rgba,
}

impl ThemePalette {
    #[must_use]
    pub fn default_dark() -> Self {
        Self {
            name: "veilshell-dark".into(),
            background: Rgba::from_rgb_u8(15, 17, 21),
            surface: Rgba::from_rgb_u8(20, 23, 29).with_alpha(0.75),
            foreground: Rgba::from_rgb_u8(244, 239, 244),
            muted: Rgba::from_rgb_u8(202, 196, 208),
            accent: Rgba::from_rgb_u8(208, 188, 255),
            error: Rgba::from_rgb_u8(255, 107, 107),
        }
    }

    #[must_use]
    pub fn meets_wcag_aa(&self) -> bool {
        self.foreground.contrast_ratio(self.background) >= 4.5
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_dark_palette_meets_wcag_aa() {
        let p = ThemePalette::default_dark();
        assert!(p.meets_wcag_aa());
        assert!(p.foreground.contrast_ratio(p.background) > 12.0);
    }

    #[test]
    fn hex_parsing_works() {
        let c = Rgba::from_hex("#9c79db").expect("valid hex");
        assert!((c.r - 156.0 / 255.0).abs() < 1e-4);
        assert!(Rgba::from_hex("bad").is_none());
    }
}
