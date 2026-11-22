//! Small, independently drawn vector-style glyphs for the touch shade.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use veilshell_core::Rgba;

use crate::{DrawList, ImageQuad, RgbaImage, RoundedRect};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum Glyph {
    Wifi,
    Bluetooth,
    Cellular,
    Sun,
    Volume,
    Rotate,
    Moon,
    Desktop,
    Keyboard,
    Power,
    Battery,
    Bell,
    Profile,
}

type Cache = HashMap<(Glyph, [u8; 3]), Arc<RgbaImage>>;
static GLYPHS: OnceLock<Mutex<Cache>> = OnceLock::new();

pub(crate) fn draw(list: &mut DrawList, glyph: Glyph, cx: f32, cy: f32, size: f32, ink: Rgba) {
    let rgb = [ink.r, ink.g, ink.b].map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8);
    let mut cache = GLYPHS
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if cache.len() > 64 {
        cache.clear();
    }
    let image = Arc::clone(
        cache
            .entry((glyph, rgb))
            .or_insert_with(|| rasterize(glyph, rgb)),
    );
    list.push_image(ImageQuad {
        x: cx - size * 0.5,
        y: cy - size * 0.5,
        w: size,
        h: size,
        radius: 0.0,
        alpha: ink.a,
        image,
    });
}

pub(crate) fn draw_battery(list: &mut DrawList, cx: f32, cy: f32, size: f32, pct: u8, ink: Rgba) {
    draw(list, Glyph::Battery, cx, cy, size, ink);
    // Keep the outline cached even as the battery level changes.
    let scale = size / 24.0;
    let width = 12.0 * scale * f32::from(pct.min(100)) / 100.0;
    if width > 0.0 {
        list.push_rect(RoundedRect::filled(
            cx - 7.0 * scale,
            cy - 2.0 * scale,
            width,
            4.0 * scale,
            (0.75 * scale).min(width * 0.5),
            ink,
        ));
    }
}

fn rasterize(glyph: Glyph, rgb: [u8; 3]) -> Arc<RgbaImage> {
    const SIDE: u32 = 48;
    let mut rgba = Vec::with_capacity((SIDE * SIDE * 4) as usize);
    for y in 0..SIDE {
        for x in 0..SIDE {
            let mut hits = 0_u8;
            for dy in [0.125, 0.375] {
                for dx in [0.125, 0.375] {
                    hits += u8::from(contains(glyph, x as f32 * 0.5 + dx, y as f32 * 0.5 + dy));
                }
            }
            rgba.extend_from_slice(&[rgb[0], rgb[1], rgb[2], (u16::from(hits) * 255 / 4) as u8]);
        }
    }
    Arc::new(RgbaImage::new(SIDE, SIDE, rgba).expect("fixed glyph dimensions"))
}

const NEAR_DIAGONAL: f32 = 8.0 * std::f32::consts::FRAC_1_SQRT_2;
const FAR_DIAGONAL: f32 = 9.7 * std::f32::consts::FRAC_1_SQRT_2;
const SUN_RAYS: [(f32, f32, f32, f32); 4] = [
    (0.0, 8.0, 0.0, 9.7),
    (8.0, 0.0, 9.7, 0.0),
    (NEAR_DIAGONAL, NEAR_DIAGONAL, FAR_DIAGONAL, FAR_DIAGONAL),
    (NEAR_DIAGONAL, -NEAR_DIAGONAL, FAR_DIAGONAL, -FAR_DIAGONAL),
];

fn keyboard(x: f32, y: f32) -> bool {
    let in_row = |top, bottom| (top..=bottom).contains(&y);
    (2.0..=22.0).contains(&x)
        && in_row(5.0, 19.0)
        && (!(4.0..=20.0).contains(&x)
            || !in_row(7.0, 17.0)
            || (in_row(9.0, 11.4) && [7.0_f32, 12.0, 17.0].iter().any(|cx| (x - cx).abs() <= 1.2))
            || (in_row(13.5, 15.4) && (7.0..=17.0).contains(&x)))
}

