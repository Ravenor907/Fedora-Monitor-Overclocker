mod doctor;
mod drm;
mod edid;
mod gamescope;
#[cfg(feature = "gui")]
mod gui;
mod install;
mod json;
mod live;
mod plan;
mod report;
mod service;
mod state;
mod system;
mod timing;
mod update;
mod x11;

use anyhow::{Context, Result, bail};
use clap::{Args, CommandFactory, Parser, Subcommand};
use drm::Connector;
use edid::Edid;
use plan::Plan;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;
use system::{Boot, Os};
use timing::{Mode, Strategy};

/// Monitor refresh-rate overclocking for SteamOS, Fedora and other Linux distros.
///
/// Works by giving the kernel a modified copy of the monitor's EDID with an
/// extra (faster) timing, so every desktop and Gamescope sees the new mode.
#[derive(Parser)]
#[command(version = update::version(), propagate_version = true)]
struct Cli {
    #[command(subcommand)]
    command: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// List display connectors, connected monitors and this system's setup.
    Detect,
    /// Show every timing and the range limits in a monitor's EDID.
    Show {
        /// Connector (e.g. DP-1) or path to an EDID file.
        source: String,
    },
    /// Write a modified EDID to a file without changing the system.
    Generate {
        /// Connector (e.g. DP-1) or path to an EDID file.
        source: String,
        /// New mode as WIDTHxHEIGHT@HZ, e.g. 2560x1440@170.
        mode: Mode,
        #[command(flatten)]
        opts: BuildOpts,
        /// Where to write the EDID.
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Try a mode live (until reboot) and revert automatically unless kept.
    Test {
        #[command(flatten)]
        target: Target,
        /// Seconds to keep it before it reverts (asked twice).
        #[arg(long, default_value_t = 10)]
        seconds: u64,
        /// Seconds after the first keep before asking again (0: ask once).
        #[arg(long, default_value_t = 300)]
        recheck: u64,
    },
    /// Install the override so it loads on every boot.
    Apply {
        #[command(flatten)]
        target: Target,
        /// Also switch the live EDID now instead of waiting for a reboot.
        #[arg(long)]
        now: bool,
        /// Print what would change without changing anything.
        #[arg(long)]
        dry_run: bool,
    },
    /// Undo a live override made by `test`.
    Reset { connector: String },
    /// Remove installed overrides (all of them if no connector is given).
    Remove {
        connector: Option<String>,
        #[arg(long)]
        dry_run: bool,
    },
    /// Re-install anything an OS update wiped and fix early-boot misses.
    /// Run automatically at boot by monitor-oc.service.
    Reapply {
        /// Wait for the GPU driver to bring up connectors first.
        #[arg(long)]
        boot: bool,
    },
    /// Show what monitor-oc has installed and whether it is active.
    Status,
    /// Overclock a handheld's built-in screen in Game Mode (SteamOS, Bazzite).
    ///
    /// Gamescope builds the built-in screen's refresh rates itself, so this
    /// extends Gamescope's profile instead of the EDID. Runs as your user, no sudo.
    Panel {
        #[command(subcommand)]
        command: PanelCmd,
    },
    /// Check this system and explain anything that will stop an overclock working.
    Doctor,
    /// Open the graphical app (also what running monitor-oc without arguments does).
    #[cfg(feature = "gui")]
    Gui {
        /// Show made-up hardware (a ROG Ally X with a 1440p monitor); changes nothing.
        #[arg(long)]
        demo: bool,
        /// Page to open first.
        #[arg(long, value_enum)]
        page: Option<gui::Page>,
        /// Fill the screen (always on in Game Mode).
        #[arg(long)]
        fullscreen: bool,
    },
    /// Install Monitor OC for your user: ~/.local/bin/monitor-oc and an app menu
    /// entry (which Steam can add for Game Mode). Re-run to upgrade. No sudo.
    Install,
    /// Update Monitor OC, switch between Stable and Testing, or roll back.
    ///
    /// Stable gets the releases the owner has signed off on. Testing gets
    /// test builds first (Stable's version plus a revision, e.g. 0.2.0.1).
    /// Every update keeps the version it replaces for --rollback.
    Update {
        /// Only report whether an update is available.
        #[arg(long)]
        check: bool,
        /// Switch to this channel first: stable or testing.
        #[arg(long, value_enum)]
        channel: Option<update::Channel>,
        /// Go back to the version the last update replaced.
        #[arg(long, conflicts_with_all = ["check", "channel"])]
        rollback: bool,
    },
    /// Remove what `install` added.
    /// Report a problem to the developer (a GitHub issue), or send waiting reports.
    ///
    /// Needs a GitHub access key (`--set-key`). With a message, it is sent
    /// with the diagnostics; without one, reports waiting to go out are sent.
    Report {
        /// What happened.
        message: Vec<String>,
        /// Save the GitHub access key (read from the terminal).
        #[arg(long, conflicts_with = "remove_key")]
        set_key: bool,
        /// Forget the access key.
        #[arg(long)]
        remove_key: bool,
    },
    /// Remove Monitor OC: the program, menu entries, update data, its Game Mode
    /// profile and settings (the built-in screen goes back to stock).
    Uninstall {
        /// Keep the built-in screen settings and ~/.config/monitor-oc.
        #[arg(long)]
        keep_settings: bool,
        /// Removing everything is the default now; accepted for old scripts.
        #[arg(long, hide = true, conflicts_with = "keep_settings")]
        purge: bool,
    },
}

#[derive(Subcommand)]
enum PanelCmd {
    /// Show the built-in screen, its stock range and what is configured.
    Status,
    /// Allow up to HZ for the next Game Mode session only (a safe trial).
    Try {
        /// Highest refresh rate to offer, e.g. 130.
        hz: u32,
        #[arg(long, value_enum, default_value_t = gamescope::Method::Clock)]
        method: gamescope::Method,
    },
    /// Keep the trial rate permanently.
    Confirm,
    /// Drop the trial and keep the previous limit.
    Discard,
    /// Go back to the stock range.
    Reset,
    /// Delete monitor-oc's Gamescope profile entirely.
    Remove,
    /// Take back a live refresh test the app didn't finish (started by the app).
    #[command(hide = true)]
    Watchdog,
}

#[derive(Args)]
struct BuildOpts {
    /// How to compute the new timing.
    #[arg(long, value_enum, default_value_t = Strategy::Auto)]
    timing: Strategy,
    /// Make the new mode the monitor's preferred (default) mode. Recommended
    /// for SteamOS Game Mode, which uses the preferred mode on external displays.
    #[arg(long)]
    preferred: bool,
}

#[derive(Args)]
struct Target {
    /// Connector such as DP-1 or HDMI-A-1 (see `monitor-oc detect`).
    connector: String,
    /// New mode as WIDTHxHEIGHT@HZ, e.g. 2560x1440@170.
    mode: Mode,
    #[command(flatten)]
    opts: BuildOpts,
    /// Start from this EDID file instead of the monitor's own EDID.
    #[arg(long)]
    edid: Option<PathBuf>,
    /// Allow built-in panels (Steam Deck screen, laptop eDP).
    #[arg(long)]
    force: bool,
}

fn main() {
    report::install_panic_hook();
    if let Err(e) = run(Cli::parse()) {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<()> {
    let Some(command) = cli.command else {
        // Launched from a menu or Steam: open the app. From a terminal
        // without a graphical session: show the commands.
        #[cfg(feature = "gui")]
        if gui::display_available() {
            return gui::run(gui::Options::default());
        }
        Cli::command().print_help()?;
        std::process::exit(2);
    };
    match command {
        #[cfg(feature = "gui")]
        Cmd::Gui {
            demo,
            page,
            fullscreen,
        } => gui::run(gui::Options {
            demo,
            page,
            fullscreen,
        }),
        Cmd::Detect => detect(),
        Cmd::Show { source } => show(&source),
        Cmd::Generate {
            source,
            mode,
            opts,
            output,
        } => {
            let base = load_source(&source)?;
            let plan = build_plan(&base, &mode, &opts)?;
            print_plan(&mode, &plan, None);
            fs::write(&output, &plan.edid.bytes)
                .with_context(|| format!("writing {}", output.display()))?;
            println!(
                "\nWrote {} ({} bytes).",
                output.display(),
                plan.edid.bytes.len()
            );
            Ok(())
        }
        Cmd::Test {
            target,
            seconds,
            recheck,
        } => test(&target, seconds, recheck),
        Cmd::Apply {
            target,
            now,
            dry_run,
        } => apply(&target, now, dry_run),
        Cmd::Reset { connector } => {
            require_root()?;
            drm::find(&connector)?.reset_override()?;
            println!("Live override on {connector} removed.");
            Ok(())
        }
        Cmd::Remove { connector, dry_run } => remove(connector.as_deref(), dry_run),
        Cmd::Reapply { boot } => reapply(boot),
        Cmd::Status => status(),
        Cmd::Panel { command } => panel(command),
        Cmd::Doctor => {
            for c in doctor::checks() {
                let tag = match c.level {
                    doctor::Level::Ok => " ok ",
                    doctor::Level::Info => "info",
                    doctor::Level::Warn => "WARN",
                    doctor::Level::Fail => "FAIL",
                };
                println!("[{tag}] {:<22} {}", c.title, c.detail);
            }
            Ok(())
        }
        Cmd::Update {
            check,
            channel,
            rollback,
        } => update::command(check, channel, rollback),
        Cmd::Report {
            message,
            set_key,
            remove_key,
        } => report::command(&message, set_key, remove_key),
        Cmd::Install => install::install(),
        Cmd::Uninstall {
            keep_settings,
            purge: _,
        } => install::uninstall(!keep_settings),
    }
}

fn panel(command: PanelCmd) -> Result<()> {
    let paths = gamescope::Paths::for_current_user()?;
    match command {
        PanelCmd::Status => {
            let panel = gamescope::Panel::detect()?;
            println!("Built-in screen: {} {}", panel.connector, panel.id.label());
            if !gamescope::gamescope_installed() {
                println!("Gamescope is not installed, so there is no Game Mode to configure.");
                return Ok(());
            }
            match gamescope::stock_profile(&panel.id) {
                Some(s) => println!(
                    "Stock range:     {}-{} Hz ({})",
                    s.min_hz,
                    s.max_hz,
                    s.pretty_name
                        .unwrap_or_else(|| s.file.display().to_string())
                ),
                None => println!("Stock range:     from the EDID ({:?} Hz)", panel.edid_rates),
            }
            for c in doctor::checks()
                .into_iter()
                .filter(|c| c.title == "Built-in overclock" || c.title == "Gamescope profile")
            {
                println!("{:<16} {}", format!("{}:", c.title), c.detail);
            }
            Ok(())
        }
        PanelCmd::Try { hz, method } => {
            let panel = gamescope::Panel::detect()?;
            if !gamescope::gamescope_installed() {
                bail!("Gamescope is not installed; this only applies to Game Mode");
            }
            gamescope::try_rate(&panel, &paths, hz, method)?;
            println!(
                "Trial armed: up to {hz} Hz for the next Game Mode session only.\n\n\
                 1. Start (or restart) Game Mode so it loads the trial.\n\
                 2. Open Monitor OC in Game Mode and press Test {hz} Hz. The screen switches\n\
                    for 10 seconds: keep it if the picture is right, otherwise it goes back\n\
                    to stock on its own. Keep it once more after 5 minutes to make it stick.\n\n\
                 Until a rate is tested and kept, Game Mode keeps the screen at its stock\n\
                 rate even if a higher one is picked in Quick Access."
            );
            Ok(())
        }
        PanelCmd::Confirm => {
            let hz = gamescope::confirm(&paths)?;
            println!("Confirmed: Game Mode now offers up to {hz} Hz on the built-in screen.");
            Ok(())
        }
        PanelCmd::Discard => {
            gamescope::discard(&paths)?;
            println!("Trial discarded.");
            Ok(())
        }
        PanelCmd::Reset => {
            gamescope::reset(&paths)?;
            println!("Back to the stock range from the next Game Mode start.");
            Ok(())
        }
        PanelCmd::Remove => {
            gamescope::remove(&paths)?;
            println!("Removed monitor-oc's Gamescope profile; restart Game Mode to finish.");
            Ok(())
        }
        PanelCmd::Watchdog => live::watchdog(),
    }
}

fn require_root() -> Result<()> {
    if !system::is_root() {
        bail!("this command changes system settings; run it with sudo");
    }
    Ok(())
}

fn load_source(source: &str) -> Result<Edid> {
    if Path::new(source).is_file() {
        let raw = fs::read(source).with_context(|| format!("reading {source}"))?;
        return Edid::parse(&raw).with_context(|| format!("parsing {source}"));
    }
    let conn = drm::find(source)?;
    original_edid(&conn).map(|(e, _)| e)
}

/// The monitor's unmodified EDID: the saved original if monitor-oc already
/// overrode it, otherwise what the kernel reports now.
fn original_edid(conn: &Connector) -> Result<(Edid, bool)> {
    let saved = state::original_path(&conn.name);
    if let Ok(raw) = fs::read(&saved) {
        return Ok((
            Edid::parse(&raw).with_context(|| format!("parsing {}", saved.display()))?,
            true,
        ));
    }
    let raw = conn.edid().with_context(|| {
        format!(
            "{} has no EDID (status: {}); is a monitor plugged in?",
            conn.name,
            conn.status()
        )
    })?;
    Ok((
        Edid::parse(&raw).with_context(|| format!("parsing {}'s EDID", conn.name))?,
        false,
    ))
}

fn target_edid(target: &Target, conn: &Connector) -> Result<(Edid, bool)> {
    match &target.edid {
        Some(path) => Ok((load_source(&path.to_string_lossy())?, true)),
        None => original_edid(conn),
    }
}

fn save_original(conn: &Connector, edid: &Edid) -> Result<()> {
    let path = state::original_path(&conn.name);
    fs::create_dir_all(state::STATE_DIR)?;
    fs::write(&path, &edid.bytes).with_context(|| format!("writing {}", path.display()))
}

fn build_plan(base: &Edid, mode: &Mode, opts: &BuildOpts) -> Result<Plan> {
    plan::build(base, mode, opts.timing, opts.preferred)
}

fn print_plan(mode: &Mode, plan: &Plan, conn: Option<&Connector>) {
    println!("Requested:  {mode}");
    println!("Timing:     {} ({})", plan.dtd, plan.strategy);
    println!("Placed in:  {}", plan.location);
    for change in &plan.limit_changes {
        println!("Range:      {change}");
    }
    for note in plan.notes(mode, conn.map(|c| c.name.as_str())) {
        println!("Note:       {note}");
    }
}

fn guard(conn: &Connector, force: bool) -> Result<()> {
    if conn.driver().as_deref() == Some("nvidia")
        || (conn.driver().is_none() && drm::nvidia_loaded())
    {
        eprintln!(
            "warning: the NVIDIA proprietary driver ignores kernel EDID overrides. Use nvidia-settings \
             (CustomEDID) instead, or the nouveau/NVK stack."
        );
    }
    if conn.is_internal() && !force && gamescope::gamescope_installed() {
        bail!(
            "{} is a built-in screen. In Game Mode, Gamescope generates its refresh rates itself and \
             ignores EDID changes, so use `monitor-oc panel try <HZ>` instead (no sudo needed). \
             Pass --force to change the EDID anyway (affects Desktop Mode only, and on AMD only after a reboot).",
            conn.name
        );
    }
    if conn.is_internal() && !force {
        if drm::is_steam_deck() {
            bail!(
                "{} is the Steam Deck's built-in screen. Its refresh range is managed by Gamescope and the \
                 panel firmware; overriding its EDID can break the display. Pass --force if you really mean it.",
                conn.name
            );
        }
        bail!(
            "{} is a built-in panel; pass --force to override it anyway.",
            conn.name
        );
    }
    if !conn.connected() {
        eprintln!(
            "warning: {} is {}; the override will take effect once a monitor is connected.",
            conn.name,
            conn.status()
        );
    }
    Ok(())
}

fn detect() -> Result<()> {
    let os = Os::detect();
    println!(
        "System:     {}",
        if os.pretty_name.is_empty() {
            "unknown"
        } else {
            &os.pretty_name
        }
    );
    println!("Boot setup: {}", Boot::detect(&os).describe());
    if drm::is_steam_deck() {
        println!(
            "Hardware:   Steam Deck (external monitors are supported; the built-in screen is not)"
        );
    }
    println!();
    for conn in drm::connectors()? {
        let driver = conn.driver().unwrap_or_else(|| "?".into());
        print!(
            "{:<12} card{} {:<8} {:<13}",
            conn.name,
            conn.card,
            driver,
            conn.status()
        );
        match conn.edid().and_then(|raw| Edid::parse(&raw).ok()) {
            Some(edid) => {
                println!(
                    "{} {:04X}  {}",
                    edid.manufacturer(),
                    edid.product_code(),
                    edid.name().unwrap_or_default()
                );
                for (_, dtd) in edid.timings() {
                    println!("{:14}{dtd}", "");
                }
            }
            None => println!(),
        }
    }
    Ok(())
}

fn show(source: &str) -> Result<()> {
    let edid = load_source(source)?;
    println!(
        "Monitor: {} {:04X} {}  ({} block(s))",
        edid.manufacturer(),
        edid.product_code(),
        edid.name().unwrap_or_default(),
        edid.block_count()
    );
    if let Some(r) = edid.range_limits() {
        println!(
            "Range limits: {}-{} Hz vertical, {}-{} kHz horizontal, max pixel clock {} MHz",
            r.min_v_hz, r.max_v_hz, r.min_h_khz, r.max_h_khz, r.max_clock_mhz
        );
    }
    println!("Detailed timings:");
    for (loc, dtd) in edid.timings() {
        println!(
            "  {dtd}\n      [{loc}]  front/sync/blank H {}/{}/{}  V {}/{}/{}",
            dtd.h_front, dtd.h_sync, dtd.h_blank, dtd.v_front, dtd.v_sync, dtd.v_blank
        );
    }
    Ok(())
}

/// Lines `test` prints that the app watches for.
pub const RECHECK_WAIT_MARKER: &str = "Kept for now. Double check in";
pub const RECHECK_MARKER: &str = "Double check:";

fn test(target: &Target, seconds: u64, recheck: u64) -> Result<()> {
    require_root()?;
    let conn = drm::find(&target.connector)?;
    guard(&conn, target.force)?;
    let (base, from_saved) = target_edid(target, &conn)?;
    let plan = build_plan(&base, &target.mode, &target.opts)?;
    print_plan(&target.mode, &plan, Some(&conn));
    if !from_saved {
        save_original(&conn, &base)?;
    }

    conn.set_override(&plan.edid.bytes)?;
    println!(
        "\nLive override active on {}. Now pick {:.0} Hz in your display settings:\n  \
         KDE: System Settings > Display Configuration   GNOME: Settings > Displays\n  \
         SteamOS Game Mode: Settings > Display (or use --preferred so it is chosen automatically)\n\n\
         If the screen goes black or shows artifacts, just wait: it reverts in {seconds} s.\n\
         Type 'keep' + Enter to keep it, or press Enter to revert now.",
        conn.name, target.mode.hz
    );

    // Every line typed (None when stdin closes, e.g. the app quit).
    let (tx, rx) = mpsc::channel::<Option<String>>();
    std::thread::spawn(move || {
        loop {
            let mut line = String::new();
            match std::io::stdin().read_line(&mut line) {
                Ok(0) | Err(_) => {
                    let _ = tx.send(None);
                    return;
                }
                Ok(_) => {
                    if tx.send(Some(line)).is_err() {
                        return;
                    }
                }
            }
        }
    });
    let kept = |timeout: u64| {
        matches!(
            rx.recv_timeout(Duration::from_secs(timeout)),
            Ok(Some(line)) if line.trim().eq_ignore_ascii_case("keep")
        )
    };
    let mut keep = kept(seconds);
    if keep && recheck > 0 {
        println!(
            "{RECHECK_WAIT_MARKER} {recheck} s: use it normally. Press Enter to revert sooner."
        );
        // Anything typed (or the app closing) in the meantime reverts.
        keep = match rx.recv_timeout(Duration::from_secs(recheck)) {
            Err(mpsc::RecvTimeoutError::Timeout) => {
                println!(
                    "{RECHECK_MARKER} still good? Type 'keep' + Enter within {seconds} s, or it reverts."
                );
                kept(seconds)
            }
            _ => false,
        };
    }
    if keep {
        println!(
            "Kept until reboot. Make it permanent with:\n  sudo monitor-oc apply {} {}{}",
            conn.name,
            target.mode,
            if target.opts.preferred {
                " --preferred"
            } else {
                ""
            }
        );
    } else {
        conn.reset_override()?;
        println!("Reverted to the monitor's own EDID.");
    }
    Ok(())
}

fn check_firmware_path_conflict() -> Result<()> {
    let ours = format!("{}={}", state::KEY_FIRMWARE_PATH, state::FIRMWARE_DIR);
    if let Some(other) = system::cmdline()
        .split_whitespace()
        .find(|a| system::arg_key(a) == state::KEY_FIRMWARE_PATH && *a != ours)
    {
        bail!(
            "this system already boots with `{other}`; monitor-oc needs `{ours}` and will not replace it"
        );
    }
    Ok(())
}

fn apply(target: &Target, now: bool, dry_run: bool) -> Result<()> {
    if !dry_run {
        require_root()?;
    }
    let os = Os::detect();
    let boot = Boot::detect(&os);
    let conn = drm::find(&target.connector)?;
    guard(&conn, target.force)?;
    check_firmware_path_conflict()?;
    let (base, from_saved) = target_edid(target, &conn)?;
    let plan = build_plan(&base, &target.mode, &target.opts)?;
    print_plan(&target.mode, &plan, Some(&conn));

    let mut entries = state::load();
    entries.retain(|e| e.connector != conn.name);
    entries.push(state::Entry {
        connector: conn.name.clone(),
        mode: target.mode.to_string(),
    });
    let firmware = state::firmware_path(&conn.name);
    println!("\nBoot setup: {}", boot.describe());

    if dry_run {
        println!("would write {}", firmware.display());
        boot.set_args(&state::MANAGED_KEYS, &state::kernel_args(&entries), true)?;
        return service::install(&os, true);
    }

    if !from_saved {
        save_original(&conn, &base)?;
    }
    fs::create_dir_all(firmware.parent().unwrap())?;
    fs::write(&firmware, &plan.edid.bytes)
        .with_context(|| format!("writing {}", firmware.display()))?;
    println!("wrote {}", firmware.display());
    state::save(&entries)?;
    boot.set_args(&state::MANAGED_KEYS, &state::kernel_args(&entries), false)?;
    service::install(&os, false)?;

    if now {
        conn.set_override(&plan.edid.bytes)?;
        println!("\nLive EDID switched; select the new mode in your display settings.");
    }
    println!(
        "\nDone. Reboot to load the new EDID at boot.{}\nIf the monitor shows no picture, boot the previous GRUB entry \
         or another input and run `sudo monitor-oc remove {}`.",
        if os.is_steamos() {
            "\nmonitor-oc.service re-applies this automatically after SteamOS updates."
        } else {
            ""
        },
        conn.name
    );
    Ok(())
}

fn remove(connector: Option<&str>, dry_run: bool) -> Result<()> {
    if !dry_run {
        require_root()?;
    }
    let os = Os::detect();
    let boot = Boot::detect(&os);
    let entries = state::load();
    let (removed, remaining): (Vec<_>, Vec<_>) = entries
        .into_iter()
        .partition(|e| connector.is_none_or(|c| e.connector == c));
    if removed.is_empty() {
        bail!(
            "nothing installed{}",
            connector.map(|c| format!(" for {c}")).unwrap_or_default()
        );
    }

    for entry in &removed {
        let firmware = state::firmware_path(&entry.connector);
        if dry_run {
            println!("would remove {}", firmware.display());
            continue;
        }
        let _ = fs::remove_file(&firmware);
        let _ = fs::remove_file(state::original_path(&entry.connector));
        if let Ok(conn) = drm::find(&entry.connector) {
            let _ = conn.reset_override();
        }
        println!("removed override for {} ({})", entry.connector, entry.mode);
    }
    if !dry_run {
        state::save(&remaining)?;
    }
    boot.set_args(
        &state::MANAGED_KEYS,
        &state::kernel_args(&remaining),
        dry_run,
    )?;
    if remaining.is_empty() {
        service::uninstall(&os, dry_run)?;
    }
    println!("Reboot to finish.");
    Ok(())
}

fn reapply(at_boot: bool) -> Result<()> {
    require_root()?;
    let entries = state::load();
    if entries.is_empty() {
        return Ok(());
    }
    let args = state::kernel_args(&entries);
    let cmdline = system::cmdline();
    if args
        .iter()
        .any(|a| !cmdline.split_whitespace().any(|c| c == a))
    {
        println!("Kernel arguments are missing from this boot (OS update?); re-installing them.");
        let os = Os::detect();
        if let Err(e) = Boot::detect(&os).set_args(&state::MANAGED_KEYS, &args, false) {
            eprintln!("warning: could not restore kernel arguments: {e:#}");
        }
    }

    for entry in &entries {
        let firmware = state::firmware_path(&entry.connector);
        let Ok(wanted) = fs::read(&firmware) else {
            eprintln!(
                "warning: {} is missing; run `monitor-oc apply` again",
                firmware.display()
            );
            continue;
        };
        let Some(conn) = wait_for_connector(&entry.connector, at_boot) else {
            continue;
        };
        if conn.edid().as_deref() == Some(&wanted[..]) {
            continue;
        }
        // The firmware may not have been readable when the driver first probed
        // (e.g. from the initramfs); a re-probe usually picks it up.
        let _ = conn.reprobe();
        if conn.edid().as_deref() == Some(&wanted[..]) {
            println!("{}: override loaded after re-probe", conn.name);
            continue;
        }
        match conn.set_override(&wanted) {
            Ok(()) => println!("{}: override applied live", conn.name),
            Err(e) => eprintln!("warning: {}: {e:#}", conn.name),
        }
    }
    Ok(())
}

fn wait_for_connector(name: &str, at_boot: bool) -> Option<Connector> {
    let tries = if at_boot { 20 } else { 1 };
    for i in 0..tries {
        if let Ok(conn) = drm::find(name)
            && conn.connected()
        {
            return Some(conn);
        }
        if i + 1 < tries {
            std::thread::sleep(Duration::from_millis(500));
        }
    }
    None
}

fn status() -> Result<()> {
    let os = Os::detect();
    println!("System:     {}", os.pretty_name);
    println!("Boot setup: {}", Boot::detect(&os).describe());
    println!(
        "Service:    {}",
        if service::installed() {
            "installed"
        } else {
            "not installed"
        }
    );
    let entries = state::load();
    if entries.is_empty() {
        println!("No overrides installed.");
        return Ok(());
    }
    let cmdline = system::cmdline();
    let args = state::kernel_args(&entries);
    let booted = args
        .iter()
        .all(|a| cmdline.split_whitespace().any(|c| c == a));
    println!(
        "Kernel args active this boot: {}",
        if booted { "yes" } else { "no (reboot needed?)" }
    );
    for entry in &entries {
        let wanted = fs::read(state::firmware_path(&entry.connector)).ok();
        let live = drm::find(&entry.connector).ok().and_then(|c| c.edid());
        let state = match (&wanted, &live) {
            (None, _) => "firmware file missing",
            (Some(w), Some(l)) if w == l => "active",
            (Some(_), Some(_)) => "installed, not active",
            (Some(_), None) => "installed, monitor not connected",
        };
        println!("  {:<10} {:<16} {state}", entry.connector, entry.mode);
    }
    Ok(())
}
