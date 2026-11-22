//! Fast analytic SDF CPU rasterizer, cached backdrop, and `PNG` / `Wayland` `ARGB8888` encoders.

use std::cell::RefCell;
use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

use veilshell_core::{Rgba, ThemePalette};

use crate::font::{
    glyph_rows, layout_text_glyphs, measure_text_width, TextLayout, GLYPH_ADVANCE, GLYPH_H, GLYPH_W,
};
use crate::primitives::{
    DrawCommand, DrawList, ImageQuad, RoundedRect, SceneOutput, TextAlign, TextLabel,
};
use crate::scene::{build_scene, build_scene_layout, build_top_strip_scene, SceneContext};
use crate::{RenderError, RenderFrame, RendererBackend, SceneKind, SnapshotSpec};

#[derive(Clone, Debug)]
struct BackdropCache {
    width: u32,
    height: u32,
    prefix: Vec<DrawCommand>,
    rgba: Vec<u8>,
}

/// Analytic SDF rounded-rectangle rasterizer (zero `unsafe`, crisp antialiased edges).
#[derive(Clone, Debug, Default)]
pub struct SdfSoftwareRenderer {
    pub context: SceneContext,
    backdrop_cache: RefCell<Option<BackdropCache>>,
}

impl SdfSoftwareRenderer {
    /// Construct a software SDF renderer with custom dynamic [`SceneContext`].
    #[must_use]
    pub const fn with_context(context: SceneContext) -> Self {
        Self {
            context,
            backdrop_cache: RefCell::new(None),
        }
    }

    /// Build layout (for hit-testing) and rasterize an interactive shell scene in one call.
    ///
    /// # Errors
    /// Returns [`RenderError::InvalidViewport`] if `width` or `height` is zero.
    pub fn render_interactive(
        &self,
        scene: SceneKind,
        width: u32,
        height: u32,
        palette: &ThemePalette,
    ) -> Result<(RenderFrame, SceneOutput), RenderError> {
        if width == 0 || height == 0 {
            return Err(RenderError::InvalidViewport);
        }
        let layout = build_scene_layout(scene, width as f32, height as f32, palette, &self.context);
        let frame = self.rasterize(width, height, &layout.draw_list)?;
        Ok((frame, layout))
    }

    /// Render the subtle top pull handle strip (`LayerRole::TopStrip`).
    ///
    /// # Errors
    /// Returns [`RenderError::InvalidViewport`] if `width` or `height` is zero.
    pub fn render_top_strip(
        &self,
        width: u32,
        height: u32,
        palette: &ThemePalette,
    ) -> Result<RenderFrame, RenderError> {
        let list = build_top_strip_scene(
            width as f32,
            height as f32,
            palette,
            self.context.top_drag_y,
        );
        self.rasterize(width, height, &list)
    }

    /// Rasterize an arbitrary [`DrawList`] into an `RGBA8` [`RenderFrame`].
    ///
    /// Reuses a cached backdrop buffer whenever the leading gradient & radial glow prefix
    /// matches the previous frame dimensions and theme.
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

        let prefix_len = list
            .commands
            .iter()
            .take_while(|cmd| {
                matches!(
                    cmd,
                    DrawCommand::BackdropGradient { .. } | DrawCommand::RadialGlow { .. }
                )
            })
            .count();

        let total_bytes = (width as usize) * (height as usize) * 4;
        let mut rgba = if prefix_len > 0 {
            let prefix = &list.commands[..prefix_len];
            let cached = {
                let borrow = self.backdrop_cache.borrow();
                borrow.as_ref().and_then(|c| {
                    (c.width == width && c.height == height && c.prefix.as_slice() == prefix)
                        .then(|| c.rgba.clone())
                })
            };

            if let Some(buf) = cached {
                buf
            } else {
                let mut buf = vec![0_u8; total_bytes];
                for cmd in prefix {
                    execute_command(&mut buf, width, height, cmd);
                }
                *self.backdrop_cache.borrow_mut() = Some(BackdropCache {
                    width,
                    height,
                    prefix: prefix.to_vec(),
                    rgba: buf.clone(),
                });
                buf
            }
        } else {
            vec![0_u8; total_bytes]
        };

        for cmd in &list.commands[prefix_len..] {
            execute_command(&mut rgba, width, height, cmd);
        }

        Ok(RenderFrame {
            width,
            height,
            rgba,
        })
    }
}

