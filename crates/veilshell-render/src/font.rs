//! TrueType / OpenType font engine (`ab_glyph`) with crisp 5x7 ASCII bitmap HUD fallback.

use std::collections::HashMap;
use std::fs;
use std::process::Command;
use std::sync::{Arc, Mutex, OnceLock};

use ab_glyph::{point, Font, FontArc, GlyphId, PxScale, ScaleFont};

use crate::primitives::{TextAlign, TextLabel};

/// Bitmap fallback glyph width in base pixels.
pub const GLYPH_W: u32 = 5;
/// Bitmap fallback glyph height in base pixels.
pub const GLYPH_H: u32 = 7;
/// Bitmap fallback horizontal advance per character in base pixels.
pub const GLYPH_ADVANCE: u32 = 6;

const STATIC_FONT_CANDIDATES: &[&str] = &[
    "/usr/share/fonts/Adwaita/AdwaitaSans-Regular.ttf",
    "/usr/share/fonts/noto/NotoSans-Regular.ttf",
    "/usr/share/fonts/TTF/DejaVuSans.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/liberation/LiberationSans-Regular.ttf",
    "/usr/share/fonts/TTF/LiberationSans-Regular.ttf",
    "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
];

/// Rasterized grayscale coverage mask for a single scaled TrueType glyph.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RasterizedGlyph {
    pub width: u32,
    pub height: u32,
    pub offset_x: i32,
    pub offset_y: i32,
    pub alpha: Vec<u8>,
}

/// Positioned TrueType glyph ready for software compositing or GPU atlas upload.
#[derive(Clone, Debug, PartialEq)]
pub struct PositionedGlyph {
    pub ch: char,
    pub px_tenths: u16,
    pub x: i32,
    pub y: i32,
    pub glyph: Arc<RasterizedGlyph>,
}

/// Resolved glyph layout for a [`TextLabel`] (`TrueType` when a system font exists, else 5x7 fallback).
#[derive(Clone, Debug, PartialEq)]
pub enum TextLayout {
    TrueType(Vec<PositionedGlyph>),
    Bitmap5x7,
}

struct SystemFont {
    font: FontArc,
    cache: Mutex<HashMap<(char, u16), Arc<RasterizedGlyph>>>,
}

static SYSTEM_FONT: OnceLock<Option<SystemFont>> = OnceLock::new();

fn try_load_font_path(path: &str) -> Option<FontArc> {
    let bytes = fs::read(path.trim()).ok()?;
    FontArc::try_from_vec(bytes).ok()
}

fn discover_system_font() -> Option<SystemFont> {
    let font = std::env::var("VEILSHELL_FONT")
        .ok()
        .and_then(|p| try_load_font_path(&p))
        .or_else(|| {
            STATIC_FONT_CANDIDATES
                .iter()
                .find_map(|p| try_load_font_path(p))
        })
        .or_else(|| {
            let output = Command::new("fc-match")
                .args(["-f", "%{file}", "sans-serif"])
                .output()
                .ok()?;
            if !output.status.success() {
                return None;
            }
            let path = String::from_utf8_lossy(&output.stdout);
            try_load_font_path(&path)
        })?;

    Some(SystemFont {
        font,
        cache: Mutex::new(HashMap::new()),
    })
}

fn system_font() -> Option<&'static SystemFont> {
    SYSTEM_FONT.get_or_init(discover_system_font).as_ref()
}

/// Return `true` if a system TrueType / OpenType sans-serif font was discovered and loaded.
#[must_use]
pub fn has_system_font() -> bool {
    system_font().is_some()
}

fn scale_to_px(scale: u32) -> f32 {
    match scale.max(1) {
        1 => 13.5,
        2 => 18.5,
        s => s as f32 * 9.5,
    }
}

