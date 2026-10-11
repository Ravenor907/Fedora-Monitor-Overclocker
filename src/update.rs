//! Updates with a Stable and a Testing channel, the same way Ally Hub does it.
//!
//! * The `main` branch is **Stable** and `testing` holds the owner's **test
//!   builds**. Each branch's `VERSION` file names the build it carries.
//! * A test build is the Stable version plus a revision: Stable 0.2.0, then
//!   testing 0.2.0.1, 0.2.0.2, ... (at most [`MAX_TEST_REVISIONS`]). When the
//!   owner ships, Stable gets the next x.y.z and testing restarts at its .1.
//! * The binaries come from the GitHub release tagged `v<VERSION>`. Test
//!   builds are pre-releases, so `releases/latest` (and with it install.sh)
//!   only ever sees Stable.
//! * Stable devices only look at `main`. Devices on Testing take whichever of
//!   `main` and `testing` has the higher build.
//! * Every install keeps the binary it replaces, so a bad build can be rolled
//!   back: from the app, with `monitor-oc update --rollback`, from the Testing
//!   Rescue menu entry (plain shell, works when the app won't start), or on
//!   its own when a new build fails to start [`MAX_FAILED_STARTS`] times.
//!
//! Downloads go through `curl` and checksums through `sha256sum`, which every
//! supported system has, so the static CLI build needs no TLS library.

use crate::install;
use anyhow::{Context, Result, bail};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const REPO: &str = "Ravenor907/monitor-oc";

/// Test builds per Stable version before a cycle has to ship (the owner's
/// rule: needing more means a cycle carries too much). Enforced by a test
/// on VERSION.
#[cfg_attr(not(test), allow(dead_code))]
pub const MAX_TEST_REVISIONS: u32 = 10;

/// Starts of a freshly installed build that may end before it counts as
/// working; the next start rolls back.
pub const MAX_FAILED_STARTS: u32 = 2;

/// Seconds the app has to run before a fresh build counts as working.
#[cfg_attr(not(feature = "gui"), allow(dead_code))]
pub const HEALTHY_AFTER_SECS: u64 = 5;

const RESCUE_ID: &str = "io.github.ravenor907.MonitorOC.Rescue";

/// This build's number, from the `VERSION` file (Cargo's version is its
/// first three parts).
pub fn version() -> &'static str {
    include_str!("../VERSION").trim()
}

/// "0.2.0.1 (testing)" for test builds, "0.2.1" for Stable.
pub fn label() -> String {
    label_of(version())
}

pub fn label_of(version: &str) -> String {
    if is_test_build(version) {
        format!("{version} (testing)")
    } else {
        version.to_string()
    }
}

pub fn is_test_build(version: &str) -> bool {
    Build::parse(version).is_some_and(|b| b.revision() > 0)
}

/// A build number: major.minor.patch plus an optional test revision.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Build([u32; 4]);

impl Build {
    pub fn parse(text: &str) -> Option<Build> {
        let text = text.trim();
        let text = text.strip_prefix('v').unwrap_or(text);
        let parts: Vec<&str> = text.split('.').collect();
        if !(3..=4).contains(&parts.len()) {
            return None;
        }
        let mut out = [0; 4];
        for (slot, part) in out.iter_mut().zip(&parts) {
            if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            *slot = part.parse().ok()?;
        }
        Some(Build(out))
    }

    pub fn current() -> Build {
        Build::parse(version()).expect("VERSION holds a valid build number")
    }

    pub fn revision(self) -> u32 {
        self.0[3]
    }

    pub fn is_test(self) -> bool {
        self.revision() > 0
    }

    pub fn label(self) -> String {
        label_of(&self.to_string())
    }
}

impl fmt::Display for Build {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [a, b, c, r] = self.0;
        if r == 0 {
            write!(f, "{a}.{b}.{c}")
        } else {
            write!(f, "{a}.{b}.{c}.{r}")
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Channel {
    Stable,
    Testing,
}

impl Channel {
    fn name(self) -> &'static str {
        match self {
            Channel::Stable => "stable",
            Channel::Testing => "testing",
        }
    }

    /// Branches whose builds this channel installs.
    fn branches(self) -> &'static [&'static str] {
        match self {
            Channel::Stable => &["main"],
            Channel::Testing => &["main", "testing"],
        }
    }
}

