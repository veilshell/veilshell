//! Fast analytic SDF CPU rasterizer, cached backdrop, and `PNG` / `Wayland` `ARGB8888` encoders.

use std::cell::RefCell;
use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

use veilshell_core::{Rgba, ThemePalette};

use crate::font::{
    glyph_rows, layout_text_glyphs_scaled, BitmapLayout, TextLayout, GLYPH_H, GLYPH_W,
};
use crate::primitives::{
    DrawCommand, DrawList, ImageQuad, PixelScale, RenderTarget, RoundedRect, SceneOutput, TextLabel,
};
use crate::scene::{build_scene, build_scene_layout, build_top_strip_scene, SceneContext};
use crate::{RenderError, RenderFrame, RendererBackend, SceneKind, SnapshotSpec};

#[derive(Clone, Debug)]
struct BackdropCache {
    target: RenderTarget,
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
            self.context.top_pressed,
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
        self.rasterize_scaled(RenderTarget::unscaled(width, height)?, list)
    }

    /// Sample the logical draw list directly on the physical pixel grid.
    ///
    /// # Errors
    /// Returns [`RenderError::InvalidViewport`] if the frame size overflows.
    pub fn rasterize_scaled(
        &self,
        target: RenderTarget,
        list: &DrawList,
    ) -> Result<RenderFrame, RenderError> {
        let (width, height) = target.physical_size();
        let scale = target.pixel_scale();

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

        let total_bytes = (width as usize)
            .checked_mul(height as usize)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or(RenderError::InvalidViewport)?;
        let mut rgba = if prefix_len > 0 {
            let prefix = &list.commands[..prefix_len];
            let cached = {
                let borrow = self.backdrop_cache.borrow();
                borrow.as_ref().and_then(|c| {
                    (c.target == target && c.prefix.as_slice() == prefix).then(|| c.rgba.clone())
                })
            };

            if let Some(buf) = cached {
                buf
            } else {
                let mut buf = vec![0_u8; total_bytes];
                for cmd in prefix {
                    execute_command(&mut buf, width, height, scale, cmd);
                }
                *self.backdrop_cache.borrow_mut() = Some(BackdropCache {
                    target,
                    prefix: prefix.to_vec(),
                    rgba: buf.clone(),
                });
                buf
            }
        } else {
            vec![0_u8; total_bytes]
        };

        for cmd in &list.commands[prefix_len..] {
            execute_command(&mut rgba, width, height, scale, cmd);
        }

        Ok(RenderFrame {
            width,
            height,
            rgba,
        })
    }
}

