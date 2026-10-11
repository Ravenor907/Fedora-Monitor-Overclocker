//! Per-user installation: the binary in `~/.local/bin` plus an app menu entry
//! and icon, so Monitor OC starts from the launcher and Steam can add it as a
//! non-Steam game for Game Mode. Only the user's home directory is touched.

use crate::{gamescope, state, system};
use anyhow::{Context, Result, bail};
use std::fs;
use std::io::ErrorKind;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Reverse-DNS application id: names the menu entry and icon, and is the
/// window class (Wayland app_id) the GUI should use so docks match them up.
pub const APP_ID: &str = "io.github.ravenor907.MonitorOC";
const ICON_SVG: &str = include_str!("../assets/io.github.ravenor907.MonitorOC.svg");
/// Pre-rendered from the SVG for launchers that do not draw SVG icons (Steam).
const ICON_PNG: &[u8] = include_bytes!("../assets/io.github.ravenor907.MonitorOC.png");

/// Where `install` puts things for one user.
#[derive(Clone, Debug, PartialEq)]
pub struct Paths {
    pub home: PathBuf,
    pub binary: PathBuf,
    pub desktop: PathBuf,
    pub icon_svg: PathBuf,
    pub icon_png: PathBuf,
}

impl Paths {
    pub fn for_current_user() -> Result<Paths> {
        if system::is_root() {
            bail!(
                "install and uninstall work on your own account; run them without sudo \
                 (as the user you play on, e.g. `deck`)"
            );
        }
        let home = std::env::var_os("HOME")
            .filter(|h| !h.is_empty())
            .context("HOME is not set")?;
        let data = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from);
        Ok(Paths::new(Path::new(&home), data.as_deref()))
    }

    /// `~/.local/bin` is fixed by the XDG spec; menu entries and icons follow
    /// `$XDG_DATA_HOME`, which only counts when it is an absolute path.
    pub fn new(home: &Path, xdg_data_home: Option<&Path>) -> Paths {
        let data = xdg_data_home
            .filter(|d| d.is_absolute())
            .map_or_else(|| home.join(".local/share"), Path::to_path_buf);
        let icons = data.join("icons/hicolor");
        Paths {
            home: home.to_path_buf(),
            binary: home.join(".local/bin/monitor-oc"),
            desktop: data.join(format!("applications/{APP_ID}.desktop")),
            icon_svg: icons.join(format!("scalable/apps/{APP_ID}.svg")),
            icon_png: icons.join(format!("256x256/apps/{APP_ID}.png")),
        }
    }

    fn menu_files(&self) -> [&Path; 3] {
        [&self.desktop, &self.icon_svg, &self.icon_png]
    }

    /// `path` with the home directory shown as `~`, for messages.
    fn show(&self, path: &Path) -> String {
        match path.strip_prefix(&self.home) {
            Ok(rest) => format!("~/{}", rest.display()),
            Err(_) => path.display().to_string(),
        }
    }
}

/// Whether this build has the graphical app; only then is a menu entry useful.
fn has_gui() -> bool {
    cfg!(feature = "gui")
}

/// The app menu entry for a binary installed at `binary` (an absolute path).
pub fn desktop_entry(binary: &Path) -> Result<String> {
    let bin = binary
        .to_str()
        .context("the install path is not valid UTF-8")?;
    if bin.chars().any(char::is_control) {
        bail!("the install path contains control characters");
    }
    Ok(format!(
        "[Desktop Entry]
Type=Application
Name=Monitor OC
GenericName=Display Overclocker
Comment=Raise your screen's refresh rate and check that it works
Exec={} gui
TryExec={}
Icon={APP_ID}
Terminal=false
Categories=Settings;HardwareSettings;
Keywords=refresh;overclock;display;monitor;hz;gamescope;
StartupWMClass={APP_ID}
",
        exec_arg(bin),
        escape_value(bin)
    ))
}

/// One `Exec` argument, quoted as the Desktop Entry spec requires, plus the
/// backslash escaping that applies to every value.
fn exec_arg(arg: &str) -> String {
    const RESERVED: &[char] = &[
        ' ', '\t', '\n', '"', '\'', '\\', '>', '<', '~', '|', '&', ';', '$', '*', '?', '#', '(',
        ')', '`',
    ];
    let arg = arg.replace('%', "%%");
    if !arg.contains(RESERVED) {
        return escape_value(&arg);
    }
    let mut quoted = String::from("\"");
    for c in arg.chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
            quoted.push('\\');
        }
        quoted.push(c);
    }
    quoted.push('"');
    escape_value(&quoted)
}