impl fmt::Display for Channel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Channel::Stable => "Stable",
            Channel::Testing => "Testing",
        })
    }
}

impl std::str::FromStr for Channel {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Channel> {
        match s.trim().to_ascii_lowercase().as_str() {
            "stable" | "main" => Ok(Channel::Stable),
            "testing" | "test" => Ok(Channel::Testing),
            other => bail!("unknown update channel {other:?} (use stable or testing)"),
        }
    }
}

/// What to tell someone before they switch to Testing.
pub const TESTING_DISCLAIMER: &[&str] = &[
    "Test builds get new fixes and features first, before the owner has run them long enough to call them stable. They can have bugs.",
    "Every update keeps the version it replaces. Roll back from Updates, with `monitor-oc update --rollback`, or with Monitor OC Testing Rescue in the app menu (Desktop Mode), which works even when a test build won't open. A build that fails to start three times in a row rolls itself back.",
];

/// `key=value` lines, the format of monitor-oc's other state files. Later
/// lines win; `bad` may repeat.
fn values(text: &str) -> Vec<(&str, &str)> {
    text.lines()
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.trim(), v.trim()))
        .collect()
}

fn last<'a>(pairs: &[(&'a str, &'a str)], key: &str) -> Option<&'a str> {
    pairs
        .iter()
        .rev()
        .find(|(k, _)| *k == key)
        .map(|(_, v)| *v)
        .filter(|v| !v.is_empty())
}

/// The user's choices, in `~/.config/monitor-oc/update.conf`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    pub channel: Channel,
    /// Install updates on their own when the app starts.
    pub auto: bool,
}

impl Settings {
    /// Someone running a test build is on Testing until they say otherwise.
    pub fn default_for(version: &str) -> Settings {
        Settings {
            channel: if is_test_build(version) {
                Channel::Testing
            } else {
                Channel::Stable
            },
            auto: true,
        }
    }

    pub fn parse(text: &str, version: &str) -> Settings {
        let pairs = values(text);
        let default = Settings::default_for(version);
        Settings {
            channel: last(&pairs, "channel")
                .and_then(|v| v.parse().ok())
                .unwrap_or(default.channel),
            auto: last(&pairs, "auto").map_or(default.auto, |v| v != "0" && v != "false"),
        }
    }

    pub fn render(&self) -> String {
        format!(
            "channel={}\nauto={}\n",
            self.channel.name(),
            u8::from(self.auto)
        )
    }
}

/// What the updater remembers, in `~/.local/share/monitor-oc/update-state`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UpdateState {
    /// The build the last update installed.
    pub installed: Option<String>,
    /// The build kept in `previous/` for a rollback.
    pub previous: Option<String>,
    /// A fresh build that hasn't run long enough yet to count as working.
    pub pending: Option<String>,
    pub starts: u32,
    /// Builds that were rolled back; never offered again.
    pub bad: Vec<String>,
}

impl UpdateState {
    pub fn parse(text: &str) -> UpdateState {
        let pairs = values(text);
        let mut bad: Vec<String> = Vec::new();
        for (k, v) in &pairs {
            if *k == "bad" && !v.is_empty() && !bad.iter().any(|b| b == v) {
                bad.push(v.to_string());
            }
        }
        UpdateState {
            installed: last(&pairs, "installed").map(String::from),
            previous: last(&pairs, "previous").map(String::from),
            pending: last(&pairs, "pending").map(String::from),
            starts: last(&pairs, "starts")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0),
            bad,
        }
    }

    pub fn render(&self) -> String {
        let mut out = String::new();
        for (key, value) in [
            ("installed", &self.installed),
            ("previous", &self.previous),
            ("pending", &self.pending),
        ] {
            if let Some(v) = value {
                out += &format!("{key}={v}\n");
            }
        }
        if self.starts > 0 {
            out += &format!("starts={}\n", self.starts);
        }
        for b in &self.bad {
            out += &format!("bad={b}\n");
        }
        out
    }

    pub fn is_bad(&self, build: Build) -> bool {
        self.bad.iter().any(|b| Build::parse(b) == Some(build))
    }
}

