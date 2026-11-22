//! Denial-style lock face and locker-local touch keyboard layout (`DrawList`).

use veilshell_core::{Rgba, ThemePalette};
use veilshell_lock_input::Keyboard;
use veilshell_render::{DrawList, RoundedRect, TextAlign};

/// Active visual stage on the primary lock surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LockStage {
    /// Clean lock face showing clock, date, and swipe-up/tap prompt.
    Cover,
    /// Password entry stage showing masked dots and the locker-local touch keyboard.
    Password,
}

/// Non-sensitive status message shown above the password field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthBanner {
    Idle,
    Checking,
    Denied,
    WaitRetry,
}

impl AuthBanner {
    #[must_use]
    pub const fn text(self) -> &'static str {
        match self {
            Self::Idle => "Enter password",
            Self::Checking => "Checking password...",
            Self::Denied => "Incorrect password",
            Self::WaitRetry => "Wait a moment before trying again",
        }
    }
}

/// Logical-pixel rectangle where `veilshell_lock_input::Keyboard` is drawn and hit-tested.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KeyboardRect {
    pub x: f64,
    pub y: f64,
    pub width: u32,
    pub height: u32,
}

/// Compute the responsive logical keyboard rectangle for a viewport.
#[must_use]
pub fn keyboard_rect(viewport_w: u32, viewport_h: u32) -> KeyboardRect {
    let margin_x = if viewport_w >= 900 { 48 } else { 16 };
    let max_w = viewport_w.saturating_sub(margin_x * 2);
    let width = max_w.min(920);
    let target_h = if viewport_h >= viewport_w {
        // Portrait tablet: slightly taller keys for comfortable thumb typing.
        ((viewport_h as f32) * 0.27).round() as u32
    } else {
        // Landscape tablet: compact 4-row keyboard leaving room for clock + password card.
        ((viewport_h as f32) * 0.36).round() as u32
    };
    let height = target_h.clamp(160, 320).min(viewport_h.saturating_sub(200));
    let bottom_margin = if viewport_h >= 700 { 28 } else { 12 };
    let x = f64::from(viewport_w.saturating_sub(width)) * 0.5;
    let y = f64::from(viewport_h.saturating_sub(height + bottom_margin));
    KeyboardRect {
        x,
        y,
        width,
        height,
    }
}

/// Snapshot of non-sensitive UI state needed to render a lock surface.
pub struct LockView<'a> {
    pub width: u32,
    pub height: u32,
    pub is_primary: bool,
    pub stage: LockStage,
    pub time_text: &'a str,
    pub date_text: &'a str,
    pub username: &'a str,
    pub masked_chars: usize,
    pub banner: AuthBanner,
    pub cover_drag_up: f32,
    pub keyboard: &'a Keyboard,
    pub keyboard_rect: KeyboardRect,
}

/// Build an opaque `DrawList` for either the primary or a secondary lock output.
#[must_use]
pub fn build_lock_draw_list(view: &LockView<'_>, palette: &ThemePalette) -> DrawList {
    let w = view.width as f32;
    let h = view.height as f32;
    let mut list = DrawList::default();

    // Opaque deep atmospheric backdrop (never transparent on a lock surface).
    let top = Rgba {
        r: 0.05,
        g: 0.08,
        b: 0.15,
        a: 1.0,
    };
    let bottom = Rgba {
        r: 0.10,
        g: 0.14,
        b: 0.24,
        a: 1.0,
    };
    list.push_backdrop(top, bottom);
    list.push_glow(
        w * 0.5,
        h * 0.24,
        w.max(h) * 0.55,
        Rgba {
            r: palette.accent.r,
            g: palette.accent.g,
            b: palette.accent.b,
            a: 0.18,
        },
    );

    if !view.is_primary || view.stage == LockStage::Cover {
        draw_cover(&mut list, view, palette, w, h);
    } else {
        draw_password_stage(&mut list, view, palette, w, h);
    }

    list
}