fn rasterize_cached(
    sys: &SystemFont,
    ch: char,
    px_size: f32,
    px_tenths: u16,
) -> Arc<RasterizedGlyph> {
    if let Ok(guard) = sys.cache.lock() {
        if let Some(cached) = guard.get(&(ch, px_tenths)) {
            return Arc::clone(cached);
        }
    }

    let id = sys.font.glyph_id(ch);
    let glyph = id.with_scale_and_position(PxScale::from(px_size), point(0.0, 0.0));
    let built = if let Some(outlined) = sys.font.outline_glyph(glyph) {
        let bb = outlined.px_bounds();
        let w = (bb.width().max(0.0).ceil() as u32).max(1);
        let h = (bb.height().max(0.0).ceil() as u32).max(1);
        let mut alpha = vec![0_u8; (w as usize) * (h as usize)];
        outlined.draw(|gx, gy, cov| {
            if gx < w && gy < h {
                let idx = (gy as usize) * (w as usize) + (gx as usize);
                alpha[idx] = (cov.clamp(0.0, 1.0) * 255.0).round() as u8;
            }
        });
        Arc::new(RasterizedGlyph {
            width: w,
            height: h,
            offset_x: bb.min.x.floor() as i32,
            offset_y: bb.min.y.floor() as i32,
            alpha,
        })
    } else {
        Arc::new(RasterizedGlyph {
            width: 0,
            height: 0,
            offset_x: 0,
            offset_y: 0,
            alpha: Vec::new(),
        })
    };

    if let Ok(mut guard) = sys.cache.lock() {
        guard.insert((ch, px_tenths), Arc::clone(&built));
    }
    built
}

/// Lay out a [`TextLabel`] into positioned TrueType glyphs (or report `Bitmap5x7` fallback).
#[must_use]
pub fn layout_text_glyphs(label: &TextLabel) -> TextLayout {
    let Some(sys) = system_font() else {
        return TextLayout::Bitmap5x7;
    };
    if label.text.is_empty() {
        return TextLayout::TrueType(Vec::new());
    }

    let scale = label.scale.max(1);
    let px_size = scale_to_px(scale);
    let px_scale = PxScale::from(px_size);
    let px_tenths = (px_size * 10.0).round() as u16;
    let scaled = sys.font.as_scaled(px_scale);

    // Filter out missing glyphs (e.g. emoji in window titles) while preserving spaces.
    let chars: Vec<(char, GlyphId)> = label
        .text
        .chars()
        .filter_map(|ch| {
            let id = sys.font.glyph_id(ch);
            if id == GlyphId(0) && !ch.is_ascii_whitespace() {
                None
            } else {
                Some((ch, id))
            }
        })
        .collect();

    if chars.is_empty() {
        return TextLayout::TrueType(Vec::new());
    }

    let mut total_advance = 0.0_f32;
    let mut prev_id: Option<GlyphId> = None;
    for &(_, id) in &chars {
        if let Some(prev) = prev_id {
            total_advance += scaled.kern(prev, id);
        }
        total_advance += scaled.h_advance(id);
        prev_id = Some(id);
    }

    let start_x = match label.align {
        TextAlign::Left => label.x,
        TextAlign::Center => label.x - total_advance * 0.5,
        TextAlign::Right => label.x - total_advance,
    };

    let cap_h = sys
        .font
        .outline_glyph(
            sys.font
                .glyph_id('H')
                .with_scale_and_position(px_scale, point(0.0, 0.0)),
        )
        .map_or_else(|| scaled.ascent() * 0.72, |g| g.px_bounds().height());
    let baseline_y = label.y + f32::midpoint(f32::from(GLYPH_H as u8) * (scale as f32), cap_h);
    let baseline_i32 = baseline_y.round() as i32;

    let mut positioned = Vec::with_capacity(chars.len());
    let mut pen_x = start_x;
    let mut prev_id: Option<GlyphId> = None;
    for (ch, id) in chars {
        if let Some(prev) = prev_id {
            pen_x += scaled.kern(prev, id);
        }
        if !ch.is_whitespace() {
            let raster = rasterize_cached(sys, ch, px_size, px_tenths);
            if raster.width > 0 && raster.height > 0 {
                positioned.push(PositionedGlyph {
                    ch,
                    px_tenths,
                    x: (pen_x.round() as i32) + raster.offset_x,
                    y: baseline_i32 + raster.offset_y,
                    glyph: raster,
                });
            }
        }
        pen_x += scaled.h_advance(id);
        prev_id = Some(id);
    }

    TextLayout::TrueType(positioned)
}