fn execute_command(rgba: &mut [u8], width: u32, height: u32, cmd: &DrawCommand) {
    match cmd {
        DrawCommand::BackdropGradient { top, bottom } => {
            draw_backdrop_gradient(rgba, width, height, *top, *bottom);
        }
        DrawCommand::RadialGlow {
            cx,
            cy,
            radius,
            color,
        } => draw_radial_glow(rgba, width, height, *cx, *cy, *radius, *color),
        DrawCommand::Rect(rect) => draw_rounded_rect(rgba, width, height, *rect),
        DrawCommand::Text(label) => draw_text(rgba, width, height, label),
        DrawCommand::Image(quad) => draw_image_quad(rgba, width, height, quad),
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

#[inline]
fn blend_pixel(buf: &mut [u8], idx: usize, src: Rgba) {
    let sa = src.a.clamp(0.0, 1.0);
    if sa <= 0.001 {
        return;
    }
    let sr = src.r.clamp(0.0, 1.0);
    let sg = src.g.clamp(0.0, 1.0);
    let sb = src.b.clamp(0.0, 1.0);

    let dst_a_u8 = buf[idx + 3];
    if sa >= 0.999 || dst_a_u8 == 0 {
        buf[idx] = (sr * 255.0).round() as u8;
        buf[idx + 1] = (sg * 255.0).round() as u8;
        buf[idx + 2] = (sb * 255.0).round() as u8;
        buf[idx + 3] = (sa * 255.0).round() as u8;
        return;
    }

    if dst_a_u8 == 255 {
        let inv = 1.0 - sa;
        let dr = f32::from(buf[idx]) / 255.0;
        let dg = f32::from(buf[idx + 1]) / 255.0;
        let db = f32::from(buf[idx + 2]) / 255.0;
        buf[idx] = ((sr * sa + dr * inv) * 255.0).round() as u8;
        buf[idx + 1] = ((sg * sa + dg * inv) * 255.0).round() as u8;
        buf[idx + 2] = ((sb * sa + db * inv) * 255.0).round() as u8;
        return;
    }

    let da = f32::from(dst_a_u8) / 255.0;
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

#[inline]
fn sdf_rounded_box(px: f32, py: f32, half_w: f32, half_h: f32, r: f32) -> f32 {
    let qx = px.abs() - (half_w - r);
    let qy = py.abs() - (half_h - r);
    if qx > 0.0 && qy > 0.0 {
        (qx * qx + qy * qy).sqrt() - r
    } else {
        qx.max(qy) - r
    }
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

    let pad = if rect.shadow_alpha > 0.001 && rect.shadow_radius > 0.5 {
        rect.shadow_radius + rect.shadow_offset_y.abs() + 2.0
    } else {
        2.0
    };
    let min_x = ((rect.x - pad).floor().max(0.0) as u32).min(w);
    let max_x = ((rect.x + rect.w + pad).ceil().max(0.0) as u32).min(w);
    let min_y = ((rect.y - pad).floor().max(0.0) as u32).min(h);
    let max_y = ((rect.y + rect.h + pad).ceil().max(0.0) as u32).min(h);

    let has_shadow = rect.shadow_alpha > 0.001 && rect.shadow_radius > 0.5;
    let has_border = rect.border_width > 0.01 && rect.border_color.a > 0.001;
    let has_sheen = rect.top_highlight_alpha > 0.001;

    for y in min_y..max_y {
        let py = y as f32 + 0.5;
        let rel_y = py - cy;
        let row_off = (y as usize) * (w as usize);
        let norm_y = ((py - rect.y) / rect.h).clamp(0.0, 1.0);
        let sheen_row = if has_sheen && norm_y < 0.35 {
            (1.0 - norm_y / 0.35) * rect.top_highlight_alpha
        } else {
            0.0
        };

        for x in min_x..max_x {
            let px = x as f32 + 0.5;
            let rel_x = px - cx;
            let idx = (row_off + x as usize) * 4;

            let dist = sdf_rounded_box(rel_x, rel_y, half_w, half_h, r);

            if has_shadow && dist > -1.0 {
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

            let fill_cov = (0.5 - dist).clamp(0.0, 1.0);
            if fill_cov > 0.001 {
                blend_pixel(buf, idx, rect.fill.with_alpha(rect.fill.a * fill_cov));

                if sheen_row > 0.001 {
                    blend_pixel(
                        buf,
                        idx,
                        Rgba {
                            r: 1.0,
                            g: 1.0,
                            b: 1.0,
                            a: sheen_row * fill_cov,
                        },
                    );
                }

                if has_border {
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
    if label.text.is_empty() || label.color.a <= 0.001 {
        return;
    }

    let width_i64 = i64::from(width);
    let height_i64 = i64::from(height);

    match layout_text_glyphs(label) {
        TextLayout::TrueType(glyphs) => {
            for pg in glyphs {
                let gw = pg.glyph.width as usize;
                for gy in 0..pg.glyph.height {
                    let py = i64::from(pg.y) + i64::from(gy);
                    if py < 0 || py >= height_i64 {
                        continue;
                    }
                    let src_row = (gy as usize) * gw;
                    let dst_row = (py as usize) * (width as usize);
                    for gx in 0..pg.glyph.width {
                        let cov = pg.glyph.alpha[src_row + gx as usize];
                        if cov == 0 {
                            continue;
                        }
                        let px = i64::from(pg.x) + i64::from(gx);
                        if px < 0 || px >= width_i64 {
                            continue;
                        }
                        let idx = (dst_row + px as usize) * 4;
                        let alpha = label.color.a * (f32::from(cov) / 255.0);
                        blend_pixel(buf, idx, label.color.with_alpha(alpha));
                    }
                }
            }
        }
        TextLayout::Bitmap5x7 => {
            let scale = label.scale.max(1);
            let total_w = measure_text_width(&label.text, scale) as f32;
            let start_x = match label.align {
                TextAlign::Left => label.x,
                TextAlign::Center => label.x - total_w * 0.5,
                TextAlign::Right => label.x - total_w,
            }
            .round() as i64;
            let start_y = label.y.round() as i64;
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
    }
}

fn draw_image_quad(buf: &mut [u8], buf_w: u32, buf_h: u32, quad: &ImageQuad) {
    if quad.w <= 0.0 || quad.h <= 0.0 || quad.alpha <= 0.001 {
        return;
    }
    let img = &quad.image;
    if img.width == 0 || img.height == 0 {
        return;
    }

    let half_w = quad.w * 0.5;
    let half_h = quad.h * 0.5;
    let cx = quad.x + half_w;
    let cy = quad.y + half_h;
    let corner_r = quad.radius.min(half_w).min(half_h).max(0.0);

    let min_x = ((quad.x - 1.0).floor().max(0.0) as u32).min(buf_w);
    let max_x = ((quad.x + quad.w + 1.0).ceil().max(0.0) as u32).min(buf_w);
    let min_y = ((quad.y - 1.0).floor().max(0.0) as u32).min(buf_h);
    let max_y = ((quad.y + quad.h + 1.0).ceil().max(0.0) as u32).min(buf_h);

    let img_w = img.width as f32;
    let img_h = img.height as f32;
    let last_src_x = img.width - 1;
    let last_src_y = img.height - 1;
    let src_stride = img.width as usize;

    for py_idx in min_y..max_y {
        let py = py_idx as f32 + 0.5;
        let rel_y = py - cy;
        let tex_v = ((py - quad.y) / quad.h).clamp(0.0, 1.0);
        let ty = (tex_v * img_h - 0.5).clamp(0.0, img_h - 1.0);
        let y0 = (ty.floor() as u32).min(last_src_y);
        let y1 = (y0 + 1).min(last_src_y);
        let fy = ty - y0 as f32;
        let row_off = (py_idx as usize) * (buf_w as usize);

        for px_idx in min_x..max_x {
            let px = px_idx as f32 + 0.5;
            let dist = sdf_rounded_box(px - cx, rel_y, half_w, half_h, corner_r);
            let cov = (0.5 - dist).clamp(0.0, 1.0);
            if cov <= 0.001 {
                continue;
            }

            let tex_u = ((px - quad.x) / quad.w).clamp(0.0, 1.0);
            let tx = (tex_u * img_w - 0.5).clamp(0.0, img_w - 1.0);
            let x0 = (tx.floor() as u32).min(last_src_x);
            let x1 = (x0 + 1).min(last_src_x);
            let fx = tx - x0 as f32;

            let i00 = (y0 as usize * src_stride + x0 as usize) * 4;
            let i10 = (y0 as usize * src_stride + x1 as usize) * 4;
            let i01 = (y1 as usize * src_stride + x0 as usize) * 4;
            let i11 = (y1 as usize * src_stride + x1 as usize) * 4;

            let w00 = (1.0 - fx) * (1.0 - fy);
            let w10 = fx * (1.0 - fy);
            let w01 = (1.0 - fx) * fy;
            let w11 = fx * fy;

            let chan = |c: usize| -> f32 {
                (f32::from(img.rgba[i00 + c]) * w00
                    + f32::from(img.rgba[i10 + c]) * w10
                    + f32::from(img.rgba[i01 + c]) * w01
                    + f32::from(img.rgba[i11 + c]) * w11)
                    / 255.0
            };
            let sa = chan(3) * quad.alpha * cov;
            if sa <= 0.001 {
                continue;
            }
            let idx = (row_off + px_idx as usize) * 4;
            blend_pixel(
                buf,
                idx,
                Rgba {
                    r: chan(0),
                    g: chan(1),
                    b: chan(2),
                    a: sa,
                },
            );
        }
    }
}
