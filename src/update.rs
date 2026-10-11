//! Updates with a Stable and a Testing channel, the same way Ally Hub does it.
//!
//! * Monitor OC is installed from the repository: `git clone`, then
//!   `scripts/install.sh`, which builds it on the device with a private Rust
//!   compiler in `~/.local/share/monitor-oc/toolchain` (like Ally Hub's
//!   Python environment). No releases, packages or prebuilt binaries.
//! * One branch, `main`. Its `VERSION` is the newest build: a test build
//!   (x.y.z.N) or a Stable version (x.y.z). `STABLE` on main names the shipped
//!   Stable version and the commit it was built from (only the owner ships).
//! * Stable devices follow `STABLE`; Testing devices take whichever of
//!   `STABLE` and main's `VERSION` is higher. An update downloads that
//!   source from GitHub, builds it with `scripts/build.sh` and installs it.
//! * A test build is the Stable version plus a revision: Stable 0.2.0, then
//!   testing 0.2.0.1, 0.2.0.2, ... (at most [`MAX_TEST_REVISIONS`]).
//! * Every install keeps the binary it replaces, so a bad build can be rolled
//!   back: from the app, with `monitor-oc update --rollback`, from the Testing
//!   Rescue menu entry (plain shell, works when the app won't start), or on
//!   its own when a new build fails to start [`MAX_FAILED_STARTS`] times.

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
    "Every update keeps the version it replaces. Roll back from Settings, with `monitor-oc update --rollback`, or with Monitor OC Testing Rescue in the app menu (Desktop Mode), which works even when a test build won't open. A build that fails to start three times in a row rolls itself back.",
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
    /// Send error reports automatically (needs an access key; see `report`).
    pub reports: bool,
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
            reports: false,
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
            reports: last(&pairs, "reports").map_or(default.reports, |v| v == "1" || v == "true"),
        }
    }

    pub fn render(&self) -> String {
        format!(
            "channel={}\nauto={}\nreports={}\n",
            self.channel.name(),
            u8::from(self.auto),
            u8::from(self.reports)
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

/// Where everything comes from: the repository itself, like Ally Hub.
fn raw_url(git_ref: &str, file: &str) -> String {
    format!("https://raw.githubusercontent.com/{REPO}/{git_ref}/{file}")
}

fn tarball_url(git_ref: &str) -> String {
    format!("https://codeload.github.com/{REPO}/tar.gz/{git_ref}")
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

fn fetch_text(url: &str) -> Result<Option<String>> {
    Ok(match curl(&["--max-time", "30", url])? {
        Fetch::Ok(body) => Some(String::from_utf8_lossy(&body).into_owned()),
        Fetch::NotFound => None,
    })
}

/// A build and the source it is built from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub build: Build,
    /// `main`, or the commit Stable was shipped from.
    pub git_ref: String,
}

/// `STABLE` on main: the shipped version and the commit it was built from.
pub fn parse_stable(text: &str) -> Option<(Build, String)> {
    let mut words = text.split_whitespace();
    let build = Build::parse(words.next()?)?;
    let commit = words.next()?;
    (commit.len() == 40 && commit.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| (build, commit.to_ascii_lowercase()))
}

/// What a check found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Check {
    /// A newer build on the chosen channel.
    pub newer: Option<Target>,
    /// Stable's build, when the running build is a test build that is newer
    /// than it (offered as "go back to Stable").
    pub stable: Option<Target>,
}

/// Stable devices follow `STABLE` on main; Testing devices also take main's
/// own `VERSION`, whichever is higher. Rolled-back builds are skipped.
pub fn check(paths: &Paths, channel: Channel) -> Result<Check> {
    let state = paths.state();
    let current = Build::current();
    let stable = fetch_text(&raw_url("main", "STABLE"))?
        .and_then(|t| parse_stable(&t))
        .map(|(build, commit)| Target {
            build,
            git_ref: commit,
        });
    let mut candidates: Vec<Target> = stable.iter().cloned().collect();
    if channel == Channel::Testing
        && let Some(build) = fetch_text(&raw_url("main", "VERSION"))?
            .as_deref()
            .and_then(Build::parse)
    {
        candidates.push(Target {
            build,
            git_ref: "main".into(),
        });
    }
    let builds: Vec<Build> = candidates.iter().map(|t| t.build).collect();
    let newer = pick_newer(current, &builds, &state)
        .and_then(|b| candidates.into_iter().find(|t| t.build == b));
    let stable =
        stable.filter(|s| current.is_test() && s.build < current && !state.is_bad(s.build));
    Ok(Check { newer, stable })
}

fn pick_newer(current: Build, builds: &[Build], state: &UpdateState) -> Option<Build> {
    builds
        .iter()
        .copied()
        .filter(|b| *b > current && !state.is_bad(*b))
        .max()
}

/// Download the source of `target`, build it with `scripts/build.sh` (the
/// private compiler the installer set up) and install it, keeping the
/// installed binary for a rollback. Returns what was installed. Takes a few
/// minutes, like the first install.
pub fn install(paths: &Paths, target: &Target) -> Result<Build> {
    let dir = paths.downloads.join(target.build.to_string());
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let result = install_from(paths, target, &dir);
    let _ = fs::remove_dir_all(&dir);
    result
}

fn install_from(paths: &Paths, target: &Target, dir: &Path) -> Result<Build> {
    let archive = dir.join("source.tar.gz");
    let src = dir.join("source");
    fs::create_dir_all(&src)?;
    let archive_str = archive
        .to_str()
        .context("download path is not valid UTF-8")?;
    match curl(&[
        "--max-time",
        "300",
        "-o",
        archive_str,
        &tarball_url(&target.git_ref),
    ])? {
        Fetch::Ok(_) => {}
        Fetch::NotFound => bail!("the source of {} isn't on GitHub", target.build.label()),
    }
    let unpacked = Command::new("tar")
        .args(["-xzf"])
        .arg(&archive)
        .arg("-C")
        .arg(&src)
        .arg("--strip-components=1")
        .status()
        .context("running tar")?;
    if !unpacked.success() {
        bail!("the download of {} is damaged", target.build.label());
    }
    let found = fs::read_to_string(src.join("VERSION"))
        .ok()
        .as_deref()
        .and_then(Build::parse)
        .context("the download has no VERSION")?;
    // main can move on between the check and the download: take the newer
    // build then (never an older or rolled-back one). A shipped commit is
    // exactly what it says.
    let ok = if target.git_ref == "main" {
        found >= target.build && found > Build::current() && !paths.state().is_bad(found)
    } else {
        found == target.build
    };
    if !ok {
        bail!("version mismatch ({found} instead of {})", target.build);
    }

    let out = Command::new("bash")
        .arg(src.join("scripts/build.sh"))
        .arg(&src)
        .output()
        .context("running the build")?;
    if !out.status.success() {
        let log = String::from_utf8_lossy(&out.stderr);
        let tail: Vec<&str> = log.lines().rev().take(25).collect();
        let tail: Vec<&str> = tail.into_iter().rev().collect();
        bail!("building {} failed:\n{}", found.label(), tail.join("\n"));
    }
    let built = String::from_utf8_lossy(&out.stdout);
    let file = PathBuf::from(built.lines().last().unwrap_or_default().trim());
    let reported = run_version(&file).context("the new build won't run")?;
    if Build::parse(&reported) != Some(found) {
        bail!("the new build says it is {reported}, not {}", found.label());
    }
    let build = found;

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
    paths.save_state(&state)?;
    Ok(build)
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
REPO='{REPO}'

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
echo "  2) Go back to Stable (downloads and builds it, a few minutes)"
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
    commit=$(curl -fsSL "https://raw.githubusercontent.com/$REPO/main/STABLE" 2> /dev/null |
        awk '{{print $2; exit}}')
    if [ -z "$commit" ]; then
        echo "Nothing has shipped to Stable yet."
    else
        src=$(mktemp -d)
        if curl -fsSL "https://codeload.github.com/$REPO/tar.gz/$commit" |
            tar -xz -C "$src" --strip-components=1 &&
            bash "$src/scripts/install.sh"; then
            set_stable
            echo
            echo "Back on Stable."
        else
            echo "Could not install Stable. Check the internet connection and try again."
        fi
        rm -rf "$src"
    fi
    ;;
3)
    "$BIN" uninstall --purge || {{ [ -x "$PREV" ] && "$PREV" uninstall --purge; }} || rm -f "$BIN"
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
            target.build.label()
        );
        return Ok(());
    }
    println!(
        "Downloading and building {} (a few minutes)…",
        target.build.label()
    );
    if let Err(e) = install(&paths, &target) {
        crate::report::auto(
            crate::report::Kind::UpdateFailure,
            &format!("Update didn't install: {e:#}"),
            &format!("`monitor-oc update` from {} failed:\n\n{e:#}", label()),
        );
        return Err(e);
    }
    println!(
        "Installed {}. Close and reopen Monitor OC to use it.\n\
         Changed your mind? monitor-oc update --rollback",
        target.build.label()
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
            reports: true,
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
    fn stable_file() {
        let sha = "0123456789abcdef0123456789abcdef01234567";
        assert_eq!(
            parse_stable(&format!("0.3.0 {sha}\n")),
            Some((b("0.3.0"), sha.to_string()))
        );
        assert_eq!(parse_stable("0.3.0"), None);
        assert_eq!(parse_stable("0.3.0 nothex"), None);
        assert_eq!(parse_stable(""), None);
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
        assert!(script.contains("main/STABLE"));
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
