//! Pure `.desktop` entry parser and freedesktop `Exec` field-code cleaner (zero I/O).

/// Parsed freedesktop `.desktop` application metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesktopApp {
    pub id: String,
    pub name: String,
    pub exec: String,
    pub icon: Option<String>,
    pub terminal: bool,
    pub categories: Vec<String>,
}

/// Parse a `.desktop` file string into a [`DesktopApp`], filtering hidden/non-app entries.
///
/// Only keys inside the `[Desktop Entry]` section are inspected; subsequent
/// `[Desktop Action ...]` sections are ignored so action names/execs never overwrite
/// the main application entry.
#[must_use]
pub fn parse_desktop_entry(src: &str, id: &str) -> Option<DesktopApp> {
    let mut in_desktop_entry = false;
    let mut entry_type: Option<&str> = None;
    let mut name: Option<String> = None;
    let mut exec: Option<String> = None;
    let mut icon: Option<String> = None;
    let mut terminal = false;
    let mut categories = Vec::new();

    for raw_line in src.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            let section = &line[1..line.len() - 1];
            if section == "Desktop Entry" {
                in_desktop_entry = true;
            } else if in_desktop_entry {
                break;
            }
            continue;
        }
        if !in_desktop_entry {
            continue;
        }
        let Some((key, val)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let val = val.trim();

        match key {
            "Type" => entry_type = Some(val),
            "NoDisplay" | "Hidden" if val.eq_ignore_ascii_case("true") => return None,
            "Name" if name.is_none() && !val.is_empty() => name = Some(val.to_owned()),
            "Exec" if exec.is_none() => {
                let cleaned = strip_exec_field_codes(val);
                if !cleaned.is_empty() {
                    exec = Some(cleaned);
                }
            }
            "Icon" if icon.is_none() && !val.is_empty() => icon = Some(val.to_owned()),
            "Terminal" => terminal = val.eq_ignore_ascii_case("true"),
            "Categories" => {
                categories = val
                    .split(';')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(ToOwned::to_owned)
                    .collect();
            }
            _ => {}
        }
    }

    if entry_type != Some("Application") {
        return None;
    }

    Some(DesktopApp {
        id: id.to_owned(),
        name: name?,
        exec: exec?,
        icon,
        terminal,
        categories,
    })
}

/// Remove freedesktop `Exec=` field codes (`%f`, `%F`, `%u`, `%U`, etc.) and normalize whitespace.
#[must_use]
pub fn strip_exec_field_codes(exec: &str) -> String {
    let mut cleaned = String::with_capacity(exec.len());
    let mut chars = exec.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '%' {
            match chars.peek().copied() {
                Some('%') => {
                    cleaned.push('%');
                    chars.next();
                }
                Some(
                    'f' | 'F' | 'u' | 'U' | 'd' | 'D' | 'n' | 'N' | 'i' | 'c' | 'k' | 'v' | 'm',
                ) => {
                    chars.next();
                }
                _ => cleaned.push('%'),
            }
        } else {
            cleaned.push(ch);
        }
    }

    cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Build a compact 2-4 character uppercase ASCII badge from an application name.
#[must_use]
pub fn app_badge(name: &str) -> String {
    let words: Vec<&str> = name
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();

    if words.len() >= 2 {
        let mut initials = String::new();
        for w in words.iter().take(4) {
            if let Some(ch) = w.chars().next() {
                initials.push(ch.to_ascii_uppercase());
            }
        }
        if initials.len() >= 2 {
            return initials;
        }
    }

    let ascii: String = name
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_uppercase())
        .take(4)
        .collect();
    if ascii.is_empty() {
        "APP".to_owned()
    } else {
        ascii
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FOOT_DESKTOP: &str = r"[Desktop Entry]
Type=Application
Exec=foot
Icon=foot
Terminal=false
Categories=System;TerminalEmulator;
Name=Foot

[Desktop Action NewWindow]
Name=New Window
Exec=foot --hold
";

    #[test]
    fn parses_valid_desktop_entry_and_ignores_actions() {
        let app = parse_desktop_entry(FOOT_DESKTOP, "foot.desktop").expect("parsed");
        assert_eq!(app.id, "foot.desktop");
        assert_eq!(app.name, "Foot");
        assert_eq!(app.exec, "foot");
        assert_eq!(app.icon.as_deref(), Some("foot"));
        assert!(!app.terminal);
        assert_eq!(app.categories, vec!["System", "TerminalEmulator"]);
    }

    #[test]
    fn filters_nodisplay_and_hidden_entries() {
        let hidden =
            "[Desktop Entry]\nType=Application\nName=Secret\nExec=secret\nNoDisplay=true\n";
        assert!(parse_desktop_entry(hidden, "secret.desktop").is_none());

        let link = "[Desktop Entry]\nType=Link\nName=Docs\nURL=https://example.com\n";
        assert!(parse_desktop_entry(link, "docs.desktop").is_none());
    }

    #[test]
    fn strips_exec_field_codes_and_preserves_flags() {
        assert_eq!(
            strip_exec_field_codes("chromium %U --ozone-platform=wayland"),
            "chromium --ozone-platform=wayland"
        );
        assert_eq!(strip_exec_field_codes("printf %%s %f"), "printf %s");
    }

    #[test]
    fn builds_compact_uppercase_badges() {
        assert_eq!(app_badge("Foot"), "FOOT");
        assert_eq!(app_badge("Chromium"), "CHRO");
        assert_eq!(app_badge("Visual Studio Code"), "VSC");
    }
}
