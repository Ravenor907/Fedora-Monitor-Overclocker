# Monitor OC guide

Everything the [README](../README.md) leaves out: how each method works in practice, the command line,
checking a result and getting out of trouble.

## Built-in screens (SteamOS Game Mode, Bazzite)

### Why Game Mode needs its own method

In Game Mode the screen is driven by **Gamescope**, Valve's compositor. For handheld panels it
knows (Steam Deck LCD and OLED, ROG Ally and Ally X, Legion Go, ...) Gamescope does not use the
modes the panel reports in its EDID. It builds every refresh rate itself from a built-in Lua
profile, and that profile stops at the panel's stock range: 48-120 Hz on the ROG Ally X. That
range is what the **Refresh Rate** slider in Quick Access offers.

That is why the usual method, giving the kernel a modified EDID (what CRU does on Windows, and the
only method earlier versions of Monitor OC had), does nothing for a built-in screen in Game Mode:
Gamescope never looks at it. On a ROG Ally X the old version refused the built-in screen, and
with `--force` it changed nothing in Game Mode.

Monitor OC now adds a small user profile, `~/.config/gamescope/scripts/zz-monitor-oc.lua`, that
extends Gamescope's own profile for your exact panel. Rates in the stock range keep Gamescope's
original timings; only the new, higher rates use timings computed by Monitor OC. It needs no root,
does not touch the read-only system and does not affect Desktop Mode.

### The trial: why a black screen is not a problem

A new rate never becomes permanent by itself:

1. **Try** a rate. It is armed for the **next Game Mode start only**.
2. When Game Mode starts, the profile knows the new rates but holds them at the stock rate: picking
   one in Quick Access changes nothing yet.
3. In Monitor OC, **Test** switches the screen to the new rate for 10 seconds. Keep it, or it goes
   back to stock on its own (a black screen included).
4. After 5 minutes it asks once more, again with 10 seconds. Keeping it then makes it permanent.
5. If the app is closed or crashes during a test, a watchdog restarts Game Mode on the stock range.
   If Game Mode starts again before a rate is kept, the trial is dropped.

Monitor OC only accepts rates above the stock maximum and at most 60% above it (up to 192 Hz on a
120 Hz panel). Few panels get anywhere near that: go up about 5 Hz at a time.

### With the app

1. Open Monitor OC in Desktop Mode, or from your Steam library in Game Mode.
2. On the **Built-in screen** page, check that it found your panel and its stock range, pick a rate
   a little above stock (for example 125 or 130 Hz on a ROG Ally X) and start the trial.
3. Start Game Mode. In Desktop Mode use **Return to Gaming Mode**; if you are already in Game Mode,
   restart it (switch to Desktop and back, or reboot) so Gamescope loads the new profile.