/// Where the updater keeps things for one user.
#[derive(Clone, Debug, PartialEq)]
pub struct Paths {
    /// `~/.local/bin/monitor-oc`, the installed program.
    pub binary: PathBuf,
    pub settings: PathBuf,
    /// `~/.local/share/monitor-oc`.
    pub data: PathBuf,
    pub state: PathBuf,
    pub previous: PathBuf,
    pub downloads: PathBuf,
    pub rescue_script: PathBuf,
    pub rescue_entry: PathBuf,
}

impl Paths {
    pub fn for_current_user() -> Result<Paths> {
        let installed = install::Paths::for_current_user()?;
        let env = |name| {
            std::env::var_os(name)
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
        };
        Ok(Paths::new(
            &installed.home,
            env("XDG_CONFIG_HOME").as_deref(),
            env("XDG_DATA_HOME").as_deref(),
        ))
    }

    pub fn new(home: &Path, config_home: Option<&Path>, data_home: Option<&Path>) -> Paths {
        let config = config_home.map_or_else(|| home.join(".config"), Path::to_path_buf);
        let share = data_home.map_or_else(|| home.join(".local/share"), Path::to_path_buf);
        let data = share.join("monitor-oc");
        Paths {
            binary: home.join(".local/bin/monitor-oc"),
            settings: config.join("monitor-oc/update.conf"),
            state: data.join("update-state"),
            previous: data.join("previous/monitor-oc"),
            downloads: data.join("downloads"),
            rescue_script: data.join("testing-rescue.sh"),
            rescue_entry: share.join(format!("applications/{RESCUE_ID}.desktop")),
            data,
        }
    }

    pub fn settings(&self) -> Settings {
        fs::read_to_string(&self.settings)
            .map(|t| Settings::parse(&t, version()))
            .unwrap_or_else(|_| Settings::default_for(version()))
    }

    pub fn save_settings(&self, settings: &Settings) -> Result<()> {
        write_file(&self.settings, settings.render().as_bytes(), 0o644)
    }

    pub fn state(&self) -> UpdateState {
        fs::read_to_string(&self.state)
            .map(|t| UpdateState::parse(&t))
            .unwrap_or_default()
    }

    pub fn save_state(&self, state: &UpdateState) -> Result<()> {
        write_file(&self.state, state.render().as_bytes(), 0o644)
    }

    /// The version kept for a rollback, if any.
    #[cfg_attr(not(feature = "gui"), allow(dead_code))]
    pub fn rollback_target(&self) -> Option<String> {
        if !self.previous.is_file() {
            return None;
        }
        Some(
            self.state()
                .previous
                .unwrap_or_else(|| "the previous version".into()),
        )
    }
}

/// Write through a temporary file and a rename, like `install` does.
fn write_file(path: &Path, bytes: &[u8], mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let dir = path.parent().context("path has no parent")?;
    fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let tmp = dir.join(format!(".{name}.new-{}", std::process::id()));
    let result = fs::write(&tmp, bytes)
        .and_then(|()| fs::set_permissions(&tmp, fs::Permissions::from_mode(mode)))
        .and_then(|()| fs::rename(&tmp, path));
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result.with_context(|| format!("writing {}", path.display()))
}

/// The release file this build updates from.
pub fn asset_name() -> &'static str {
    if cfg!(feature = "gui") {
        "monitor-oc-x86_64"
    } else {
        "monitor-oc-x86_64-musl"
    }
}

fn version_url(branch: &str) -> String {
    format!("https://raw.githubusercontent.com/{REPO}/{branch}/VERSION")
}

fn release_url(build: Build, file: &str) -> String {
    format!("https://github.com/{REPO}/releases/download/v{build}/{file}")
}

/// curl's exit code for an HTTP error such as 404 with `--fail`.
const CURL_HTTP_ERROR: i32 = 22;

enum Fetch {
    Ok(Vec<u8>),
    NotFound,
}

