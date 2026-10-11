//! Error reports, the same way Ally Hub does them: problems become GitHub
//! issues on the owner's repository, so they can be fixed without anyone
//! having to describe them.
//!
//! * **Automatic reports** (off by default, Settings): crashes, updates that
//!   didn't install, builds that rolled themselves back, refresh tests and
//!   privileged commands that failed.
//! * **Report a problem** (Settings or Diagnostics, or `monitor-oc report`):
//!   the user's own words plus everything needed to act on them; sent even
//!   with automatic reports off.
//! * Both need a GitHub **access key** (a fine-grained token that can write
//!   issues on the repository), kept in `~/.local/share/monitor-oc/github-key`.
//!
//! Reports are scrubbed (home folder, user and host names), the same problem
//! on the same version is sent at most every [`REPEAT_SECS`], and at most
//! [`MAX_PER_DAY`] go out a day. They wait in `reports/` until they are sent,
//! so nothing is lost offline or when the app crashes.

use crate::json;
use crate::update::{self, Paths};
use anyhow::{Context, Result, bail};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const MAX_PER_DAY: usize = 25;
pub const REPEAT_SECS: u64 = 6 * 3600;
/// GitHub allows 65,536 characters in an issue body or comment.
const BODY_MAX: usize = 60_000;
const LOG_MAX: u64 = 256 * 1024;
pub const LABEL: &str = "auto-report";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(feature = "gui"), allow(dead_code))]
pub enum Kind {
    /// From the user (Report a problem).
    User,
    Crash,
    UpdateFailure,
    TestFailure,
    CommandFailure,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::User => "user",
            Kind::Crash => "crash",
            Kind::UpdateFailure => "update-failure",
            Kind::TestFailure => "test-failure",
            Kind::CommandFailure => "command-failure",
        }
    }
}

/// What [`queue`] did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Queued {
    Queued(PathBuf),
    /// Automatic reports are off.
    Off,
    NoKey,
    /// The same problem was reported recently.
    Duplicate,
    /// The daily maximum was reached.
    Limit,
}

impl Queued {
    /// Plain words for the user when a report wasn't queued.
    pub fn hint(&self) -> &'static str {
        match self {
            Queued::Queued(_) => "",
            Queued::Off => {
                "Turn on error reports (Settings) to have problems like this fixed automatically."
            }
            Queued::NoKey => "Error reports need a GitHub access key. Add one in Settings.",
            Queued::Duplicate => {
                "This problem was already reported recently, so it wasn't sent again."
            }
            Queued::Limit => "Today's maximum of reports was reached; more go out tomorrow.",
        }
    }
}

fn reports_dir(p: &Paths) -> PathBuf {
    p.data.join("reports")
}
fn sent_file(p: &Paths) -> PathBuf {
    p.data.join("reports-sent")
}
fn key_file(p: &Paths) -> PathBuf {
    p.data.join("github-key")
}
fn log_file(p: &Paths) -> PathBuf {
    p.data.join("monitor-oc.log")
}

fn now() -> u64 {
    crate::gamescope::now_secs()
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let tmp = path.with_extension(format!("new-{}", std::process::id()));
    let result = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)
        .and_then(|mut f| f.write_all(bytes))
        .and_then(|()| fs::rename(&tmp, path));
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result.with_context(|| format!("writing {}", path.display()))
}

// ---------- activity log ----------

/// One line in the activity log that goes with reports.
pub fn log(component: &str, msg: &str) {
    let Ok(paths) = Paths::for_current_user() else {
        return;
    };
    let path = log_file(&paths);
    if fs::metadata(&path).is_ok_and(|m| m.len() > LOG_MAX) {
        let _ = fs::rename(&path, path.with_extension("log.1"));
    }
    let _ = fs::create_dir_all(&paths.data);
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(&path) {
        let line = msg.replace('\n', " ");
        let _ = writeln!(f, "{} [{component}] {line}", now());
    }
}

fn log_tail(paths: &Paths, lines: usize) -> String {
    let text = fs::read_to_string(log_file(paths)).unwrap_or_default();
    let all: Vec<&str> = text.lines().collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}

// ---------- access key ----------

pub fn has_key(paths: &Paths) -> bool {
    key(paths).is_some()
}

fn key(paths: &Paths) -> Option<String> {
    fs::read_to_string(key_file(paths))
        .ok()
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty())
}

