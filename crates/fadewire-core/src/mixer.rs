//! Platform-neutral model of the audio world (sinks + app streams) and the
//! resolution of a fader's target onto concrete nodes.
//!
//! Semantics carried over from the Windows sibling:
//! - A **category** fader moves every member app together.
//! - **Unassigned** ("Everything Else") drives every stream that is *not* in
//!   any category and *not* directly targeted by another fader, so it never
//!   fights a dedicated app fader.

use crate::config::{Config, Target};

/// An output device (PipeWire sink) as seen by the mixer backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SinkNode {
    pub index: u32,
    /// Node name (e.g. `alsa_output.usb-...`).
    pub name: String,
    /// Human-readable description (what pavucontrol shows).
    pub description: String,
    /// Channel count, needed to build a volume write.
    pub channels: u8,
    /// Current volume in percent (display only).
    pub volume_pct: u32,
}

/// One application playback stream (PipeWire stream node / pulse sink-input).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamNode {
    pub index: u32,
    /// `application.name` (falling back to the stream's media name).
    pub app_name: String,
    /// `application.process.binary` — the app identity faders match on.
    pub binary: String,
    pub channels: u8,
    /// Current volume in percent (display only).
    pub volume_pct: u32,
}

/// A snapshot of everything a fader could drive.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct World {
    pub sinks: Vec<SinkNode>,
    pub streams: Vec<StreamNode>,
}

/// The concrete nodes a fader currently drives.
#[derive(Debug, Default)]
pub struct Resolved<'w> {
    pub sinks: Vec<&'w SinkNode>,
    pub streams: Vec<&'w StreamNode>,
}

fn contains_ci(haystack: &str, needle: &str) -> bool {
    !needle.is_empty() && haystack.to_lowercase().contains(&needle.to_lowercase())
}

fn eq_ci(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// Resolve the targets of `cfg.fader[fader_index]` against a world snapshot.
///
/// The whole config is needed (not just the one fader) because Unassigned
/// depends on every category and on the other faders' direct app targets.
pub fn resolve_targets<'w>(cfg: &Config, fader_index: usize, world: &'w World) -> Resolved<'w> {
    let mut out = Resolved::default();
    let Some(fader) = cfg.fader.get(fader_index) else {
        return out;
    };
    match &fader.target {
        Target::Sink { name_match } => {
            out.sinks = world
                .sinks
                .iter()
                .filter(|s| contains_ci(&s.description, name_match) || contains_ci(&s.name, name_match))
                .collect();
        }
        Target::App { binary } => {
            out.streams = world
                .streams
                .iter()
                .filter(|s| eq_ci(&s.binary, binary))
                .collect();
        }
        Target::Category { name } => {
            let members: Vec<&str> = cfg
                .category
                .iter()
                .find(|c| eq_ci(&c.name, name))
                .map(|c| c.members.iter().map(String::as_str).collect())
                .unwrap_or_default();
            out.streams = world
                .streams
                .iter()
                .filter(|s| members.iter().any(|m| eq_ci(&s.binary, m)))
                .collect();
        }
        Target::Unassigned => {
            out.streams = world
                .streams
                .iter()
                .filter(|s| {
                    let in_category = cfg
                        .category
                        .iter()
                        .any(|c| c.members.iter().any(|m| eq_ci(&s.binary, m)));
                    let directly_targeted = cfg.fader.iter().any(|f| {
                        matches!(&f.target, Target::App { binary } if eq_ci(&s.binary, binary))
                    });
                    !in_category && !directly_targeted
                })
                .collect();
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{CategoryDef, FaderConfig};

    fn sink(index: u32, name: &str, description: &str) -> SinkNode {
        SinkNode {
            index,
            name: name.into(),
            description: description.into(),
            channels: 2,
            volume_pct: 100,
        }
    }

    fn stream(index: u32, binary: &str) -> StreamNode {
        StreamNode {
            index,
            app_name: binary.into(),
            binary: binary.into(),
            channels: 2,
            volume_pct: 100,
        }
    }

    fn world() -> World {
        World {
            sinks: vec![
                sink(1, "alsa_output.usb-audeze", "Audeze Maxwell Game"),
                sink(2, "alsa_output.pci-hdmi", "HDMI Audio"),
            ],
            streams: vec![
                stream(10, "discord"),
                stream(11, "spotify"),
                stream(12, "factorio"),
                stream(13, "firefox"),
            ],
        }
    }

    fn cfg_with(faders: Vec<FaderConfig>, categories: Vec<CategoryDef>) -> Config {
        Config {
            fader: faders,
            category: categories,
        }
    }

    fn fader(target: Target) -> FaderConfig {
        FaderConfig {
            target,
            ..FaderConfig::default()
        }
    }

    #[test]
    fn sink_matches_case_insensitive_substring() {
        let cfg = cfg_with(
            vec![fader(Target::Sink {
                name_match: "maxwell game".into(),
            })],
            vec![],
        );
        let w = world();
        let r = resolve_targets(&cfg, 0, &w);
        assert_eq!(r.sinks.iter().map(|s| s.index).collect::<Vec<_>>(), [1]);
        assert!(r.streams.is_empty());
    }

    #[test]
    fn app_matches_the_binary_exactly() {
        let cfg = cfg_with(
            vec![fader(Target::App {
                binary: "Discord".into(),
            })],
            vec![],
        );
        let w = world();
        let r = resolve_targets(&cfg, 0, &w);
        assert_eq!(r.streams.iter().map(|s| s.index).collect::<Vec<_>>(), [10]);
    }

    #[test]
    fn category_moves_its_members_together() {
        let cfg = cfg_with(
            vec![fader(Target::Category {
                name: "Music".into(),
            })],
            vec![CategoryDef {
                name: "Music".into(),
                members: vec!["spotify".into(), "amberol".into()],
            }],
        );
        let w = world();
        let r = resolve_targets(&cfg, 0, &w);
        assert_eq!(r.streams.iter().map(|s| s.index).collect::<Vec<_>>(), [11]);
    }

    #[test]
    fn unassigned_excludes_categories_and_direct_targets() {
        let cfg = cfg_with(
            vec![
                fader(Target::App {
                    binary: "discord".into(),
                }),
                fader(Target::Unassigned),
            ],
            vec![CategoryDef {
                name: "Music".into(),
                members: vec!["spotify".into()],
            }],
        );
        let w = world();
        let r = resolve_targets(&cfg, 1, &w);
        // discord is directly targeted, spotify is in a category:
        // Everything Else gets factorio + firefox only.
        assert_eq!(
            r.streams.iter().map(|s| s.index).collect::<Vec<_>>(),
            [12, 13]
        );
    }

    #[test]
    fn missing_category_or_fader_resolves_to_nothing() {
        let cfg = cfg_with(
            vec![fader(Target::Category {
                name: "Nope".into(),
            })],
            vec![],
        );
        let w = world();
        assert!(resolve_targets(&cfg, 0, &w).streams.is_empty());
        assert!(resolve_targets(&cfg, 99, &w).streams.is_empty());
    }
}
