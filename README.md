<div align="center">

<img src="assets/io.github.ravenor907.MonitorOC.svg" alt="" width="96">

# Monitor OC

**Raise your screen's refresh rate on Linux, test it safely, keep what works.**

[![CI](https://github.com/Ravenor907/monitor-oc/actions/workflows/ci.yml/badge.svg)](https://github.com/Ravenor907/monitor-oc/actions/workflows/ci.yml)
![Version](https://img.shields.io/badge/dynamic/regex?url=https%3A%2F%2Fraw.githubusercontent.com%2FRavenor907%2Fmonitor-oc%2Fmain%2FVERSION&search=.%2B&label=stable&color=2ea44f)
![License](https://img.shields.io/badge/license-GPL--3.0-blue)

</div>

- **Handheld screens in Game Mode** (SteamOS, Bazzite): ROG Ally / Ally X, Steam Deck, Legion Go
  and others. The ROG Ally X's 120 Hz panel, for example, can run 125, 130 Hz or more.
- **External monitors** on SteamOS (docked), Bazzite, Fedora and most other distros, in Game
  Mode and on every desktop.
- **Made for handhelds:** full controller support in Game Mode, touch-friendly, sized for 7" screens.
- **Safe by design:** every rate is switched on for 10 seconds and goes back by itself unless you
  keep it, then checked again after 5 minutes. A black screen fixes itself.
- **Keeps itself up to date**, with a rollback for every update.

> [!WARNING]
> Overclocking a screen is at your own risk. Go up in small steps (about 5 Hz).

## Install

One time, about 5 minutes, needs an internet connection.

**1. Switch to Desktop Mode.** Press the **Steam** button, go to **Power**, then **Switch to Desktop**.

**2. Open Konsole.** Click the app launcher in the bottom-left corner, then **System → Konsole**.
No keyboard? Press **Steam + X** for the on-screen one.

**3. Paste this and press Enter.** Right-click → **Paste**, or **Ctrl + Shift + V**.

```bash
(git -C ~/MonitorOC pull || git clone https://github.com/Ravenor907/monitor-oc ~/MonitorOC) && bash ~/MonitorOC/scripts/install.sh
```

It's finished when it says **Done!** The first time it sets up a private Rust compiler in your home
folder and builds Monitor OC (no sudo, nothing on SteamOS's read-only system, so it survives SteamOS
updates).

**4. Open Monitor OC** from the desktop icon, the app menu, or your Game Mode library (the installer adds
it there for you on SteamOS). Everything works in Game Mode with the controller; Steam's keyboard opens
when something needs typing.

That's it. Monitor OC keeps itself up to date from here on.

<details>
<summary><b>Troubleshooting</b></summary>

- **"git: command not found"**: [download the ZIP](https://github.com/Ravenor907/monitor-oc/archive/refs/heads/main.zip),
  extract it into your home folder as `MonitorOC`, open Konsole there and run `bash scripts/install.sh`.
- **It asks for a password**: the installer never does. Only changes to external monitors need one later.
- **The build stops with an error**: run the command again (a dropped connection is the usual cause). If it
  keeps failing, open a [bug report](https://github.com/Ravenor907/monitor-oc/issues/new?template=bug_report.yml)
  with the last lines it printed.

</details>

## Use

**Built-in screen (Game Mode)**

1. On the **Built-in screen** page, pick a rate a little above stock and start a trial.
2. Restart Game Mode so it loads the trial, then open Monitor OC again and press **Test**.
3. The screen switches. **Keep** it within 10 seconds if the picture is right, and once more
   after 5 minutes. No answer, or a black screen, and it goes back to stock on its own.

**External monitors:** the **External monitors** page tests a mode live with the same two checks,
then installs it for every boot (that step asks for your password).

**Verify** measures the real refresh rate and checks for skipped frames. **Diagnostics** explains
anything that would stop an overclock from working.

Controller: D-pad or left stick to move, **A** to press, **B** to go back, **LB/RB** for pages.

Everything is also on the command line: `monitor-oc --help`.

## Updates

Monitor OC updates itself when it starts (Settings, or `monitor-oc update`). The channel is a setting
in the app:

| Channel | What you get |
|---|---|
| **Stable** (default) | Builds the owner has run and signed off. |
| **Testing** | New fixes first, numbered Stable + revision (0.2.0.1, 0.2.0.2, ...). Can have bugs. |

Every update keeps the version it replaces: **Roll back** in Settings or
`monitor-oc update --rollback`. A build that fails to start three times rolls itself back, and on
Testing, **Monitor OC Testing Rescue** in the app menu works even if the app won't open.

## Error reports

Like Ally Hub, problems can go straight to the developer as GitHub issues. In Settings, add a GitHub
access key and turn on **Send error reports automatically** (crashes, failed updates and failed tests),
or press **Report a problem** (also on Diagnostics, or `monitor-oc report "what happened"`). Reports
include the diagnostics and recent activity, with your user and host names taken out.

## Uninstall

Press **Uninstall** in Settings, or run this in Konsole:

```bash
bash ~/MonitorOC/scripts/uninstall.sh
```

It removes the app, its menu entries, settings and private compiler, and the built-in screen goes back
to stock. Remove the Steam shortcut too if you added one.

## More

- [Guide](docs/GUIDE.md): command line, timings, verifying, diagnostics, recovery, supported systems
- [How it works](docs/HOW-IT-WORKS.md) and building from source
- [Changelog](docs/CHANGELOG.md) · [Contributing](docs/CONTRIBUTING.md)
- Found a problem? **Report a problem** in the app, or
  [open an issue](https://github.com/Ravenor907/monitor-oc/issues/new/choose).

GPL-3.0 · made by [Ravenor907](https://github.com/Ravenor907)
