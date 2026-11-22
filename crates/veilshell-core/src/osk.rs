//! Pure-Rust touch on-screen keyboard state machine, multi-layer layouts, and XKB keymap.

/// Active layout layer presented on the touch keyboard.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum OskLayer {
    #[default]
    Letters,
    Numbers,
    Symbols,
}

/// Shift modifier state for alphabetic typing.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ShiftState {
    #[default]
    Off,
    OneShot,
    CapsLock,
}

/// Logical key action assigned to an [`OskKeySpec`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OskKeyAction {
    Char(char),
    Backspace,
    Enter,
    Tab,
    Escape,
    Space,
    ArrowLeft,
    ArrowRight,
    ArrowUp,
    ArrowDown,
    ToggleTerminal,
    ToggleShift,
    ToggleCtrl,
    SwitchLayer(OskLayer),
    Dismiss,
}

/// Declarative key specification emitted for rendering and hit-testing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OskKeySpec {
    pub id: &'static str,
    pub label: &'static str,
    pub width_units: f32,
    pub action: OskKeyAction,
    pub is_modifier: bool,
    pub is_accent: bool,
}

/// Event emitted to the Wayland input-method / virtual-keyboard backend after a key tap.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OskEmit {
    None,
    Text {
        text: String,
        evdev_code: Option<u32>,
        shift: bool,
        ctrl: bool,
    },
    SpecialKey {
        evdev_code: u32,
        ctrl: bool,
    },
    Dismiss,
}

/// Pure-Rust on-screen keyboard state (active layout, Shift/Caps, one-shot Ctrl).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct OskState {
    pub layer: OskLayer,
    pub shift: ShiftState,
    pub ctrl: bool,
    pub terminal: bool,
}

impl OskState {
    /// Build the key rows for the current layout and optional terminal shortcuts.
    #[must_use]
    pub fn rows(self) -> Vec<Vec<OskKeySpec>> {
        match self.layer {
            OskLayer::Letters => {
                let mut rows = self.letters_rows();
                if self.terminal {
                    rows.insert(0, terminal_utility_row());
                }
                rows
            }
            OskLayer::Numbers => numbers_rows(),
            OskLayer::Symbols => symbols_rows(),
        }
    }

    /// Whether a held key should emit repeating, balanced key strokes.
    #[must_use]
    pub fn can_repeat(key_id: &str) -> bool {
        matches!(key_id, "del" | "left" | "right" | "up" | "down")
    }

    /// Process tapping key `key_id` and emit the resulting text or virtual-key event.
    pub fn press(&mut self, key_id: &str) -> OskEmit {
        let Some(spec) = self
            .rows()
            .into_iter()
            .flatten()
            .find(|key| key.id == key_id)
        else {
            return OskEmit::None;
        };

        match spec.action {
            OskKeyAction::ToggleShift => {
                self.shift = match self.shift {
                    ShiftState::Off => ShiftState::OneShot,
                    ShiftState::OneShot => ShiftState::CapsLock,
                    ShiftState::CapsLock => ShiftState::Off,
                };
                OskEmit::None
            }
            OskKeyAction::ToggleCtrl => {
                self.ctrl = !self.ctrl;
                OskEmit::None
            }
            OskKeyAction::ToggleTerminal => {
                self.terminal = !self.terminal;
                OskEmit::None
            }
            OskKeyAction::SwitchLayer(next) => {
                self.layer = next;
                if self.shift == ShiftState::OneShot {
                    self.shift = ShiftState::Off;
                }
                OskEmit::None
            }
            OskKeyAction::Char(base) => {
                let ch = if self.layer == OskLayer::Letters && self.shift != ShiftState::Off {
                    base.to_ascii_uppercase()
                } else {
                    base
                };
                if self.shift == ShiftState::OneShot {
                    self.shift = ShiftState::Off;
                }
                let ctrl = self.ctrl;
                self.ctrl = false;
                let (evdev_code, shift) = match char_to_evdev(ch) {
                    Some((code, s)) => (Some(code), s),
                    None => (None, false),
                };
                OskEmit::Text {
                    text: ch.to_string(),
                    evdev_code,
                    shift,
                    ctrl,
                }
            }
            OskKeyAction::Space => {
                let ctrl = self.ctrl;
                self.ctrl = false;
                OskEmit::Text {
                    text: " ".to_owned(),
                    evdev_code: Some(57),
                    shift: false,
                    ctrl,
                }
            }
            OskKeyAction::Backspace => self.emit_special(14),
            OskKeyAction::Enter => self.emit_special(28),
            OskKeyAction::Tab => self.emit_special(15),
            OskKeyAction::Escape => self.emit_special(1),
            OskKeyAction::ArrowLeft => self.emit_special(105),
            OskKeyAction::ArrowRight => self.emit_special(106),
            OskKeyAction::ArrowUp => self.emit_special(103),
            OskKeyAction::ArrowDown => self.emit_special(108),
            OskKeyAction::Dismiss => OskEmit::Dismiss,
        }
    }

