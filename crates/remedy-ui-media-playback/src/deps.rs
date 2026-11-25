//! What media needs from the system and is not there: one typed description
//! ([`MissingDependency`]) that every path returns through [`MediaError::Missing`](crate::MediaError::Missing), so every app
//! tells people the same plain thing: which component is missing, what it is needed for, whether
//! it is not installed or installed but unable to run here, and the package to install on the
//! common distributions. The catalog below names every system component RemedyUI's media loads.

use std::fmt;

/// A system component media needs that is not installed, or is installed but cannot run here.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct MissingDependency {
    /// The component, as people know it ("GStreamer's FFmpeg plugins (gst-libav)").
    pub name: String,
    /// What it is needed for ("decoding H.264 video and shared screens").
    pub purpose: String,
    /// Whether it is absent or present but unusable.
    pub state: DependencyState,
    /// How to install it: (distribution family, package or packages).
    pub install: Vec<(String, String)>,
    /// Components any one of which would do, each with its own state ([`any_of`]): an H.264
    /// decoder is the graphics card's or the processor's.
    pub alternatives: Vec<MissingDependency>,
}

/// Absent, or present but unable to run.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DependencyState {
    /// Not installed (or not found where the system keeps it).
    NotInstalled,
    /// Installed, but it cannot run on this machine; the reason says why (no usable device).
    Unusable(String),
}

impl MissingDependency {
    /// `name`, needed for `purpose`, not installed; installed with `install`.
    pub fn not_installed(
        name: impl Into<String>,
        purpose: impl Into<String>,
        install: &[(&str, &str)],
    ) -> Self {
        MissingDependency {
            name: name.into(),
            purpose: purpose.into(),
            state: DependencyState::NotInstalled,
            install: install.iter().map(|(d, p)| ((*d).to_owned(), (*p).to_owned())).collect(),
            alternatives: Vec::new(),
        }
    }

    /// The same component, installed but unable to run here because of `why`.
    pub fn unusable(mut self, why: impl Into<String>) -> Self {
        self.state = DependencyState::Unusable(why.into());
        self
    }

    /// The same component needed for `purpose` instead.
    pub fn for_purpose(mut self, purpose: impl Into<String>) -> Self {
        self.purpose = purpose.into();
        self
    }
}

impl fmt::Display for MissingDependency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.alternatives.is_empty() {
            write!(f, "{} needs {}, and none here can be used:", capital(&self.purpose), self.name)?;
            for a in &self.alternatives {
                match &a.state {
                    DependencyState::NotInstalled => write!(f, " {} is not installed", a.name)?,
                    DependencyState::Unusable(why) => {
                        write!(f, " {} cannot run on this computer ({})", a.name, why.trim_end_matches('.'))?
                    }
                }
                f.write_str(";")?;
            }
            if !self.install.is_empty() {
                let how: Vec<String> = self.install.iter().map(|(d, p)| format!("{d}: {p}")).collect();
                write!(f, " the simplest is to install {} ({}).", self.install_name(), how.join("; "))?;
            }
            return Ok(());
        }
        match &self.state {
            DependencyState::NotInstalled => {
                write!(f, "{} needs {}, which is not installed.", capital(&self.purpose), self.name)?
            }
            DependencyState::Unusable(why) => {
                write!(
                    f,
                    "{} needs {}, which is installed but cannot run on this computer: {}.",
                    capital(&self.purpose),
                    self.name,
                    why.trim_end_matches('.')
                )?;
            }
        }
        if !self.install.is_empty() {
            let how: Vec<String> = self.install.iter().map(|(d, p)| format!("{d}: {p}")).collect();
            let verb = if matches!(self.state, DependencyState::NotInstalled) { "Install" } else { "Check" };
            write!(f, " {verb} it with your package manager ({}).", how.join("; "))?;
        }
        Ok(())
    }
}

impl MissingDependency {
    fn install_name(&self) -> &str {
        self.alternatives
            .iter()
            .find(|a| a.install == self.install)
            .map_or(self.name.as_str(), |a| a.name.as_str())
    }
}

