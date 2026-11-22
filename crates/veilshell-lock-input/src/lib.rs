//! Locker-local password keyboard layout and touch handling. No credential
//! buffer, authentication, Wayland connection, or unlock capability.
//!
//! Use one instance for a selected seat and lock surface. The caller must
//! validate their identities, translate surface-local positions to the keyboard
//! rectangle's origin, and report downs even outside that rectangle. Route all
//! motion/up events for the tracked IDs and handle actions only at a touch frame.

use std::collections::BTreeSet;

/// An intent for the locker's private password editor, not a keycode to inject.
/// `Submit` asks to authenticate; `Cancel` asks to discard the input/attempt.
#[derive(PartialEq, Eq)]
#[must_use]
pub enum Action {
    Character(char),
    Backspace,
    Cancel,
    Submit,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Letters,
    Numbers,
    Symbols,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Key {
    Character(char),
    Backspace,
    Cancel,
    Submit,
    Shift,
    NextPage,
}

/// Geometry and legend for drawing a key; hit testing uses the same rectangles.
pub struct KeyFace {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub label: String,
}

#[derive(Clone, Copy)]
struct Cell {
    key: Key,
    column: u8,
    row: u8,
    span: u8,
}

impl Cell {
    fn bounds(self, column_width: f64, row_height: f64) -> (f64, f64, f64, f64) {
        // All sides have a two-logical-pixel inset. Gap touches do nothing.
        (
            f64::from(self.column) * column_width + 2.0,
            f64::from(self.row) * row_height + 2.0,
            f64::from(self.span) * column_width - 4.0,
            row_height - 4.0,
        )
    }
}

/// A single-touch keyboard with an explicit password-submit key. Covers printable
/// ASCII; the locker still needs physical input and, eventually, localized OSKs.
pub struct Keyboard {
    page: Page,
    shift: bool,
    column_width: f64,
    row_height: f64,
    contacts: BTreeSet<i32>,
    armed: Option<(i32, Cell)>,
    pending: Option<Key>,
}

impl Default for Keyboard {
    fn default() -> Self {
        Self {
            page: Page::Letters,
            shift: false,
            column_width: 0.0,
            row_height: 0.0,
            contacts: BTreeSet::new(),
            armed: None,
            pending: None,
        }
    }
}

impl Keyboard {
    /// Configure in logical pixels. Call even on a same-size reconfigure if the
    /// surface/input mapping changed. A too-small rectangle is inactive.
    pub fn configure(&mut self, width: u32, height: u32) {
        self.disarm();
        if width >= 300 && height >= 128 {
            self.column_width = f64::from(width) / 10.0;
            self.row_height = f64::from(height) / 4.0;
        } else {
            self.column_width = 0.0;
            self.row_height = 0.0;
        }
    }

    pub fn faces(&self) -> impl Iterator<Item = KeyFace> + '_ {
        self.cells().map(|cell| {
            let (x, y, width, height) = cell.bounds(self.column_width, self.row_height);
            let label = match cell.key {
                Key::Character(' ') => "Space",
                Key::Character(c) => {
                    return KeyFace {
                        x,
                        y,
                        width,
                        height,
                        label: c.to_string(),
                    };
                }
                Key::Backspace => "⌫",
                Key::Cancel => "Cancel",
                Key::Submit => "Unlock",
                Key::Shift if self.shift => "⬆",
                Key::Shift => "⇧",
                Key::NextPage => match self.page {
                    Page::Letters => "123",
                    Page::Numbers => "#+=",
                    Page::Symbols => "ABC",
                },
            };
            KeyFace {
                x,
                y,
                width,
                height,
                label: label.into(),
            }
        })
    }

