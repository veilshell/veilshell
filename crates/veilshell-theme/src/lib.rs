//! Theme provider abstractions and adapters for `VeilShell`.
//!
//! The core shell depends only on [`ThemePalette`]. Distro-specific theme
//! files—such as `Omarchy` Aether `colors.toml`—are parsed here into that
//! neutral palette.

use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use veilshell_core::{Rgba, ThemePalette};

/// Errors produced when loading a theme from disk or another provider.
#[derive(Debug)]
pub enum ThemeError {
    /// Underlying filesystem error.
    Io(std::io::Error),
    /// Provider configuration could not be resolved (e.g. missing `$HOME`).
    Unavailable(&'static str),
}

impl fmt::Display for ThemeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "theme I/O error: {err}"),
            Self::Unavailable(msg) => write!(f, "theme provider unavailable: {msg}"),
        }
    }
}

impl std::error::Error for ThemeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            Self::Unavailable(_) => None,
        }
    }
}

impl From<std::io::Error> for ThemeError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

/// Source of a [`ThemePalette`].
pub trait ThemeProvider {
    /// Human-readable provider identifier.
    fn name(&self) -> &'static str;

    /// Load the current [`ThemePalette`].
    ///
    /// # Errors
    /// Returns [`ThemeError`] if the underlying theme source cannot be read.
    fn load(&self) -> Result<ThemePalette, ThemeError>;
}

/// Built-in fallback dark palette that requires no external files.
#[derive(Clone, Copy, Debug, Default)]
pub struct DefaultThemeProvider;

impl ThemeProvider for DefaultThemeProvider {
    fn name(&self) -> &'static str {
        "veilshell-default"
    }

    fn load(&self) -> Result<ThemePalette, ThemeError> {
        Ok(ThemePalette::default_dark())
    }
}

/// Reads an `Omarchy` Aether `colors.toml` file into a [`ThemePalette`].
#[derive(Clone, Debug)]
pub struct OmarchyAetherProvider {
    path: PathBuf,
}

impl OmarchyAetherProvider {
    /// Create a provider pointing at an explicit `colors.toml` path.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Resolve the standard `$HOME/.local/state/omarchy/current/theme/colors.toml` path.
    #[must_use]
    pub fn default_path() -> Option<PathBuf> {
        Some(omarchy_current_dir()?.join("theme/colors.toml"))
    }

    /// Return the configured `colors.toml` path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Read-only Omarchy state directory, also used to locate the current wallpaper.
#[must_use]
pub fn omarchy_current_dir() -> Option<PathBuf> {
    let absolute_var = |name| {
        std::env::var_os(name)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
    };
    let state = absolute_var("XDG_STATE_HOME")
        .or_else(|| absolute_var("HOME").map(|path| path.join(".local/state")))?;
    Some(state.join("omarchy/current"))
}

impl ThemeProvider for OmarchyAetherProvider {
    fn name(&self) -> &'static str {
        "omarchy-aether"
    }

    fn load(&self) -> Result<ThemePalette, ThemeError> {
        let contents = fs::read_to_string(&self.path)?;
        Ok(parse_aether_colors(&contents, "omarchy-aether"))
    }
}

/// Parse flat `key = "#rrggbb"` entries from an `Omarchy` Aether `colors.toml`.
///
/// Missing or malformed keys fall back cleanly to [`ThemePalette::default_dark`].
#[must_use]
pub fn parse_aether_colors(input: &str, name: &str) -> ThemePalette {
    let mut map: HashMap<&str, &str> = HashMap::new();
    for raw_line in input.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('[') {
            continue;
        }
        let Some((key, rest)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = strip_inline_comment(rest).trim().trim_matches('"');
        if !key.is_empty() && !value.is_empty() {
            map.insert(key, value);
        }
    }

    let fallback = ThemePalette::default_dark();
    let pick = |keys: &[&str]| -> Option<Rgba> {
        keys.iter()
            .find_map(|k| map.get(k).and_then(|v| Rgba::from_hex(v)))
    };

    let background = pick(&["background", "dark_background"]).unwrap_or(fallback.background);
    let surface_rgb =
        pick(&["lighter_background", "surface", "selection"]).unwrap_or(fallback.surface);
    let foreground = pick(&["foreground", "light_foreground", "bright_foreground"])
        .unwrap_or(fallback.foreground);
    let muted = pick(&["muted", "dark_foreground", "color8"]).unwrap_or(fallback.muted);
    let accent =
        pick(&["accent", "magenta", "blue", "color4", "color5"]).unwrap_or(fallback.accent);
    let error = pick(&["red", "bright_red", "error", "color1"]).unwrap_or(fallback.error);

    ThemePalette {
        name: name.to_owned(),
        background,
        surface: surface_rgb.with_alpha(0.76),
        foreground,
        muted,
        accent,
        error,
    }
}

fn strip_inline_comment(value: &str) -> &str {
    let mut in_quotes = false;
    for (idx, ch) in value.char_indices() {
        match ch {
            '"' => in_quotes = !in_quotes,
            '#' if !in_quotes => return &value[..idx],
            _ => {}
        }
    }
    value
}

/// Load the `Omarchy` Aether palette if present on disk, otherwise return the built-in dark theme.
#[must_use]
pub fn load_best_available() -> ThemePalette {
    if let Some(path) = OmarchyAetherProvider::default_path() {
        if let Ok(palette) = OmarchyAetherProvider::new(path).load() {
            return palette;
        }
    }
    ThemePalette::default_dark()
}

/// Format an [`Rgba`] color as `#rrggbb` (ignoring alpha).
#[must_use]
pub fn format_hex(color: Rgba) -> String {
    let r = (color.r.clamp(0.0, 1.0) * 255.0).round() as u8;
    let g = (color.g.clamp(0.0, 1.0) * 255.0).round() as u8;
    let b = (color.b.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{r:02x}{g:02x}{b:02x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_AETHER: &str = r##"
# Generated by omarchy-aether.
mode = "dark"
accent = "#a6a1e7"
selection = "#23232c"
muted = "#8e8e9a"
background = "#15151d"
lighter_background = "#1e1e27"
foreground = "#b9b8cb"
red = "#f57b9c"
"##;

    #[test]
    fn parses_real_omarchy_aether_colors() {
        let palette = parse_aether_colors(SAMPLE_AETHER, "omarchy-aether");
        assert_eq!(palette.name, "omarchy-aether");
        assert_eq!(format_hex(palette.background), "#15151d");
        assert_eq!(format_hex(palette.surface), "#1e1e27");
        assert_eq!(format_hex(palette.foreground), "#b9b8cb");
        assert_eq!(format_hex(palette.accent), "#a6a1e7");
        assert_eq!(format_hex(palette.error), "#f57b9c");
        assert!(palette.meets_wcag_aa());
    }

    #[test]
    fn falls_back_gracefully_on_empty_input() {
        let palette = parse_aether_colors("", "empty");
        let default = ThemePalette::default_dark();
        assert_eq!(
            format_hex(palette.background),
            format_hex(default.background)
        );
        assert_eq!(format_hex(palette.accent), format_hex(default.accent));
    }
}