    fn emit_special(&mut self, evdev_code: u32) -> OskEmit {
        let ctrl = self.ctrl;
        self.ctrl = false;
        OskEmit::SpecialKey { evdev_code, ctrl }
    }

    fn letters_rows(self) -> Vec<Vec<OskKeySpec>> {
        let shf_label = if self.shift == ShiftState::CapsLock {
            "CAPS"
        } else {
            "SHF"
        };
        let row0_keys = [
            ("q", "Q"),
            ("w", "W"),
            ("e", "E"),
            ("r", "R"),
            ("t", "T"),
            ("y", "Y"),
            ("u", "U"),
            ("i", "I"),
            ("o", "O"),
            ("p", "P"),
        ];
        let row1_keys = [
            ("a", "A"),
            ("s", "S"),
            ("d", "D"),
            ("f", "F"),
            ("g", "G"),
            ("h", "H"),
            ("j", "J"),
            ("k", "K"),
            ("l", "L"),
        ];
        let row2_letters = [
            ("z", "Z"),
            ("x", "X"),
            ("c", "C"),
            ("v", "V"),
            ("b", "B"),
            ("n", "N"),
            ("m", "M"),
        ];

        let row0 = row0_keys
            .into_iter()
            .map(|(id, label)| char_key(id, label, id.as_bytes()[0] as char))
            .collect();
        let row1 = row1_keys
            .into_iter()
            .map(|(id, label)| char_key(id, label, id.as_bytes()[0] as char))
            .collect();

        let mut row2 = Vec::with_capacity(row2_letters.len() + 2);
        row2.push(mod_key("shf", shf_label, 1.4, OskKeyAction::ToggleShift));
        for (id, label) in row2_letters {
            row2.push(char_key(id, label, id.as_bytes()[0] as char));
        }
        row2.push(action_key(
            "del",
            "DEL",
            1.4,
            OskKeyAction::Backspace,
            false,
        ));

        let row3 = vec![
            action_key(
                "123",
                "123",
                1.3,
                OskKeyAction::SwitchLayer(OskLayer::Numbers),
                false,
            ),
            mod_key("ctl", "CTL", 1.1, OskKeyAction::ToggleCtrl),
            mod_key("term", "TERM", 1.55, OskKeyAction::ToggleTerminal),
            char_key(",", ",", ','),
            action_key("space", "SPACE", 4.0, OskKeyAction::Space, false),
            char_key(".", ".", '.'),
            action_key("hide", "HIDE", 1.2, OskKeyAction::Dismiss, false),
            action_key("enter", "ENTER", 1.6, OskKeyAction::Enter, true),
        ];

        vec![row0, row1, row2, row3]
    }
}

#[cfg(test)]
mod terminal_tests {
    use super::*;

