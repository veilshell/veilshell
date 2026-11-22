//! App keyboard geometry inspired by Denial's mobile panel, shared by drawing and input.

use veilshell_core::{OskKeyAction, OskKeySpec, OskState, ShiftState, ThemePalette};

use crate::font::{fit_text, GLYPH_H};
use crate::shade_glyphs::{self, Glyph};
use crate::{HitRect, HitTarget, RoundedRect, SceneContext, SceneOutput, TextAlign};

struct KeyGrid {
    center: f32,
    pitch: f32,
    gap: f32,
    row_gap: f32,
    top: f32,
    strip_h: f32,
    row_h: f32,
    sides: bool,
}

impl KeyGrid {
    fn new(width: f32, height: f32, landscape: bool) -> Self {
        let pad_x = 14.0_f32.min(width * 0.027);
        let top = 12.0_f32.min(height * 0.05);
        let gap = 6.0_f32.min(width * 0.012);
        let row_gap = 7.0_f32.min(height * 0.028);
        let inner_w = width - 2.0 * pad_x;
        let inner_h = height - 2.0 * top;
        // Ten typing units, three for each side cluster, and half a unit
        // between each cluster and the typing rows. Don't stretch on ultrawides.
        let side_pitch = ((inner_w + gap) / 17.0).min(72.0);
        let typing_h = inner_h - 3.0 * row_gap;
        // Only use side groups when the typing keys still have room. On short
        // widescreen panels they save vertical space.
        let sides = landscape && side_pitch - gap >= 36.0;
        // Portrait and narrow panels give the shortcuts a shallower top row.
        let (pitch, strip_h, row_h) = if sides {
            (side_pitch, 0.0, typing_h / 4.0)
        } else {
            let strip_h = 46.0_f32.min(inner_h * 0.17);
            (
                ((inner_w + gap) / 10.0).min(72.0),
                strip_h,
                (typing_h - row_gap - strip_h) / 4.0,
            )
        };
        Self {
            center: width * 0.5,
            pitch,
            gap,
            row_gap,
            top,
            strip_h,
            row_h,
            sides,
        }
    }

    fn row_y(&self, row: usize) -> f32 {
        let typing_top = if self.sides {
            self.top
        } else {
            self.top + self.strip_h + self.row_gap
        };
        typing_top + row as f32 * (self.row_h + self.row_gap)
    }

    fn bounds(&self, start: f32, span: f32, y: f32, h: f32) -> HitRect {
        HitRect::new(
            self.center + start * self.pitch + self.gap * 0.5,
            y,
            span * self.pitch - self.gap,
            h,
        )
    }

    fn shortcut(&self, action: OskKeyAction) -> Option<HitRect> {
        use OskKeyAction::{
            ArrowDown, ArrowLeft, ArrowRight, ArrowUp, Dismiss, Escape, Tab, ToggleCtrl,
            ToggleTerminal,
        };
        // Fixed slots in key units relative to the panel center. Toggling TERM
        // or changing layers reveals keys without shifting the other targets.
        if self.sides {
            let (left, span, row) = match action {
                Escape => (-8.5, 1.5, 2),
                Tab => (-7.0, 1.5, 2),
                ToggleTerminal => (-8.5, 1.5, 3),
                ToggleCtrl => (-7.0, 1.5, 3),
                ArrowLeft => (5.5, 1.0, 3),
                ArrowUp => (6.5, 1.0, 2),
                ArrowDown => (6.5, 1.0, 3),
                ArrowRight => (7.5, 1.0, 3),
                Dismiss => (7.5, 1.0, 2),
                _ => return None,
            };
            Some(self.bounds(left, span, self.row_y(row), self.row_h))
        } else {
            let (left, span) = match action {
                ToggleTerminal => (-5.0, 1.25),
                ToggleCtrl => (-3.75, 1.25),
                Escape => (-2.5, 1.0),
                Tab => (-1.5, 1.0),
                ArrowLeft => (-0.5, 1.0),
                ArrowUp => (0.5, 1.0),
                ArrowDown => (1.5, 1.0),
                ArrowRight => (2.5, 1.0),
                Dismiss => (3.5, 1.5),
                _ => return None,
            };
            Some(self.bounds(left, span, self.top, self.strip_h))
        }
    }
}

