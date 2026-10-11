//! Built-in screens under Gamescope (SteamOS Game Mode, Bazzite, ChimeraOS).
//!
//! Gamescope ignores the kernel's mode list for handheld panels it knows
//! (Steam Deck, ROG Ally, Legion Go, ...): it builds every refresh rate itself
//! from a Lua profile capped at the panel's stock range. Overclocking those
//! panels therefore means extending that profile, which Gamescope lets users do
//! with Lua files in `~/.config/gamescope/scripts`. No root is needed.

use crate::drm::{self, Connector};
use crate::edid::Edid;
use anyhow::{Context, Result, bail};
use std::fs;
use std::path::{Path, PathBuf};

const TEMPLATE: &str = include_str!("gamescope.lua");
const SCRIPT_NAME: &str = "zz-monitor-oc.lua";
const SYSTEM_SCRIPT_DIRS: [&str; 2] = ["/usr/share/gamescope/scripts", "/etc/gamescope/scripts"];
/// Highest rate monitor-oc will configure, as a multiple of the stock maximum.
const MAX_FACTOR: f64 = 1.6;

/// What Gamescope matches a display on (see `LookupDisplay` in Gamescope).
#[derive(Clone, Debug, PartialEq)]
pub struct PanelId {
    pub vendor: String,
    pub product: u16,
    pub model: String,
    pub data_string: String,
}

impl PanelId {
    pub fn from_edid(edid: &Edid) -> PanelId {
        PanelId {
            vendor: edid.manufacturer(),
            product: edid.product_code(),
            model: edid.descriptor_text(0xFC).unwrap_or_default(),
            data_string: edid.descriptor_text(0xFE).unwrap_or_default(),
        }
    }

    pub fn label(&self) -> String {
        let name = [&self.model, &self.data_string]
            .into_iter()
            .find(|s| !s.is_empty())
            .cloned()
            .unwrap_or_else(|| "unnamed panel".into());
        format!("{} {} (product 0x{:04X})", self.vendor, name, self.product)
    }
}

/// A built-in panel and what its EDID says.
#[derive(Clone, Debug)]
pub struct Panel {
    pub connector: String,
    pub id: PanelId,
    pub width: u16,
    pub height: u16,
    /// Rounded refresh rates the EDID lists at the native resolution.
    pub edid_rates: Vec<u32>,
}

impl Panel {
    pub fn from_edid(connector: &str, edid: &Edid) -> Result<Panel> {
        let timings = edid.timings();
        let (_, native) = timings
            .first()
            .context("the panel's EDID has no detailed timings")?;
        let mut edid_rates: Vec<u32> = timings
            .iter()
            .filter(|(_, d)| d.h_active == native.h_active && d.v_active == native.v_active)
            .map(|(_, d)| d.refresh_hz().round() as u32)
            .collect();
        edid_rates.sort_unstable();
        edid_rates.dedup();
        Ok(Panel {
            connector: connector.to_string(),
            id: PanelId::from_edid(edid),
            width: native.h_active,
            height: native.v_active,
            edid_rates,
        })
    }

    /// The first connected built-in panel with a readable EDID.
    pub fn detect() -> Result<Panel> {
        let conn = internal_connector().context("no connected built-in screen found")?;
        let raw = conn
            .edid()
            .context("the built-in screen has no readable EDID")?;
        Panel::from_edid(&conn.name, &Edid::parse(&raw)?)
    }
}

pub fn internal_connector() -> Option<Connector> {
    drm::connectors()
        .ok()?
        .into_iter()
        .find(|c| c.is_internal() && c.connected() && c.edid().is_some())
}

/// Gamescope's own profile for a panel, found by scanning the system scripts.
#[derive(Clone, Debug, PartialEq)]
pub struct StockProfile {
    pub file: PathBuf,
    pub pretty_name: Option<String>,
    pub min_hz: u32,
    pub max_hz: u32,
}

/// Best-effort: find the system display script that names this panel and
/// read its refresh-rate list. Used for display and validation only; the
/// generated Lua asks Gamescope itself at runtime.
pub fn stock_profile(id: &PanelId) -> Option<StockProfile> {
    let quoted = |s: &str| format!("\"{s}\"");
    let mut files = Vec::new();
    for dir in SYSTEM_SCRIPT_DIRS {
        collect_lua(Path::new(dir), &mut files);
    }
    files.sort();
    files.into_iter().find_map(|file| {
        let text = fs::read_to_string(&file).ok()?;
        let names_panel = text.contains(&quoted(&id.vendor))
            && [&id.model, &id.data_string]
                .iter()
                .any(|s| !s.is_empty() && text.contains(&quoted(s)));
        if !names_panel {
            return None;
        }
        let (min_hz, max_hz) = parse_rate_list(&text)?;
        let pretty_name = text.lines().find_map(|l| {
            let rest = l
                .trim()
                .strip_prefix("pretty_name")?
                .trim_start()
                .strip_prefix('=')?;
            Some(
                rest.trim()
                    .trim_end_matches(',')
                    .trim_matches('"')
                    .to_string(),
            )
        });
        Some(StockProfile {
            file,
            pretty_name,
            min_hz,
            max_hz,
        })
    })
}