fn curl(args: &[&str]) -> Result<Fetch> {
    let out = Command::new("curl")
        .args(["-fsSL", "--retry", "2", "--connect-timeout", "15"])
        .args(args)
        .output()
        .context("running curl (is it installed?)")?;
    if out.status.success() {
        return Ok(Fetch::Ok(out.stdout));
    }
    if out.status.code() == Some(CURL_HTTP_ERROR) {
        return Ok(Fetch::NotFound);
    }
    let err = String::from_utf8_lossy(&out.stderr);
    bail!(
        "could not reach GitHub ({})",
        err.trim().trim_start_matches("curl: ")
    )
}

/// The build a branch holds, or None while the branch has no VERSION file.
fn branch_build(branch: &str) -> Result<Option<Build>> {
    match curl(&["--max-time", "30", &version_url(branch)])? {
        Fetch::NotFound => Ok(None),
        Fetch::Ok(body) => {
            let text = String::from_utf8_lossy(&body);
            Build::parse(&text)
                .map(Some)
                .with_context(|| format!("the {branch} branch has an unreadable VERSION"))
        }
    }
}

/// What a check found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Check {
    /// A newer build on the chosen channel.
    pub newer: Option<Build>,
    /// Stable's build, when the running build is a test build that is newer
    /// than it (offered as "go back to Stable").
    pub stable: Option<Build>,
}

/// The newest build of `channel` that wasn't rolled back.
pub fn check(paths: &Paths, channel: Channel) -> Result<Check> {
    let state = paths.state();
    let current = Build::current();
    let mut builds = Vec::new();
    let mut stable = None;
    for branch in channel.branches() {
        if let Some(build) = branch_build(branch)? {
            if *branch == "main" {
                stable = Some(build);
            }
            builds.push(build);
        }
    }
    if channel == Channel::Stable && stable.is_none() {
        stable = branch_build("main")?;
    }
    let newer = pick_newer(current, &builds, &state);
    let stable = stable.filter(|s| current.is_test() && *s < current && !state.is_bad(*s));
    Ok(Check { newer, stable })
}

fn pick_newer(current: Build, builds: &[Build], state: &UpdateState) -> Option<Build> {
    builds
        .iter()
        .copied()
        .filter(|b| *b > current && !state.is_bad(*b))
        .max()
}

/// Download, verify and install `build` (newer or older), keeping the
/// installed binary for a rollback. Returns what was installed.
pub fn install(paths: &Paths, build: Build) -> Result<Build> {
    if std::env::consts::ARCH != "x86_64" {
        bail!("releases are only built for x86_64");
    }
    let asset = asset_name();
    let dir = paths.downloads.join(build.to_string());
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let result = install_from(paths, build, asset, &dir);
    let _ = fs::remove_dir_all(&dir);
    result.map(|()| build)
}