pub fn save_key(paths: &Paths, key: &str) -> Result<()> {
    let key = key.trim();
    if key.is_empty() || key.chars().any(|c| c.is_whitespace() || c.is_control()) {
        bail!("that doesn't look like a GitHub access key");
    }
    write_private(&key_file(paths), key.as_bytes())
}

pub fn remove_key(paths: &Paths) -> Result<()> {
    match fs::remove_file(key_file(paths)) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}

// ---------- building a report ----------

/// A short stable id for "the same problem" (FNV-1a).
pub fn fingerprint(parts: &[&str]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in parts.join("|").bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")[..10].to_string()
}

fn read_trim(path: &str) -> String {
    fs::read_to_string(path)
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

/// Version, system, device and session, for every report.
pub fn environment() -> String {
    let os = fs::read_to_string("/etc/os-release").unwrap_or_default();
    let get = |key: &str| {
        os.lines()
            .find_map(|l| l.strip_prefix(&format!("{key}=")))
            .map(|v| v.trim_matches('"').to_string())
            .unwrap_or_else(|| "?".into())
    };
    let channel = Paths::for_current_user()
        .map(|p| p.settings().channel.to_string())
        .unwrap_or_else(|_| "?".into());
    format!(
        "- Monitor OC: {} ({channel} channel)\n- System: {} {} (build {})\n- Kernel: {}\n\
         - Device: {} {}\n- Session: {}",
        update::label(),
        get("NAME"),
        get("VERSION_ID"),
        get("BUILD_ID"),
        read_trim("/proc/sys/kernel/osrelease"),
        read_trim("/sys/class/dmi/id/sys_vendor"),
        read_trim("/sys/class/dmi/id/product_name"),
        if crate::gamescope::in_game_mode() {
            "Game Mode"
        } else {
            "Desktop"
        },
    )
}

/// Take personal details out: the home folder, user name and host name.
pub fn scrub(text: &str) -> String {
    let mut out = text.to_string();
    if let Some(home) = std::env::var_os("HOME")
        .map(|h| h.to_string_lossy().into_owned())
        .filter(|h| h.len() > 1)
    {
        out = out.replace(&home, "~");
    }
    for (value, stand_in) in [
        (std::env::var("USER").unwrap_or_default(), "user"),
        (read_trim("/proc/sys/kernel/hostname"), "host"),
    ] {
        if value.len() >= 3 {
            out = out.replace(&value, stand_in);
        }
    }
    out
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// A queued report on disk: `key=value` lines, `---`, then the body.
#[derive(Clone, Debug, PartialEq)]
pub struct Report {
    pub kind: String,
    pub title: String,
    pub fingerprint: String,
    pub version: String,
    pub testing: bool,
    pub time: u64,
    pub body: String,
}

impl Report {
    fn render(&self) -> String {
        format!(
            "kind={}\ntitle={}\nfingerprint={}\nversion={}\ntesting={}\ntime={}\n---\n{}",
            self.kind,
            self.title.replace('\n', " "),
            self.fingerprint,
            self.version,
            u8::from(self.testing),
            self.time,
            self.body
        )
    }

    fn parse(text: &str) -> Option<Report> {
        let (head, body) = text.split_once("\n---\n")?;
        let get = |k: &str| {
            head.lines()
                .find_map(|l| l.strip_prefix(&format!("{k}=")))
                .map(String::from)
        };
        Some(Report {
            kind: get("kind")?,
            title: get("title")?,
            fingerprint: get("fingerprint")?,
            version: get("version").unwrap_or_default(),
            testing: get("testing").as_deref() == Some("1"),
            time: get("time").and_then(|t| t.parse().ok()).unwrap_or(0),
            body: body.to_string(),
        })
    }
}

fn sent_times(paths: &Paths) -> Vec<(String, u64)> {
    fs::read_to_string(sent_file(paths))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let (k, t) = l.rsplit_once(' ')?;
            Some((k.to_string(), t.parse().ok()?))
        })
        .collect()
}

fn pending(paths: &Paths) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(reports_dir(paths))
        .map(|d| {
            d.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "report"))
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    files
}

pub fn pending_count(paths: &Paths) -> usize {
    pending(paths).len()
}

