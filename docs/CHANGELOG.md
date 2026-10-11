# Changelog

## 0.2.0.2 (testing)

Everything works in Game Mode, with the controller:

- **External monitors in Game Mode.** Testing, installing and removing a mode asks for the administrator
  password in the app (Game Mode can't show the system prompt); it's kept only while the app is open.
  Steam's keyboard opens by itself for password and text fields. Uninstalling asks the same way when
  external overrides have to go.
- **Restart Game Mode** button on the Built-in screen page, for loading a trial or changed settings
  without leaving Game Mode.
- **Exit Monitor OC** button in Game Mode.
- Controller: the right stick scrolls; up and down leave a text field; the resolution drop-down is now a
  ‹ › picker the D-pad can use.
- In Game Mode a new external mode is made the preferred one by default (Gamescope uses that).
- The installer adds Monitor OC to the Game Mode library on SteamOS (once).

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
- **Installed like Ally Hub.** `git clone` and `scripts/install.sh`, which builds Monitor OC on the device
  with a private compiler (no releases or downloads of prebuilt files); updates download the new source
  and build it; `scripts/uninstall.sh` removes everything. Stable or Testing is chosen in the app.
- **Uninstall fixed.** `monitor-oc uninstall` (and the install script's `--uninstall`) now removes everything,
  including the Game Mode profile and settings (`--keep-settings` to keep them), takes care of external
  monitor overrides through a password prompt, and the app has an Uninstall button.

## 0.2.0

- First release with the graphical app.
