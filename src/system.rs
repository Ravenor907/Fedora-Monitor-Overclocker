//! Distro detection and the per-distro ways of changing kernel arguments.

use anyhow::{Context, Result, bail};
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::process::Command;

pub struct Os {
    pub id: String,
    pub pretty_name: String,
}

impl Os {
    pub fn detect() -> Os {
        let text = fs::read_to_string("/etc/os-release")
            .or_else(|_| fs::read_to_string("/usr/lib/os-release"))
            .unwrap_or_default();
        let get = |key: &str| {
            text.lines()
                .find_map(|l| l.strip_prefix(key)?.strip_prefix('='))
                .map(|v| v.trim().trim_matches(['"', '\'']).to_string())
                .unwrap_or_default()
        };
        Os {
            id: get("ID"),
            pretty_name: get("PRETTY_NAME"),
        }
    }

    pub fn is_steamos(&self) -> bool {
        self.id == "steamos"
    }
}

/// How kernel arguments are made persistent on this system.
#[derive(Debug, PartialEq)]
pub enum Boot {
    /// SteamOS: /etc/default/grub + update-grub (with the read-only root unlocked briefly).
    SteamOs,
    /// Fedora Atomic, Bazzite, Universal Blue: `rpm-ostree kargs`.
    RpmOstree,
    /// Fedora Workstation, RHEL family: `grubby`.
    Grubby,
    /// Any other GRUB system: /etc/default/grub + regenerate grub.cfg.
    Grub { update: Vec<String> },
    /// Unknown bootloader: print the arguments for the user to add.
    Manual,
}

impl Boot {
    pub fn detect(os: &Os) -> Boot {
        if os.is_steamos() {
            Boot::SteamOs
        } else if Path::new("/run/ostree-booted").exists() && which("rpm-ostree") {
            Boot::RpmOstree
        } else if which("grubby") {
            Boot::Grubby
        } else if Path::new("/etc/default/grub").exists() {
            if which("update-grub") {
                Boot::Grub {
                    update: vec!["update-grub".into()],
                }
            } else if which("grub2-mkconfig") && Path::new("/boot/grub2/grub.cfg").exists() {
                Boot::Grub {
                    update: vec![
                        "grub2-mkconfig".into(),
                        "-o".into(),
                        "/boot/grub2/grub.cfg".into(),
                    ],
                }
            } else if which("grub-mkconfig") && Path::new("/boot/grub/grub.cfg").exists() {
                Boot::Grub {
                    update: vec![
                        "grub-mkconfig".into(),
                        "-o".into(),
                        "/boot/grub/grub.cfg".into(),
                    ],
                }
            } else {
                Boot::Manual
            }
        } else {
            Boot::Manual
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Boot::SteamOs => "SteamOS GRUB (/etc/default/grub + update-grub)".into(),
            Boot::RpmOstree => "rpm-ostree kargs".into(),
            Boot::Grubby => "grubby".into(),
            Boot::Grub { update } => format!("GRUB (/etc/default/grub + {})", update.join(" ")),
            Boot::Manual => "unknown bootloader (manual setup)".into(),
        }
    }

    /// Remove every argument whose key is in `keys`, then add `add`.
    pub fn set_args(&self, keys: &[&str], add: &[String], dry_run: bool) -> Result<()> {
        match self {
            Boot::RpmOstree => {
                let current = run_capture(&["rpm-ostree", "kargs"])?;
                let mut cmd = vec!["rpm-ostree".to_string(), "kargs".to_string()];
                for arg in current.split_whitespace() {
                    if keys.contains(&arg_key(arg)) && !add.iter().any(|a| a == arg) {
                        cmd.push(format!("--delete={arg}"));
                    }
                }
                for arg in add {
                    if !current.split_whitespace().any(|a| a == arg) {
                        cmd.push(format!("--append={arg}"));
                    }
                }
                if cmd.len() == 2 {
                    println!("Kernel arguments already up to date.");
                    return Ok(());
                }
                run(&cmd, dry_run)
            }
            Boot::Grubby => {
                let all = "--update-kernel=ALL".to_string();
                run(
                    &[
                        "grubby".into(),
                        all.clone(),
                        format!("--remove-args={}", keys.join(" ")),
                    ],
                    dry_run,
                )?;
                if add.is_empty() {
                    return Ok(());
                }
                run(
                    &["grubby".into(), all, format!("--args={}", add.join(" "))],
                    dry_run,
                )
            }
            Boot::Grub { update } => {
                edit_default_grub(keys, add, dry_run)?;
                run(update, dry_run)
            }
            Boot::SteamOs => {
                edit_default_grub(keys, add, dry_run)?;
                let locked = run_capture(&["steamos-readonly", "status"])
                    .map(|s| s.trim() == "enabled")
                    .unwrap_or(false);
                if locked {
                    run(&["steamos-readonly", "disable"], dry_run)?;
                }
                let result = run(&["update-grub"], dry_run);
                if locked {
                    run(&["steamos-readonly", "enable"], dry_run)?;
                }
                result
            }
            Boot::Manual => {
                println!(
                    "Could not detect a supported bootloader. Remove any existing {} arguments and add:\n    {}",
                    keys.join(" / "),
                    add.join(" ")
                );
                Ok(())
            }
        }
    }
}

