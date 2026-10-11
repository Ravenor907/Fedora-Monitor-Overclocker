# How Monitor OC works


**Built-in screens:** Gamescope reads Lua scripts from `~/.config/gamescope/scripts` after its own.
Monitor OC's script looks up the profile Gamescope would use for your panel, copies it, adds the
rates up to your limit and generates their timings on demand (`dynamic_modegen`). During a trial it
generates the stock mode for any untested rate; the app unlocks one rate for a deadline in the
state file and asks Gamescope to switch through the `GAMESCOPE_DYNAMIC_REFRESH` root window
property (the one Steam's slider uses), then switches back unless it is kept. It reads its
settings from `~/.config/monitor-oc/gamescope.conf` and writes what it did to
`~/.config/monitor-oc/gamescope-report`, which `panel status`, `doctor` and the app show.

**External monitors:**

1. It reads the monitor's EDID from `/sys/class/drm/card*-*/edid` and saves an untouched copy in
   `/etc/monitor-oc/`.
2. It adds the new mode as a detailed timing in a free slot of the base block or CTA-861
   extension. Modes above 655 MHz (the limit for a standard EDID timing, reached by 1440p at about
   160 Hz or more) go into a DisplayID extension instead. It also raises the range-limits
   descriptor and fixes all checksums.
3. It writes the result to `/etc/firmware/edid/monitor-oc-<connector>.bin` and adds the kernel
   arguments `firmware_class.path=/etc/firmware drm.edid_firmware=DP-1:edid/monitor-oc-DP-1.bin`.
   `/etc` is used because it survives SteamOS updates and is writable on Fedora Atomic.
4. It installs `monitor-oc.service`, a boot-time oneshot that re-adds the kernel arguments if an
   OS update removed them, and checks that each monitor received its override (re-probing or
   applying it live if the GPU driver probed before the file was readable).

`test` uses the kernel's debugfs `edid_override` with a simulated hotplug, so nothing is written to
disk except the backup of the original EDID.

## Building

```sh
cargo build --release          # the app and the command line: target/release/monitor-oc
cargo build --release --no-default-features --target x86_64-unknown-linux-musl   # static CLI only
cargo test                     # luajit and desktop-file-validate, if installed, enable two more tests
./target/release/monitor-oc install
```

The GUI build needs no extra system packages: its windowing and OpenGL libraries are loaded at
runtime. The icon is [`assets/io.github.ravenor907.MonitorOC.svg`](assets/io.github.ravenor907.MonitorOC.svg);
after changing it, regenerate the PNG that Steam uses:
`rsvg-convert -w 256 -h 256 assets/io.github.ravenor907.MonitorOC.svg -o assets/io.github.ravenor907.MonitorOC.png`.

## Releases

There is one branch, `main`, and no GitHub releases or packages. Every push to `main` that passes CI is
built and published on the project's GitHub Pages site: `testing/` always gets the new build, `stable/`
only when `VERSION` is a Stable version (x.y.z). Each folder holds the binary, its `.sha256` and `VERSION`.
install.sh and the app's updater download from there; the Stable / Testing choice is a setting in the app.