/// Save a report to send. `force` (Report a problem) skips the automatic
/// reports setting and the repeat check.
pub fn queue(
    paths: &Paths,
    kind: Kind,
    title: &str,
    details: &str,
    fingerprint_of: Option<&str>,
    force: bool,
) -> Queued {
    if !force && !paths.settings().reports {
        return Queued::Off;
    }
    if !has_key(paths) {
        return Queued::NoKey;
    }
    let fp = fingerprint(&[kind.name(), fingerprint_of.unwrap_or(title)]);
    let version = update::version().to_string();
    let t = now();
    if !force {
        let key = format!("{fp}@{version}");
        if sent_times(paths)
            .iter()
            .any(|(k, at)| *k == key && t.saturating_sub(*at) < REPEAT_SECS)
            || pending(paths)
                .iter()
                .any(|p| p.to_string_lossy().contains(&fp))
        {
            return Queued::Duplicate;
        }
    }
    let today = pending(paths)
        .iter()
        .filter(|p| {
            fs::metadata(p)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|m| m.elapsed().ok())
                .is_some_and(|age| age.as_secs() < 86_400)
        })
        .count();
    if today >= MAX_PER_DAY {
        return Queued::Limit;
    }
    let body = format!(
        "{details}\n\n### Environment\n{}\n\n### Activity log (last lines)\n```\n{}\n```\n",
        environment(),
        log_tail(paths, 60)
    );
    let report = Report {
        kind: kind.name().into(),
        title: scrub(truncate(title.trim(), 110)),
        fingerprint: fp.clone(),
        version: version.clone(),
        testing: update::is_test_build(&version),
        time: t,
        body: truncate(&scrub(&body), BODY_MAX).to_string(),
    };
    let path = reports_dir(paths).join(format!("{t}-{fp}.report"));
    match write_private(&path, report.render().as_bytes()) {
        Ok(()) => {
            log(
                "report",
                &format!("queued {}: {}", kind.name(), report.title),
            );
            Queued::Queued(path)
        }
        Err(_) => Queued::Off,
    }
}

/// "Report a problem": the user's words, plus the diagnostics.
pub fn user_report(paths: &Paths, text: &str, context: &str) -> Queued {
    let text = text.trim();
    let title = if text.is_empty() {
        "Problem reported from Monitor OC".to_string()
    } else {
        text.lines().next().unwrap_or(text).to_string()
    };
    let checks: String = crate::doctor::checks()
        .iter()
        .map(|c| format!("- {:?} {}: {}\n", c.level, c.title, c.detail))
        .collect();
    let details = format!(
        "Reported by hand{}:\n\n{text}\n\n### Diagnostics\n{checks}",
        if context.is_empty() {
            String::new()
        } else {
            format!(" on the {context} page")
        }
    );
    queue(
        paths,
        Kind::User,
        &title,
        &details,
        Some(&format!("{}{text}", now())),
        true,
    )
}

/// Queue a report for an error, if automatic reports are on.
pub fn auto(kind: Kind, title: &str, details: &str) -> Queued {
    log(kind.name(), title);
    let Ok(paths) = Paths::for_current_user() else {
        return Queued::Off;
    };
    let q = queue(&paths, kind, title, details, None, false);
    if matches!(q, Queued::Queued(_)) {
        send_in_background();
    }
    q
}

/// Crashes become reports (sent on the next start).
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        previous(info);
        let msg = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown panic".into());
        let at = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_else(|| "?".into());
        let thread = std::thread::current()
            .name()
            .unwrap_or("unnamed")
            .to_string();
        if let Ok(paths) = Paths::for_current_user() {
            let _ = queue(
                &paths,
                Kind::Crash,
                &format!("crash at {at}: {msg}"),
                &format!(
                    "Monitor OC crashed.\n\n- Where: `{at}` (thread {thread})\n- Message: {msg}"
                ),
                Some(&at),
                false,
            );
        }
    }));
}

// ---------- sending ----------