fn draw_cover(list: &mut DrawList, view: &LockView<'_>, palette: &ThemePalette, w: f32, h: f32) {
    let drag_offset = view.cover_drag_up.clamp(0.0, 120.0);
    let clock_y = (h * 0.28 - drag_offset * 0.5).max(48.0);
    list.push_text(
        w * 0.5,
        clock_y,
        view.time_text,
        8,
        palette.foreground,
        TextAlign::Center,
    );
    if !view.date_text.is_empty() {
        list.push_text(
            w * 0.5,
            clock_y + 78.0,
            view.date_text,
            2,
            palette.muted,
            TextAlign::Center,
        );
    }

    let badge_w = 180.0_f32.min(w - 48.0);
    let badge_x = (w - badge_w) * 0.5;
    let badge_y = clock_y + 124.0;
    list.push_rect(
        RoundedRect::filled(
            badge_x,
            badge_y,
            badge_w,
            34.0,
            17.0,
            Rgba {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 0.10,
            },
        )
        .with_glass_border(
            Rgba {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 0.22,
            },
            1.0,
            0.14,
        ),
    );
    list.push_text(
        w * 0.5,
        badge_y + 10.0,
        if view.is_primary {
            view.username
        } else {
            "Locked"
        },
        2,
        palette.foreground,
        TextAlign::Center,
    );

    if view.is_primary {
        let cue_y = (h - 74.0 - drag_offset * 0.35).max(badge_y + 64.0);
        list.push_text(
            w * 0.5,
            cue_y,
            "Swipe up or tap to unlock",
            2,
            palette.muted,
            TextAlign::Center,
        );
        let pill_w = 132.0_f32.min(w * 0.35);
        list.push_rect(RoundedRect::filled(
            (w - pill_w) * 0.5,
            h - 24.0,
            pill_w,
            5.0,
            2.5,
            Rgba {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 0.55,
            },
        ));
    }
}

fn draw_password_stage(
    list: &mut DrawList,
    view: &LockView<'_>,
    palette: &ThemePalette,
    w: f32,
    h: f32,
) {
    let kbd = view.keyboard_rect;
    let header_space = (kbd.y as f32).max(180.0);

    // Compact clock + user header above the password card.
    let clock_y = (header_space * 0.12).clamp(24.0, 68.0);
    list.push_text(
        w * 0.5,
        clock_y,
        view.time_text,
        5,
        palette.foreground,
        TextAlign::Center,
    );
    list.push_text(
        w * 0.5,
        clock_y + 46.0,
        view.username,
        2,
        palette.muted,
        TextAlign::Center,
    );

    // Password pill card.
    let card_w = (w - 64.0).clamp(260.0, 440.0);
    let card_h = 56.0;
    let card_x = (w - card_w) * 0.5;
    let card_y = (clock_y + 92.0).min((kbd.y as f32 - card_h - 46.0).max(clock_y + 68.0));

    let border_color = match view.banner {
        AuthBanner::Denied => Rgba {
            r: 0.95,
            g: 0.38,
            b: 0.42,
            a: 0.85,
        },
        AuthBanner::Checking => Rgba {
            r: palette.accent.r,
            g: palette.accent.g,
            b: palette.accent.b,
            a: 0.85,
        },
        AuthBanner::Idle | AuthBanner::WaitRetry => Rgba {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 0.28,
        },
    };

    list.push_rect(
        RoundedRect::filled(
            card_x,
            card_y,
            card_w,
            card_h,
            18.0,
            Rgba {
                r: 0.10,
                g: 0.14,
                b: 0.24,
                a: 0.92,
            },
        )
        .with_glass_border(border_color, 1.5, 0.18)
        .with_shadow(18.0, 6.0, 0.35),
    );

    draw_password_dots(list, view.masked_chars, palette, w, card_y, card_h);

    // Status banner below password card.
    let banner_color = match view.banner {
        AuthBanner::Denied | AuthBanner::WaitRetry => Rgba {
            r: 0.98,
            g: 0.52,
            b: 0.55,
            a: 1.0,
        },
        AuthBanner::Checking => palette.accent,
        AuthBanner::Idle => palette.muted,
    };
    list.push_text(
        w * 0.5,
        card_y + card_h + 14.0,
        view.banner.text(),
        2,
        banner_color,
        TextAlign::Center,
    );

    // Keyboard tray backdrop + keys using the exact rectangles from `veilshell-lock-input`.
    draw_keyboard_tray(list, view, palette, w, h, card_y + card_h + 36.0);
}