    /// Start tracking a contact. A second contact invalidates the tap rather
    /// than allowing either finger to type or submit.
    pub fn down(&mut self, id: i32, x: f64, y: f64) {
        let can_arm = self.contacts.is_empty() && self.pending.is_none();
        self.disarm();
        self.contacts.insert(id);
        if can_arm {
            let armed = self
                .cells()
                .find(|cell| self.contains(*cell, x, y))
                .map(|cell| (id, cell));
            self.armed = armed;
        }
    }

    /// Dragging outside the starting key permanently disarms that contact.
    pub fn motion(&mut self, id: i32, x: f64, y: f64) {
        if let Some((armed_id, cell)) = self.armed {
            if armed_id == id && !self.contains(cell, x, y) {
                self.armed = None;
            }
        }
    }

    pub fn up(&mut self, id: i32) {
        if !self.contacts.remove(&id) {
            return;
        }
        if let Some((armed_id, cell)) = self.armed.take() {
            if id == armed_id {
                self.pending = Some(cell.key);
            }
        }
    }

    /// Commit a completed tap at `wl_touch.frame`. Return at most one intent.
    pub fn frame(&mut self) -> Option<Action> {
        match self.pending.take()? {
            Key::Character(c) => {
                self.shift = false;
                Some(Action::Character(c))
            }
            Key::Backspace => Some(Action::Backspace),
            Key::Cancel => {
                self.page = Page::Letters;
                self.shift = false;
                Some(Action::Cancel)
            }
            Key::Submit => Some(Action::Submit),
            Key::Shift => {
                self.shift = !self.shift;
                None
            }
            Key::NextPage => {
                self.page = match self.page {
                    Page::Letters => Page::Numbers,
                    Page::Numbers => Page::Symbols,
                    Page::Symbols => Page::Letters,
                };
                self.shift = false;
                None
            }
        }
    }

    /// Discard any tap but keep tracking fingers already down. Use when the
    /// editor loses eligibility (e.g. authentication starts or a UI changes).
    pub fn disarm(&mut self) {
        self.armed = None;
        self.pending = None;
    }

    /// Handle `wl_touch.cancel` or retire the bound input stream on seat/surface
    /// loss. Never route subsequent events from a retired stream here.
    pub fn cancel_touches(&mut self) {
        self.disarm();
        self.contacts.clear();
        self.page = Page::Letters;
        self.shift = false;
    }

    fn contains(&self, cell: Cell, x: f64, y: f64) -> bool {
        let (left, top, width, height) = cell.bounds(self.column_width, self.row_height);
        x >= left && x < left + width && y >= top && y < top + height
    }

    fn cells(&self) -> impl Iterator<Item = Cell> + '_ {
        let rows: [&[u8]; 3] = match self.page {
            Page::Letters => [b"qwertyuiop", b" asdfghjkl", b"  zxcvbnm"],
            Page::Numbers => [b"1234567890", b"!@#$%^&*()", b"-_=+[]{}?"],
            Page::Symbols => [b"~`|\\;:'\",.", b"<>/", b""],
        };
        (0..4).flat_map(move |row| {
            (0..10).filter_map(move |column| {
                if self.column_width == 0.0 {
                    return None;
                }
                let (key, span) = match (row, column) {
                    (3, 0) => (Key::NextPage, 1),
                    (3, 1) => (Key::Backspace, 1),
                    (3, 2) => (Key::Character(' '), 4),
                    (3, 6) => (Key::Cancel, 2),
                    (3, 8) => (Key::Submit, 2),
                    (3, _) => return None,
                    (2, 0) if self.page == Page::Letters => (Key::Shift, 2),
                    _ => {
                        let byte = *rows[usize::from(row)].get(usize::from(column))?;
                        if byte == b' ' {
                            return None;
                        }
                        let c = char::from(byte);
                        (
                            Key::Character(if self.shift {
                                c.to_ascii_uppercase()
                            } else {
                                c
                            }),
                            1,
                        )
                    }
                };
                Some(Cell {
                    key,
                    column,
                    row,
                    span,
                })
            })
        })
    }
}

#[cfg(test)]
mod tests;