/// A GitHub API call with the access key. Returns the HTTP status and body.
fn github(paths: &Paths, method: &str, path: &str, body: Option<&str>) -> Result<(u16, String)> {
    let key = key(paths).context("no GitHub access key")?;
    let dir = reports_dir(paths);
    let tag = format!("{}-{}", std::process::id(), now());
    let header = dir.join(format!(".auth-{tag}"));
    let req = dir.join(format!(".req-{tag}"));
    let resp = dir.join(format!(".resp-{tag}"));
    write_private(&header, format!("Authorization: Bearer {key}\n").as_bytes())?;
    let mut cmd = Command::new("curl");
    cmd.args([
        "-sS",
        "--connect-timeout",
        "15",
        "--max-time",
        "60",
        "-X",
        method,
    ])
    .arg("-H")
    .arg(format!("@{}", header.display()))
    .args([
        "-H",
        "Accept: application/vnd.github+json",
        "-H",
        "X-GitHub-Api-Version: 2022-11-28",
        "-H",
        "User-Agent: monitor-oc",
        "-w",
        "%{http_code}",
        "-o",
    ])
    .arg(&resp);
    if let Some(body) = body {
        write_private(&req, body.as_bytes())?;
        cmd.args(["-H", "Content-Type: application/json", "--data-binary"])
            .arg(format!("@{}", req.display()));
    }
    cmd.arg(format!("https://api.github.com{path}"));
    let out = cmd.output();
    let text = fs::read_to_string(&resp).unwrap_or_default();
    for f in [&header, &req, &resp] {
        let _ = fs::remove_file(f);
    }
    let out = out.context("running curl")?;
    let status = String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .unwrap_or(0);
    if status == 0 {
        bail!(
            "could not reach GitHub ({})",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok((status, text))
}

/// Split long text into comment-sized pieces.
fn chunks(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let piece = truncate(rest, BODY_MAX);
        out.push(piece);
        rest = &rest[piece.len()..];
    }
    out
}

/// Send what is queued. Returns (sent, failed).
pub fn upload(paths: &Paths) -> (usize, usize) {
    if !has_key(paths) {
        return (0, 0);
    }
    let auto = paths.settings().reports;
    let files: Vec<(PathBuf, Report)> = pending(paths)
        .into_iter()
        .filter_map(|p| {
            let r = Report::parse(&fs::read_to_string(&p).ok()?)?;
            Some((p, r))
        })
        .filter(|(_, r)| auto || r.kind == Kind::User.name())
        .collect();
    if files.is_empty() {
        return (0, 0);
    }
    // One sender at a time (the app and the CLI may both try).
    let lock = reports_dir(paths).join(".sending");
    if fs::metadata(&lock)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|m| m.elapsed().ok())
        .is_some_and(|age| age.as_secs() > 600)
    {
        let _ = fs::remove_file(&lock);
    }
    if fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock)
        .is_err()
    {
        return (0, 0);
    }
    let result = upload_locked(paths, files);
    let _ = fs::remove_file(&lock);
    result
}

fn upload_locked(paths: &Paths, files: Vec<(PathBuf, Report)>) -> (usize, usize) {
    let repo = update::REPO;
    let open: Vec<(u64, String)> = github(
        paths,
        "GET",
        &format!("/repos/{repo}/issues?state=open&per_page=100&labels={LABEL}"),
        None,
    )
    .ok()
    .filter(|(s, _)| *s == 200)
    .and_then(|(_, body)| json::Json::parse(&body))
    .map(|v| {
        v.as_array()
            .iter()
            .filter_map(|i| {
                Some((
                    i.get("number")?.as_u64()?,
                    i.get("title")?.as_str()?.to_string(),
                ))
            })
            .collect()
    })
    .unwrap_or_default();
    let (mut sent, mut failed) = (0, 0);
    let mut times = sent_times(paths);
    for (path, r) in files {
        let existing = open.iter().find(|(_, t)| t.contains(&r.fingerprint));
        let status = match existing {
            Some((number, _)) => {
                let body = format!(
                    "Happened again (v{}).\n\n{}",
                    update::label_of(&r.version),
                    r.body
                );
                let mut status = 0;
                for piece in chunks(&body) {
                    status = github(
                        paths,
                        "POST",
                        &format!("/repos/{repo}/issues/{number}/comments"),
                        Some(&format!("{{\"body\":{}}}", json::string(piece))),
                    )
                    .map_or(0, |(s, _)| s);
                }
                status
            }
            None => {
                let prefix = if r.kind == Kind::User.name() {
                    "[report]"
                } else {
                    "[auto]"
                };
                let title = format!("{prefix} {} ({})", r.title, r.fingerprint);
                let mut labels = vec![LABEL.to_string(), r.kind.clone()];
                if r.testing {
                    labels.push("testing".into());
                }
                let labels_json: Vec<String> = labels.iter().map(|l| json::string(l)).collect();
                let with = format!(
                    "{{\"title\":{},\"body\":{},\"labels\":[{}]}}",
                    json::string(&title),
                    json::string(&r.body),
                    labels_json.join(",")
                );
                let path_api = format!("/repos/{repo}/issues");
                match github(paths, "POST", &path_api, Some(&with)) {
                    // Labels not allowed for this key: send without them.
                    Ok((422, _)) => {
                        let without = format!(
                            "{{\"title\":{},\"body\":{}}}",
                            json::string(&title),
                            json::string(&r.body)
                        );
                        github(paths, "POST", &path_api, Some(&without)).map_or(0, |(s, _)| s)
                    }
                    Ok((s, _)) => s,
                    Err(_) => 0,
                }
            }
        };
        if status == 200 || status == 201 {
            let _ = fs::remove_file(&path);
            times.push((format!("{}@{}", r.fingerprint, r.version), now()));
            sent += 1;
        } else {
            failed += 1;
            log("report", &format!("sending failed (HTTP {status})"));
            if matches!(status, 0 | 401 | 403 | 404) {
                break;
            }
        }
    }
    let week_ago = now().saturating_sub(7 * 86_400);
    let kept: String = times
        .iter()
        .filter(|(_, t)| *t > week_ago)
        .map(|(k, t)| format!("{k} {t}\n"))
        .collect();
    let _ = write_private(&sent_file(paths), kept.as_bytes());
    (sent, failed)
}