fn draw_password_dots(
    list: &mut DrawList,
    masked_chars: usize,
    palette: &ThemePalette,
    w: f32,
    card_y: f32,
    card_h: f32,
) {
    let visible_dots = masked_chars.min(24);
    if visible_dots == 0 {
        list.push_text(
            w * 0.5,
            card_y + 20.0,
            "Password",
            2,
            Rgba {
                r: palette.muted.r,
                g: palette.muted.g,
                b: palette.muted.b,
                a: 0.65,
            },
            TextAlign::Center,
        );
        return;
    }
    let dot_d = 10.0_f32;
    let gap = 7.0_f32;
    let total_w = (visible_dots as f32) * dot_d + ((visible_dots - 1) as f32) * gap;
    let start_x = w * 0.5 - total_w * 0.5;
    let dot_y = card_y + (card_h - dot_d) * 0.5;
    for i in 0..visible_dots {
        list.push_rect(RoundedRect::filled(
            start_x + (i as f32) * (dot_d + gap),
            dot_y,
            dot_d,
            dot_d,
            dot_d * 0.5,
            palette.foreground,
        ));
    }
}

fn draw_keyboard_tray(
    list: &mut DrawList,
    view: &LockView<'_>,
    palette: &ThemePalette,
    w: f32,
    h: f32,
    min_top: f32,
) {
    let kbd = view.keyboard_rect;
    if kbd.width >= 300 && kbd.height >= 128 && (kbd.y as f32) < h {
        let tray_pad = 10.0_f32;
        list.push_rect(
            RoundedRect::filled(
                (kbd.x as f32 - tray_pad).max(4.0),
                (kbd.y as f32 - tray_pad).max(min_top),
                (kbd.width as f32 + tray_pad * 2.0).min(w - 8.0),
                (kbd.height as f32 + tray_pad * 2.0).min(h - kbd.y as f32 + tray_pad),
                20.0,
                Rgba {
                    r: 0.08,
                    g: 0.11,
                    b: 0.19,
                    a: 0.88,
                },
            )
            .with_glass_border(
                Rgba {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: 0.14,
                },
                1.0,
                0.10,
            ),
        );

        for face in view.keyboard.faces() {
            let x = (kbd.x + face.x) as f32;
            let y = (kbd.y + face.y) as f32;
            let fw = face.width as f32;
            let fh = face.height as f32;

            let is_submit = face.label == "Unlock";
            let is_action = matches!(
                face.label.as_str(),
                "Cancel" | "⌫" | "⇧" | "⬆" | "123" | "#+=" | "ABC"
            );

            let fill = if is_submit {
                Rgba {
                    r: palette.accent.r,
                    g: palette.accent.g,
                    b: palette.accent.b,
                    a: 0.90,
                }
            } else if is_action {
                Rgba {
                    r: 0.17,
                    g: 0.22,
                    b: 0.34,
                    a: 0.92,
                }
            } else {
                Rgba {
                    r: 0.22,
                    g: 0.27,
                    b: 0.40,
                    a: 0.92,
                }
            };

            let label = match face.label.as_str() {
                "⌫" => "DEL",
                "⇧" => "SHIFT",
                "⬆" => "SHIFT*",
                other => other,
            };

            list.push_rect(
                RoundedRect::filled(x, y, fw, fh, 10.0, fill).with_glass_border(
                    Rgba {
                        r: 1.0,
                        g: 1.0,
                        b: 1.0,
                        a: if is_submit { 0.35 } else { 0.16 },
                    },
                    1.0,
                    0.12,
                ),
            );
            let text_scale = if label.len() > 2 { 2 } else { 3 };
            let text_y = y + (fh - (text_scale as f32 * 7.0)) * 0.5;
            list.push_text(
                x + fw * 0.5,
                text_y,
                label,
                text_scale,
                palette.foreground,
                TextAlign::Center,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use veilshell_render::SdfSoftwareRenderer;

    #[test]
    fn renders_cover_and_password_stages_opaquely() {
        let mut keyboard = Keyboard::default();
        let rect = keyboard_rect(1200, 800);
        keyboard.configure(rect.width, rect.height);
        let palette = ThemePalette::default_dark();
        let renderer = SdfSoftwareRenderer::default();

        for stage in [LockStage::Cover, LockStage::Password] {
            let view = LockView {
                width: 1200,
                height: 800,
                is_primary: true,
                stage,
                time_text: "15:45",
                date_text: "Sunday, November 22",
                username: "user",
                masked_chars: 6,
                banner: AuthBanner::Idle,
                cover_drag_up: 0.0,
                keyboard: &keyboard,
                keyboard_rect: rect,
            };
            let list = build_lock_draw_list(&view, &palette);
            let frame = renderer.rasterize(600, 400, &list).expect("rasterizes");
            assert!(frame.rgba.chunks_exact(4).all(|px| px[3] == 255));
        }
    }
}
