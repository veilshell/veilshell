//! Fast analytic SDF CPU rasterizer and `PNG` / `Wayland` `ARGB8888` encoders.

use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

use veilshell_core::{Rgba, ThemePalette};

use crate::font::{glyph_rows, measure_text_width, GLYPH_ADVANCE, GLYPH_H, GLYPH_W};
use crate::primitives::{DrawCommand, DrawList, RoundedRect, TextAlign, TextLabel};
use crate::scene::{build_scene, SceneContext};
use crate::{RenderError, RenderFrame, RendererBackend, SnapshotSpec};

/// Analytic SDF rounded-rectangle rasterizer (zero `unsafe`, crisp antialiased edges).
#[derive(Clone, Debug, Default)]
pub struct SdfSoftwareRenderer {
    pub context: SceneContext,
}

impl SdfSoftwareRenderer {
    /// Construct a software SDF renderer with custom dynamic [`SceneContext`].
    #[must_use]
    pub const fn with_context(context: SceneContext) -> Self {
        Self { context }
    }

    /// Rasterize an arbitrary [`DrawList`] into an `RGBA8` [`RenderFrame`].
    ///
    /// # Errors
    /// Returns [`RenderError::InvalidViewport`] if `width` or `height` is zero.
    pub fn rasterize(
        &self,
        width: u32,
        height: u32,
        list: &DrawList,
    ) -> Result<RenderFrame, RenderError> {
        if width == 0 || height == 0 {
            return Err(RenderError::InvalidViewport);
        }
        let mut rgba = vec![0_u8; (width as usize) * (height as usize) * 4];
        for cmd in &list.commands {
            match cmd {
                DrawCommand::BackdropGradient { top, bottom } => {
                    draw_backdrop_gradient(&mut rgba, width, height, *top, *bottom);
                }
                DrawCommand::RadialGlow {
                    cx,
                    cy,
                    radius,
                    color,
                } => draw_radial_glow(&mut rgba, width, height, *cx, *cy, *radius, *color),
                DrawCommand::Rect(rect) => draw_rounded_rect(&mut rgba, width, height, *rect),
                DrawCommand::Text(label) => draw_text(&mut rgba, width, height, label),
            }
        }
        Ok(RenderFrame {
            width,
            height,
            rgba,
        })
    }
}

impl RendererBackend for SdfSoftwareRenderer {
    fn name(&self) -> &'static str {
        "sdf-software"
    }

    fn render_snapshot(
        &mut self,
        spec: &SnapshotSpec,
        palette: &ThemePalette,
    ) -> Result<RenderFrame, RenderError> {
        let (w, h) = spec.viewport.physical_pixels();
        if w == 0 || h == 0 || !spec.viewport.scale.is_finite() || spec.viewport.scale <= 0.0 {
            return Err(RenderError::InvalidViewport);
        }
        let list = build_scene(spec.scene, w as f32, h as f32, palette, &self.context);
        self.rasterize(w, h, &list)
    }
}