    #[test]
    fn terminal_row_sends_shortcuts_without_leaving_letters() {
        let mut osk = OskState::default();
        assert_eq!(osk.press("up"), OskEmit::None);
        assert_eq!(osk.press("term"), OskEmit::None);
        assert!(osk.terminal);
        assert_eq!(osk.layer, OskLayer::Letters);
        for (id, evdev_code) in [
            ("esc", 1),
            ("tab", 15),
            ("left", 105),
            ("up", 103),
            ("down", 108),
            ("right", 106),
        ] {
            assert_eq!(
                osk.press(id),
                OskEmit::SpecialKey {
                    evdev_code,
                    ctrl: false
                }
            );
        }
        osk.press("term");
        assert!(!osk.terminal);
        assert_eq!(osk.press("down"), OskEmit::None);
    }
}

fn terminal_utility_row() -> Vec<OskKeySpec> {
    [
        ("esc", "ESC", OskKeyAction::Escape),
        ("tab", "TAB", OskKeyAction::Tab),
        ("left", "<-", OskKeyAction::ArrowLeft),
        ("up", "UP", OskKeyAction::ArrowUp),
        ("down", "DOWN", OskKeyAction::ArrowDown),
        ("right", "->", OskKeyAction::ArrowRight),
    ]
    .into_iter()
    .map(|(id, label, action)| action_key(id, label, 1.0, action, false))
    .collect()
}

fn numbers_rows() -> Vec<Vec<OskKeySpec>> {
    vec![
        vec![
            char_key("1", "1", '1'),
            char_key("2", "2", '2'),
            char_key("3", "3", '3'),
            char_key("4", "4", '4'),
            char_key("5", "5", '5'),
            char_key("6", "6", '6'),
            char_key("7", "7", '7'),
            char_key("8", "8", '8'),
            char_key("9", "9", '9'),
            char_key("0", "0", '0'),
        ],
        vec![
            char_key("@", "@", '@'),
            char_key("#", "#", '#'),
            char_key("$", "$", '$'),
            char_key("%", "%", '%'),
            char_key("&", "&", '&'),
            char_key("-", "-", '-'),
            char_key("+", "+", '+'),
            char_key("(", "(", '('),
            char_key(")", ")", ')'),
            char_key("/", "/", '/'),
        ],
        vec![
            action_key(
                "sym",
                "SYM",
                1.4,
                OskKeyAction::SwitchLayer(OskLayer::Symbols),
                false,
            ),
            char_key("*", "*", '*'),
            char_key("\"", "\"", '"'),
            char_key("'", "'", '\''),
            char_key(":", ":", ':'),
            char_key(";", ";", ';'),
            char_key("!", "!", '!'),
            char_key("?", "?", '?'),
            action_key("del", "DEL", 1.4, OskKeyAction::Backspace, false),
        ],
        bottom_utility_row(),
    ]
}

fn symbols_rows() -> Vec<Vec<OskKeySpec>> {
    vec![
        vec![
            char_key("~", "~", '~'),
            char_key("`", "`", '`'),
            char_key("|", "|", '|'),
            char_key("^", "^", '^'),
            char_key("=", "=", '='),
            char_key("{", "{", '{'),
            char_key("}", "}", '}'),
            char_key("[", "[", '['),
            char_key("]", "]", ']'),
            char_key("\\", "\\", '\\'),
        ],
        vec![
            char_key("!", "!", '!'),
            char_key("@", "@", '@'),
            char_key("#", "#", '#'),
            char_key("$", "$", '$'),
            char_key("%", "%", '%'),
            char_key("_", "_", '_'),
            char_key("<", "<", '<'),
            char_key(">", ">", '>'),
            char_key("(", "(", '('),
            char_key(")", ")", ')'),
        ],
        vec![
            action_key(
                "123",
                "123",
                1.4,
                OskKeyAction::SwitchLayer(OskLayer::Numbers),
                false,
            ),
            char_key("+", "+", '+'),
            char_key("-", "-", '-'),
            char_key("*", "*", '*'),
            char_key("/", "/", '/'),
            char_key("=", "=", '='),
            char_key("?", "?", '?'),
            char_key(":", ":", ':'),
            action_key("del", "DEL", 1.4, OskKeyAction::Backspace, false),
        ],
        bottom_utility_row(),
    ]
}