fn execute_command(rgba: &mut [u8], width: u32, height: u32, scale: PixelScale, cmd: &DrawCommand) {
    match cmd {
        DrawCommand::BackdropGradient { top, bottom } => {
            draw_backdrop_gradient(rgba, width, height, *top, *bottom);
        }
        DrawCommand::RadialGlow {
            cx,
            cy,
            radius,
            color,
        } => {
            if *radius > 1.0 {
                draw_radial_glow(
                    rgba,
                    width,
                    height,
                    *cx * scale.x,
                    *cy * scale.y,
                    *radius * scale.uniform(),
                    *color,
                );
            }
        }
        DrawCommand::Rect(rect) => {
            draw_rounded_rect(rgba, width, height, rect.scaled(scale), scale);
        }
        DrawCommand::Text(label) => draw_text(rgba, width, height, label, scale),
        DrawCommand::Image(quad) => draw_image_quad(rgba, width, height, quad, scale),
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
        let target = spec.viewport.render_target()?;
        let list = build_scene(
            spec.scene,
            spec.viewport.width as f32,
            spec.viewport.height as f32,
            palette,
            &self.context,
        );
        self.rasterize_scaled(target, &list)
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
        self.write_argb8888_premultiplied(&mut out);
        out
    }

    /// Convert to premultiplied `BGRA8`, writing directly into a caller-provided buffer.
    ///
    /// # Panics
    /// Panics if `out` is too short to hold the frame.
    pub fn write_argb8888_premultiplied(&self, out: &mut [u8]) {
        let out = &mut out[..self.rgba.len()];
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
    if color.a <= 0.001 {
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

fn draw_rounded_rect(buf: &mut [u8], w: u32, h: u32, rect: RoundedRect, scale: PixelScale) {
    if rect.w <= 0.0 || rect.h <= 0.0 {
        return;
    }
    let half_w = rect.w * 0.5;
    let half_h = rect.h * 0.5;
    let cx = rect.x + half_w;
    let cy = rect.y + half_h;
    let r = rect.radius.min(half_w).min(half_h).max(0.0);

    let has_shadow = rect.shadow_alpha > 0.001 && rect.shadow_radius > 0.5 * scale.uniform();
    let pad = if has_shadow {
        rect.shadow_radius + rect.shadow_offset_y.abs() + 2.0
    } else {
        2.0
    };
    let min_x = ((rect.x - pad).floor().max(0.0) as u32).min(w);
    let max_x = ((rect.x + rect.w + pad).ceil().max(0.0) as u32).min(w);
    let min_y = ((rect.y - pad).floor().max(0.0) as u32).min(h);
    let max_y = ((rect.y + rect.h + pad).ceil().max(0.0) as u32).min(h);

    let has_border = rect.border_width > 0.01 * scale.uniform() && rect.border_color.a > 0.001;
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

fn draw_text(buf: &mut [u8], width: u32, height: u32, label: &TextLabel, scale: PixelScale) {
    if label.text.is_empty() || label.color.a <= 0.001 {
        return;
    }

    let width_i64 = i64::from(width);
    let height_i64 = i64::from(height);

    match layout_text_glyphs_scaled(label, scale) {
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
            draw_bitmap_text(buf, width, height, label, scale);
        }
    }
}

fn draw_bitmap_text(buf: &mut [u8], width: u32, height: u32, label: &TextLabel, scale: PixelScale) {
    let layout = BitmapLayout::new(label, scale);
    // Round the cell edges to pixel centers, matching the GPU fragment test.
    let edge = |px: f32, extent: u32| ((px - 0.5).ceil() as u32).min(extent);
    for (idx, ch) in label.text.chars().enumerate() {
        let gx = layout.glyph_x(idx);
        for (ry, mask) in glyph_rows(ch)
            .into_iter()
            .enumerate()
            .take(GLYPH_H as usize)
        {
            let y0 = edge(layout.y + ry as f32 * layout.cell_y, height);
            let y1 = edge(layout.y + (ry + 1) as f32 * layout.cell_y, height);
            for rx in 0..GLYPH_W {
                if mask & (1 << (GLYPH_W - 1 - rx)) == 0 {
                    continue;
                }
                let x0 = edge(gx + rx as f32 * layout.cell_x, width);
                let x1 = edge(gx + (rx + 1) as f32 * layout.cell_x, width);
                for y in y0..y1 {
                    let row_off = y as usize * width as usize;
                    for x in x0..x1 {
                        blend_pixel(buf, (row_off + x as usize) * 4, label.color);
                    }
                }
            }
        }
    }
}

fn draw_image_quad(buf: &mut [u8], buf_w: u32, buf_h: u32, quad: &ImageQuad, scale: PixelScale) {
    if quad.w <= 0.0 || quad.h <= 0.0 || quad.alpha <= 0.001 {
        return;
    }
    let img = &quad.image;
    if img.width == 0 || img.height == 0 {
        return;
    }

    let [x, y, w, h] = quad.physical_bounds(scale);
    let half_w = w * 0.5;
    let half_h = h * 0.5;
    let cx = x + half_w;
    let cy = y + half_h;
    let corner_r = (quad.radius * scale.uniform())
        .min(half_w)
        .min(half_h)
        .max(0.0);

    let min_x = ((x - 1.0).floor().max(0.0) as u32).min(buf_w);
    let max_x = ((x + w + 1.0).ceil().max(0.0) as u32).min(buf_w);
    let min_y = ((y - 1.0).floor().max(0.0) as u32).min(buf_h);
    let max_y = ((y + h + 1.0).ceil().max(0.0) as u32).min(buf_h);

    let img_w = img.width as f32;
    let img_h = img.height as f32;
    let last_src_x = img.width - 1;
    let last_src_y = img.height - 1;
    let src_stride = img.width as usize;

    for py_idx in min_y..max_y {
        let py = py_idx as f32 + 0.5;
        let rel_y = py - cy;
        let tex_v = ((py - y) / h).clamp(0.0, 1.0);
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

            let tex_u = ((px - x) / w).clamp(0.0, 1.0);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TextAlign;

    #[test]
    fn analytic_edges_are_antialiased_on_the_physical_grid() {
        let mut list = DrawList::default();
        list.push_rect(RoundedRect::filled(
            4.25,
            2.0,
            20.0,
            8.0,
            0.0,
            Rgba::from_rgb_u8(255, 255, 255),
        ));
        let renderer = SdfSoftwareRenderer::default();
        for (target, edge, alpha) in [
            (RenderTarget::new((40, 20), (60, 30)).unwrap(), 6, 159),
            (RenderTarget::new((40, 20), (80, 40)).unwrap(), 8, 128),
        ] {
            let frame = renderer.rasterize_scaled(target, &list).unwrap();
            assert_eq!((frame.width, frame.height), target.physical_size());
            let at = |x| frame.rgba[((6 * frame.width + x) * 4 + 3) as usize];
            assert_eq!(at(edge - 1), 0);
            assert_eq!(at(edge), alpha);
            assert_eq!(at(edge + 1), 255);
        }
    }

    #[test]
    fn bitmap_fallback_samples_fractional_cells_without_font_discovery() {
        let label = TextLabel {
            x: 4.0,
            y: 4.0,
            text: "D".into(),
            scale: 1,
            color: Rgba::from_rgb_u8(255, 255, 255),
            align: TextAlign::Left,
        };
        let mut rgba = [0; 20 * 20 * 4];
        draw_bitmap_text(&mut rgba, 20, 20, &label, PixelScale { x: 1.25, y: 1.5 });
        let alpha = |x: usize, y: usize| rgba[(y * 20 + x) * 4 + 3];
        assert_eq!(alpha(5, 5), 0);
        assert_eq!(alpha(4, 6), 0);
        assert_eq!(alpha(5, 6), 255);
        assert_eq!(alpha(8, 6), 255);
        assert_eq!(alpha(9, 6), 0);
        assert_eq!(alpha(9, 7), 255);
    }
}
