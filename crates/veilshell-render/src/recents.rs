//! Mobile overview: a carousel of exposed window previews, not nested cards.

use std::sync::Arc;

use veilshell_core::{Rgba, ThemePalette};

use crate::scene::push_bottom_pill_bar;
use crate::{
    font, HitRect, HitTarget, ImageQuad, RecentsCard, RenderTarget, RoundedRect, SceneContext,
    SceneOutput, TextAlign,
};

const SCALE: f32 = 0.63;
const PAGE_GAP: f32 = 16.0;
const LANDSCAPE_OVERLAP: f32 = 0.25;
const LANDSCAPE_DEPTH: f32 = 0.08;
const APP_ICON: f32 = 48.0;
const LABEL_GAP: f32 = 12.0;

fn fit_preview((max_w, max_h): (f32, f32), (w, h): (u32, u32)) -> (f32, f32) {
    let (w, h) = (w.max(1) as f32, h.max(1) as f32);
    let fit = (max_w / w).min(max_h / h);
    (w * fit, h * fit)
}

/// Physical size of a centered preview, rounded up to cover its footprint.
/// Use the centered size for neighbors too, so they're sharp when swiped into place.
#[must_use]
pub fn recents_preview_pixels(target: RenderTarget, source: (u32, u32)) -> (u32, u32) {
    let (w, h) = target.logical_size();
    let (w, h) = fit_preview((w as f32 * SCALE, h as f32 * SCALE), source);
    let scale = target.pixel_scale();
    ((w * scale.x).ceil() as u32, (h * scale.y).ceil() as u32)
}

/// The travel required to move the overview by one centered window.
#[must_use]
pub fn recents_stride(width: f32, height: f32) -> f32 {
    if width > height {
        (width * SCALE * (1.0 - LANDSCAPE_OVERLAP)).max(1.0)
    } else {
        (width * SCALE + PAGE_GAP).max(1.0)
    }
}

pub(crate) fn build_recents(
    out: &mut SceneOutput,
    width: f32,
    height: f32,
    palette: &ThemePalette,
    ctx: &SceneContext,
) {
    let progress = ctx.overlay_progress.clamp(0.0, 1.0);
    // A plain dark scrim over the compositor. Do not sample the wallpaper blur
    // or paint the colored backdrop used by Quick Settings.
    out.draw_list.push_rect(RoundedRect::filled(
        0.0,
        0.0,
        width,
        height,
        0.0,
        Rgba::from_rgb_u8(5, 6, 8).with_alpha(0.35 * progress),
    ));
    out.push_hit(
        HitRect::new(0.0, 0.0, width, height),
        HitTarget::DismissOverlay,
    );
    let cards = &ctx.recents_cards;
    if cards.is_empty() {
        out.draw_list.push_text(
            width * 0.5,
            height * 0.5 + (1.0 - progress) * 28.0,
            "No open windows",
            2,
            palette.foreground.with_alpha(progress),
            TextAlign::Center,
        );
    } else {
        let center = ctx.recents_index.min(cards.len() - 1);
        let position = ctx
            .recents_position
            .unwrap_or(center as f32)
            .clamp(-0.2, (cards.len() - 1) as f32 + 0.2);
        let first = (position.floor() as usize).saturating_sub(1);
        let last = (position.ceil() as usize + 1).min(cards.len() - 1);
        let layout = DeckLayout {
            width,
            height,
            position,
            progress,
        };
        let mut order: Vec<_> = (first..=last).collect();
        // Draw the centered window above its neighbors so taps and imagery
        // agree in the overlapping landscape deck.
        order.sort_by(|&a, &b| {
            (b as f32 - position)
                .abs()
                .total_cmp(&(a as f32 - position).abs())
        });
        for index in order {
            layout.draw_card(out, &cards[index], index, ctx, palette);
        }
    }
    push_bottom_pill_bar(&mut out.draw_list, width, height, palette, ctx);
    if ctx.recents_position.is_some() || ctx.recents_throw.is_some() || progress < 1.0 {
        out.hit_regions.clear();
    }
}

struct DeckLayout {
    width: f32,
    height: f32,
    position: f32,
    progress: f32,
}

