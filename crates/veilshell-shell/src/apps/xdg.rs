use std::collections::HashMap;
use std::env;
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

pub(super) struct DataDirs {
    pub(super) home: Option<PathBuf>,
    pub(super) user: Option<PathBuf>,
    pub(super) system: Vec<PathBuf>,
}

impl DataDirs {
    pub(super) fn from_env() -> Self {
        Self::new(
            env::var_os("HOME").as_deref(),
            env::var_os("XDG_DATA_HOME").as_deref(),
            env::var_os("XDG_DATA_DIRS").as_deref(),
        )
    }

    pub(super) fn new(home: Option<&OsStr>, user: Option<&OsStr>, system: Option<&OsStr>) -> Self {
        let home = absolute(home);
        let user = absolute(user).or_else(|| home.as_ref().map(|path| path.join(".local/share")));
        let mut system = path_list(system, "/usr/local/share:/usr/share");
        if let Some(user) = &user {
            system.retain(|dir| dir != user);
        }
        Self { home, user, system }
    }

    pub(super) fn paths(&self) -> impl Iterator<Item = &Path> {
        self.user.iter().chain(&self.system).map(PathBuf::as_path)
    }
}

pub(super) fn absolute(value: Option<&OsStr>) -> Option<PathBuf> {
    value.map(PathBuf::from).filter(|path| path.is_absolute())
}

pub(super) fn path_list(value: Option<&OsStr>, default: &str) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for path in env::split_paths(
        value
            .filter(|s| !s.is_empty())
            .unwrap_or(OsStr::new(default)),
    ) {
        if path.is_absolute() && !paths.contains(&path) {
            paths.push(path);
        }
    }
    paths
}

pub(super) fn read_text(path: &Path) -> Option<String> {
    String::from_utf8(read_bytes(path, 1024 * 1024)?).ok()
}

pub(super) fn open_bounded(path: &Path, limit: u64) -> Option<File> {
    let meta = fs::metadata(path).ok()?;
    (meta.is_file() && meta.len() <= limit)
        .then(|| File::open(path).ok())
        .flatten()
}

pub(super) fn read_bytes(path: &Path, limit: u64) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    open_bounded(path, limit)?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    (bytes.len() as u64 <= limit).then_some(bytes)
}

pub(super) type Group<'a> = HashMap<&'a str, &'a str>;

pub(super) fn keyfile(text: &str) -> HashMap<&str, Group<'_>> {
    let mut groups: HashMap<_, Group<'_>> = HashMap::new();
    let mut group = None;
    for line in text.trim_start_matches('\u{feff}').lines().map(str::trim) {
        if let Some(name) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            group = Some(name);
            groups.entry(name).or_default();
        } else if !line.starts_with('#') {
            if let (Some(name), Some((key, value))) = (group, line.split_once('=')) {
                groups
                    .entry(name)
                    .or_default()
                    .insert(key.trim(), value.trim());
            }
        }
    }
    groups
}