impl RenderFrame {
    /// Save this `RGBA8` frame to a `PNG` file on disk.
    ///
    /// # Errors
    /// Returns [`RenderError::Io`] or [`RenderError::Encode`] if writing the `PNG` fails.
    pub fn save_png(&self, path: impl AsRef<Path>) -> Result<(), RenderError> {
        let file = File::create(path)?;
        let writer = BufWriter::new(file);
        let mut encoder = png::Encoder::new(writer, self.width, self.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut png_writer = encoder
            .write_header()
            .map_err(|e| RenderError::Encode(e.to_string()))?;
        png_writer
            .write_image_data(&self.rgba)
            .map_err(|e| RenderError::Encode(e.to_string()))?;
        Ok(())
    }

    /// Convert straight `RGBA8` pixels into premultiplied `BGRA8` (`wl_shm` `Format::Argb8888` on little-endian).
    #[must_use]
    pub fn to_argb8888_premultiplied(&self) -> Vec<u8> {
        let mut out = vec![0_u8; self.rgba.len()];
        for (src, dst) in self.rgba.chunks_exact(4).zip(out.chunks_exact_mut(4)) {
            let a = u16::from(src[3]);
            let r = ((u16::from(src[0]) * a + 127) / 255) as u8;
            let g = ((u16::from(src[1]) * a + 127) / 255) as u8;
            let b = ((u16::from(src[2]) * a + 127) / 255) as u8;
            dst[0] = b;
            dst[1] = g;
            dst[2] = r;
            dst[3] = src[3];
        }
        out
    }
}

fn blend_pixel(buf: &mut [u8], idx: usize, src: Rgba) {
    let sa = src.a.clamp(0.0, 1.0);
    if sa <= 0.001 {
        return;
    }
    let sr = src.r.clamp(0.0, 1.0);
    let sg = src.g.clamp(0.0, 1.0);
    let sb = src.b.clamp(0.0, 1.0);

    let da = f32::from(buf[idx + 3]) / 255.0;
    let out_a = sa + da * (1.0 - sa);
    if out_a <= 0.0001 {
        buf[idx..idx + 4].fill(0);
        return;
    }

    let dr = f32::from(buf[idx]) / 255.0;
    let dg = f32::from(buf[idx + 1]) / 255.0;
    let db = f32::from(buf[idx + 2]) / 255.0;

    let inv = 1.0 - sa;
    let or = (sr * sa + dr * da * inv) / out_a;
    let og = (sg * sa + dg * da * inv) / out_a;
    let ob = (sb * sa + db * da * inv) / out_a;

    buf[idx] = (or.clamp(0.0, 1.0) * 255.0).round() as u8;
    buf[idx + 1] = (og.clamp(0.0, 1.0) * 255.0).round() as u8;
    buf[idx + 2] = (ob.clamp(0.0, 1.0) * 255.0).round() as u8;
    buf[idx + 3] = (out_a.clamp(0.0, 1.0) * 255.0).round() as u8;
}

fn draw_backdrop_gradient(buf: &mut [u8], width: u32, height: u32, top: Rgba, bottom: Rgba) {
    let denom = (height.saturating_sub(1)).max(1) as f32;
    for y in 0..height {
        let frac = y as f32 / denom;
        let row_col = Rgba {
            r: top.r + (bottom.r - top.r) * frac,
            g: top.g + (bottom.g - top.g) * frac,
            b: top.b + (bottom.b - top.b) * frac,
            a: top.a + (bottom.a - top.a) * frac,
        };
        let red = (row_col.r.clamp(0.0, 1.0) * 255.0).round() as u8;
        let green = (row_col.g.clamp(0.0, 1.0) * 255.0).round() as u8;
        let blue = (row_col.b.clamp(0.0, 1.0) * 255.0).round() as u8;
        let alpha = (row_col.a.clamp(0.0, 1.0) * 255.0).round() as u8;
        let row_start = (y as usize) * (width as usize) * 4;
        for px in buf[row_start..row_start + (width as usize) * 4].chunks_exact_mut(4) {
            px[0] = red;
            px[1] = green;
            px[2] = blue;
            px[3] = alpha;
        }
    }
}

fn draw_radial_glow(buf: &mut [u8], w: u32, h: u32, cx: f32, cy: f32, radius: f32, color: Rgba) {
    if radius <= 1.0 || color.a <= 0.001 {
        return;
    }
    let min_x = ((cx - radius).floor().max(0.0) as u32).min(w);
    let max_x = ((cx + radius).ceil().max(0.0) as u32).min(w);
    let min_y = ((cy - radius).floor().max(0.0) as u32).min(h);
    let max_y = ((cy + radius).ceil().max(0.0) as u32).min(h);
    let inv_r2 = 1.0 / (radius * radius);

    for y in min_y..max_y {
        let dy = y as f32 + 0.5 - cy;
        let dy2 = dy * dy;
        let row_off = (y as usize) * (w as usize);
        for x in min_x..max_x {
            let dx = x as f32 + 0.5 - cx;
            let dist2 = (dx * dx + dy2) * inv_r2;
            if dist2 >= 1.0 {
                continue;
            }
            let falloff = (1.0 - dist2) * (1.0 - dist2);
            let idx = (row_off + x as usize) * 4;
            blend_pixel(buf, idx, color.with_alpha(color.a * falloff));
        }
    }
}

fn sdf_rounded_box(px: f32, py: f32, half_w: f32, half_h: f32, r: f32) -> f32 {
    let qx = px.abs() - (half_w - r);
    let qy = py.abs() - (half_h - r);
    let outside = (qx.max(0.0).hypot(qy.max(0.0))) - r;
    let inside = qx.max(qy).min(0.0);
    outside + inside
}

fn draw_rounded_rect(buf: &mut [u8], w: u32, h: u32, rect: RoundedRect) {
    if rect.w <= 0.0 || rect.h <= 0.0 {
        return;
    }
    let half_w = rect.w * 0.5;
    let half_h = rect.h * 0.5;
    let cx = rect.x + half_w;
    let cy = rect.y + half_h;
    let r = rect.radius.min(half_w).min(half_h).max(0.0);

    let pad = rect.shadow_radius + rect.shadow_offset_y.abs() + 2.0;
    let min_x = ((rect.x - pad).floor().max(0.0) as u32).min(w);
    let max_x = ((rect.x + rect.w + pad).ceil().max(0.0) as u32).min(w);
    let min_y = ((rect.y - pad).floor().max(0.0) as u32).min(h);
    let max_y = ((rect.y + rect.h + pad).ceil().max(0.0) as u32).min(h);

    for y in min_y..max_y {
        let py = y as f32 + 0.5;
        let rel_y = py - cy;
        let row_off = (y as usize) * (w as usize);
        for x in min_x..max_x {
            let px = x as f32 + 0.5;
            let rel_x = px - cx;
            let idx = (row_off + x as usize) * 4;

            let dist = sdf_rounded_box(rel_x, rel_y, half_w, half_h, r);

            // Soft drop shadow outside the shape
            if rect.shadow_alpha > 0.001 && rect.shadow_radius > 0.5 && dist > -1.0 {
                let sdist = sdf_rounded_box(rel_x, rel_y - rect.shadow_offset_y, half_w, half_h, r);
                if sdist < rect.shadow_radius {
                    let norm = (sdist.max(0.0) / rect.shadow_radius).clamp(0.0, 1.0);
                    let falloff = (1.0 - norm) * (1.0 - norm);
                    let shadow_cov = (dist + 0.5).clamp(0.0, 1.0);
                    blend_pixel(
                        buf,
                        idx,
                        Rgba {
                            r: 0.0,
                            g: 0.0,
                            b: 0.0,
                            a: rect.shadow_alpha * falloff * shadow_cov,
                        },
                    );
                }
            }

            // Main antialiased fill
            let fill_cov = (0.5 - dist).clamp(0.0, 1.0);
            if fill_cov > 0.001 {
                blend_pixel(buf, idx, rect.fill.with_alpha(rect.fill.a * fill_cov));

                // Subtle vertical top specular glass sheen
                if rect.top_highlight_alpha > 0.001 {
                    let norm_y = ((py - rect.y) / rect.h).clamp(0.0, 1.0);
                    if norm_y < 0.35 {
                        let sheen = (1.0 - norm_y / 0.35) * rect.top_highlight_alpha * fill_cov;
                        blend_pixel(
                            buf,
                            idx,
                            Rgba {
                                r: 1.0,
                                g: 1.0,
                                b: 1.0,
                                a: sheen,
                            },
                        );
                    }
                }

                // Crisp inner glass border
                if rect.border_width > 0.01 && rect.border_color.a > 0.001 {
                    let inner_cov = (dist + rect.border_width + 0.5).clamp(0.0, 1.0);
                    let border_cov = fill_cov * inner_cov;
                    if border_cov > 0.001 {
                        blend_pixel(
                            buf,
                            idx,
                            rect.border_color
                                .with_alpha(rect.border_color.a * border_cov),
                        );
                    }
                }
            }
        }
    }
}

fn draw_text(buf: &mut [u8], width: u32, height: u32, label: &TextLabel) {
    let scale = label.scale.max(1);
    let total_w = measure_text_width(&label.text, scale) as f32;
    let start_x = match label.align {
        TextAlign::Left => label.x,
        TextAlign::Center => label.x - total_w * 0.5,
        TextAlign::Right => label.x - total_w,
    }
    .round() as i64;
    let start_y = label.y.round() as i64;
    let width_i64 = i64::from(width);
    let height_i64 = i64::from(height);
    let advance_i64 = i64::from(GLYPH_ADVANCE * scale);

    for (ch_idx, ch) in label.text.chars().enumerate() {
        let rows = glyph_rows(ch);
        let Ok(ch_off) = i64::try_from(ch_idx) else {
            break;
        };
        let gx = start_x + ch_off * advance_i64;
        for (ry, mask) in rows.iter().enumerate().take(GLYPH_H as usize) {
            for rx in 0..GLYPH_W {
                if (mask & (1 << (GLYPH_W - 1 - rx))) == 0 {
                    continue;
                }
                for sy in 0..scale {
                    let py = start_y + i64::from(ry as u32 * scale + sy);
                    if py < 0 || py >= height_i64 {
                        continue;
                    }
                    let row_off = (py as usize) * (width as usize);
                    for sx in 0..scale {
                        let px = gx + i64::from(rx * scale + sx);
                        if px < 0 || px >= width_i64 {
                            continue;
                        }
                        let idx = (row_off + px as usize) * 4;
                        blend_pixel(buf, idx, label.color);
                    }
                }
            }
        }
    }
}