fn escape_value(s: &str) -> String {
    s.replace('\\', "\\\\")
}

/// Create `path` through a temporary file in the same directory and a rename,
/// so a running copy of the binary or a launcher reading the menu entry never
/// sees a half-written file. Re-running replaces the file in place.
fn put(path: &Path, mode: u32, fill: impl FnOnce(&Path) -> std::io::Result<()>) -> Result<()> {
    let dir = path.parent().context("install path has no parent")?;
    fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let tmp = dir.join(format!(".{name}.new-{}", std::process::id()));
    let result = fill(&tmp)
        .and_then(|()| fs::set_permissions(&tmp, fs::Permissions::from_mode(mode)))
        .and_then(|()| fs::rename(&tmp, path));
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result.with_context(|| format!("writing {}", path.display()))
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

fn remove_if_present(path: &Path) -> Result<bool> {
    match fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e).with_context(|| format!("removing {}", path.display())),
    }
}

/// Copy `exe` to `paths.binary` and, with `menu`, add the menu entry and
/// icons. Without `menu` (a CLI-only build) a menu entry left by an earlier
/// install is removed, since it would start a GUI this binary does not have.
pub fn install_files(paths: &Paths, exe: &Path, menu: bool) -> Result<()> {
    if !same_file(exe, &paths.binary) {
        put(&paths.binary, 0o755, |tmp| fs::copy(exe, tmp).map(drop))?;
    }
    if !menu {
        for path in paths.menu_files() {
            remove_if_present(path)?;
        }
        return Ok(());
    }
    let entry = desktop_entry(&paths.binary)?;
    put(&paths.desktop, 0o644, |tmp| fs::write(tmp, entry))?;
    put(&paths.icon_svg, 0o644, |tmp| fs::write(tmp, ICON_SVG))?;
    put(&paths.icon_png, 0o644, |tmp| fs::write(tmp, ICON_PNG))
}

/// Delete everything `install_files` creates; returns what was there.
pub fn remove_files(paths: &Paths) -> Result<Vec<PathBuf>> {
    let mut removed = Vec::new();
    for path in [
        &*paths.binary,
        &paths.desktop,
        &paths.icon_svg,
        &paths.icon_png,
    ] {
        if remove_if_present(path)? {
            removed.push(path.to_path_buf());
        }
    }
    Ok(removed)
}

/// Ask the desktop to re-read its menus. Best effort: the tools are optional
/// and launchers also notice new entries on their own, just later.
fn refresh_menus(paths: &Paths) {
    if let Some(apps) = paths.desktop.parent().filter(|d| d.is_dir()) {
        quietly(&["update-desktop-database", &apps.to_string_lossy()]);
    }
    if let Some(kde) = ["kbuildsycoca6", "kbuildsycoca5"]
        .into_iter()
        .find(|c| system::which(c))
    {
        quietly(&[kde]);
    }
}

fn quietly(cmd: &[&str]) {
    if system::which(cmd[0]) {
        let _ = Command::new(cmd[0])
            .args(&cmd[1..])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

fn on_path(dir: &Path) -> bool {
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d == dir))
}

/// `monitor-oc install`: install or upgrade the running binary for this user.
pub fn install() -> Result<()> {
    let paths = Paths::for_current_user()?;
    let exe = std::env::current_exe().context("locating the running monitor-oc binary")?;
    let upgrade = paths.binary.exists();
    let menu = has_gui();
    install_files(&paths, &exe, menu)?;
    refresh_menus(&paths);

    println!(
        "{} monitor-oc {}:",
        if upgrade { "Updated" } else { "Installed" },
        env!("CARGO_PKG_VERSION")
    );
    println!("  {}", paths.show(&paths.binary));
    if menu {
        for path in paths.menu_files() {
            println!("  {}", paths.show(path));
        }
    }

    println!("\nNext steps:");
    if menu {
        println!(
            "  * Open \"Monitor OC\" from your app menu, or run `monitor-oc` in a terminal.\n  \
             * To use it in Game Mode (SteamOS, Bazzite): in Desktop Mode open Steam >\n    \
             Games > Add a Non-Steam Game to My Library, tick \"Monitor OC\" and click\n    \
             Add Selected Programs. It then shows up in your library in Game Mode."
        );
    } else {
        println!(
            "  * This is the command-line build, so no app menu entry was added. For the app,\n    \
             install monitor-oc-x86_64 from the releases page instead."
        );
    }
    println!(
        "  * Check what works on this system: monitor-oc doctor\n  \
         * Remove it again: monitor-oc uninstall"
    );

    let bin_dir = paths.binary.parent().unwrap_or(&paths.home);
    if !on_path(bin_dir) {
        println!(
            "\nNote: {} is not on your PATH, so a terminal will not find `monitor-oc`\n\
             yet. To fix that, add this line to ~/.bashrc and open a new terminal:\n  \
             export PATH=\"$HOME/.local/bin:$PATH\"",
            paths.show(bin_dir)
        );
    }
    Ok(())
}