/// `name`, needed for `purpose`, which any one of `alternatives` would provide, none of which
/// can be used here (each says why); the install hint is the last alternative's, which should
/// be the one that works everywhere (the processor's decoder or encoder).
pub fn any_of(name: &str, purpose: &str, alternatives: Vec<MissingDependency>) -> MissingDependency {
    let install = alternatives.last().map(|a| a.install.clone()).unwrap_or_default();
    let state = if alternatives.iter().all(|a| a.state == DependencyState::NotInstalled) {
        DependencyState::NotInstalled
    } else {
        DependencyState::Unusable("none of the alternatives runs".into())
    };
    MissingDependency { name: name.to_owned(), purpose: purpose.to_owned(), state, install, alternatives }
}

/// A GStreamer support library (`lib`: "libgstvideo-1.0"), part of the base plugins.
pub fn gst_library(lib: &str, purpose: &str) -> MissingDependency {
    MissingDependency::not_installed(
        format!("GStreamer's {lib} library"),
        purpose,
        &[
            ("Debian/Ubuntu", "gstreamer1.0-plugins-base"),
            ("Fedora", "gstreamer1-plugins-base"),
            ("Arch", "gst-plugins-base"),
        ],
    )
}

/// The sound server (PulseAudio, or PipeWire's PulseAudio server) is not running, for `why`.
pub fn sound_server(why: &str) -> MissingDependency {
    MissingDependency::not_installed(
        "a running sound server (PulseAudio, or PipeWire with pipewire-pulse)",
        "sharing what the computer plays",
        &[("Debian/Ubuntu", "pipewire-pulse"), ("Fedora", "pipewire-pulseaudio"), ("Arch", "pipewire-pulse")],
    )
    .unusable(why)
}

fn capital(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(first) => first.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

/// GStreamer itself (the core library and its app elements), loaded at run time on Linux.
pub fn gstreamer() -> MissingDependency {
    MissingDependency::not_installed(
        "GStreamer 1.x",
        "playing and recording sound and video",
        &[
            ("Debian/Ubuntu", "gstreamer1.0-plugins-base"),
            ("Fedora", "gstreamer1-plugins-base"),
            ("Arch", "gst-plugins-base"),
        ],
    )
}

/// The GStreamer plugin set that holds element `factory` ("avdec_h264", "h264parse",
/// "pipewiresrc"), needed for `purpose`.
pub fn element(factory: &str, purpose: &str) -> MissingDependency {
    let (name, install): (String, &[(&str, &str)]) = match plugin_set(factory) {
        PluginSet::Base => (
            "GStreamer's base plugins".into(),
            &[
                ("Debian/Ubuntu", "gstreamer1.0-plugins-base"),
                ("Fedora", "gstreamer1-plugins-base"),
                ("Arch", "gst-plugins-base"),
            ],
        ),
        PluginSet::Good => (
            "GStreamer's good plugins".into(),
            &[
                ("Debian/Ubuntu", "gstreamer1.0-plugins-good"),
                ("Fedora", "gstreamer1-plugins-good"),
                ("Arch", "gst-plugins-good"),
            ],
        ),
        PluginSet::Bad => (
            "GStreamer's bad plugins".into(),
            &[
                ("Debian/Ubuntu", "gstreamer1.0-plugins-bad"),
                ("Fedora", "gstreamer1-plugins-bad-free"),
                ("Arch", "gst-plugins-bad"),
            ],
        ),
        PluginSet::Ugly => (
            "GStreamer's ugly plugins".into(),
            &[
                ("Debian/Ubuntu", "gstreamer1.0-plugins-ugly"),
                ("Fedora", "gstreamer1-plugins-ugly"),
                ("Arch", "gst-plugins-ugly"),
            ],
        ),
        PluginSet::Libav => (
            "GStreamer's FFmpeg plugins (gst-libav)".into(),
            &[
                ("Debian/Ubuntu", "gstreamer1.0-libav"),
                ("Fedora", "gstreamer1-plugin-libav"),
                ("Arch", "gst-libav"),
            ],
        ),
        PluginSet::OpenH264 => (
            "GStreamer's OpenH264 plugin".into(),
            &[
                ("Debian/Ubuntu", "gstreamer1.0-plugins-bad"),
                ("Fedora", "gstreamer1-plugin-openh264"),
                ("Arch", "gst-plugins-bad and openh264"),
            ],
        ),
        PluginSet::Vaapi => (
            "GStreamer's VA-API plugin".into(),
            &[
                ("Debian/Ubuntu", "gstreamer1.0-vaapi"),
                ("Fedora", "gstreamer1-vaapi"),
                ("Arch", "gstreamer-vaapi"),
            ],
        ),
        PluginSet::PipeWire => (
            "GStreamer's PipeWire plugin".into(),
            &[
                ("Debian/Ubuntu", "gstreamer1.0-pipewire"),
                ("Fedora", "pipewire-gstreamer"),
                ("Arch", "gst-plugin-pipewire"),
            ],
        ),
    };
    MissingDependency::not_installed(format!("{name}, for the {factory} element"), purpose, install)
}

/// The plugin sets of GStreamer's distribution.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PluginSet {
    Base,
    Good,
    Bad,
    Ugly,
    Libav,
    OpenH264,
    Vaapi,
    PipeWire,
}