/// Compute total pixel width of a single-line ASCII string at a given integer `scale`.
#[must_use]
pub fn measure_text_width(text: &str, scale: u32) -> u32 {
    let count = u32::try_from(text.chars().count()).unwrap_or(0);
    if count == 0 {
        return 0;
    }
    (count * GLYPH_ADVANCE - 1) * scale.max(1)
}

/// Fit a single line to the renderer's actual font advance, with a short ellipsis.
pub(crate) fn fit_text(text: &str, scale: u32, width: f32) -> String {
    let text = text.trim();
    let advance = |value: &str| {
        let Some(sys) = system_font() else {
            return measure_text_width(value, scale) as f32;
        };
        let scaled = sys.font.as_scaled(PxScale::from(scale_to_px(scale)));
        let mut sum = 0.0;
        let mut previous = None;
        for ch in value.chars() {
            let id = sys.font.glyph_id(ch);
            if id == GlyphId(0) && !ch.is_ascii_whitespace() {
                continue;
            }
            sum += previous.map_or(0.0, |prev| scaled.kern(prev, id)) + scaled.h_advance(id);
            previous = Some(id);
        }
        sum
    };
    if advance(text) <= width {
        return text.to_owned();
    }
    let mut shortened = String::from("..");
    if advance(&shortened) > width {
        return String::new();
    }
    for ch in text.chars() {
        let end = shortened.len() - 2;
        shortened.insert(end, ch);
        if advance(&shortened) > width {
            shortened.remove(end);
            break;
        }
    }
    shortened
}