/// `monitor-oc uninstall`: remove what `install` added, and with `purge` also
/// the built-in screen settings and monitor-oc's config directory.
pub fn uninstall(purge: bool) -> Result<()> {
    let paths = Paths::for_current_user()?;
    let overrides = state::load();
    if !overrides.is_empty() {
        let list: Vec<String> = overrides
            .iter()
            .map(|e| format!("{} {}", e.connector, e.mode))
            .collect();
        let exe = std::env::current_exe().unwrap_or_else(|_| paths.binary.clone());
        bail!(
            "external-monitor overrides are still installed ({}). They live outside your home \
             directory and need root to remove, so do that first:\n  sudo {} remove\n\
             then run `monitor-oc uninstall` again",
            list.join(", "),
            exe.display()
        );
    }

    let mut removed = remove_files(&paths)?;
    refresh_menus(&paths);
    let screen = gamescope::Paths::for_current_user()?;
    if purge {
        let had_profile = gamescope::installed(&screen);
        gamescope::remove(&screen)?;
        if had_profile {
            removed.push(screen.script.clone());
        }
        if let Some(config) = screen.state.parent().filter(|d| d.exists()) {
            fs::remove_dir_all(config).with_context(|| format!("removing {}", config.display()))?;
            removed.push(config.to_path_buf());
        }
    }

    if removed.is_empty() {
        println!("monitor-oc is not installed for this user; nothing to remove.");
    } else {
        println!("Removed:");
        for path in &removed {
            println!("  {}", paths.show(path));
        }
    }
    if purge && removed.contains(&screen.script) {
        println!("\nRestart Game Mode to go back to the built-in screen's stock refresh range.");
    } else if !purge && gamescope::installed(&screen) {
        println!(
            "\nYour built-in screen settings were kept, and Game Mode keeps using them.\n\
             To remove them too, delete these and restart Game Mode:\n  {}\n  {}",
            paths.show(&screen.script),
            paths.show(screen.state.parent().unwrap_or(&screen.state))
        );
    }
    if removed.contains(&paths.desktop) {
        println!(
            "\nIf you added Monitor OC to Steam, remove that shortcut there too (in the\n\
             library: Manage > Remove non-Steam game from your library)."
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("monitor-oc-install-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn paths_follow_xdg_data_home() {
        let home = Path::new("/home/deck");
        let p = Paths::new(home, None);
        assert_eq!(p.binary, Path::new("/home/deck/.local/bin/monitor-oc"));
        assert_eq!(
            p.desktop,
            Path::new(
                "/home/deck/.local/share/applications/io.github.ravenor907.MonitorOC.desktop"
            )
        );
        assert_eq!(
            p.icon_svg,
            Path::new(
                "/home/deck/.local/share/icons/hicolor/scalable/apps/io.github.ravenor907.MonitorOC.svg"
            )
        );
        assert_eq!(
            p.icon_png,
            Path::new(
                "/home/deck/.local/share/icons/hicolor/256x256/apps/io.github.ravenor907.MonitorOC.png"
            )
        );

        let custom = Paths::new(home, Some(Path::new("/data/share")));
        assert_eq!(
            custom.desktop,
            Path::new("/data/share/applications/io.github.ravenor907.MonitorOC.desktop")
        );
        // The binary does not move with XDG_DATA_HOME.
        assert_eq!(custom.binary, p.binary);
        // Empty or relative values are ignored, as the XDG spec says.
        assert_eq!(Paths::new(home, Some(Path::new(""))), p);
        assert_eq!(Paths::new(home, Some(Path::new("share"))), p);
        assert_eq!(
            p.show(&p.desktop),
            format!("~/.local/share/applications/{APP_ID}.desktop")
        );
        assert_eq!(p.show(Path::new("/etc/x")), "/etc/x");
    }

    #[test]
    fn desktop_entry_contents() {
        let entry = desktop_entry(Path::new("/home/deck/.local/bin/monitor-oc")).unwrap();
        for line in [
            "[Desktop Entry]",
            "Type=Application",
            "Name=Monitor OC",
            "GenericName=Display Overclocker",
            "Exec=/home/deck/.local/bin/monitor-oc gui",
            "TryExec=/home/deck/.local/bin/monitor-oc",
            "Icon=io.github.ravenor907.MonitorOC",
            "Terminal=false",
            "Categories=Settings;HardwareSettings;",
            "Keywords=refresh;overclock;display;monitor;hz;gamescope;",
            "StartupWMClass=io.github.ravenor907.MonitorOC",
        ] {
            assert!(entry.lines().any(|l| l == line), "missing {line:?}");
        }
        assert!(entry.lines().skip(1).all(|l| l.contains('=')));
        assert!(desktop_entry(Path::new("/home/a\nb/monitor-oc")).is_err());
    }

    #[test]
    fn exec_arguments_are_quoted() {
        assert_eq!(exec_arg("/home/deck/x"), "/home/deck/x");
        assert_eq!(exec_arg("/home/my user/x"), "\"/home/my user/x\"");
        // `$` and `\` are escaped inside quotes, then every `\` is doubled
        // for the value itself; `%` is doubled for field codes.
        assert_eq!(exec_arg("/a$b\\c%d"), r#""/a\\$b\\\\c%%d""#);
        assert_eq!(exec_arg("/100%"), "/100%%");
    }

    #[test]
    fn install_upgrade_and_remove() {
        let dir = temp_dir("roundtrip");
        let exe = dir.join("downloaded-monitor-oc");
        fs::write(&exe, b"binary v1").unwrap();
        let paths = Paths::new(&dir.join("home"), None);

        install_files(&paths, &exe, true).unwrap();
        assert_eq!(fs::read(&paths.binary).unwrap(), b"binary v1");
        let mode = fs::metadata(&paths.binary).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755);
        assert_eq!(
            fs::read_to_string(&paths.desktop).unwrap(),
            desktop_entry(&paths.binary).unwrap()
        );
        assert_eq!(fs::read_to_string(&paths.icon_svg).unwrap(), ICON_SVG);
        assert_eq!(fs::read(&paths.icon_png).unwrap(), ICON_PNG);

        // Re-running upgrades in place and leaves no temporary files behind.
        fs::write(&exe, b"binary v2").unwrap();
        install_files(&paths, &exe, true).unwrap();
        assert_eq!(fs::read(&paths.binary).unwrap(), b"binary v2");
        let bin_dir = paths.binary.parent().unwrap();
        assert_eq!(fs::read_dir(bin_dir).unwrap().count(), 1);

        // Running the installed copy's own `install` keeps it intact.
        install_files(&paths, &paths.binary, true).unwrap();
        assert_eq!(fs::read(&paths.binary).unwrap(), b"binary v2");

        // A CLI-only build drops the menu entry that would start the GUI.
        install_files(&paths, &exe, false).unwrap();
        assert!(paths.binary.exists());
        assert!(paths.menu_files().iter().all(|p| !p.exists()));

        install_files(&paths, &exe, true).unwrap();
        let removed = remove_files(&paths).unwrap();
        assert_eq!(removed.len(), 4);
        assert!(!paths.binary.exists() && paths.menu_files().iter().all(|p| !p.exists()));
        assert!(remove_files(&paths).unwrap().is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    /// Checks the generated menu entry and the double-click installer with
    /// desktop-file-validate (from desktop-file-utils).
    #[test]
    fn desktop_files_validate() {
        if !system::which("desktop-file-validate") {
            assert!(
                std::env::var_os("CI").is_none(),
                "desktop-file-validate is required in CI"
            );
            eprintln!("skipping: desktop-file-validate not installed");
            return;
        }
        let dir = temp_dir("validate");
        let generated = dir.join(format!("{APP_ID}.desktop"));
        let entry = desktop_entry(Path::new("/home/deck/.local/bin/monitor-oc")).unwrap();
        fs::write(&generated, entry).unwrap();
        let installer =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("packaging/Install-Monitor-OC.desktop");
        let out = Command::new("desktop-file-validate")
            .arg(&generated)
            .arg(&installer)
            .output()
            .unwrap();
        let _ = fs::remove_dir_all(&dir);
        let text = String::from_utf8_lossy(&out.stdout) + String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success() && text.trim().is_empty(), "{text}");
    }
}
