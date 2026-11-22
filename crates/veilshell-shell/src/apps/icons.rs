//! Freedesktop icon lookup. Theme settings and indexes are only read, never written.

use std::cell::OnceCell;
use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use veilshell_render::RgbaImage;

use super::xdg::{self, DataDirs, Group};

mod image;
pub(crate) use image::decode_bitmap;

const FALLBACK_THEME: &str = "hicolor";
const LOGICAL_SIZE: u32 = 85;
const SCALE: u32 = 2;
const PIXELS: u32 = LOGICAL_SIZE * SCALE;

pub(super) struct IconLookup {
    roots: Vec<PathBuf>,
    legacy_root: Option<PathBuf>,
    theme: String,
    themes: OnceCell<Vec<Vec<Slot>>>,
}

impl IconLookup {
    pub(super) fn from_env() -> Self {
        let data = DataDirs::from_env();
        let mut lookup = Self::new(&data, preferred_theme(&data).as_deref());
        let pixmaps = PathBuf::from("/usr/share/pixmaps");
        if !lookup.roots.contains(&pixmaps) {
            lookup.roots.push(pixmaps);
        }
        lookup
    }

    pub(super) fn new(data: &DataDirs, theme: Option<&str>) -> Self {
        let roots = data
            .user
            .iter()
            .map(|path| path.join("icons"))
            .chain(data.home.iter().map(|path| path.join(".icons")))
            .chain(data.system.iter().map(|path| path.join("icons")))
            .chain(data.paths().map(|path| path.join("pixmaps")))
            .collect();
        Self {
            roots,
            legacy_root: data
                .user
                .as_ref()
                .map(|path| path.join("applications/icons")),
            // Some distributions provide a `default` index.theme that inherits their
            // chosen default. If absent or cursor-only, proceed directly to hicolor.
            theme: theme
                .filter(|name| valid_theme(name))
                .unwrap_or("default")
                .to_owned(),
            themes: OnceCell::new(),
        }
    }

    pub(super) fn resolve(&self, name: &str) -> Option<PathBuf> {
        self.find(name, |path, _| path.is_file().then(|| path.to_owned()))
    }

    pub(super) fn load(&self, name: &str) -> Option<Arc<RgbaImage>> {
        self.find(name, image::load).map(Arc::new)
    }

    fn find<T>(
        &self,
        name: &str,
        mut use_path: impl FnMut(&Path, Format) -> Option<T>,
    ) -> Option<T> {
        let path = Path::new(name);
        if path.is_absolute() {
            return use_path(path, Format::from_path(path)?);
        }
        // Named icons are single filename components, never relative paths.
        if path.file_name()? != name {
            return None;
        }
        // Accept named icons with an explicit extension too (common in desktop files).
        let explicit = Format::from_path(path);
        let stem = if explicit.is_some() {
            path.file_stem()?.to_str()?
        } else {
            name
        };
        let filenames = [format!("{stem}.png"), format!("{stem}.svg")];
        let mut probe = |dir: &Path, format: Format| {
            let canonical = &filenames[format as usize];
            if explicit == Some(format) && name != canonical {
                if let Some(value) = use_path(&dir.join(name), format) {
                    return Some(value);
                }
            }
            use_path(&dir.join(canonical), format)
        };

        // Try an explicit extension first within each theme, not at the
        // expense of bypassing a higher-priority theme or an unthemed root.
        for theme in self.themes.get_or_init(|| self.theme_slots()) {
            for format in [explicit, explicit.map(Format::other)]
                .into_iter()
                .take(1 + usize::from(explicit.is_some()))
            {
                if let Some(value) = theme
                    .iter()
                    .filter(|slot| format.is_none_or(|preferred| slot.format == preferred))
                    .find_map(|slot| probe(&slot.directory, slot.format))
                {
                    return Some(value);
                }
            }
        }

        let formats = explicit.map_or(Format::ALL, |format| [format, format.other()]);
        self.roots
            .iter()
            // Compatibility for user-created web apps, after all standard roots.
            .chain(&self.legacy_root)
            .find_map(|root| formats.iter().find_map(|&format| probe(root, format)))
    }

    fn theme_slots(&self) -> Vec<Vec<Slot>> {
        let mut themes = Vec::new();
        let mut seen = HashSet::new();
        // `default` is an optional distribution-provided icon theme alias, not
        // a guessed installed theme. Skip it if hicolor was explicitly chosen.
        let mut todo = if self.theme == FALLBACK_THEME {
            Vec::new()
        } else {
            vec!["default".to_owned(), self.theme.clone()]
        };
        while let Some(name) = todo.pop() {
            // hicolor is last even if listed explicitly in the middle of Inherits.
            if name == FALLBACK_THEME || !seen.insert(name.clone()) {
                continue;
            }
            let (slots, parents) = self.read_theme(&name);
            themes.push(slots);
            todo.extend(parents.into_iter().rev());
        }
        themes.push(self.read_theme(FALLBACK_THEME).0);
        themes
    }