/// Return 7 rows of 5-bit masks (MSB = leftmost pixel) for an ASCII character.
#[must_use]
#[allow(clippy::too_many_lines)]
pub const fn glyph_rows(ch: char) -> [u8; 7] {
    match ch.to_ascii_uppercase() {
        'A' => [0x0E, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'B' => [0x1E, 0x11, 0x11, 0x1E, 0x11, 0x11, 0x1E],
        'C' => [0x0E, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0E],
        'D' => [0x1C, 0x12, 0x11, 0x11, 0x11, 0x12, 0x1C],
        'E' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x1F],
        'F' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x10],
        'G' => [0x0E, 0x11, 0x10, 0x17, 0x11, 0x11, 0x0F],
        'H' => [0x11, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'I' => [0x0E, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0E],
        'J' => [0x07, 0x02, 0x02, 0x02, 0x12, 0x12, 0x0C],
        'K' => [0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11],
        'L' => [0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1F],
        'M' => [0x11, 0x1B, 0x15, 0x15, 0x11, 0x11, 0x11],
        'N' => [0x11, 0x19, 0x15, 0x13, 0x11, 0x11, 0x11],
        'O' => [0x0E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'P' => [0x1E, 0x11, 0x11, 0x1E, 0x10, 0x10, 0x10],
        'Q' => [0x0E, 0x11, 0x11, 0x11, 0x15, 0x12, 0x0D],
        'R' => [0x1E, 0x11, 0x11, 0x1E, 0x14, 0x12, 0x11],
        'S' => [0x0F, 0x10, 0x10, 0x0E, 0x01, 0x01, 0x1E],
        'T' => [0x1F, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
        'U' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'V' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x0A, 0x04],
        'W' => [0x11, 0x11, 0x11, 0x15, 0x15, 0x1B, 0x11],
        'X' => [0x11, 0x11, 0x0A, 0x04, 0x0A, 0x11, 0x11],
        'Y' => [0x11, 0x11, 0x0A, 0x04, 0x04, 0x04, 0x04],
        'Z' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x10, 0x1F],
        '0' => [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E],
        '1' => [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E],
        '2' => [0x0E, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1F],
        '3' => [0x1E, 0x01, 0x01, 0x0E, 0x01, 0x01, 0x1E],
        '4' => [0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02],
        '5' => [0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E],
        '6' => [0x06, 0x08, 0x10, 0x1E, 0x11, 0x11, 0x0E],
        '7' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
        '8' => [0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E],
        '9' => [0x0E, 0x11, 0x11, 0x0F, 0x01, 0x02, 0x0C],
        ':' => [0x00, 0x04, 0x04, 0x00, 0x04, 0x04, 0x00],
        '.' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x0C, 0x0C],
        ',' => [0x00, 0x00, 0x00, 0x00, 0x04, 0x04, 0x08],
        '-' => [0x00, 0x00, 0x00, 0x1F, 0x00, 0x00, 0x00],
        '+' => [0x00, 0x04, 0x04, 0x1F, 0x04, 0x04, 0x00],
        '/' => [0x01, 0x01, 0x02, 0x04, 0x08, 0x10, 0x10],
        '%' => [0x18, 0x19, 0x02, 0x04, 0x08, 0x13, 0x03],
        '(' => [0x02, 0x04, 0x08, 0x08, 0x08, 0x04, 0x02],
        ')' => [0x08, 0x04, 0x02, 0x02, 0x02, 0x04, 0x08],
        '!' => [0x04, 0x04, 0x04, 0x04, 0x04, 0x00, 0x04],
        '?' => [0x0E, 0x11, 0x01, 0x02, 0x04, 0x00, 0x04],
        '_' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x1F],
        '@' => [0x0E, 0x11, 0x17, 0x15, 0x17, 0x10, 0x0E],
        '#' => [0x0A, 0x0A, 0x1F, 0x0A, 0x1F, 0x0A, 0x0A],
        '$' => [0x04, 0x0F, 0x14, 0x0E, 0x05, 0x1E, 0x04],
        '&' => [0x0C, 0x12, 0x14, 0x08, 0x15, 0x12, 0x0D],
        '*' => [0x00, 0x15, 0x0E, 0x1F, 0x0E, 0x15, 0x00],
        '=' => [0x00, 0x00, 0x1F, 0x00, 0x1F, 0x00, 0x00],
        '\'' => [0x04, 0x04, 0x08, 0x00, 0x00, 0x00, 0x00],
        '"' => [0x0A, 0x0A, 0x14, 0x00, 0x00, 0x00, 0x00],
        ';' => [0x00, 0x04, 0x04, 0x00, 0x04, 0x04, 0x08],
        '<' => [0x02, 0x04, 0x08, 0x10, 0x08, 0x04, 0x02],
        '>' => [0x08, 0x04, 0x02, 0x01, 0x02, 0x04, 0x08],
        '[' => [0x0E, 0x08, 0x08, 0x08, 0x08, 0x08, 0x0E],
        ']' => [0x0E, 0x02, 0x02, 0x02, 0x02, 0x02, 0x0E],
        '{' => [0x06, 0x08, 0x08, 0x10, 0x08, 0x08, 0x06],
        '}' => [0x0C, 0x02, 0x02, 0x01, 0x02, 0x02, 0x0C],
        '\\' => [0x10, 0x10, 0x08, 0x04, 0x02, 0x01, 0x01],
        '|' => [0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
        '~' => [0x00, 0x00, 0x08, 0x15, 0x02, 0x00, 0x00],
        '`' => [0x08, 0x04, 0x02, 0x00, 0x00, 0x00, 0x00],
        '^' => [0x04, 0x0A, 0x11, 0x00, 0x00, 0x00, 0x00],
        _ => [0x00; 7],
    }
}
