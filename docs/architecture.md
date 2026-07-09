# FadeWire architecture

Decisions from the initial whiteboard (July 2026), recorded so the code can be
judged against them.

## Shape: daemon + CLI + GUI, in that order

- **`fadewired`** owns everything stateful: HID input, the signal path, the
  PipeWire session, config. It is a **systemd user service on
  `default.target`** — deliberately *not* `graphical-session.target` — because
  it needs no compositor. That makes faders work under gamescope Game Mode
  (Bazzite's headline mode has no tray and no floating windows) and on
  compositors without XDG autostart (Hyprland, Niri).
- **`fadewire`** (CLI) talks to the daemon over **D-Bus**
  (`xyz.splitlogic.FadeWire`). This is the integration surface for tiling-WM
  users (compositor keybinds), scripts, and bar modules.
- **GUI later**, as a separate process over the same D-Bus API. The tray
  (StatusNotifierItem) is optional sugar: stock GNOME, Hyprland, and Niri may
  have no tray host, so nothing may depend on it.

## Audio: PipeWire

- Outputs = **sink nodes**; per-app volume = **stream nodes**
  (`Stream/Output/Audio`), matched on `application.process.binary` /
  `application.name` / Flatpak app id — richer identity than Windows process
  names.
- Hotplug and failover come from registry events (no polling).
- **Volume scale**: native `channelVolumes` are linear amplitude; user-facing
  percentages are cubic. `fadewire_core::percent_to_channel_volume` holds the
  mapping. Via the PulseAudio compat API (`pipewire-pulse`), set
  `PA_VOLUME_NORM * p / 100` and the server handles it — that's the
  acceptable MVP shortcut.
- Category semantics carried over from the Windows app: a category moves its
  member apps together; **Unassigned** ("Everything Else") is every live
  stream not in a category *and not directly targeted by another fader*.

## Input: hidraw + evdev

- Fader report: vendor HID page `0xFF00`, report id 2, up to **eight signed
  16-bit LE axes** of raw wiper mV (~0..3300). Same report over USB and BLE
  (BlueZ HOG also lands on hidraw), so device matching stays
  transport-agnostic: prefer VID `0x1d50` / PID `0x615e`, fall back to any
  device exposing the vendor usage.
- Access is granted by a **udev uaccess rule** (packaged); no input-group
  membership.
- **Hotkeys are evdev-first**: reading key events is inherently pass-through
  (Discord-style — never swallows), works on every compositor including under
  gamescope, and can be scoped to the ZMK keyboard's event node + F13–F24.
  The GlobalShortcuts portal is *consumed-key* semantics and is missing on
  COSMIC and Niri — offered later only as an opt-in fallback.

## Signal path (ported from zmk-volume-fader, kept equivalent)

```
raw mV ─ snap-band EMA ─ taper curve ─ mute detent ─ cap ─ hysteresis ─ %
         (±60 snaps,      (Bourns-      (raw-gated,   (max   (0.9%)
          0.85 keep)       inverse)      latched+15)   %)
```

Why each stage exists is documented on `fadewire_core::filter::AxisFilter`;
the tuning history lives in the Windows repo's commit log.

## Packaging

- **CachyOS/Arch**: `packaging/arch/PKGBUILD` installs binaries, the udev
  rule (`/usr/lib/udev/rules.d`), and the user unit
  (`/usr/lib/systemd/user`). AUR first; request adoption into CachyOS's
  optimized repos (github.com/CachyOS/CachyOS-PKGBUILDS) once stable.
- **Bazzite**: Flathub Flatpak (`xyz.splitlogic.FadeWire`) with
  `--socket=pulseaudio` + `--device=all` (USB portal when it matures), and
  the Background portal for Game Mode. The udev rule is unavoidably
  host-side: first-run UX detects "device present but inaccessible" and
  offers the one-liner. rpm-ostree layering is explicitly not a target
  (discouraged by Bazzite's own docs).

## Config

TOML at `$XDG_CONFIG_HOME/fadewire/config.toml`; model in
`fadewire_core::config`, example in `docs/config.example.toml`. The daemon
owns the file; the GUI/CLI edit through the daemon (single writer).