/// Send queued reports without blocking the caller.
pub fn send_in_background() {
    let _ = std::thread::Builder::new()
        .name("reports".into())
        .spawn(|| {
            if let Ok(paths) = Paths::for_current_user() {
                upload(&paths);
            }
        });
}

/// `monitor-oc report`.
pub fn command(message: &[String], set_key: bool, remove: bool) -> Result<()> {
    let paths = Paths::for_current_user()?;
    if remove {
        remove_key(&paths)?;
        println!("Access key removed.");
        return Ok(());
    }
    if set_key {
        println!("Paste the GitHub access key and press Enter:");
        let mut line = String::new();
        std::io::stdin().read_line(&mut line)?;
        save_key(&paths, &line)?;
        println!("Access key saved.");
        return Ok(());
    }
    if !message.is_empty() {
        match user_report(&paths, &message.join(" "), "") {
            Queued::Queued(_) => println!("Report saved."),
            other => bail!("{}", other.hint()),
        }
    }
    let waiting = pending_count(&paths);
    if waiting == 0 {
        println!("No reports waiting.");
        return Ok(());
    }
    if !has_key(&paths) {
        bail!(
            "{} report(s) waiting, but there's no access key: monitor-oc report --set-key",
            waiting
        );
    }
    let (sent, failed) = upload(&paths);
    println!(
        "Sent {sent} report(s){}.",
        if failed > 0 {
            format!(", {failed} failed")
        } else {
            String::new()
        }
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_paths(name: &str) -> (PathBuf, Paths) {
        let root =
            std::env::temp_dir().join(format!("monitor-oc-report-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let paths = Paths::new(&root, None, None);
        (root, paths)
    }

    #[test]
    fn fingerprints_are_stable() {
        assert_eq!(fingerprint(&["crash", "x"]), fingerprint(&["crash", "x"]));
        assert_ne!(fingerprint(&["crash", "x"]), fingerprint(&["crash", "y"]));
        assert_eq!(fingerprint(&["a"]).len(), 10);
    }

    #[test]
    fn reports_round_trip() {
        let r = Report {
            kind: "crash".into(),
            title: "crash at a.rs:1: boom".into(),
            fingerprint: "0123456789".into(),
            version: "0.2.0.1".into(),
            testing: true,
            time: 42,
            body: "line\n---\nmore".into(),
        };
        assert_eq!(Report::parse(&r.render()), Some(r));
    }

    #[test]
    fn queue_rules() {
        let (root, p) = temp_paths("queue");
        // No key: nothing queued, even when asked by hand.
        assert_eq!(queue(&p, Kind::User, "t", "d", None, true), Queued::NoKey);
        save_key(&p, "github_pat_example").unwrap();
        // Automatic reports are off by default.
        assert_eq!(queue(&p, Kind::Crash, "t", "d", None, false), Queued::Off);
        let mut s = p.settings();
        s.reports = true;
        p.save_settings(&s).unwrap();
        assert!(matches!(
            queue(&p, Kind::Crash, "t", "d", None, false),
            Queued::Queued(_)
        ));
        // The same problem waiting already: not again.
        assert_eq!(
            queue(&p, Kind::Crash, "t", "d", None, false),
            Queued::Duplicate
        );
        assert_eq!(pending_count(&p), 1);
        assert!(save_key(&p, "has space").is_err());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn long_text_is_cut_on_char_boundaries() {
        let s = "é".repeat(10);
        assert_eq!(truncate(&s, 5), "éé");
        assert_eq!(chunks(&"x".repeat(BODY_MAX + 5)).len(), 2);
    }
}