pub fn arg_key(arg: &str) -> &str {
    arg.split_once('=').map_or(arg, |(k, _)| k)
}

const DEFAULT_GRUB: &str = "/etc/default/grub";
const CMDLINE_VAR: &str = "GRUB_CMDLINE_LINUX_DEFAULT";

/// Rewrite the GRUB_CMDLINE_LINUX_DEFAULT line of /etc/default/grub contents.
pub fn edit_grub_text(content: &str, keys: &[&str], add: &[String]) -> String {
    let render = |existing: &str| {
        let mut args: Vec<String> = existing
            .split_whitespace()
            .filter(|a| !keys.contains(&arg_key(a)))
            .map(String::from)
            .collect();
        args.extend(add.iter().cloned());
        format!("{CMDLINE_VAR}=\"{}\"", args.join(" "))
    };
    let mut found = false;
    let mut lines: Vec<String> = content
        .lines()
        .map(|line| {
            match line
                .trim_start()
                .strip_prefix(CMDLINE_VAR)
                .and_then(|r| r.strip_prefix('='))
            {
                Some(value) if !found => {
                    found = true;
                    render(value.trim().trim_matches(['"', '\'']))
                }
                _ => line.to_string(),
            }
        })
        .collect();
    if !found {
        lines.push(render(""));
    }
    lines.join("\n") + "\n"
}

fn edit_default_grub(keys: &[&str], add: &[String], dry_run: bool) -> Result<()> {
    let content =
        fs::read_to_string(DEFAULT_GRUB).with_context(|| format!("reading {DEFAULT_GRUB}"))?;
    let updated = edit_grub_text(&content, keys, add);
    if updated == content {
        return Ok(());
    }
    let new_line = updated
        .lines()
        .find(|l| l.starts_with(CMDLINE_VAR))
        .unwrap_or_default();
    if dry_run {
        println!("would set in {DEFAULT_GRUB}: {new_line}");
        return Ok(());
    }
    let backup = format!("{DEFAULT_GRUB}.monitor-oc.bak");
    if !Path::new(&backup).exists() {
        fs::copy(DEFAULT_GRUB, &backup).with_context(|| format!("backing up to {backup}"))?;
    }
    fs::write(DEFAULT_GRUB, &updated).with_context(|| format!("writing {DEFAULT_GRUB}"))?;
    println!("updated {DEFAULT_GRUB}: {new_line}");
    Ok(())
}

pub fn which(cmd: &str) -> bool {
    let path = std::env::var("PATH").unwrap_or_default();
    path.split(':')
        .chain(["/usr/sbin", "/sbin", "/usr/bin", "/bin"])
        .any(|dir| Path::new(dir).join(cmd).is_file())
}

pub fn run<S: AsRef<str>>(cmd: &[S], dry_run: bool) -> Result<()> {
    let cmd: Vec<&str> = cmd.iter().map(AsRef::as_ref).collect();
    println!(
        "{} {}",
        if dry_run { "would run:" } else { "+" },
        cmd.join(" ")
    );
    if dry_run {
        return Ok(());
    }
    let status = Command::new(cmd[0])
        .args(&cmd[1..])
        .status()
        .with_context(|| format!("running {}", cmd[0]))?;
    if !status.success() {
        bail!("`{}` failed ({status})", cmd.join(" "));
    }
    Ok(())
}

pub fn run_capture(cmd: &[&str]) -> Result<String> {
    let out = Command::new(cmd[0])
        .args(&cmd[1..])
        .output()
        .with_context(|| format!("running {}", cmd[0]))?;
    if !out.status.success() {
        bail!(
            "`{}` failed: {}",
            cmd.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

pub fn cmdline() -> String {
    fs::read_to_string("/proc/cmdline").unwrap_or_default()
}

pub fn is_root() -> bool {
    fs::metadata("/proc/self")
        .map(|m| m.uid() == 0)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grub_edit_replaces_our_args_only() {
        let input = "GRUB_TIMEOUT=5\nGRUB_CMDLINE_LINUX_DEFAULT=\"quiet splash drm.edid_firmware=DP-1:edid/old.bin\"\n";
        let out = edit_grub_text(
            input,
            &["drm.edid_firmware"],
            &["drm.edid_firmware=DP-1:edid/new.bin".into()],
        );
        assert_eq!(
            out,
            "GRUB_TIMEOUT=5\nGRUB_CMDLINE_LINUX_DEFAULT=\"quiet splash drm.edid_firmware=DP-1:edid/new.bin\"\n"
        );
    }

    #[test]
    fn grub_edit_adds_missing_line_and_removes() {
        let out = edit_grub_text("GRUB_TIMEOUT=5\n", &[], &["a=b".into()]);
        assert_eq!(out, "GRUB_TIMEOUT=5\nGRUB_CMDLINE_LINUX_DEFAULT=\"a=b\"\n");
        let out = edit_grub_text(&out, &["a"], &[]);
        assert_eq!(out, "GRUB_TIMEOUT=5\nGRUB_CMDLINE_LINUX_DEFAULT=\"\"\n");
    }
}