fn install_from(paths: &Paths, build: Build, asset: &str, dir: &Path) -> Result<()> {
    let file = dir.join(asset);
    let sums = format!("{asset}.sha256");
    for (name, to) in [(asset, file.clone()), (sums.as_str(), dir.join(&sums))] {
        let to_str = to.to_str().context("download path is not valid UTF-8")?;
        match curl(&["--max-time", "600", "-o", to_str, &release_url(build, name)])? {
            Fetch::Ok(_) => {}
            Fetch::NotFound => bail!(
                "{} isn't published yet; its release is probably still building. Try again in a few minutes",
                build.label()
            ),
        }
    }
    let ok = Command::new("sha256sum")
        .args(["-c", "--status", sums.as_str()])
        .current_dir(dir)
        .status()
        .context("running sha256sum")?
        .success();
    if !ok {
        bail!("the download of {} failed its checksum", build.label());
    }
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&file, fs::Permissions::from_mode(0o755))?;
    }
    let reported = run_version(&file).context("the downloaded build won't run")?;
    if Build::parse(&reported) != Some(build) {
        bail!("the download says it is {reported}, not {}", build.label());
    }

    // Keep what is installed now, so this can be undone.
    let mut state = paths.state();
    let replaced = if paths.binary.is_file() {
        let version = run_version(&paths.binary).unwrap_or_else(|_| version().to_string());
        let bytes = fs::read(&paths.binary)
            .with_context(|| format!("reading {}", paths.binary.display()))?;
        write_file(&paths.previous, &bytes, 0o755)?;
        Some(version)
    } else {
        None
    };

    // The new build installs itself, which also refreshes the menu entry.
    let out = Command::new(&file)
        .arg("install")
        .output()
        .context("running the new build's installer")?;
    if !out.status.success() {
        bail!(
            "the new build's installer failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    state.previous = replaced.or(state.previous);
    state.installed = Some(build.to_string());
    state.pending = Some(build.to_string());
    state.starts = 0;
    paths.save_state(&state)
}

/// What `monitor-oc --version` of `binary` says, e.g. "0.2.0.1".
fn run_version(binary: &Path) -> Result<String> {
    let out = Command::new(binary)
        .arg("--version")
        .output()
        .with_context(|| format!("running {}", binary.display()))?;
    if !out.status.success() {
        bail!("{} --version failed", binary.display());
    }
    let text = String::from_utf8_lossy(&out.stdout);
    text.split_whitespace()
        .last()
        .map(String::from)
        .context("no version in the output")
}

/// Put the kept build back and never offer the one it replaces again.
/// Returns the version that is installed now.
pub fn rollback(paths: &Paths) -> Result<String> {
    if !paths.previous.is_file() {
        bail!("no earlier version is kept, so there is nothing to roll back to");
    }
    let mut state = paths.state();
    let bad = state
        .installed
        .clone()
        .or_else(|| run_version(&paths.binary).ok());
    let out = Command::new(&paths.previous)
        .arg("install")
        .output()
        .context("running the previous version's installer")?;
    if !out.status.success() {
        bail!(
            "the previous version would not install: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let restored = state
        .previous
        .take()
        .or_else(|| run_version(&paths.previous).ok())
        .unwrap_or_else(|| "the previous version".into());
    if let Some(bad) = bad.filter(|b| Build::parse(b) != Build::parse(&restored))
        && !state.bad.contains(&bad)
    {
        state.bad.push(bad);
    }
    state.installed = Some(restored.clone());
    state.pending = None;
    state.starts = 0;
    paths.save_state(&state)?;
    let _ = fs::remove_file(&paths.previous);
    Ok(restored)
}

/// Called before the app opens its window. Counts starts of a fresh build;
/// once it has failed [`MAX_FAILED_STARTS`] times, rolls back and returns the
/// version that was given up on.
#[cfg_attr(not(feature = "gui"), allow(dead_code))]
pub fn startup_check(paths: &Paths) -> Option<String> {
    let mut state = paths.state();
    let current = version();
    if state.pending.as_deref() != Some(current) {
        return None;
    }
    if state.starts >= MAX_FAILED_STARTS && paths.previous.is_file() {
        return rollback(paths).ok().map(|_| current.to_string());
    }
    state.starts += 1;
    let _ = paths.save_state(&state);
    None
}

/// The running build works: stop counting its starts.
#[cfg_attr(not(feature = "gui"), allow(dead_code))]
pub fn mark_healthy(paths: &Paths) {
    let mut state = paths.state();
    if state.pending.as_deref() == Some(version()) {
        state.pending = None;
        state.starts = 0;
        let _ = paths.save_state(&state);
    }
}

/// Single-quote for sh.
fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

fn rescue_script(paths: &Paths) -> String {
    let q = |p: &Path| sh_quote(&p.to_string_lossy());
    format!(
        r#"#!/bin/sh
# Monitor OC Testing Rescue. Plain shell on purpose: it has to work when a
# test build of Monitor OC won't even start.
BIN={bin}
PREV={prev}
STATE={state}
SETTINGS={settings}
ENTRY={entry}
DATA={data}
STABLE='https://raw.githubusercontent.com/{REPO}/main/install.sh'

set_stable() {{
    mkdir -p "$(dirname "$SETTINGS")"
    {{ grep -v '^channel=' "$SETTINGS" 2>/dev/null; echo channel=stable; }} > "$SETTINGS.new" &&
        mv "$SETTINGS.new" "$SETTINGS"
    printf 'pending=\n' >> "$STATE"
    rm -f "$ENTRY"
}}

echo "Monitor OC Testing Rescue"
echo
echo "  1) Roll back to the previous version"
echo "  2) Go back to Stable (installs the latest stable release)"
echo "  3) Uninstall Monitor OC"
echo "  4) Close"
echo
printf 'Choose 1-4: '
read -r choice
echo
case $choice in
1)
    if [ ! -x "$PREV" ]; then
        echo "No earlier version is kept, so there is nothing to roll back to."
    elif "$PREV" install > /dev/null; then
        bad=$(sed -n 's/^installed=//p' "$STATE" 2> /dev/null | tail -n 1)
        good=$(sed -n 's/^previous=//p' "$STATE" 2> /dev/null | tail -n 1)
        {{
            [ -n "$bad" ] && echo "bad=$bad"
            echo "installed=$good"
            echo "previous="
            echo "pending="
        }} >> "$STATE"
        rm -f "$PREV"
        echo "Rolled back${{good:+ to $good}}. Open Monitor OC again."
    else
        echo "The previous version would not install."
    fi
    ;;
2)
    if curl -fsSL "$STABLE" | sh; then
        set_stable
        echo
        echo "Back on Stable."
    else
        echo "Could not install Stable. Check the internet connection and try again."
    fi
    ;;
3)
    "$BIN" uninstall || {{ [ -x "$PREV" ] && "$PREV" uninstall; }} || rm -f "$BIN"
    rm -rf "$DATA"
    rm -f "$ENTRY"
    ;;
esac
echo
printf 'Press Enter to close.'
read -r _
"#,
        bin = q(&paths.binary),
        prev = q(&paths.previous),
        state = q(&paths.state),
        settings = q(&paths.settings),
        entry = q(&paths.rescue_entry),
        data = q(&paths.data),
    )
}