    fn read_theme(&self, name: &str) -> (Vec<Slot>, Vec<String>) {
        let roots: Vec<_> = self
            .roots
            .iter()
            .map(|dir| dir.join(name))
            .filter(|dir| dir.is_dir())
            .collect();
        // One index describes all copies of this theme, even when only one base
        // directory ships an index (e.g. Flatpak's hicolor exports do not).
        let ThemeIndex { mut dirs, parents } = roots
            .iter()
            .filter_map(|root| xdg::read_text(&root.join("index.theme")))
            .find_map(|text| parse_index(&text))
            .unwrap_or_default();
        if name == FALLBACK_THEME {
            extend_hicolor_dirs(&mut dirs, &roots);
        }

        let mut slots = Vec::new();
        for (relative, size) in dirs {
            for root in &roots {
                let path = root.join(&relative);
                if path.is_dir() {
                    for format in Format::ALL {
                        slots.push(Slot {
                            directory: path.clone(),
                            format,
                            rank: size.rank(format),
                        });
                    }
                }
            }
        }
        // Stable sort preserves index and root order for equivalent candidates.
        slots.sort_by_key(|slot| slot.rank);
        (slots, parents)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Format {
    Png,
    Svg,
}

impl Format {
    const ALL: [Self; 2] = [Self::Png, Self::Svg];

    fn other(self) -> Self {
        match self {
            Self::Png => Self::Svg,
            Self::Svg => Self::Png,
        }
    }

    fn from_path(path: &Path) -> Option<Self> {
        let extension = path.extension()?.to_str()?;
        if extension.eq_ignore_ascii_case("png") {
            Some(Self::Png)
        } else if extension.eq_ignore_ascii_case("svg") {
            Some(Self::Svg)
        } else {
            None
        }
    }
}

struct Slot {
    directory: PathBuf,
    format: Format,
    rank: (bool, bool, u32, u32),
}

struct Size {
    nominal: u32,
    scale: u32,
    min: u32,
    max: u32,
}

impl Size {
    fn from_group(group: &Group<'_>) -> Option<Self> {
        let number = |key| group.get(key).and_then(|s| s.parse::<u32>().ok());
        let nominal = number("Size").filter(|&size| size > 0)?;
        let scale = number("Scale").filter(|&scale| scale > 0).unwrap_or(1);
        let (min, max) = match group.get("Type").copied().unwrap_or("Threshold") {
            "Fixed" => (nominal, nominal),
            "Scalable" => (
                number("MinSize").unwrap_or(nominal),
                number("MaxSize").unwrap_or(nominal),
            ),
            "Threshold" => {
                let threshold = number("Threshold").unwrap_or(2);
                (
                    nominal.saturating_sub(threshold),
                    nominal.saturating_add(threshold),
                )
            }
            _ => return None,
        };
        (min <= max).then_some(Self {
            nominal,
            scale,
            min,
            max,
        })
    }

    fn rank(&self, format: Format) -> (bool, bool, u32, u32) {
        let native = self.nominal.saturating_mul(self.scale);
        let exact = self.scale == SCALE && (self.min..=self.max).contains(&LOGICAL_SIZE);
        let closest = PIXELS.clamp(
            self.min.saturating_mul(self.scale),
            self.max.saturating_mul(self.scale),
        );
        // Keep a near-exact threshold match, but prefer downsampling to obvious
        // upsampling. A vector can always render at the requested resolution.
        let undersized =
            format == Format::Png && native.saturating_add(u32::from(exact) * 2 * SCALE) < PIXELS;
        if undersized {
            // Scalable directories can also contain bitmaps. If all available
            // sources are undersized, prefer the largest native bitmap.
            return (true, false, PIXELS - native, closest.abs_diff(PIXELS));
        }
        (
            undersized,
            !exact,
            closest.abs_diff(PIXELS),
            native.abs_diff(PIXELS),
        )
    }
}

#[derive(Default)]
struct ThemeIndex {
    dirs: Vec<(PathBuf, Size)>,
    parents: Vec<String>,
}

fn parse_index(text: &str) -> Option<ThemeIndex> {
    let groups = xdg::keyfile(text);
    let index = groups.get("Icon Theme")?;
    let read_list = |key| {
        index
            .get(key)
            .into_iter()
            .flat_map(|value| value.split(','))
            .map(str::trim)
    };
    let mut seen = HashSet::new();
    let dirs = read_list("Directories")
        .chain(read_list("ScaledDirectories"))
        .filter_map(|name| {
            let path = Path::new(name);
            if name.is_empty()
                || !path
                    .components()
                    .all(|part| matches!(part, Component::Normal(_)))
                || !seen.insert(name)
            {
                return None;
            }
            Some((path.to_owned(), Size::from_group(groups.get(name)?)?))
        })
        .collect();
    let parents = read_list("Inherits")
        .filter(|name| valid_theme(name))
        .map(str::to_owned)
        .collect();
    Some(ThemeIndex { dirs, parents })
}

fn extend_hicolor_dirs(dirs: &mut Vec<(PathBuf, Size)>, roots: &[PathBuf]) {
    // On minimal installations hicolor may have no index. Also recognize app
    // exports under conventional size dirs omitted by older hicolor indexes.
    let mut inferred = Vec::new();
    let mut seen: HashSet<_> = dirs.iter().map(|(path, _)| path.clone()).collect();
    for root in roots {
        for entry in fs::read_dir(root).into_iter().flatten().flatten() {
            let file_name = entry.file_name();
            let Some((name, size)) = file_name
                .to_str()
                .and_then(|name| conventional_size(name).map(|size| (name, size)))
            else {
                continue;
            };
            let relative = Path::new(name).join("apps");
            if root.join(&relative).is_dir() && seen.insert(relative.clone()) {
                inferred.push((relative, size));
            }
        }
    }
    inferred.sort_by(|(a, _), (b, _)| a.cmp(b));
    dirs.extend(inferred);
}

fn conventional_size(dir: &str) -> Option<Size> {
    if matches!(dir, "scalable" | "symbolic") {
        return Some(Size {
            nominal: 128,
            scale: 1,
            min: 1,
            max: u32::MAX,
        });
    }
    let (size, scale) = if let Some((size, scale)) = dir.split_once('@') {
        (size, scale.trim_end_matches('x').parse::<u32>().ok()?)
    } else {
        (dir, 1)
    };
    let (width, height) = size.split_once('x')?;
    let nominal = width.parse::<u32>().ok()?.min(height.parse().ok()?);
    (nominal > 0 && scale > 0).then_some(Size {
        nominal,
        scale,
        min: nominal,
        max: nominal,
    })
}

fn valid_theme(name: &str) -> bool {
    Path::new(name).file_name().is_some_and(|part| part == name)
        && name.bytes().all(|ch| ch.is_ascii_graphic() && ch != b',')
}

fn theme_value(value: &str) -> Option<String> {
    let value = value.trim();
    let unquoted = value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .or_else(|| {
            value
                .strip_prefix('\'')
                .and_then(|value| value.strip_suffix('\''))
        })
        .unwrap_or(value);
    valid_theme(unquoted).then(|| unquoted.to_owned())
}

fn gtk_theme(config: &Path) -> Option<String> {
    ["gtk-4.0/settings.ini", "gtk-3.0/settings.ini"]
        .into_iter()
        .filter_map(|name| xdg::read_text(&config.join(name)))
        .find_map(|text| {
            theme_value(
                xdg::keyfile(&text)
                    .get("Settings")?
                    .get("gtk-icon-theme-name")?,
            )
        })
}

fn preferred_theme(data: &DataDirs) -> Option<String> {
    env::var("VEILSHELL_ICON_THEME")
        .ok()
        .and_then(|value| theme_value(&value))
        .or_else(|| {
            let config = xdg::absolute(env::var_os("XDG_CONFIG_HOME").as_deref())
                .or_else(|| data.home.as_ref().map(|home| home.join(".config")))?;
            gtk_theme(&config)
        })
        .or_else(session_dconf_theme)
        .or_else(|| {
            xdg::path_list(env::var_os("XDG_CONFIG_DIRS").as_deref(), "/etc/xdg")
                .iter()
                .find_map(|path| gtk_theme(path))
        })
        .or_else(|| gtk_theme(Path::new("/etc")))
}

fn session_dconf_theme() -> Option<String> {
    // Optional: read the actual GSettings value persisted by desktop sessions.
    // `dconf read` gives no value if unset (unlike gsettings' schema default),
    // requires neither GTK nor a display, and doesn't start the settings writer.
    let output = Command::new("dconf")
        .args(["read", "/org/gnome/desktop/interface/icon-theme"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    theme_value(std::str::from_utf8(&output.stdout).ok()?)
}
