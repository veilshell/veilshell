use std::ffi::OsStr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use super::*;

const SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 12">
  <path fill="red" fill-opacity=".5" d="M0 0h12v12H0z"/>
</svg>"#;

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let serial = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "veilshell-apps-{}-{time}-{serial}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }

    fn file(&self, name: &str, bytes: impl AsRef<[u8]>) -> PathBuf {
        let path = self.path(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, bytes).unwrap();
        path
    }

    fn png(&self, name: &str, width: u32, height: u32) -> PathBuf {
        let path = self.path(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        veilshell_render::RenderFrame {
            width,
            height,
            rgba: [12, 20, 40, 255].repeat((width * height) as usize),
        }
        .save_png(&path)
        .unwrap();
        path
    }

    fn data(&self) -> DataDirs {
        let system =
            std::env::join_paths([self.path("flatpak/exports/share"), self.path("system")])
                .unwrap();
        DataDirs::new(
            Some(self.0.as_os_str()),
            Some(self.path("user").as_os_str()),
            Some(&system),
        )
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn xdg_data_defaults_and_precedence() {
    let home = Some(OsStr::new("/home/test"));
    let defaults = DataDirs::new(home, Some(OsStr::new("relative")), Some(OsStr::new("")));
    assert_eq!(
        defaults.paths().collect::<Vec<_>>(),
        ["/home/test/.local/share", "/usr/local/share", "/usr/share"].map(Path::new)
    );
    let custom = DataDirs::new(
        home,
        Some(OsStr::new("/user-data")),
        Some(OsStr::new("/export::relative:/user-data:/export:/system")),
    );
    assert_eq!(
        custom.paths().collect::<Vec<_>>(),
        ["/user-data", "/export", "/system"].map(Path::new)
    );
}

#[test]
fn discovers_real_xdg_entries_and_exported_icons() {
    let fixture = Fixture::new();
    let entry = "[Desktop Entry]\nType=Application\nExec=example\n";
    fixture.file(
        "user/applications/editors/Text.desktop",
        format!("{entry}Name=User editor\nIcon=miss\n"),
    );
    fixture.file(
        "system/applications/editors-Text.desktop",
        format!("{entry}Name=System editor\n"),
    );
    fixture.file(
        "user/applications/hide-me.desktop",
        "[Desktop Entry]\nHidden=true\n",
    );
    fixture.file(
        "system/applications/hide-me.desktop",
        format!("{entry}Name=Hidden by user\n"),
    );
    let target = fixture.file(
        "flatpak/active/export/share/applications/com.example.Flat.desktop",
        "[Desktop Entry]\nType=Application\nName=Flatpak\n\
         Icon=com.example.Flat\nExec=flatpak run com.example.Flat\n",
    );
    let export = fixture.file("flatpak/exports/share/applications/.keep", "");
    let dir = export.parent().unwrap();
    std::os::unix::fs::symlink(&target, dir.join("com.example.Flat.desktop")).unwrap();
    std::os::unix::fs::symlink(dir, dir.join("ignored-cycle")).unwrap();
    fixture.file(
        "system/icons/hicolor/index.theme",
        "[Icon Theme]\nDirectories=vector/apps\n\
         [vector/apps]\nType=Scalable\nSize=128\nMinSize=8\nMaxSize=512\n",
    );
    let icon_target = fixture.file("flatpak/active/com.example.Flat.svg", SVG);
    let icon_export =
        fixture.path("flatpak/exports/share/icons/hicolor/vector/apps/com.example.Flat.svg");
    fs::create_dir_all(icon_export.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(icon_target, &icon_export).unwrap();

    let data = fixture.data();
    let apps = discover_in(&data);
    assert_eq!(
        apps.iter().map(|app| app.id.as_str()).collect::<Vec<_>>(),
        ["com.example.Flat.desktop", "editors-Text.desktop"]
    );
    assert_eq!(apps[1].name, "User editor");
    let lookup = IconLookup::new(&data, None);
    let tiles = make_tiles(&apps, |name| lookup.load(name));
    assert_eq!(tiles[0].exec, "flatpak run com.example.Flat");
    assert_eq!(tiles[0].icon.as_ref().unwrap().width, 170);
    assert_eq!(tiles[1].badge, "UE");
    assert!(tiles[1].icon.is_none());
}

#[test]
fn inherits_themes_depth_first_and_uses_hicolor_then_unthemed() {
    let fixture = Fixture::new();
    fixture.file(
        "user/icons/Child/index.theme",
        "[Icon Theme]\nInherits=First,hicolor,Second\nDirectories=small/apps\n\
         [small/apps]\nType=Fixed\nSize=24\n",
    );
    fixture.file(
        "system/icons/Child/index.theme",
        "[Icon Theme]\nDirectories=ignored\n[ignored]\nSize=170\n",
    );
    fixture.file(
        "system/icons/First/index.theme",
        "[Icon Theme]\nInherits=Child\nDirectories=vector\n\
         [vector]\nType=Scalable\nSize=85\nMinSize=8\nMaxSize=512\nScale=2\n",
    );
    fixture.file(
        "system/icons/Second/index.theme",
        "[Icon Theme]\nDirectories=any-dir\n[any-dir]\nSize=256\n",
    );

    let child = fixture.png("system/icons/Child/small/apps/shared.png", 24, 24);
    fixture.file("system/icons/First/vector/shared.svg", SVG);
    let inherited_svg = fixture.file("system/icons/First/vector/vector.svg", SVG);
    let second = fixture.png("system/icons/Second/any-dir/second.png", 256, 256);
    fixture.png("system/icons/hicolor/192x192/apps/second.png", 192, 192);
    let hicolor = fixture.png("system/icons/hicolor/20x20/apps/fallback.png", 20, 20);
    fixture.file("user/icons/fallback.svg", SVG);
    let pixmap = fixture.file("system/pixmaps/pixmap.svg", SVG);
    fixture.png("user/applications/icons/pixmap.png", 48, 48);
    let legacy = fixture.png("user/applications/icons/web-app.png", 48, 48);

    let lookup = IconLookup::new(&fixture.data(), Some("Child"));
    assert_eq!(lookup.resolve("shared"), Some(child));
    assert_eq!(lookup.resolve("second"), Some(second));
    assert_eq!(lookup.resolve("fallback"), Some(hicolor));
    assert_eq!(lookup.resolve("pixmap"), Some(pixmap));
    assert_eq!(lookup.resolve("web-app"), Some(legacy));
    assert_eq!(lookup.load("web-app").unwrap().width, 48);
    assert_eq!(lookup.resolve("vector.svg"), Some(inherited_svg));
    let image = lookup.load("vector").unwrap();
    assert_eq!((image.width, image.height), (170, 85));
    assert_eq!(&image.rgba[..4], &[255, 0, 0, 128]);
    assert_eq!(&image.rgba[169 * 4..170 * 4], &[0; 4]);
    assert!(lookup.resolve("../pixmap").is_none());
    assert!(lookup.load("not-installed").is_none());
}

#[test]
fn ranks_scaled_directories_and_preserves_small_png_as_last_resort() {
    let fixture = Fixture::new();
    fixture.file(
        "system/icons/hicolor/index.theme",
        "[Icon Theme]\n\
         Directories=large,small,huge,unit,tiny-scales,vector\n\
         ScaledDirectories=hidpi,near\n\
         [hidpi]\nSize=85\nScale=2\nType=Fixed\n\
         [near]\nSize=84\nScale=2\nThreshold=1\n\
         [unit]\nSize=170\nType=Fixed\n\
         [large]\nSize=192\nType=Fixed\n\
         [small]\nSize=128\nType=Fixed\n\
         [tiny-scales]\nSize=32\nType=Scalable\nMinSize=1\nMaxSize=512\n\
         [vector]\nSize=48\nType=Scalable\nMinSize=1\nMaxSize=512\n\
         [huge]\nSize=4294967295\nScale=4294967295\nThreshold=4294967295\n",
    );

    fixture.png("system/icons/hicolor/unit/dense.png", 170, 170);
    fixture.png("system/icons/hicolor/hidpi/dense.png", 170, 170);
    let user_dense = fixture.png("user/icons/hicolor/hidpi/dense.png", 170, 170);
    let near = fixture.png("system/icons/hicolor/near/match.png", 168, 168);
    fixture.png("system/icons/hicolor/unit/match.png", 170, 170);
    let large = fixture.png("system/icons/hicolor/large/pixel.png", 192, 192);
    fixture.png("system/icons/hicolor/small/pixel.png", 128, 128);
    fixture.png("system/icons/hicolor/huge/other.png", 32, 32);
    let small = fixture.png("system/icons/hicolor/small/bitmap-only.png", 128, 128);
    fixture.png("system/icons/hicolor/tiny-scales/bitmap-only.png", 32, 32);
    fixture.png("system/icons/hicolor/tiny-scales/prefer-vector.png", 32, 32);
    let vector = fixture.file("system/icons/hicolor/vector/prefer-vector.svg", SVG);

    let lookup = IconLookup::new(&fixture.data(), None);
    assert_eq!(lookup.resolve("dense"), Some(user_dense));
    assert_eq!(lookup.resolve("match"), Some(near));
    assert_eq!(lookup.resolve("pixel"), Some(large));
    assert_eq!(lookup.resolve("bitmap-only"), Some(small));
    assert_eq!(lookup.resolve("prefer-vector"), Some(vector));
    assert_eq!(lookup.load("pixel").unwrap().width, 170);
    assert_eq!(lookup.load("bitmap-only").unwrap().width, 128);
}

#[test]
fn skips_corrupt_files_and_honors_absolute_icons() {
    let fixture = Fixture::new();
    fixture.file("user/icons/same.png", "not an image");
    fixture.png("system/icons/same.png", 24, 24);
    let svg = fixture.file("absolute/test icon.SVG", SVG);
    let png = fixture.png("absolute/icon.png", 256, 128);
    let oversized = fixture.png("absolute/oversized.png", 2049, 1);
    assert_eq!(resolve_icon_path(svg.to_str().unwrap()), Some(svg.clone()));

    let lookup = IconLookup::new(&fixture.data(), None);
    assert_eq!(lookup.load(svg.to_str().unwrap()).unwrap().height, 85);
    let icon = lookup.load(png.to_str().unwrap()).unwrap();
    assert_eq!((icon.width, icon.height), (170, 85));
    assert!(lookup.load(oversized.to_str().unwrap()).is_none());
    assert_eq!(lookup.load("same").unwrap().width, 24);
}

#[test]
fn decodes_jpeg_mislabeled_as_png() {
    // A generated 200×40 grayscale JPEG (solid gray 175), kept inline so the
    // fixture needs neither a test-only encoder nor any installed image tools.
    const JPEG: &[u8] = b"\
        \xff\xd8\xff\xe0\x00\x10\x4a\x46\x49\x46\x00\x01\x01\x00\x00\x01\x00\x01\x00\x00\xff\xdb\x00\x43\
        \x00\x08\x06\x06\x07\x06\x05\x08\x07\x07\x07\x09\x09\x08\x0a\x0c\x14\x0d\x0c\x0b\x0b\x0c\x19\x12\
        \x13\x0f\x14\x1d\x1a\x1f\x1e\x1d\x1a\x1c\x1c\x20\x24\x2e\x27\x20\x22\x2c\x23\x1c\x1c\x28\x37\x29\
        \x2c\x30\x31\x34\x34\x34\x1f\x27\x39\x3d\x38\x32\x3c\x2e\x33\x34\x32\xff\xc0\x00\x0b\x08\x00\x28\
        \x00\xc8\x01\x01\x11\x00\xff\xc4\x00\x15\x00\x01\x01\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\
        \x00\x00\x00\x00\x06\xff\xc4\x00\x14\x10\x01\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\
        \x00\x00\x00\xff\xda\x00\x08\x01\x01\x00\x00\x3f\x00\xaf\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\
        \x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x7f\xff\xd9";
    let fixture = Fixture::new();
    fixture.file("user/icons/WebApp.png", JPEG);
    let lookup = IconLookup::new(&fixture.data(), None);
    let icon = lookup
        .load("WebApp")
        .expect("decode JPEG bytes at a PNG path");
    assert_eq!((icon.width, icon.height), (170, 34));
    assert_eq!(&icon.rgba[..4], &[175, 175, 175, 255]);
}

#[test]
fn preserves_tiles_and_shares_decoded_icons_within_a_conversion() {
    let fixture = Fixture::new();
    let icon = fixture.file("icon with spaces.svg", SVG);
    let entry = format!(
        "[Desktop Entry]\nType=Application\nExec=foot\nName=Foot\nIcon={}\n",
        icon.to_str().unwrap().replace(' ', "\\s")
    );
    fixture.file("user/applications/foot.desktop", &entry);
    fixture.file("user/applications/foot-alias.desktop", &entry);
    fixture.file(
        "user/applications/btop.desktop",
        "[Desktop Entry]\nType=Application\nExec=btop\nName=btop++\nTerminal=true\n",
    );
    let apps = discover_in(&fixture.data());
    let tiles = apps_to_tiles(&apps);
    assert_eq!(tiles.len(), 3);
    assert_eq!(tiles[0].exec, "foot");
    assert!(tiles[1].exec.contains("btop"));
    assert!(tiles[1].icon.is_none());
    assert!(Arc::ptr_eq(
        tiles[0].icon.as_ref().unwrap(),
        tiles[2].icon.as_ref().unwrap()
    ));
}