fn rescue_entry(script: &Path) -> Result<String> {
    let script = script
        .to_str()
        .context("the rescue path is not valid UTF-8")?;
    if script.chars().any(char::is_control) {
        bail!("the rescue path contains control characters");
    }
    Ok(format!(
        "[Desktop Entry]
Type=Application
Name=Monitor OC Testing Rescue
Comment=Roll back a Monitor OC test build, go back to Stable, or uninstall
Exec=sh {}
Icon={}
Terminal=true
Categories=Settings;HardwareSettings;
",
        install::exec_arg(script),
        install::APP_ID
    ))
}

/// On Testing, keep the rescue script and its menu entry; otherwise remove them.
pub fn sync_rescue(paths: &Paths, channel: Channel) -> Result<()> {
    if channel == Channel::Testing {
        write_file(&paths.rescue_script, rescue_script(paths).as_bytes(), 0o755)?;
        write_file(
            &paths.rescue_entry,
            rescue_entry(&paths.rescue_script)?.as_bytes(),
            0o644,
        )
    } else {
        for path in [&paths.rescue_script, &paths.rescue_entry] {
            match fs::remove_file(path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e).with_context(|| format!("removing {}", path.display())),
            }
        }
        Ok(())
    }
}

/// Switch channels and keep the rescue entry in step.
pub fn set_channel(paths: &Paths, channel: Channel) -> Result<Settings> {
    let mut settings = paths.settings();
    settings.channel = channel;
    paths.save_settings(&settings)?;
    sync_rescue(paths, channel)?;
    Ok(settings)
}

/// For `uninstall`: the kept build, downloads, rescue entry and update state.
pub fn remove_all(paths: &Paths) -> Result<Vec<PathBuf>> {
    let mut removed = Vec::new();
    if paths.rescue_entry.exists() {
        fs::remove_file(&paths.rescue_entry)
            .with_context(|| format!("removing {}", paths.rescue_entry.display()))?;
        removed.push(paths.rescue_entry.clone());
    }
    if paths.data.exists() {
        fs::remove_dir_all(&paths.data)
            .with_context(|| format!("removing {}", paths.data.display()))?;
        removed.push(paths.data.clone());
    }
    Ok(removed)
}

