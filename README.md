# FadeWire

[![CI](https://github.com/Aleblazer/fadewire/actions/workflows/ci.yml/badge.svg)](https://github.com/Aleblazer/fadewire/actions/workflows/ci.yml)

**FadeWire** is a hardware & virtual fader mixer for **PipeWire** — ride the
volume of your games, chat, music, and output devices with **real slide
faders**, on-screen faders, hotkeys, or compositor keybinds. By
[Split Logic](https://www.splitlogic.xyz).

> ⚠️ **Pre-alpha.** The project scaffold, ported signal math, and Arch/CachyOS
> packaging exist; the PipeWire and hidraw backends are the current milestone.
> Nothing is usable yet — star/watch if you want to follow along.

## Why

- **Per-app mixing with physical throw** — point a fader at an output device
  (sink), a single app, a category of apps, or "Everything Else", and ride
  levels without alt-tabbing. Game vs. chat, finally on hardware.
- **Daemon-first** — a small `fadewired` service does all the work, so faders
  keep working under **gamescope Game Mode**, on tiling compositors, or with
  no GUI running at all.
- **Scriptable** — a `fadewire` CLI over D-Bus. Hyprland/Niri users bind
  `fadewire nudge Chat -5` instead of fighting global-hotkey portals.
- **No hardware required** — virtual faders work standalone, and mix freely
  with physical ones.

FadeWire is the Linux sibling of
[zmk-volume-fader](https://github.com/Aleblazer/zmk-volume-fader) (Windows).
The signal-path math — calibration tapers, the snap-band noise filter, the
mute detent, output hysteresis — is ported from there, where it was tuned
against real slide-pot hardware. It pairs perfectly with
[ZMK](https://zmk.dev) keyboards running the
[zmk-hid-io fader fork](https://github.com/Aleblazer/zmk-hid-io/tree/absolute-faders)
and with Split Logic's upcoming **SailStone** fader unit — but any device
exposing the vendor-page fader report works.

## Architecture

```
fadewired ── systemd user service (default.target — no compositor needed)
  ├─ hidraw reader        vendor HID page 0xFF00, up to 8 × 16-bit axes
  ├─ PipeWire mixer       sinks (outputs) + stream nodes (per-app)
  ├─ evdev hotkeys        optional, pass-through, F13–F24-friendly
  └─ D-Bus service        xyz.splitlogic.FadeWire
fadewire  ── CLI over D-Bus (scripts, compositor binds, bar modules)
GUI       ── (later) setup, calibration, virtual faders; optional SNI tray
```

See [docs/architecture.md](docs/architecture.md) for the full design and
[docs/config.example.toml](docs/config.example.toml) for the config format.

## Install

### CachyOS / Arch

An AUR package (`fadewire-git`) ships when the daemon does; the PKGBUILD
already lives in [packaging/arch/](packaging/arch/). It installs the udev
rule and a systemd user unit, so setup is:

```sh
paru -S fadewire-git          # or: makepkg -si in packaging/arch
systemctl --user enable --now fadewire.service
```

### Bazzite (planned)

Flathub package + a one-line udev rule install — see the roadmap.

### From source

```sh
cargo build --release --workspace
sudo install -Dm644 packaging/udev/70-fadewire.rules /etc/udev/rules.d/70-fadewire.rules
sudo udevadm control --reload-rules && sudo udevadm trigger
```

## Roadmap

- [x] Workspace scaffold, CI, MIT
- [x] Core signal math ported from zmk-volume-fader, with tests
      (tapers, snap-band EMA, mute detent, hysteresis, cubic volume mapping)
- [x] Config model (`~/.config/fadewire/config.toml`)
- [x] Arch packaging: PKGBUILD + udev rules + systemd user unit
- [x] PipeWire mixer backend (initial: sink + per-app stream volumes via
      pipewire-pulse, categories + Everything Else, 1 Hz poll; native
      pipewire-rs events later) — try `fadewired list` / `fadewired set`
- [x] hidraw reader (USB + BLE, usage-page matching, auto-reconnect) feeding
      the ported signal path — physical faders drive volumes end-to-end;
      `fadewired watch` dumps raw axes for hardware debugging
- [x] D-Bus service (`xyz.splitlogic.FadeWire`) + real `fadewire` CLI:
      `status` / `list` / `set` / `nudge` / `mute` — virtual faders work,
      levels + mute persist in `~/.local/state/fadewire/`, and compositor
      keybinds can drive volumes (`bind = , F13, exec, fadewire nudge Chat 5`)
- [ ] evdev hotkeys (pass-through, F13–F24)
- [ ] AUR publication; CachyOS repo request once stable
- [ ] GUI (setup wizard, calibration, live faders) + SNI tray
- [ ] Flathub package for Bazzite (Background portal for Game Mode)

## License

[MIT](LICENSE)
