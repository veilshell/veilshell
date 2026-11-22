//! Small, independently drawn vector-style glyphs for the touch shade.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use veilshell_core::Rgba;

use crate::{DrawList, ImageQuad, RgbaImage};

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

fn contains(glyph: Glyph, x: f32, y: f32) -> bool {
    let line = |x0: f32, y0: f32, x1: f32, y1: f32, r: f32| {
        let dx = x1 - x0;
        let dy = y1 - y0;
        let t = ((x - x0) * dx + (y - y0) * dy) / (dx * dx + dy * dy);
        let t = t.clamp(0.0, 1.0);
        (x - x0 - dx * t).hypot(y - y0 - dy * t) <= r
    };
    let circle = |cx: f32, cy: f32, r: f32| (x - cx).hypot(y - cy) <= r;
    let rect = |x0, y0, x1, y1| x >= x0 && x <= x1 && y >= y0 && y <= y1;
    let ring = |cx: f32, cy: f32, r: f32| ((x - cx).hypot(y - cy) - r).abs() <= 1.05;
    match glyph {
        Glyph::Wifi => {
            circle(12.0, 18.8, 1.5)
                || (y < 18.0
                    && (x - 12.0).abs() <= (20.0 - y) * 0.95
                    && [5.0, 9.0, 13.0].into_iter().any(|r| ring(12.0, 19.0, r)))
        }
        Glyph::Bluetooth => {
            line(12.0, 3.0, 12.0, 21.0, 1.0)
                || [
                    (7.0, 7.0, 16.5, 16.0),
                    (7.0, 17.0, 16.5, 8.0),
                    (12.0, 3.0, 16.5, 8.0),
                    (12.0, 21.0, 16.5, 16.0),
                ]
                .into_iter()
                .any(|(a, b, c, d)| line(a, b, c, d, 1.0))
        }
        Glyph::Cellular => (0..4).any(|i| {
            let left = 4.0 + i as f32 * 4.2;
            rect(left, 16.5 - i as f32 * 3.5, left + 2.4, 20.0)
        }),
        Glyph::Sun => {
            ring(12.0, 12.0, 4.6)
                || (0..8).any(|i| {
                    let (sin, cos) = (i as f32 * std::f32::consts::FRAC_PI_4).sin_cos();
                    line(
                        12.0 + 8.1 * cos,
                        12.0 + 8.1 * sin,
                        12.0 + 10.0 * cos,
                        12.0 + 10.0 * sin,
                        0.9,
                    )
                })
        }
        Glyph::Volume => {
            rect(3.5, 9.0, 7.0, 15.0)
                || ((7.0..=12.0).contains(&x) && (y - 12.0).abs() <= x - 4.0)
                || (x > 13.5 && (ring(11.5, 12.0, 6.1) || ring(11.5, 12.0, 9.4)))
        }
        Glyph::Rotate => {
            (ring(12.0, 12.0, 8.2) && (x < 14.5 || y > 11.0))
                || line(17.0, 3.5, 17.0, 8.0, 1.0)
                || line(13.0, 8.0, 17.0, 8.0, 1.0)
                || (rect(9.0, 8.0, 14.0, 16.0) && !rect(10.3, 9.3, 12.7, 14.7))
        }
        Glyph::Moon => circle(11.5, 12.5, 8.5) && !circle(15.6, 8.5, 7.9),
        Glyph::Desktop => {
            (rect(3.0, 4.0, 21.0, 17.0) && !rect(4.8, 5.8, 19.2, 15.2))
                || rect(10.8, 16.3, 13.2, 20.0)
                || rect(7.5, 19.0, 16.5, 20.7)
        }
        Glyph::Power => {
            (ring(12.0, 13.0, 7.4) && (y > 8.0 || (x - 12.0).abs() > 3.1))
                || line(12.0, 3.0, 12.0, 12.0, 1.1)
        }
        Glyph::Battery => {
            (rect(2.0, 7.0, 20.0, 17.0) && !rect(3.5, 8.5, 18.5, 15.5))
                || rect(20.0, 10.0, 22.0, 14.0)
                || rect(5.0, 10.0, 15.5, 14.0)
        }
        Glyph::Bell => {
            (circle(12.0, 10.0, 5.5) && y < 11.0)
                || rect(6.5, 9.5, 17.5, 15.5)
                || rect(5.0, 15.0, 19.0, 17.5)
                || (circle(12.0, 19.0, 2.0) && y > 18.5)
        }
        Glyph::Profile => {
            (circle(12.0, 12.0, 8.0) && !circle(12.0, 12.0, 6.2))
                || line(12.0, 13.0, 16.0, 8.8, 1.2)
        }
    }
}