/// `monitor-oc update`.
pub fn command(check_only: bool, channel: Option<Channel>, undo: bool) -> Result<()> {
    let paths = Paths::for_current_user()?;
    if undo {
        let restored = rollback(&paths)?;
        println!("Rolled back to {}.", label_of(&restored));
        return Ok(());
    }
    let mut settings = paths.settings();
    if let Some(channel) = channel {
        if channel == Channel::Testing && settings.channel != Channel::Testing {
            println!("Switching to Testing.\n");
            for line in TESTING_DISCLAIMER {
                println!("{line}\n");
            }
        }
        settings = set_channel(&paths, channel)?;
        println!("Update channel: {}.", settings.channel);
    }
    println!("Installed: {}. Checking {}…", label(), settings.channel);
    let found = check(&paths, settings.channel)?;
    let target = match (found.newer, found.stable) {
        (Some(newer), _) => Some(newer),
        // Leaving Testing: go back to Stable's build when asked to update.
        (None, Some(stable)) if settings.channel == Channel::Stable => Some(stable),
        _ => None,
    };
    let Some(target) = target else {
        println!("Up to date.");
        return Ok(());
    };
    if check_only {
        println!(
            "{} is available. Run `monitor-oc update` to install it.",
            target.label()
        );
        return Ok(());
    }
    println!("Installing {}…", target.label());
    install(&paths, target)?;
    println!(
        "Installed {}. Close and reopen Monitor OC to use it.\n\
         Changed your mind? monitor-oc update --rollback",
        target.label()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(s: &str) -> Build {
        Build::parse(s).unwrap()
    }

    #[test]
    fn version_file_matches_cargo() {
        let build = Build::current();
        let cargo = Build::parse(env!("CARGO_PKG_VERSION")).unwrap();
        assert_eq!(
            Build([build.0[0], build.0[1], build.0[2], 0]),
            cargo,
            "VERSION must be Cargo's version plus an optional test revision"
        );
        assert!(
            build.revision() <= MAX_TEST_REVISIONS,
            "at most {MAX_TEST_REVISIONS} test revisions per cycle: ship or rethink this one"
        );
    }

    #[test]
    fn builds_parse_and_order() {
        assert_eq!(b("0.2.0").to_string(), "0.2.0");
        assert_eq!(b("v0.2.0.3").to_string(), "0.2.0.3");
        assert_eq!(b(" 1.2.3\n"), Build([1, 2, 3, 0]));
        assert!(b("0.2.0.1") > b("0.2.0"));
        assert!(b("0.2.1") > b("0.2.0.9"));
        assert!(b("0.2.0.10") > b("0.2.0.9"));
        for bad in ["", "1.2", "1.2.3.4.5", "1.2.x", "1..2", "-1.2.3"] {
            assert_eq!(Build::parse(bad), None, "{bad:?}");
        }
        assert!(is_test_build("0.2.0.1"));
        assert!(!is_test_build("0.2.0"));
        assert_eq!(label_of("0.2.0.1"), "0.2.0.1 (testing)");
    }

    #[test]
    fn settings_round_trip_with_defaults() {
        assert_eq!(Settings::default_for("0.2.0.1").channel, Channel::Testing);
        assert_eq!(Settings::default_for("0.2.0").channel, Channel::Stable);
        let s = Settings {
            channel: Channel::Testing,
            auto: false,
        };
        assert_eq!(Settings::parse(&s.render(), "0.2.0"), s);
        assert_eq!(
            Settings::parse("channel=nonsense\n", "0.2.0"),
            Settings::default_for("0.2.0")
        );
        // The rescue script appends lines; the last one wins.
        assert_eq!(
            Settings::parse("channel=testing\nchannel=stable\n", "0.2.0.1").channel,
            Channel::Stable
        );
    }

    #[test]
    fn state_reads_what_the_rescue_script_appends() {
        let text = "installed=0.2.0.2\nprevious=0.2.0.1\npending=0.2.0.2\nstarts=2\n\
                    bad=0.2.0.2\ninstalled=0.2.0.1\nprevious=\npending=\n";
        let state = UpdateState::parse(text);
        assert_eq!(state.installed.as_deref(), Some("0.2.0.1"));
        assert_eq!(state.previous, None);
        assert_eq!(state.pending, None);
        assert_eq!(state.bad, vec!["0.2.0.2".to_string()]);
        assert_eq!(UpdateState::parse(&state.render()), state);
    }

    #[test]
    fn newest_good_build_wins() {
        let state = UpdateState {
            bad: vec!["0.2.0.3".into()],
            ..UpdateState::default()
        };
        let cur = b("0.2.0.1");
        assert_eq!(
            pick_newer(cur, &[b("0.2.0"), b("0.2.0.2")], &state),
            Some(b("0.2.0.2"))
        );
        assert_eq!(pick_newer(cur, &[b("0.2.0"), b("0.2.0.3")], &state), None);
        assert_eq!(
            pick_newer(cur, &[b("0.2.1"), b("0.2.0.3")], &state),
            Some(b("0.2.1"))
        );
    }

    #[test]
    fn paths_follow_xdg() {
        let p = Paths::new(Path::new("/home/deck"), None, None);
        assert_eq!(
            p.settings,
            Path::new("/home/deck/.config/monitor-oc/update.conf")
        );
        assert_eq!(
            p.previous,
            Path::new("/home/deck/.local/share/monitor-oc/previous/monitor-oc")
        );
        assert_eq!(
            p.rescue_entry,
            Path::new(
                "/home/deck/.local/share/applications/io.github.ravenor907.MonitorOC.Rescue.desktop"
            )
        );
        let p = Paths::new(
            Path::new("/h"),
            Some(Path::new("/c")),
            Some(Path::new("/d")),
        );
        assert_eq!(p.settings, Path::new("/c/monitor-oc/update.conf"));
        assert_eq!(p.data, Path::new("/d/monitor-oc"));
    }

    #[test]
    fn rescue_script_quotes_paths() {
        let p = Paths::new(Path::new("/home/it's me"), None, None);
        let script = rescue_script(&p);
        assert!(script.starts_with("#!/bin/sh\n"));
        assert!(script.contains(r"BIN='/home/it'\''s me/.local/bin/monitor-oc'"));
        assert!(script.contains("/main/install.sh"));
        let entry = rescue_entry(&p.rescue_script).unwrap();
        assert!(entry.contains("Terminal=true"));
        assert!(
            entry.contains("Exec=sh \"/home/it's me/.local/share/monitor-oc/testing-rescue.sh\"")
        );
    }

    #[test]
    fn rescue_script_is_valid_sh() {
        let p = Paths::new(Path::new("/home/deck"), None, None);
        let ok = Command::new("sh")
            .args(["-n", "-c", &rescue_script(&p)])
            .status()
            .map(|s| s.success());
        // `sh` is always there on Linux CI; skip quietly elsewhere.
        if let Ok(ok) = ok {
            assert!(ok, "the rescue script has a syntax error");
        }
    }

    #[test]
    fn sync_rescue_writes_and_removes() {
        let root = std::env::temp_dir().join(format!("monitor-oc-rescue-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let p = Paths::new(&root, None, None);
        sync_rescue(&p, Channel::Testing).unwrap();
        assert!(p.rescue_script.is_file() && p.rescue_entry.is_file());
        sync_rescue(&p, Channel::Stable).unwrap();
        assert!(!p.rescue_script.exists() && !p.rescue_entry.exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn startup_counts_and_clears() {
        let root = std::env::temp_dir().join(format!("monitor-oc-start-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let p = Paths::new(&root, None, None);
        p.save_state(&UpdateState {
            pending: Some(version().into()),
            ..UpdateState::default()
        })
        .unwrap();
        // No kept build: count, never roll back.
        for n in 1..=MAX_FAILED_STARTS + 1 {
            assert_eq!(startup_check(&p), None);
            assert_eq!(p.state().starts, n);
        }
        mark_healthy(&p);
        assert_eq!(p.state().pending, None);
        assert_eq!(p.state().starts, 0);
        let _ = fs::remove_dir_all(&root);
    }
}