impl DeckLayout {
    fn draw_card(
        &self,
        out: &mut SceneOutput,
        card: &RecentsCard,
        index: usize,
        ctx: &SceneContext,
        palette: &ThemePalette,
    ) {
        let distance = index as f32 - self.position;
        let landscape = self.width > self.height;
        let depth = if landscape {
            distance.abs().min(1.4) * LANDSCAPE_DEPTH
        } else {
            0.0
        };
        let throw = ctx
            .recents_throw
            .as_ref()
            .filter(|(id, _)| id == &card.id)
            .map_or(0.0, |(_, offset)| *offset);
        let scale = SCALE * (1.0 - depth) * (1.0 - (-throw / self.height).clamp(0.0, 1.0) * 0.18);
        let center_x = self.width * 0.5 + recents_stride(self.width, self.height) * distance;
        let center_y = self.height * 0.5 + (1.0 - self.progress) * 48.0 + throw;
        let max_w = self.width * scale;
        let max_h = self.height * scale;
        let (w, h) = card.thumbnail.as_ref().map_or((max_w, max_h), |thumb| {
            fit_preview((max_w, max_h), (thumb.width, thumb.height))
        });
        let rect = HitRect::new(center_x - w * 0.5, center_y - h * 0.5, w, h);
        if rect.x + rect.w <= 0.0 || rect.x >= self.width {
            return;
        }
        let radius = 28.0_f32.min(w.min(h) * 0.14);
        if let Some(thumb) = &card.thumbnail {
            out.draw_list.push_image(ImageQuad {
                x: rect.x,
                y: rect.y,
                w,
                h,
                radius,
                alpha: self.progress,
                image: Arc::clone(thumb),
            });
        } else {
            // A truthful fallback for a window that cannot be captured.
            out.draw_list.push_rect(RoundedRect::filled(
                rect.x,
                rect.y,
                w,
                h,
                radius,
                palette.background.with_alpha(0.93 * self.progress),
            ));
            out.draw_list.push_text(
                center_x,
                center_y - 7.0,
                "Preview unavailable",
                1,
                palette.muted.with_alpha(self.progress),
                TextAlign::Center,
            );
        }
        let app = ctx.home_apps.iter().find(|app| {
            let id = app.id.strip_suffix(".desktop").unwrap_or(&app.id);
            card.app_id.eq_ignore_ascii_case(id) || card.app_id.eq_ignore_ascii_case(&app.label)
        });
        let name = app.map_or_else(
            || {
                if card.title.is_empty() {
                    card.app_id.as_str()
                } else {
                    card.title.as_str()
                }
            },
            |app| app.label.as_str(),
        );
        let icon = app.and_then(|app| app.icon.as_ref());
        self.draw_label(out, rect, name, icon, palette);
        out.push_hit(
            rect,
            HitTarget::FocusWindow {
                id: card.id.clone(),
            },
        );
    }

    fn draw_label(
        &self,
        out: &mut SceneOutput,
        rect: HitRect,
        name: &str,
        icon: Option<&Arc<crate::RgbaImage>>,
        palette: &ThemePalette,
    ) {
        let size = APP_ICON.min(rect.y.max(8.0) * 0.72).min(rect.w * 0.24);
        let y = rect.y - LABEL_GAP - size;
        if let Some(icon) = icon {
            out.draw_list.push_image(ImageQuad {
                x: rect.x,
                y,
                w: size,
                h: size,
                radius: 0.0,
                image: Arc::clone(icon),
                alpha: self.progress,
            });
        } else {
            let initial = name
                .chars()
                .next()
                .map_or_else(|| "?".to_owned(), |ch| ch.to_uppercase().collect());
            out.draw_list.push_rect(RoundedRect::filled(
                rect.x,
                y,
                size,
                size,
                size * 0.24,
                palette.surface.with_alpha(0.9 * self.progress),
            ));
            out.draw_list.push_text(
                rect.x + size * 0.5,
                y + (size - 14.0) * 0.5,
                initial,
                2,
                palette.foreground.with_alpha(self.progress),
                TextAlign::Center,
            );
        }
        out.draw_list.push_text(
            rect.x + size + LABEL_GAP,
            y + (size - 14.0) * 0.5,
            font::fit_text(name, 2, rect.w - size - LABEL_GAP),
            2,
            palette.foreground.with_alpha(self.progress),
            TextAlign::Left,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_footprint_uses_physical_pixels_in_both_orientations() {
        let landscape = RenderTarget::new((1280, 853), (1920, 1280)).unwrap();
        let portrait = RenderTarget::new((853, 1280), (1280, 1920)).unwrap();
        let unscaled = RenderTarget::unscaled(853, 1280).unwrap();
        assert_eq!(recents_preview_pixels(landscape, (1920, 1280)), (1210, 807));
        assert_eq!(recents_preview_pixels(portrait, (1280, 1920)), (807, 1210));
        assert_eq!(recents_preview_pixels(unscaled, (1280, 1920)), (538, 807));
    }

    #[test]
    fn preview_footprint_fits_the_window_aspect_before_scaling_to_pixels() {
        let target = RenderTarget::new((960, 640), (1920, 1280)).unwrap();
        assert_eq!(recents_preview_pixels(target, (600, 900)), (538, 807));
    }
}
