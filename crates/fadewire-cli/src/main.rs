//! FadeWire control CLI (pre-alpha skeleton).
//!
//! Once the daemon exposes its D-Bus service (`xyz.splitlogic.FadeWire`),
//! this binary becomes the scripting surface — the idiomatic integration for
//! Hyprland/Niri users is a compositor keybind that calls `fadewire nudge`.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--version") | Some("version") => {
            println!("fadewire {}", env!("CARGO_PKG_VERSION"));
        }
        _ => {
            println!(
                "fadewire {} — control CLI for fadewired (pre-alpha)",
                env!("CARGO_PKG_VERSION")
            );
            println!();
            println!("Planned commands (land with the daemon's D-Bus service):");
            println!("  fadewire status                  daemon + device state");
            println!("  fadewire list                    faders and their targets");
            println!("  fadewire set <fader> <pct>       set a virtual fader's level");
            println!("  fadewire nudge <fader> <±pct>    step a virtual fader (bind me in Hyprland/Niri!)");
            println!("  fadewire mute <fader>            toggle a virtual fader's mute");
        }
    }
}