fn plugin_set(factory: &str) -> PluginSet {
    match factory {
        f if f.starts_with("avdec_") || f.starts_with("avenc_") || f.starts_with("avdemux_") => {
            PluginSet::Libav
        }
        f if f.starts_with("openh264") => PluginSet::OpenH264,
        f if f.starts_with("vaapi") => PluginSet::Vaapi,
        f if f.starts_with("pipewire") => PluginSet::PipeWire,
        f if f.starts_with("x264") || f.starts_with("mpeg2dec") || f == "a52dec" => PluginSet::Ugly,
        f if f.starts_with("v4l2")
            || f.starts_with("autoaudio")
            || f.starts_with("autovideo")
            || f.starts_with("pulse")
            || f.starts_with("ximagesrc")
            || f == "matroskademux"
            || f == "qtdemux"
            || f == "vp8dec"
            || f == "vp8enc"
            || f == "souphttpsrc" =>
        {
            PluginSet::Good
        }
        f if f.ends_with("parse")
            || f.starts_with("va")
            || f.starts_with("nv")
            || f.starts_with("vulkan") =>
        {
            PluginSet::Bad
        }
        _ => PluginSet::Base,
    }
}

/// The VA-API video driver for this machine's graphics (Intel's or Mesa's), for `why`.
pub fn va_driver(why: &str) -> MissingDependency {
    MissingDependency::not_installed(
        "a VA-API video driver",
        "using the graphics card for video",
        &[
            ("Debian/Ubuntu", "intel-media-va-driver or mesa-va-drivers"),
            ("Fedora", "intel-media-driver or mesa-va-drivers"),
            ("Arch", "intel-media-driver or libva-mesa-driver"),
        ],
    )
    .unusable(why)
}

/// NVIDIA's driver with its video engines (NVDEC, NVENC) and CUDA, for `why`.
pub fn nvidia_driver(why: &str) -> MissingDependency {
    MissingDependency::not_installed(
        "an NVIDIA graphics card with NVIDIA's driver",
        "using the NVIDIA graphics card for video",
        &[
            ("Debian/Ubuntu", "nvidia-driver"),
            ("Fedora", "akmod-nvidia (RPM Fusion)"),
            ("Arch", "nvidia-utils"),
        ],
    )
    .unusable(why)
}

/// The desktop's screen-sharing portal over PipeWire (Wayland), for `why`.
pub fn pipewire_portal(why: &str) -> MissingDependency {
    MissingDependency::not_installed(
        "the desktop's screen-sharing portal (xdg-desktop-portal with PipeWire)",
        "sharing the screen on Wayland",
        &[
            ("Debian/Ubuntu", "xdg-desktop-portal, xdg-desktop-portal-gnome or -kde or -wlr, pipewire"),
            ("Fedora", "xdg-desktop-portal-gnome or -kde, pipewire"),
            ("Arch", "xdg-desktop-portal-gnome or -kde or -wlr, pipewire"),
        ],
    )
    .unusable(why)
}

