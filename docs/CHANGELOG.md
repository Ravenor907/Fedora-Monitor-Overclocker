# Changelog

## 0.2.0.1 (testing)

- **Game Mode scaling fixed.** The app no longer draws everything about three times too big (and larger than
  the screen) in Game Mode on the ROG Ally X and other 1080p handhelds.
- **Controller support in Game Mode.** D-pad or left stick moves between controls, A presses, B closes a
  dialog, LB/RB switch pages. No Steam Input layout needed.
- **Updates page with Stable and Testing channels.** Monitor OC can update itself, keeps the version it
  replaces, and can roll back. Test builds get fixes first; a test build that fails to start three times
  rolls itself back, and "Monitor OC Testing Rescue" in the app menu can roll back, return to Stable or
  uninstall even when a test build won't open.
- New command: `monitor-oc update` (`--check`, `--channel stable|testing`, `--rollback`).

## 0.2.0

- First release with the graphical app.