fn contains(glyph: Glyph, x: f32, y: f32) -> bool {
    const STROKE: f32 = 1.05;
    let distance_sq = |cx: f32, cy: f32| (x - cx) * (x - cx) + (y - cy) * (y - cy);
    let circle = |cx: f32, cy: f32, r: f32| distance_sq(cx, cy) <= r * r;
    let line = |x0: f32, y0: f32, x1: f32, y1: f32| {
        let dx = x1 - x0;
        let dy = y1 - y0;
        let t = ((x - x0) * dx + (y - y0) * dy) / (dx * dx + dy * dy);
        let t = t.clamp(0.0, 1.0);
        circle(x0 + dx * t, y0 + dy * t, STROKE)
    };
    let ring = |cx: f32, cy: f32, r: f32| {
        distance_sq(cx, cy) >= (r - STROKE) * (r - STROKE) && circle(cx, cy, r + STROKE)
    };
    let rounded = |left: f32, top: f32, right: f32, bottom: f32, r: f32| {
        circle(
            x.clamp(left + r, right - r),
            y.clamp(top + r, bottom - r),
            r,
        )
    };
    let rect = |x0, y0, x1, y1| x >= x0 && x <= x1 && y >= y0 && y <= y1;
    match glyph {
        Glyph::Wifi => {
            circle(12.0, 19.0, 1.45)
                || [5.0, 9.0, 13.0].into_iter().any(|r| {
                    let dx = (x - 12.0).abs();
                    (dx <= (19.0 - y) * 0.89 && ring(12.0, 19.0, r))
                        || circle(12.0 + r * 0.664, 19.0 - r * 0.747, STROKE)
                        || circle(12.0 - r * 0.664, 19.0 - r * 0.747, STROKE)
                })
        }
        Glyph::Bluetooth => {
            line(12.0, 3.0, 12.0, 21.0)
                || [
                    (7.0, 7.0, 16.5, 16.0),
                    (7.0, 17.0, 16.5, 8.0),
                    (12.0, 3.0, 16.5, 8.0),
                    (12.0, 21.0, 16.5, 16.0),
                ]
                .into_iter()
                .any(|(a, b, c, d)| line(a, b, c, d))
        }
        Glyph::Cellular => (0..4).any(|i| {
            let left = 3.0 + i as f32 * 4.9;
            rounded(left, 16.0 - i as f32 * 3.5, left + 2.7, 20.5, 0.55)
        }),
        Glyph::Sun => {
            ring(12.0, 12.0, 4.6)
                || SUN_RAYS.into_iter().any(|(x0, y0, x1, y1)| {
                    line(12.0 + x0, 12.0 + y0, 12.0 + x1, 12.0 + y1)
                        || line(12.0 - x0, 12.0 - y0, 12.0 - x1, 12.0 - y1)
                })
        }
        Glyph::Volume => {
            rounded(3.0, 9.0, 7.5, 15.0, 0.75)
                || ((6.5..=11.5).contains(&x) && (y - 12.0).abs() <= 3.0 + (x - 6.5) * 0.65)
                || (x - 11.5 >= (y - 12.0).abs() * 0.90
                    && (ring(11.5, 12.0, 5.5) || ring(11.5, 12.0, 9.0)))
        }
        Glyph::Rotate => {
            (ring(12.0, 12.0, 8.5) && (x < 17.7 || y >= 8.0))
                || line(18.5, 3.8, 18.5, 8.0)
                || line(14.7, 8.0, 18.5, 8.0)
                || (rounded(8.0, 7.6, 15.0, 17.0, 1.3) && !rect(9.5, 9.2, 13.5, 15.4))
        }
        Glyph::Moon => circle(11.5, 12.5, 8.5) && !circle(15.6, 8.5, 7.9),
        Glyph::Desktop => {
            (rounded(2.5, 3.8, 21.5, 16.8, 1.5) && !rect(4.5, 5.8, 19.5, 14.8))
                || line(12.0, 16.0, 12.0, 20.0)
                || line(8.0, 20.0, 16.0, 20.0)
        }
        Glyph::Keyboard => keyboard(x, y),
        Glyph::Power => {
            (ring(12.0, 13.0, 7.7) && y >= 7.1)
                || circle(7.0, 7.1, STROKE)
                || circle(17.0, 7.1, STROKE)
                || line(12.0, 3.2, 12.0, 12.0)
        }
        Glyph::Battery => {
            (rounded(2.0, 7.0, 20.0, 17.0, 2.0) && !rounded(4.0, 9.0, 18.0, 15.0, 0.75))
                || rounded(19.5, 10.0, 22.4, 14.0, 0.7)
        }
        Glyph::Bell => {
            (ring(12.0, 9.5, 5.0) && y <= 9.5)
                || line(7.0, 9.5, 7.0, 13.0)
                || line(17.0, 9.5, 17.0, 13.0)
                || line(7.0, 13.0, 5.0, 16.5)
                || line(17.0, 13.0, 19.0, 16.5)
                || line(5.0, 16.5, 19.0, 16.5)
                || (circle(12.0, 19.0, 1.9) && y >= 18.5)
        }
        Glyph::Profile => {
            (ring(12.0, 12.0, 8.1) && y <= 18.0)
                || line(7.0, 18.0, 17.0, 18.0)
                || line(12.0, 13.4, 15.8, 9.1)
                || circle(12.0, 13.4, 1.55)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DrawCommand;

    #[test]
    fn shade_battery_caches_the_outline_independently_of_charge() {
        let mut list = DrawList::default();
        let ink = Rgba::from_rgb_u8(255, 255, 255);
        for pct in [0, 50, 100] {
            draw_battery(&mut list, 12.0, 12.0, 24.0, pct, ink);
        }
        let [DrawCommand::Image(empty), DrawCommand::Image(half), DrawCommand::Rect(half_fill), DrawCommand::Image(full), DrawCommand::Rect(full_fill)] =
            list.commands.as_slice()
        else {
            panic!("one outline per glyph, with fill only for nonzero charge");
        };
        assert!(Arc::ptr_eq(&empty.image, &half.image));
        assert!(Arc::ptr_eq(&empty.image, &full.image));
        assert!((half_fill.w * 2.0 - full_fill.w).abs() < f32::EPSILON);
    }
}