4. Open Monitor OC in Game Mode and press **Test**. Keep it within 10 seconds if it looks right.
5. Use it for 5 minutes (the **Verify** page measures it, see [Verify](#verify-it-works)), then keep
   it once more when asked. Stay in Monitor OC or come back before the 5 minutes are up.

### With the command line

Run these as your normal user, never with `sudo` (they refuse):

```sh
monitor-oc panel status      # the panel, its stock range and what is set up
monitor-oc panel try 130     # arm a 130 Hz trial for the next Game Mode start
# start Game Mode, test it in the app (Test, keep twice)
monitor-oc panel confirm     # or keep it without the live test (at your own risk)
monitor-oc panel discard     # or: drop the trial, keep the previous limit
monitor-oc panel reset       # back to the stock range (keeps Monitor OC's files)
monitor-oc panel remove      # delete Monitor OC's Gamescope profile entirely
```

`panel try` takes `--method`:

- `clock` (default) keeps the panel's blanking and raises the pixel clock, like CRU's default.
- `porch` shrinks the vertical blanking first and only raises the pixel clock if that is not
  enough. It is gentler on the display link and harder on the panel's timing controller. Try it if
  `clock` flickers or goes black at a rate.

> **Hardware status:** the Game Mode method is new. It is tested in CI against a simulation of
> Gamescope's display scripts (LuaJIT), but not yet confirmed on every handheld. Please report
> what works on yours, with the output of `monitor-oc doctor`.

## External monitors

For a monitor on DisplayPort or HDMI, Monitor OC gives the kernel a modified copy of the monitor's
EDID with an extra, faster mode. Because the change is at the kernel level, Gamescope (Game Mode
while docked), KDE, GNOME, Hyprland and X11 all see the new mode as if the monitor supported it
natively. This needs root, because it changes the kernel command line.

On SteamOS, `sudo` needs a password: if you never set one, run `passwd` in Konsole first.

### With the app

The **External monitors** page lists the connected monitors and their modes and takes you through
the same steps as below: a live test that reverts by itself, then installing the mode for every
boot. For the steps that need root it asks for your password (through `pkexec`) or shows the exact
command to run.

### With the command line

```sh
# 1. Find your monitor's connector (DP-1, HDMI-A-1, ...) and its current modes.
monitor-oc detect

# 2. Try a mode live. Pick it in your display settings and type "keep" within
#    10 s, then again when asked after 5 minutes. Anything else, or a black
#    screen, reverts. (--seconds and --recheck change the times.)
sudo ~/.local/bin/monitor-oc test DP-1 2560x1440@170

# 3. Install it for every boot, then reboot. On SteamOS use --preferred so
#    Game Mode picks the new refresh rate automatically.
sudo ~/.local/bin/monitor-oc apply DP-1 2560x1440@170 --preferred

# Check it, or undo it.
monitor-oc status
sudo ~/.local/bin/monitor-oc remove DP-1
```

More commands:

- `monitor-oc show DP-1` lists every timing and the range limits in the monitor's EDID (also
  accepts an EDID file).
- `monitor-oc generate DP-1 2560x1440@170 -o oc.bin` writes the modified EDID to a file without
  changing the system.
- `sudo ~/.local/bin/monitor-oc apply ... --dry-run` prints exactly what would change.
- `sudo ~/.local/bin/monitor-oc reset DP-1` drops a live override, for example after stopping
  `test` with Ctrl-C.

**Secure Boot:** with Secure Boot on, the kernel's lockdown mode blocks the debugfs interface that
`test` uses, so live testing fails. `apply` followed by a reboot still works, so test carefully
with small steps; `monitor-oc doctor` tells you whether lockdown is on.

**GPU drivers:** AMD (amdgpu) and Intel (i915, xe) work. The **NVIDIA proprietary driver ignores
kernel EDID overrides**; use `nvidia-settings` there, or nouveau/NVK.

**Built-in screens:** `test` and `apply` refuse built-in panels (eDP) where Gamescope is installed
and point you to `monitor-oc panel` instead. `--force` overrides the EDID anyway, which only
affects Desktop Mode.

### Choosing timings (`--timing`)

- `auto` (default) uses `native` if the monitor already has a mode at that resolution, otherwise
  `cvt-rb2`.
- `native` keeps the monitor's own blanking and only raises the pixel clock. Most successful
  monitor overclocks are done this way, because the monitor already accepts these timings.
- `cvt-rb2` uses VESA reduced blanking v2 and the lowest pixel clock. Try it when `native` fails or
  when you hit cable or port bandwidth limits.
- `cvt-rb` uses VESA reduced blanking v1, which is the most widely accepted.

## Verify it works

A screen can accept a faster signal and still not show more frames. Check two things:

**1. Is the new mode actually in use?** The app's **Verify** page measures how often new frames
reach the screen. If the measured rate matches the rate you set (for example 130 Hz), the mode is
active. If it shows the old rate, the mode was not applied: in Game Mode, check the Refresh Rate
slider and that no frame limit is set; on a desktop, check the display settings.

For the built-in screen, `monitor-oc panel status` (and the **Built-in overclock** and
**Gamescope profile** lines in [Diagnostics](#diagnostics)) shows what Gamescope itself did:

| You see | It means |
|---|---|
| `loaded 2 min ago; extended to 130 Hz` | Gamescope read Monitor OC's profile at the start of this session and offers rates up to 130 Hz. |
| `last mode 1920x1080@130 at 309.6 MHz` | The mode Gamescope last generated, i.e. what the slider asked for, and its pixel clock. |
| `settings changed since, restart Game Mode` | You tried or confirmed something after Game Mode started; it takes effect at the next start. |
| `has not loaded monitor-oc's profile yet` | Game Mode has not started since Monitor OC was set up. |
| `trial ended without confirmation` | Game Mode restarted before you confirmed, so the trial was dropped (this is the safety net). |

**2. Is the panel skipping frames?** Some panels show a picture at a higher rate but silently drop
frames, which looks like stutter and defeats the point. Use a frame-skip photo test (the app's
Verify page has one; [testufo.com/frameskipping](https://www.testufo.com/frameskipping) works in a
browser on the desktop):

1. Show the test pattern: a block steps one square to the right on every frame.
2. Take a photo of the screen with a phone camera at a long-ish exposure, about 1/10 s (use the
   camera's manual or "pro" mode).
3. Look at the photo. **Every square along the path lit evenly**: no frames are skipped.
   **Gaps or a few squares much brighter than the rest**: frames are being dropped. Go back down
   a few Hz.

## Diagnostics

`monitor-oc doctor` (and the app's **Diagnostics** page) checks your system and explains anything
that would stop an overclock from working. Each line is marked `ok`, `info`, `WARN` or `FAIL`:

| Check | What it tells you |
|---|---|
| System, Hardware | The OS and the device, e.g. a ROG Ally X. |
| GPU driver | `amdgpu`/`i915`/`xe` are fine; NVIDIA's proprietary driver ignores EDID overrides. |
| Boot setup | How kernel arguments are made permanent for external monitors on this system. |
| Kernel lockdown | Whether Secure Boot blocks live testing of external monitors. |
| Gamescope, Built-in screen, Stock Game Mode range | Whether there is a Game Mode, which panel was found, and Gamescope's stock range for it. |
| Built-in overclock, Gamescope profile | The confirmed rate, any trial and its state, and what Gamescope loaded (see [Verify](#verify-it-works)). |
| Kernel arguments, connectors | For external monitors: whether the installed overrides are active this boot. |

Please include this output when you report a problem.

## Recovery

**Built-in screen, black screen in Game Mode:** wait 10 seconds; the test goes back to stock on its
own. If it doesn't, hold the power button until the device turns off and start it again: the trial
was not kept, so Gamescope starts with the stock range.

If a rate you already confirmed misbehaves later, lower the Refresh Rate slider, or go back to
stock with `monitor-oc panel reset` (or the Built-in screen page) and restart Game Mode. Deleting
`~/.config/gamescope/scripts/zz-monitor-oc.lua` and restarting Game Mode has the same effect. If
Game Mode itself comes up dark with a confirmed rate, connect an external display (or log in over
SSH) to do that.

**External monitor shows no picture:**

- `test` reverts by itself after 10 seconds; just wait.
- Handheld docked: unplug the monitor. The built-in screen is not affected. Then run
  `sudo ~/.local/bin/monitor-oc remove`.
- Desktop PC: plug the monitor into another port or input, or switch to a text console
  (Ctrl+Alt+F3), log in and run `sudo ~/.local/bin/monitor-oc remove`.
- Last resort: in the GRUB menu, press `e` and delete the `drm.edid_firmware=...` argument for one
  boot, then run `remove`.

## Supported systems

| System | Built-in screen in Game Mode | External monitors: made permanent with | Survives OS updates |
|---|---|---|---|
| SteamOS 3 (Steam Deck, ROG Ally / Ally X, Legion Go, ...) | Gamescope profile | `/etc/default/grub` + `update-grub` | Yes; `monitor-oc.service` re-applies at boot if an update dropped it |
| Bazzite and other Universal Blue / Fedora Atomic images | Gamescope profile (Game Mode images) | `rpm-ostree kargs` | Yes |
| Fedora Workstation, RHEL family | Only in a Gamescope session | `grubby` | Yes |
| Other GRUB distros (Arch, Ubuntu, ...) | Only in a Gamescope session | `/etc/default/grub` + `update-grub` / `grub-mkconfig` | Yes |
| systemd-boot and others | Only in a Gamescope session | Prints the kernel arguments for you to add | n/a |

Needs an x86_64 CPU. The app needs X11 or Wayland with OpenGL, which every desktop and Game Mode
provide.