fn bottom_utility_row() -> Vec<OskKeySpec> {
    vec![
        action_key(
            "abc",
            "ABC",
            1.3,
            OskKeyAction::SwitchLayer(OskLayer::Letters),
            false,
        ),
        action_key("esc", "ESC", 1.1, OskKeyAction::Escape, false),
        action_key("tab", "TAB", 1.1, OskKeyAction::Tab, false),
        action_key("space", "SPACE", 3.2, OskKeyAction::Space, false),
        action_key("left", "<-", 0.9, OskKeyAction::ArrowLeft, false),
        action_key("right", "->", 0.9, OskKeyAction::ArrowRight, false),
        action_key("hide", "HIDE", 1.1, OskKeyAction::Dismiss, false),
        action_key("enter", "ENTER", 1.4, OskKeyAction::Enter, true),
    ]
}

const fn char_key(id: &'static str, label: &'static str, ch: char) -> OskKeySpec {
    OskKeySpec {
        id,
        label,
        width_units: 1.0,
        action: OskKeyAction::Char(ch),
        is_modifier: false,
        is_accent: false,
    }
}

const fn mod_key(
    id: &'static str,
    label: &'static str,
    width_units: f32,
    action: OskKeyAction,
) -> OskKeySpec {
    OskKeySpec {
        id,
        label,
        width_units,
        action,
        is_modifier: true,
        is_accent: false,
    }
}

const fn action_key(
    id: &'static str,
    label: &'static str,
    width_units: f32,
    action: OskKeyAction,
    is_accent: bool,
) -> OskKeySpec {
    OskKeySpec {
        id,
        label,
        width_units,
        action,
        is_modifier: false,
        is_accent,
    }
}

/// Map an `ASCII` character to its `Linux` `evdev` keycode and whether `Shift` is required.
#[must_use]
pub const fn char_to_evdev(ch: char) -> Option<(u32, bool)> {
    match ch {
        '1' => Some((2, false)),
        '!' => Some((2, true)),
        '2' => Some((3, false)),
        '@' => Some((3, true)),
        '3' => Some((4, false)),
        '#' => Some((4, true)),
        '4' => Some((5, false)),
        '$' => Some((5, true)),
        '5' => Some((6, false)),
        '%' => Some((6, true)),
        '6' => Some((7, false)),
        '^' => Some((7, true)),
        '7' => Some((8, false)),
        '&' => Some((8, true)),
        '8' => Some((9, false)),
        '*' => Some((9, true)),
        '9' => Some((10, false)),
        '(' => Some((10, true)),
        '0' => Some((11, false)),
        ')' => Some((11, true)),
        '-' => Some((12, false)),
        '_' => Some((12, true)),
        '=' => Some((13, false)),
        '+' => Some((13, true)),
        'q' => Some((16, false)),
        'Q' => Some((16, true)),
        'w' => Some((17, false)),
        'W' => Some((17, true)),
        'e' => Some((18, false)),
        'E' => Some((18, true)),
        'r' => Some((19, false)),
        'R' => Some((19, true)),
        't' => Some((20, false)),
        'T' => Some((20, true)),
        'y' => Some((21, false)),
        'Y' => Some((21, true)),
        'u' => Some((22, false)),
        'U' => Some((22, true)),
        'i' => Some((23, false)),
        'I' => Some((23, true)),
        'o' => Some((24, false)),
        'O' => Some((24, true)),
        'p' => Some((25, false)),
        'P' => Some((25, true)),
        '[' => Some((26, false)),
        '{' => Some((26, true)),
        ']' => Some((27, false)),
        '}' => Some((27, true)),
        'a' => Some((30, false)),
        'A' => Some((30, true)),
        's' => Some((31, false)),
        'S' => Some((31, true)),
        'd' => Some((32, false)),
        'D' => Some((32, true)),
        'f' => Some((33, false)),
        'F' => Some((33, true)),
        'g' => Some((34, false)),
        'G' => Some((34, true)),
        'h' => Some((35, false)),
        'H' => Some((35, true)),
        'j' => Some((36, false)),
        'J' => Some((36, true)),
        'k' => Some((37, false)),
        'K' => Some((37, true)),
        'l' => Some((38, false)),
        'L' => Some((38, true)),
        ';' => Some((39, false)),
        ':' => Some((39, true)),
        '\'' => Some((40, false)),
        '"' => Some((40, true)),
        '`' => Some((41, false)),
        '~' => Some((41, true)),
        '\\' => Some((43, false)),
        '|' => Some((43, true)),
        'z' => Some((44, false)),
        'Z' => Some((44, true)),
        'x' => Some((45, false)),
        'X' => Some((45, true)),
        'c' => Some((46, false)),
        'C' => Some((46, true)),
        'v' => Some((47, false)),
        'V' => Some((47, true)),
        'b' => Some((48, false)),
        'B' => Some((48, true)),
        'n' => Some((49, false)),
        'N' => Some((49, true)),
        'm' => Some((50, false)),
        'M' => Some((50, true)),
        ',' => Some((51, false)),
        '<' => Some((51, true)),
        '.' => Some((52, false)),
        '>' => Some((52, true)),
        '/' => Some((53, false)),
        '?' => Some((53, true)),
        ' ' => Some((57, false)),
        _ => None,
    }
}

