# monitor-oc

Monitor refresh-rate overclocking for Linux. It's built for **SteamOS** first and also works on **Fedora**
(Workstation and Atomic/Bazzite) and most other distros.

monitor-oc gives the kernel a modified copy of your monitor's EDID (the data block in which a monitor
lists its supported modes) with an extra, faster mode added. The change happens at the kernel level,
so Gamescope (SteamOS Game Mode), KDE, GNOME, Hyprland and X11 all see the new mode as if the monitor
supported it natively. You don't need any per-desktop tricks.

It ships as a single static binary with no dependencies, so the same file runs on every distro.

> **Overclocking a monitor is at your own risk.** Most monitors either work or show a black screen or
> "out of range" message at an unsupported refresh rate. Use `test` first. It reverts automatically.

## Supported systems

| System | How it's made persistent | Survives OS updates |
|---|---|---|
| SteamOS 3 (Steam Deck, other handhelds) | `/etc/default/grub` + `update-grub` | Yes. `monitor-oc.service` re-applies at boot if an update dropped it |
| Fedora Atomic, Bazzite, Universal Blue | `rpm-ostree kargs` | Yes |
| Fedora Workstation, RHEL family | `grubby` | Yes |
| Other GRUB distros (Arch, Ubuntu, …) | `/etc/default/grub` + `update-grub` / `grub-mkconfig` | Yes |
| systemd-boot and others | prints the kernel arguments for you to add | — |

GPU drivers: **AMD (amdgpu) and Intel (i915/xe)** are supported. The **NVIDIA proprietary driver
ignores kernel EDID overrides**. Use `nvidia-settings` there instead, or nouveau/NVK.

The **Steam Deck's built-in screen is not supported**. Gamescope manages its refresh range, so
monitor-oc targets the external monitor you dock to. It refuses internal panels unless you pass
`--force`.

## Install

Download `monitor-oc-x86_64` from the [Releases](https://github.com/ravenor907/fedora-monitor-overclocker/releases)
page, or build it with `cargo build --release`.

```sh
chmod +x monitor-oc-x86_64
sudo ./monitor-oc-x86_64 detect
```

**On SteamOS:** switch to Desktop Mode and open Konsole. If you've never set a password, run `passwd`
first, because `sudo` needs one. Nothing has to be installed, and you don't need to unlock the read-only
system yourself: monitor-oc unlocks it only while it runs `update-grub`, then locks it again.

## Usage

```sh
# 1. Find your monitor's connector (DP-1, HDMI-A-1, ...) and its current modes.
monitor-oc detect

# 2. Try a mode live. Pick it in your display settings within 30 s, then
#    type "keep" to keep it until reboot. Anything else, or a black screen, reverts.
sudo monitor-oc test DP-1 2560x1440@170

# 3. Make it permanent (reboot afterwards). On SteamOS use --preferred so
#    Game Mode picks the new refresh rate automatically.
sudo monitor-oc apply DP-1 2560x1440@170 --preferred

# Check it, or undo it.
monitor-oc status
sudo monitor-oc remove DP-1
```

Other commands:

- `monitor-oc show DP-1` lists every timing and the range limits in the monitor's EDID (also accepts an
  EDID file path).
- `monitor-oc generate DP-1 2560x1440@170 -o oc.bin` writes the modified EDID to a file without touching
  the system.
- `sudo monitor-oc apply ... --dry-run` prints exactly what would change.
- `sudo monitor-oc reset DP-1` drops a live override (for example after interrupting `test` with Ctrl-C).

### Graphical app

Run `monitor-oc` without arguments (or `monitor-oc gui`) in Desktop Mode or Game Mode to open the app:

- **Built-in screen**: raise the refresh limit Game Mode offers on a handheld's own screen (ROG Ally / Ally X,
  Steam Deck, Legion Go, ...). Start a trial, switch to Game Mode, pick the rate in Quick Access >
  Performance, then come back and keep it. No password needed. If the screen goes black, hold the power
  button: the trial turns itself off on the next start.
- **External monitors**: pick a mode and see the exact timing and EDID change before anything is written.
  *Test live* reverts by itself after 20 s unless you keep it; *Apply* installs it for every boot. Both ask for
  your password through polkit, so they need Desktop Mode.
- **Verify**: measures the refresh rate the window is really shown at, counts skipped frames, and has
  motion and frame-skipping tests.
- **Diagnostics**: `monitor-oc doctor` with a *Copy report* button.

In Game Mode (add monitor-oc to Steam as a non-Steam game) it opens full screen with larger controls. It
works with a controller mapped to keys: D-pad/arrows move, A/Enter presses, Tab cycles, F1-F4 or
Ctrl+Tab switch pages, Ctrl +/- change the size. `monitor-oc gui --demo` shows it with made-up hardware and
changes nothing.

### Choosing timings (`--timing`)

- `auto` (default) uses `native` if the monitor already has a mode at that resolution, otherwise `cvt-rb2`.
- `native` keeps the monitor's own blanking intervals and only raises the pixel clock. This is how most
  successful monitor overclocks are done, because the monitor's controller already accepts these timings.
- `cvt-rb2` uses VESA reduced blanking v2 and the lowest pixel clock. Try it when `native` fails or when
  you hit cable or port bandwidth limits.
- `cvt-rb` uses VESA reduced blanking v1, which is the most widely accepted.

Increase the refresh rate in small steps (for example 144 → 155 → 165) and check for frame skipping
with a frame-skip test site such as testufo.com. Some monitors show a picture at a higher rate but
silently drop frames.

## How it works

1. It reads the monitor's EDID from `/sys/class/drm/card*-*/edid` and saves an untouched copy in
   `/etc/monitor-oc/`.
2. It adds the new mode as a detailed timing in a free slot of the base block or CTA-861 extension.
   Modes above 655 MHz (the limit for a standard EDID timing, reached by 1440p at about 160 Hz or more)
   go into a DisplayID extension instead. It also raises the range-limits descriptor and fixes all
   checksums.
3. It writes the result to `/etc/firmware/edid/monitor-oc-<connector>.bin` and adds these kernel arguments:
   `firmware_class.path=/etc/firmware drm.edid_firmware=DP-1:edid/monitor-oc-DP-1.bin`.
   `/etc` is used because it survives SteamOS updates and is writable on Fedora Atomic.
4. It installs `monitor-oc.service`, a boot-time oneshot that:
   - re-adds the kernel arguments if an OS update removed them, and
   - checks that each monitor actually received its override. If the GPU driver probed before the
     firmware file was readable (from the initramfs), the service re-probes the monitor or applies the
     override live through debugfs.

`test` uses the kernel's debugfs `edid_override` together with a simulated hotplug, so nothing is
written to disk except the backup of the original EDID.

## Recovery

The override only affects the connector it was applied to. If a mode doesn't work:

- **Steam Deck:** unplug the external monitor. The built-in screen is unaffected. Then run
  `sudo monitor-oc remove`.
- **Desktop:** plug the monitor into another port or input, or switch to a TTY (Ctrl+Alt+F3), and run
  `sudo monitor-oc remove`.
- **Last resort:** in the GRUB menu, press `e` and delete the `drm.edid_firmware=...` argument for one boot.

## Building

```sh
cargo test
cargo build --release                                                          # CLI + graphical app
cargo build --release --no-default-features --target x86_64-unknown-linux-musl # static CLI-only binary
```

The graphical app (the default `gui` feature) uses egui with OpenGL and loads X11, Wayland and OpenGL at
runtime, so the binary links nothing beyond glibc and builds without extra development packages.

## License

GPL-3.0. See [LICENSE](LICENSE).
