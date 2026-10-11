# Monitor OC: notes for Claude

Monitor OC is the owner's refresh-rate overclocking app and CLI (`monitor-oc`) for Linux, first of all his
**ASUS ROG Xbox Ally X on SteamOS** (built-in screen in Game Mode, external monitors when docked). Rust, one
binary: the egui app (`gui` feature, default) plus the CLI; `--no-default-features` builds a static CLI-only
binary. Read this fully before changing anything.

Never put the owner's real name anywhere in the project (code, docs, commits, issues, release notes, app text).
Use "the owner" or his GitHub handle, Ravenor907.

## Building and checking

CI (`.github/workflows/ci.yml`) is the gate: `cargo fmt --check`, clippy with `-D warnings` for both feature
sets, `cargo test` for both, shellcheck on install.sh, desktop-file-validate. All must pass before anything is
merged into `testing` or `main`. Keep the static CLI build free of new dependencies (updates use `curl` and
`sha256sum` through `std::process`, controller input reads `/dev/input` with `std` only).

## Branches and update channels (the owner's call, same model as Ally Hub)

- `main` is **Stable**: what install.sh and every Stable device get. `testing` holds the owner's **test
  builds**. Devices on Testing (`~/.config/monitor-oc/update.conf`, `channel=testing`) install whichever of
  `main` and `testing` has the higher build; Stable devices never look at `testing`. See `src/update.rs`.
- **Two numbers.** `Cargo.toml`'s version is x.y.z. The `VERSION` file is the build: equal to Cargo's on
  Stable, Cargo's plus a revision on testing (Cargo 0.2.0 -> testing 0.2.0.1, 0.2.0.2, ...). A test enforces
  that VERSION starts with Cargo's version. `--version`, the app and reports show VERSION, with "(testing)"
  for test builds.
- **At most 10 test revisions per cycle** (`MAX_TEST_REVISIONS`, enforced by a test). Needing more means the
  cycle carries too much; keep each one laser focused.
- **Only the owner ships.** Nothing goes to `main` unless the owner says to ship that build, in his own words.
  No scheduled ships, no "Stable tomorrow". Every change, fixes included, goes to `testing`; then say it's
  ready and wait. Stable only ever gets builds the owner has run on his own device.
- The testing branch's VERSION must always be **higher** than main's, or test devices fall back to main.
  Never delete the `testing` branch.
- **No GitHub releases, tags or packages (the owner's call, like Ally Hub).** Distribution is the `dist`
  branch: every push to `main` or `testing` that passes CI is built and published there by the `publish`
  job, in `stable/` or `testing/` (binary `monitor-oc-x86_64`, its `.sha256`, `VERSION`), as one commit
  that is force-replaced each time. install.sh and `src/update.rs` read from there. Never create releases
  or tags, and never commit binaries anywhere else.
- **Releasing a test build:** bump the revision in VERSION, add a `docs/CHANGELOG.md` entry, merge into
  `testing` via a PR once CI is green. The push publishes `dist/testing`.
- **Shipping ("ship it"):** bump `Cargo.toml` to the next x.y.z (patch for fixes, minor for real features),
  set VERSION to the same, update `Cargo.lock`'s monitor-oc version, merge testing into main (publishes
  `dist/stable`), then reset `testing` to main so the next cycle starts at x.y.z.1. The first ship after
  the switch to `dist` also retires the old v0.1.0/v0.2.0 GitHub releases and tags (old main's install.sh
  still points at them until then).
- Leaving Testing: the app's Updates page offers "Go back to Stable x.y.z" (`update::install` of an older
  build); `monitor-oc update --channel stable` does the same.

## Safety nets (never weaken these)

- Every install keeps the replaced binary in `~/.local/share/monitor-oc/previous/` for a rollback (Updates
  page, `monitor-oc update --rollback`).
- A fresh build that fails to start `MAX_FAILED_STARTS` times rolls itself back before opening a window
  (`update::startup_check`, called first thing in `gui::run`); the app marks a build healthy after
  `HEALTHY_AFTER_SECS`. Rolled-back builds are never offered again.
- **Testing Rescue:** on the Testing channel a plain-shell script and menu entry ("Monitor OC Testing Rescue")
  can roll back, go back to Stable or uninstall without running any app code. Keep it working when the
  binary is broken.
- Downloads are checked against the release's `.sha256` and must report the expected `--version` before
  anything is replaced. Never change the update source (`update::REPO`, branch names, asset names).
- Overclock safety: during a built-in screen trial the Lua profile holds every untested rate at stock;
  the app unlocks one (`live`, `live_until`, `live_pid` in the state file) for `live::CONFIRM_SECS` (10 s),
  then after the first keep until the double check at `live::RECHECK_SECS` (5 min). No answer reverts.
  `monitor-oc panel watchdog` (started via systemd-run outside the app's process tree) restarts Game Mode
  if the app dies or misses a deadline. External `test` uses the same 10 s + 5 min checks. Never shorten
  these, remove a check, or let a rate become permanent without both keeps.
- `uninstall` removes everything by default (settings and the Game Mode profile included);
  `--keep-settings` keeps them, `--purge` is accepted for old scripts.

## Game Mode

- Gamescope's Xwayland has no Xft.dpi, so winit guesses a ~3.25x scale from the panel size; `gui::run` pins
  `WINIT_X11_SCALE_FACTOR=1` in Game Mode and `App::apply_zoom` cancels any native scale there. The UI zoom
  (150% default on 1080p handhelds) is then the only scaling.
- Controllers: `gui/gamepad.rs` reads `/dev/input` (Steam's default layout for non-Steam games sends gamepad
  events, not keys) and feeds keys into egui: D-pad/left stick = arrows, A = Enter, B = Escape,
  LB/RB = PageUp/PageDown. Skips devices in `SDL_GAMECONTROLLER_IGNORE_DEVICES`. Every new control must be
  reachable and usable with arrows + Enter (use the widgets in `gui/widgets.rs`, which draw focus rings).