/// Self-contained US-QWERTY `XKB` keymap uploaded to `zwp_virtual_keyboard_v1`.
///
/// Includes all four `XKB` required types (`ONE_LEVEL`, `TWO_LEVEL`, `ALPHABETIC`, `KEYPAD`)
/// and avoids invalid `virtual_modifiers None` declarations so both `libxkbcommon` and
/// `Xwayland`/`xkbcomp` compile and apply it without warnings or crashes.
pub const US_QWERTY_XKB_KEYMAP: &str = r#"xkb_keymap {
  xkb_keycodes "veilshell" {
    minimum = 8;
    maximum = 255;
    <ESC>  = 9;
    <AE01> = 10; <AE02> = 11; <AE03> = 12; <AE04> = 13; <AE05> = 14;
    <AE06> = 15; <AE07> = 16; <AE08> = 17; <AE09> = 18; <AE10> = 19;
    <AE11> = 20; <AE12> = 21; <BKSP> = 22; <TAB>  = 23;
    <AD01> = 24; <AD02> = 25; <AD03> = 26; <AD04> = 27; <AD05> = 28;
    <AD06> = 29; <AD07> = 30; <AD08> = 31; <AD09> = 32; <AD10> = 33;
    <AD11> = 34; <AD12> = 35; <RTRN> = 36; <LCTL> = 37;
    <AC01> = 38; <AC02> = 39; <AC03> = 40; <AC04> = 41; <AC05> = 42;
    <AC06> = 43; <AC07> = 44; <AC08> = 45; <AC09> = 46; <AC10> = 47;
    <AC11> = 48; <TLDE> = 49; <LFSH> = 50; <BKSL> = 51;
    <AB01> = 52; <AB02> = 53; <AB03> = 54; <AB04> = 55; <AB05> = 56;
    <AB06> = 57; <AB07> = 58; <AB08> = 59; <AB09> = 60; <AB10> = 61;
    <SPCE> = 65; <CAPS> = 66;
    <UP>   = 111; <LEFT> = 113; <RGHT> = 114; <DOWN> = 116;
    indicator 1 = "Caps Lock";
  };
  xkb_types "veilshell" {
    virtual_modifiers NumLock;
    type "ONE_LEVEL" {
      modifiers = none;
      level_name[Level1] = "Any";
    };
    type "TWO_LEVEL" {
      modifiers = Shift;
      map[Shift] = Level2;
      level_name[Level1] = "Base";
      level_name[Level2] = "Shift";
    };
    type "ALPHABETIC" {
      modifiers = Shift+Lock;
      map[Shift] = Level2;
      map[Lock] = Level2;
      level_name[Level1] = "Base";
      level_name[Level2] = "Caps";
    };
    type "KEYPAD" {
      modifiers = Shift+NumLock;
      map[Shift] = Level2;
      map[NumLock] = Level2;
      level_name[Level1] = "Base";
      level_name[Level2] = "Number";
    };
  };
  xkb_compatibility "veilshell" {
    virtual_modifiers NumLock;
    interpret.useModMapMods = AnyLevel;
    interpret.repeat = False;
    interpret Any+AnyOf(all) { action = SetMods(modifiers=modMapMods,clearLocks); };
    indicator "Caps Lock" {
      whichModState = locked;
      modifiers = Lock;
    };
  };
  xkb_symbols "veilshell" {
    name[group1] = "English (US)";
    key <ESC>  { [ Escape ] };
    key <AE01> { [ 1, exclam ] };
    key <AE02> { [ 2, at ] };
    key <AE03> { [ 3, numbersign ] };
    key <AE04> { [ 4, dollar ] };
    key <AE05> { [ 5, percent ] };
    key <AE06> { [ 6, asciicircum ] };
    key <AE07> { [ 7, ampersand ] };
    key <AE08> { [ 8, asterisk ] };
    key <AE09> { [ 9, parenleft ] };
    key <AE10> { [ 0, parenright ] };
    key <AE11> { [ minus, underscore ] };
    key <AE12> { [ equal, plus ] };
    key <BKSP> { [ BackSpace ] };
    key <TAB>  { [ Tab ] };
    key <AD01> { [ q, Q ] };
    key <AD02> { [ w, W ] };
    key <AD03> { [ e, E ] };
    key <AD04> { [ r, R ] };
    key <AD05> { [ t, T ] };
    key <AD06> { [ y, Y ] };
    key <AD07> { [ u, U ] };
    key <AD08> { [ i, I ] };
    key <AD09> { [ o, O ] };
    key <AD10> { [ p, P ] };
    key <AD11> { [ bracketleft, braceleft ] };
    key <AD12> { [ bracketright, braceright ] };
    key <RTRN> { [ Return ] };
    key <LCTL> { [ Control_L ] };
    key <AC01> { [ a, A ] };
    key <AC02> { [ s, S ] };
    key <AC03> { [ d, D ] };
    key <AC04> { [ f, F ] };
    key <AC05> { [ g, G ] };
    key <AC06> { [ h, H ] };
    key <AC07> { [ j, J ] };
    key <AC08> { [ k, K ] };
    key <AC09> { [ l, L ] };
    key <AC10> { [ semicolon, colon ] };
    key <AC11> { [ apostrophe, quotedbl ] };
    key <TLDE> { [ grave, asciitilde ] };
    key <LFSH> { [ Shift_L ] };
    key <BKSL> { [ backslash, bar ] };
    key <AB01> { [ z, Z ] };
    key <AB02> { [ x, X ] };
    key <AB03> { [ c, C ] };
    key <AB04> { [ v, V ] };
    key <AB05> { [ b, B ] };
    key <AB06> { [ n, N ] };
    key <AB07> { [ m, M ] };
    key <AB08> { [ comma, less ] };
    key <AB09> { [ period, greater ] };
    key <AB10> { [ slash, question ] };
    key <SPCE> { [ space ] };
    key <CAPS> { [ Caps_Lock ] };
    key <UP>   { [ Up ] };
    key <LEFT> { [ Left ] };
    key <RGHT> { [ Right ] };
    key <DOWN> { [ Down ] };
    modifier_map Shift { <LFSH> };
    modifier_map Lock { <CAPS> };
    modifier_map Control { <LCTL> };
  };
};
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shift_one_shot_auto_releases_and_caps_locks() {
        let mut osk = OskState::default();
        assert_eq!(
            osk.press("a"),
            OskEmit::Text {
                text: "a".into(),
                evdev_code: Some(30),
                shift: false,
                ctrl: false,
            }
        );

        assert_eq!(osk.press("shf"), OskEmit::None);
        assert_eq!(osk.shift, ShiftState::OneShot);
        assert_eq!(
            osk.press("a"),
            OskEmit::Text {
                text: "A".into(),
                evdev_code: Some(30),
                shift: true,
                ctrl: false,
            }
        );
        assert_eq!(osk.shift, ShiftState::Off);

        // Two taps enter CapsLock and stay latched until toggled off.
        assert_eq!(osk.press("shf"), OskEmit::None);
        assert_eq!(osk.press("shf"), OskEmit::None);
        assert_eq!(osk.shift, ShiftState::CapsLock);
        assert_eq!(
            osk.press("z"),
            OskEmit::Text {
                text: "Z".into(),
                evdev_code: Some(44),
                shift: true,
                ctrl: false,
            }
        );
        assert_eq!(osk.shift, ShiftState::CapsLock);
        assert_eq!(osk.press("shf"), OskEmit::None);
        assert_eq!(osk.shift, ShiftState::Off);
    }

    #[test]
    fn numbers_and_symbols_layers_emit_text_and_evdev() {
        let mut osk = OskState::default();
        assert_eq!(osk.press("123"), OskEmit::None);
        assert_eq!(osk.layer, OskLayer::Numbers);
        assert_eq!(
            osk.press("@"),
            OskEmit::Text {
                text: "@".into(),
                evdev_code: Some(3),
                shift: true,
                ctrl: false,
            }
        );

        assert_eq!(osk.press("sym"), OskEmit::None);
        assert_eq!(osk.layer, OskLayer::Symbols);
        assert_eq!(
            osk.press("~"),
            OskEmit::Text {
                text: "~".into(),
                evdev_code: Some(41),
                shift: true,
                ctrl: false,
            }
        );
        assert_eq!(
            osk.press("left"),
            OskEmit::SpecialKey {
                evdev_code: 105,
                ctrl: false,
            }
        );
        assert_eq!(osk.press("hide"), OskEmit::Dismiss);
    }

    #[test]
    fn ctrl_modifier_applies_once_and_resets() {
        let mut osk = OskState::default();
        assert_eq!(osk.press("ctl"), OskEmit::None);
        assert!(osk.ctrl);
        assert_eq!(
            osk.press("c"),
            OskEmit::Text {
                text: "c".into(),
                evdev_code: Some(46),
                shift: false,
                ctrl: true,
            }
        );
        assert!(!osk.ctrl);

        assert_eq!(osk.press("ctl"), OskEmit::None);
        assert_eq!(
            osk.press("del"),
            OskEmit::SpecialKey {
                evdev_code: 14,
                ctrl: true,
            }
        );
        assert!(!osk.ctrl);
    }

    #[test]
    fn every_printable_osk_key_maps_to_valid_evdev_code() {
        for layer in [OskLayer::Letters, OskLayer::Numbers, OskLayer::Symbols] {
            let state = OskState {
                layer,
                ..OskState::default()
            };
            for key in state.rows().into_iter().flatten() {
                if let OskKeyAction::Char(ch) = key.action {
                    assert!(
                        char_to_evdev(ch).is_some(),
                        "missing evdev mapping for '{ch}' in {layer:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn xkb_keymap_includes_all_four_required_types_and_no_none_vmod() {
        assert!(!US_QWERTY_XKB_KEYMAP.contains("virtual_modifiers None"));
        for required in [
            "\"ONE_LEVEL\"",
            "\"TWO_LEVEL\"",
            "\"ALPHABETIC\"",
            "\"KEYPAD\"",
        ] {
            assert!(
                US_QWERTY_XKB_KEYMAP.contains(required),
                "missing required XKB type {required}"
            );
        }
    }
}