pub(crate) fn build_osk(
    out: &mut SceneOutput,
    width: f32,
    height: f32,
    palette: &ThemePalette,
    ctx: &SceneContext,
) {
    if width <= 0.0 || height <= 0.0 {
        return;
    }
    out.draw_list.push_rect(
        RoundedRect::filled(
            0.0,
            0.0,
            width,
            height,
            0.0,
            palette.surface.with_alpha(0.92),
        )
        .with_glass_border(palette.foreground.with_alpha(0.16), 1.5, 0.10),
    );
    let landscape = ctx
        .osk_output_size
        .map_or(width > 3.0 * height, |(w, h)| w > h);
    let grid = KeyGrid::new(width, height, landscape);
    let state = ctx.osk_state;
    for (index, row) in state.typing_rows().into_iter().enumerate() {
        let y = grid.row_y(index);
        let mut x = -0.5 * row.iter().map(|key| key.width_units).sum::<f32>();
        for key in row {
            let rect = grid.bounds(x, key.width_units, y, grid.row_h);
            draw_key(out, key, rect, palette, state);
            x += key.width_units;
        }
    }
    for key in state.shortcut_keys() {
        if let Some(rect) = grid.shortcut(key.action) {
            draw_key(out, key, rect, palette, state);
        }
    }
}

