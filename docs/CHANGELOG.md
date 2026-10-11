# Changelog

## 0.2.0.1 (testing)

- **Game Mode scaling fixed.** The app no longer draws everything about three times too big (and larger than
  the screen) in Game Mode on the ROG Ally X and other 1080p handhelds.
- **Controller support in Game Mode.** D-pad or left stick moves between controls, A presses, B closes a
  dialog, LB/RB switch pages. No Steam Input layout needed.
- **Settings page with Stable and Testing channels.** Monitor OC can update itself, keeps the version it
  replaces, and can roll back. Test builds get fixes first; a test build that fails to start three times
  rolls itself back, and "Monitor OC Testing Rescue" in the app menu can roll back, return to Stable or
  uninstall even when a test build won't open.
- New command: `monitor-oc update` (`--check`, `--channel stable|testing`, `--rollback`).
- **Automatic revert that actually works.** Built-in screen trials are now tested from the app in Game Mode:
  the screen switches, and without "Keep" within 10 seconds (or with a black screen) it goes back to stock by
  itself. A second 10-second check after 5 minutes makes it permanent. Until a rate passes, Game Mode holds
  it at the stock rate even if it is picked in Quick Access, and a watchdog restarts Game Mode on stock if
  the app dies mid-test.
- External monitor tests use the same 10 seconds plus a double check after 5 minutes.
- **Error reports like Ally Hub.** With a GitHub access key in Settings, crashes, failed updates and failed
  tests can be reported automatically, and **Report a problem** sends your own description with the
  diagnostics. Also `monitor-oc report`.
- Builds are no longer GitHub releases: install.sh and updates download them from the project's site, and
  Stable or Testing is chosen in the app.
- **Uninstall fixed.** `monitor-oc uninstall` (and the install script's `--uninstall`) now removes everything,
  including the Game Mode profile and settings (`--keep-settings` to keep them), takes care of external
  monitor overrides through a password prompt, and the app has an Uninstall button.

## 0.2.0

- First release with the graphical app.