/// The PulseAudio client library (also served by PipeWire), for `why`.
pub fn libpulse(why: &str) -> MissingDependency {
    MissingDependency::not_installed(
        "the PulseAudio client library (libpulse)",
        "sharing what the computer plays",
        &[("Debian/Ubuntu", "libpulse0"), ("Fedora", "pulseaudio-libs"), ("Arch", "libpulse")],
    )
    .unusable(why)
}

/// An X server extension (`ext`: "SHM", "Composite", "XFixes", "RandR") the X server does not
/// offer, needed for `purpose`.
pub fn x_extension(ext: &str, purpose: &str) -> MissingDependency {
    MissingDependency {
        name: format!("the X server's {ext} extension"),
        purpose: purpose.to_owned(),
        state: DependencyState::Unusable(format!("this X server does not offer {ext}")),
        install: Vec::new(),
        alternatives: Vec::new(),
    }
}

/// Windows' Media Foundation, which "N" editions leave out.
pub fn media_foundation(why: &str) -> MissingDependency {
    MissingDependency::not_installed(
        "Windows' Media Foundation",
        "playing and recording sound and video",
        &[("Windows N editions", "the Media Feature Pack (Settings, Apps, Optional features)")],
    )
    .unusable(why)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_plugin_names_its_set_purpose_and_packages() {
        let m = element("avdec_h264", "showing shared screens");
        let s = m.to_string();
        assert!(s.starts_with("Showing shared screens needs GStreamer's FFmpeg plugins (gst-libav), for the avdec_h264 element, which is not installed."), "{s}");
        assert!(
            s.contains("Debian/Ubuntu: gstreamer1.0-libav")
                && s.contains("Fedora: gstreamer1-plugin-libav")
                && s.contains("Arch: gst-libav"),
            "{s}"
        );
        assert_eq!(plugin_set("h264parse"), PluginSet::Bad);
        assert_eq!(plugin_set("x264enc"), PluginSet::Ugly);
        assert_eq!(plugin_set("vaapih264dec"), PluginSet::Vaapi);
        assert_eq!(plugin_set("vah264dec"), PluginSet::Bad);
        assert_eq!(plugin_set("appsrc"), PluginSet::Base);
        assert_eq!(plugin_set("pipewiresrc"), PluginSet::PipeWire);
        assert_eq!(plugin_set("autoaudiosrc"), PluginSet::Good);
    }

    #[test]
    fn installed_but_unusable_reads_differently_from_absent() {
        let m = nvidia_driver("nvh264dec is installed, but no usable NVIDIA GPU was found");
        let s = m.to_string();
        assert!(s.contains("which is installed but cannot run on this computer: nvh264dec is installed, but no usable NVIDIA GPU was found."), "{s}");
        assert!(s.contains("Check it with your package manager"), "{s}");
        let x = x_extension("Composite", "sharing one window").to_string();
        assert_eq!(
            x,
            "Sharing one window needs the X server's Composite extension, which is installed but cannot run on this computer: this X server does not offer Composite."
        );
    }

    #[test]
    fn none_of_the_alternatives_lists_each_and_suggests_the_last() {
        let purpose = "showing shared screens";
        let all = any_of(
            "an H.264 decoder",
            purpose,
            vec![
                element("nvh264dec", purpose).unusable("no usable NVIDIA GPU"),
                element("avdec_h264", purpose),
            ],
        );
        assert!(matches!(all.state, DependencyState::Unusable(_)));
        let s = all.to_string();
        assert!(
            s.starts_with("Showing shared screens needs an H.264 decoder, and none here can be used:"),
            "{s}"
        );
        assert!(
            s.contains("for the nvh264dec element cannot run on this computer (no usable NVIDIA GPU);"),
            "{s}"
        );
        assert!(s.contains("for the avdec_h264 element is not installed;"), "{s}");
        assert!(s.contains("the simplest is to install GStreamer's FFmpeg plugins (gst-libav), for the avdec_h264 element (Debian/Ubuntu: gstreamer1.0-libav"), "{s}");
        let absent = any_of("an H.264 decoder", purpose, vec![element("avdec_h264", purpose)]);
        assert_eq!(absent.state, DependencyState::NotInstalled);
    }
}