fn draw_key(
    out: &mut SceneOutput,
    key: OskKeySpec,
    rect: HitRect,
    palette: &ThemePalette,
    state: OskState,
) {
    use OskKeyAction::{
        ArrowDown, ArrowLeft, ArrowRight, ArrowUp, Char, Space, ToggleCtrl, ToggleShift,
        ToggleTerminal,
    };
    let active = match key.action {
        ToggleShift => state.shift != ShiftState::Off,
        ToggleCtrl => state.ctrl,
        ToggleTerminal => state.terminal,
        _ => false,
    };
    let (fill, ink) = if active || key.is_accent {
        (palette.accent.with_alpha(0.90), palette.background)
    } else {
        let fill = match key.action {
            Char(_) => palette.foreground.with_alpha(0.09),
            Space => palette.foreground.with_alpha(0.16),
            _ => palette.background.with_alpha(0.78),
        };
        (fill, palette.foreground)
    };
    let HitRect { x, y, w, h } = rect;
    let short_edge = w.min(h);
    out.draw_list.push_rect(
        RoundedRect::filled(x, y, w, h, (short_edge * 0.2).min(10.0), fill).with_glass_border(
            palette.foreground.with_alpha(0.14),
            1.0,
            0.08,
        ),
    );
    let cx = x + w * 0.5;
    let cy = y + h * 0.5;
    let arrow = match key.action {
        ArrowLeft => Some(Glyph::ArrowLeft),
        ArrowRight => Some(Glyph::ArrowRight),
        ArrowUp => Some(Glyph::ArrowUp),
        ArrowDown => Some(Glyph::ArrowDown),
        _ => None,
    };
    if let Some(glyph) = arrow {
        shade_glyphs::draw(
            &mut out.draw_list,
            glyph,
            cx,
            cy,
            (short_edge * 0.68).min(23.0),
            ink,
        );
    } else {
        let scale = if matches!(key.action, Char(_)) && short_edge >= 18.0 {
            2
        } else {
            1
        };
        out.draw_list.push_text(
            cx,
            cy - (GLYPH_H * scale) as f32 * 0.5,
            fit_text(key.label, scale, w - 4.0),
            scale,
            ink,
            TextAlign::Center,
        );
    }
    out.push_hit(rect, HitTarget::OskKey { label: key.id });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{build_scene_layout, DrawCommand, SceneKind};
    use veilshell_core::OskLayer;

    fn layout(output: (u32, u32), panel_h: f32, state: OskState) -> SceneOutput {
        build_scene_layout(
            SceneKind::Osk,
            output.0 as f32,
            panel_h,
            &ThemePalette::default_dark(),
            &SceneContext {
                osk_state: state,
                osk_output_size: Some(output),
                ..SceneContext::default()
            },
        )
    }

    fn terminal() -> OskState {
        OskState {
            terminal: true,
            ..OskState::default()
        }
    }

    fn key(out: &SceneOutput, label: &'static str) -> HitRect {
        out.hit_regions
            .iter()
            .find(|hit| hit.target == (HitTarget::OskKey { label }))
            .unwrap_or_else(|| panic!("missing key: {label}"))
            .rect
    }

    #[test]
    fn typing_rows_share_a_pitch_with_a_stagger_and_a_centered_spacebar() {
        for (output, panel_h) in [((960, 640), 213.0), ((640, 960), 320.0)] {
            let out = layout(output, panel_h, terminal());
            let (q, w, a, p, space) = (
                key(&out, "q"),
                key(&out, "w"),
                key(&out, "a"),
                key(&out, "p"),
                key(&out, "space"),
            );
            assert!((a.x - q.x - (w.x - q.x) * 0.5).abs() < 0.001);
            assert!((a.w - q.w).abs() < 0.001);
            assert!((q.x + p.x + p.w - output.0 as f32).abs() < 0.001);
            assert!((space.x * 2.0 + space.w - output.0 as f32).abs() < 0.001);
            assert!(space.w > q.w * 4.0);

            let numbers = layout(
                output,
                panel_h,
                OskState {
                    layer: OskLayer::Numbers,
                    ..terminal()
                },
            );
            assert_eq!(q, key(&numbers, "1"));
            assert_eq!(space, key(&numbers, "space"));
            for label in ["term", "ctl", "hide"] {
                assert_eq!(key(&out, label), key(&numbers, label));
            }
        }
    }

    #[test]
    fn shortcuts_follow_the_output_not_the_panel_and_do_not_shift_typing() {
        // Both panels are wide. Only the full landscape output gets side groups.
        let landscape = layout((800, 640), 213.0, terminal());
        let portrait = layout((800, 960), 213.0, terminal());
        let q = key(&landscape, "q");
        let p = key(&landscape, "p");
        let up = key(&landscape, "up");
        let down = key(&landscape, "down");
        assert!(key(&landscape, "tab").x + key(&landscape, "tab").w < q.x);
        assert!(key(&landscape, "left").x > p.x + p.w);
        assert!(up.y < down.y && (up.x - down.x).abs() < 0.001);
        for shortcut in terminal().shortcut_keys() {
            let rect = key(&portrait, shortcut.id);
            assert!(rect.y + rect.h < key(&portrait, "q").y);
        }

        let off = layout((800, 640), 213.0, OskState::default());
        assert!(!off
            .hit_regions
            .iter()
            .any(|hit| hit.target == HitTarget::OskKey { label: "up" }));
        for label in ["term", "ctl", "hide", "q", "space"] {
            assert_eq!(key(&landscape, label), key(&off, label));
        }
    }

    #[test]
    fn narrow_landscape_has_a_compact_shortcut_strip() {
        let out = layout((560, 480), 160.0, terminal());
        let up = key(&out, "up");
        assert!(up.y + up.h < key(&out, "q").y);
        for name in ["TERM", "CTRL", "HIDE", "ENTER"] {
            assert!(out
                .draw_list
                .commands
                .iter()
                .any(|c| matches!(c, DrawCommand::Text(text) if text.text == name)));
        }
    }

    #[test]
    fn shift_updates_letter_labels_without_moving_the_keys() {
        let original = key(&layout((640, 960), 320.0, OskState::default()), "q");
        for (shift, label) in [
            (ShiftState::Off, "q"),
            (ShiftState::OneShot, "Q"),
            (ShiftState::CapsLock, "Q"),
        ] {
            let state = OskState {
                shift,
                ..OskState::default()
            };
            let out = layout((640, 960), 320.0, state);
            let rect = key(&out, "q");
            assert!(out.draw_list.commands.iter().any(|cmd| matches!(
                cmd,
                DrawCommand::Text(text) if text.text == label && rect.contains(text.x, text.y)
            )));
            assert_eq!(rect, original);
        }
    }

    #[test]
    fn drawn_keys_and_hit_regions_agree_on_every_layer() {
        for (output, panel_h) in [
            ((960, 640), 213.0),
            ((640, 960), 320.0),
            ((390, 518), 172.0),
            ((560, 480), 160.0),
            ((960, 294), 98.0),
            ((3440, 1440), 480.0),
        ] {
            for layer in [OskLayer::Letters, OskLayer::Numbers, OskLayer::Symbols] {
                let state = OskState {
                    layer,
                    ..terminal()
                };
                let out = layout(output, panel_h, state);
                let expected = state
                    .typing_rows()
                    .into_iter()
                    .flatten()
                    .chain(state.shortcut_keys());
                for spec in expected {
                    assert!(key(&out, spec.id).w > 0.0);
                }
                let caps: Vec<_> = out
                    .draw_list
                    .commands
                    .iter()
                    .skip(1)
                    .filter_map(|command| match command {
                        DrawCommand::Rect(cap) => Some(HitRect::new(cap.x, cap.y, cap.w, cap.h)),
                        _ => None,
                    })
                    .collect();
                assert_eq!(caps.len(), out.hit_regions.len());
                for (i, (rect, hit)) in caps.into_iter().zip(&out.hit_regions).enumerate() {
                    assert_eq!(rect, hit.rect);
                    assert!(rect.x >= 0.0 && rect.x + rect.w <= output.0 as f32 + 0.001);
                    assert!(rect.h > 0.0 && rect.y >= 0.0 && rect.y + rect.h <= panel_h + 0.001);
                    assert_eq!(
                        out.hit_test(rect.x + rect.w * 0.5, rect.y + rect.h * 0.5),
                        Some(&hit.target)
                    );
                    for other in &out.hit_regions[..i] {
                        let b = other.rect;
                        assert!(
                            rect.x + rect.w <= b.x
                                || b.x + b.w <= rect.x
                                || rect.y + rect.h <= b.y
                                || b.y + b.h <= rect.y
                        );
                    }
                }
            }
        }
    }
}