fn collect_lua(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_lua(&path, out);
        } else if path.extension().is_some_and(|e| e == "lua") {
            out.push(path);
        }
    }
}

/// Min and max of the first `{ ... }` list after a `refresh_rates` name.
fn parse_rate_list(text: &str) -> Option<(u32, u32)> {
    let start = text.find("refresh_rates")?;
    let open = start + text[start..].find('{')?;
    let close = open + text[open..].find('}')?;
    let rates: Vec<u32> = text[open + 1..close]
        .split(|c: char| !c.is_ascii_digit())
        .filter_map(|n| n.parse().ok())
        .collect();
    Some((*rates.iter().min()?, *rates.iter().max()?))
}

pub fn gamescope_installed() -> bool {
    crate::system::which("gamescope") || Path::new(SYSTEM_SCRIPT_DIRS[0]).exists()
}

/// True when running inside a Gamescope session (Game Mode).
pub fn in_game_mode() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|v| v.eq_ignore_ascii_case("gamescope"))
        || std::env::var_os("GAMESCOPE_WAYLAND_DISPLAY").is_some()
}

#[derive(Clone, Debug, PartialEq)]
pub struct Paths {
    pub script: PathBuf,
    pub state: PathBuf,
    pub report: PathBuf,
}

impl Paths {
    /// Gamescope reads `$XDG_CONFIG_HOME/gamescope/scripts` (default `~/.config`).
    pub fn for_current_user() -> Result<Paths> {
        if crate::system::is_root() {
            bail!(
                "built-in screen settings belong to your user account, not root; run this without sudo"
            );
        }
        let config = match std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
            Some(dir) => PathBuf::from(dir),
            None => {
                PathBuf::from(std::env::var_os("HOME").context("HOME is not set")?).join(".config")
            }
        };
        Ok(Paths::under(&config))
    }

    pub fn under(config: &Path) -> Paths {
        Paths {
            script: config.join("gamescope/scripts").join(SCRIPT_NAME),
            state: config.join("monitor-oc/gamescope.conf"),
            report: config.join("monitor-oc/gamescope-report"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum Method {
    /// Keep the panel's blanking and raise the pixel clock (like CRU's default).
    #[default]
    Clock,
    /// Shrink the vertical blanking first at a fixed pixel clock, then raise the
    /// clock only if needed. Gentler on the link, harder on the panel's timing controller.
    Porch,
}

impl Method {
    fn as_str(self) -> &'static str {
        match self {
            Method::Clock => "clock",
            Method::Porch => "porch",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrialState {
    /// Waiting for the next Game Mode start.
    Armed,
    /// Enabled in the Gamescope session that started after it was armed.
    Running,
    /// Gamescope started again before it was confirmed, so it was dropped.
    Expired,
}

/// Mirrors the key=value file the Lua script reads and updates.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct State {
    /// Confirmed limit in Hz (0 = stock).
    pub max: u32,
    pub method: Method,
    pub trial: Option<(u32, TrialState)>,
}

impl State {
    pub fn parse(text: &str) -> State {
        let get = |key: &str| {
            text.lines().find_map(|l| {
                let (k, v) = l.split_once('=')?;
                (k.trim() == key).then(|| v.trim().to_string())
            })
        };
        let trial_state = match get("trial_state").as_deref() {
            Some("armed") => Some(TrialState::Armed),
            Some("running") => Some(TrialState::Running),
            Some("expired") => Some(TrialState::Expired),
            _ => None,
        };
        State {
            max: get("max").and_then(|v| v.parse().ok()).unwrap_or(0),
            method: if get("method").as_deref() == Some("porch") {
                Method::Porch
            } else {
                Method::Clock
            },
            trial: get("trial").and_then(|v| v.parse().ok()).zip(trial_state),
        }
    }

    pub fn render(&self) -> String {
        let mut out = format!("max={}\nmethod={}\n", self.max, self.method.as_str());
        if let Some((hz, state)) = self.trial {
            let state = match state {
                TrialState::Armed => "armed",
                TrialState::Running => "running",
                TrialState::Expired => "expired",
            };
            out += &format!("trial={hz}\ntrial_state={state}\n");
        }
        out
    }

    pub fn load(paths: &Paths) -> State {
        fs::read_to_string(&paths.state)
            .map(|t| State::parse(&t))
            .unwrap_or_default()
    }

    pub fn save(&self, paths: &Paths) -> Result<()> {
        write_file(&paths.state, &self.render())
    }
}

/// What the Lua script recorded the last time Gamescope loaded it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Report {
    pub loaded_at: Option<u64>,
    pub base: Option<String>,
    pub base_name: Option<String>,
    pub stock_min: u32,
    pub stock_max: u32,
    pub limit: u32,
    pub extended: bool,
    pub trial: Option<String>,
    pub last_mode: Option<String>,
    pub last_clock_khz: Option<u64>,
}

impl Report {
    pub fn parse(text: &str) -> Report {
        let get = |key: &str| {
            text.lines().find_map(|l| {
                let (k, v) = l.split_once('=')?;
                (k.trim() == key)
                    .then(|| v.trim().to_string())
                    .filter(|v| v != "none")
            })
        };
        let num = |key: &str| get(key).and_then(|v| v.parse().ok());
        Report {
            loaded_at: num("loaded_at"),
            base: get("base"),
            base_name: get("base_name"),
            stock_min: num("stock_min").unwrap_or(0) as u32,
            stock_max: num("stock_max").unwrap_or(0) as u32,
            limit: num("limit").unwrap_or(0) as u32,
            extended: get("extended").as_deref() == Some("yes"),
            trial: get("trial"),
            last_mode: get("last_mode"),
            last_clock_khz: num("last_clock_khz"),
        }
    }

    pub fn load(paths: &Paths) -> Option<Report> {
        fs::read_to_string(&paths.report)
            .ok()
            .map(|t| Report::parse(&t))
    }
}

fn write_file(path: &Path, text: &str) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    fs::write(path, text).with_context(|| format!("writing {}", path.display()))
}

/// A Lua string literal for arbitrary text.
fn lua_string(s: &str) -> String {
    let mut out = String::from("\"");
    for b in s.bytes() {
        match b {
            b'"' => out.push_str("\\\""),
            b'\\' => out.push_str("\\\\"),
            0x20..=0x7E => out.push(b as char),
            _ => out.push_str(&format!("\\{b:03}")),
        }
    }
    out.push('"');
    out
}

pub fn render_script(panel: &Panel, paths: &Paths) -> String {
    let rates: Vec<String> = panel.edid_rates.iter().map(u32::to_string).collect();
    TEMPLATE
        .replace("@@VENDOR@@", &lua_string(&panel.id.vendor))
        .replace("@@PRODUCT@@", &panel.id.product.to_string())
        .replace("@@MODEL@@", &lua_string(&panel.id.model))
        .replace("@@DATA_STRING@@", &lua_string(&panel.id.data_string))
        .replace(
            "@@STATE_PATH@@",
            &lua_string(&paths.state.to_string_lossy()),
        )
        .replace(
            "@@REPORT_PATH@@",
            &lua_string(&paths.report.to_string_lossy()),
        )
        .replace("@@EDID_RATES@@", &rates.join(", "))
}

/// The panel's stock maximum in Game Mode: Gamescope's profile if we can read
/// it, otherwise the EDID.
pub fn stock_max(panel: &Panel, stock: Option<&StockProfile>) -> u32 {
    let edid_max = panel.edid_rates.last().copied().unwrap_or(0);
    stock.map_or(edid_max, |s| s.max_hz.max(edid_max))
}

pub fn max_allowed(stock_max: u32) -> u32 {
    ((stock_max as f64) * MAX_FACTOR).floor() as u32
}

/// Arm a one-session trial at `hz`.
pub fn try_rate(panel: &Panel, paths: &Paths, hz: u32, method: Method) -> Result<State> {
    let stock = stock_profile(&panel.id);
    let stock_max = stock_max(panel, stock.as_ref());
    if hz <= stock_max {
        bail!("{hz} Hz is within the stock range (up to {stock_max} Hz); pick something higher");
    }
    if hz > max_allowed(stock_max) {
        bail!(
            "{hz} Hz is more than {:.0}% above the stock {stock_max} Hz; monitor-oc stops at {} Hz",
            (MAX_FACTOR - 1.0) * 100.0,
            max_allowed(stock_max)
        );
    }
    let mut state = State::load(paths);
    state.method = method;
    state.trial = Some((hz, TrialState::Armed));
    write_file(&paths.script, &render_script(panel, paths))?;
    state.save(paths)?;
    Ok(state)
}

/// Make the current trial the confirmed limit.
pub fn confirm(paths: &Paths) -> Result<u32> {
    let mut state = State::load(paths);
    let Some((hz, trial_state)) = state.trial else {
        bail!("there is no trial to confirm");
    };
    if trial_state == TrialState::Armed {
        bail!("the {hz} Hz trial has not run yet; start Game Mode and test it first");
    }
    state.max = hz;
    state.trial = None;
    state.save(paths)?;
    Ok(hz)
}

pub fn discard(paths: &Paths) -> Result<()> {
    let mut state = State::load(paths);
    state.trial = None;
    state.save(paths)
}

/// Go back to the stock range but keep monitor-oc's files.
pub fn reset(paths: &Paths) -> Result<()> {
    State {
        method: State::load(paths).method,
        ..State::default()
    }
    .save(paths)
}

pub fn remove(paths: &Paths) -> Result<()> {
    for path in [&paths.script, &paths.state, &paths.report] {
        match fs::remove_file(path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                return Err(e).with_context(|| format!("removing {}", path.display()));
            }
            _ => {}
        }
    }
    Ok(())
}

pub fn installed(paths: &Paths) -> bool {
    paths.script.exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_round_trip() {
        let s = State {
            max: 130,
            method: Method::Porch,
            trial: Some((140, TrialState::Running)),
        };
        assert_eq!(State::parse(&s.render()), s);
        assert_eq!(State::parse(""), State::default());
        // The Lua side writes keys in the same format.
        let lua = "max=0\nmethod=clock\ntrial=135\ntrial_state=expired\n";
        assert_eq!(State::parse(lua).trial, Some((135, TrialState::Expired)));
    }

    #[test]
    fn report_parse() {
        let r = Report::parse(
            "loaded_at=1760000000\nbase=rogally_lcd\nbase_name=ASUS ROG Ally / ROG Ally X LCD\nstock_min=48\n\
             stock_max=120\nlimit=135\nmethod=clock\ntrial=running\nextended=yes\nlast_refresh=135\n\
             last_mode=1920x1080@135\nlast_clock_khz=301234\n",
        );
        assert_eq!(r.base.as_deref(), Some("rogally_lcd"));
        assert_eq!(
            (r.stock_min, r.stock_max, r.limit, r.extended),
            (48, 120, 135, true)
        );
        assert_eq!(r.last_clock_khz, Some(301234));
        assert_eq!(Report::parse("base=none\n").base, None);
    }

    #[test]
    fn rate_list_parsing() {
        let text =
            "local x_refresh_rates = {\n 48, 49,\n 120\n}\ngamescope.config.known_displays.x = {";
        assert_eq!(parse_rate_list(text), Some((48, 120)));
        assert_eq!(parse_rate_list("nothing here"), None);
    }

    #[test]
    fn lua_strings_are_escaped() {
        assert_eq!(lua_string("TL070\"FV\\"), "\"TL070\\\"FV\\\\\"");
        assert_eq!(lua_string("a\nb"), "\"a\\010b\"");
    }

    fn ally_panel() -> Panel {
        Panel {
            connector: "eDP-1".into(),
            id: PanelId {
                vendor: "TMX".into(),
                product: 2,
                model: "TL070FVXS01-0".into(),
                data_string: String::new(),
            },
            width: 1920,
            height: 1080,
            edid_rates: vec![120],
        }
    }

    #[test]
    fn template_placeholders_all_filled() {
        let script = render_script(
            &ally_panel(),
            &Paths::under(Path::new("/home/deck/.config")),
        );
        assert!(!script.contains("@@"), "unfilled placeholder");
        assert!(script.contains("vendor = \"TMX\""));
        assert!(script.contains("\"/home/deck/.config/monitor-oc/gamescope.conf\""));
    }

    /// Runs the generated script under LuaJIT (Gamescope's Lua) against a mock
    /// of Gamescope's display lookup and mode switching.
    #[test]
    fn lua_harness() {
        if !crate::system::which("luajit") {
            assert!(std::env::var_os("CI").is_none(), "luajit is required in CI");
            eprintln!("skipping: luajit not installed");
            return;
        }
        let dir = std::env::temp_dir().join(format!("monitor-oc-lua-{}", std::process::id()));
        let paths = Paths::under(&dir);
        fs::create_dir_all(paths.state.parent().unwrap()).unwrap();
        write_file(&paths.script, &render_script(&ally_panel(), &paths)).unwrap();
        let out = std::process::Command::new("luajit")
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/gamescope_harness.lua"
            ))
            .args([&paths.script, &paths.state, &paths.report])
            .output()
            .unwrap();
        let _ = fs::remove_dir_all(&dir);
        assert!(
            out.status.success(),
            "harness failed:\n{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }
}
