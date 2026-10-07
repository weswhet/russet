//! Native replacement for the read-only `installer` queries Russet uses.
//! It doesn't install anything.
//!
//! [`restart_action`] matches `installer -query RestartAction -pkg`:
//!
//! - A product archive (one with a `Distribution`) reports the most severe
//!   `onConclusion` of its `pkg-ref` elements. Its components' own
//!   `postinstall-action` values are ignored.
//! - A component package reports its `PackageInfo` `postinstall-action`.
//!
//! Severity runs `None` < `RecommendRestart` < `RequireLogout` <
//! `RequireRestart` < `RequireShutdown`. These rules were checked against
//! `installer` on macOS with packages built by `pkgbuild` and
//! `productbuild`; see `compatibility/apple-tools/installer.md`.
#![forbid(unsafe_code)]

/// What a package asks for when it finishes installing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum RestartAction {
    None,
    RecommendRestart,
    RequireLogout,
    RequireRestart,
    RequireShutdown,
}

impl RestartAction {
    /// The value `installer -query RestartAction` prints.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::RecommendRestart => "RecommendRestart",
            Self::RequireLogout => "RequireLogout",
            Self::RequireRestart => "RequireRestart",
            Self::RequireShutdown => "RequireShutdown",
        }
    }

    /// Parses a Distribution `onConclusion` value, ignoring case:
    /// `productbuild` itself writes `RequireLogOut`. Unknown values count as
    /// `None`.
    pub fn from_on_conclusion(value: &str) -> Self {
        match value.to_ascii_lowercase().as_str() {
            "recommendrestart" => Self::RecommendRestart,
            "requirelogout" => Self::RequireLogout,
            "requirerestart" => Self::RequireRestart,
            "requireshutdown" => Self::RequireShutdown,
            _ => Self::None,
        }
    }

    /// Parses a PackageInfo `postinstall-action` value.
    pub fn from_postinstall_action(value: &str) -> Self {
        match value.to_ascii_lowercase().as_str() {
            "logout" => Self::RequireLogout,
            "restart" => Self::RequireRestart,
            "shutdown" => Self::RequireShutdown,
            _ => Self::None,
        }
    }

    /// Parses a bundle package's `IFPkgFlagRestartAction`. Current macOS
    /// `installer` no longer reads bundle packages, so this follows the
    /// key's documented values rather than a comparison.
    pub fn from_bundle_flag(value: &str) -> Self {
        match value {
            "RecommendedRestart" => Self::RecommendRestart,
            "RequiredLogout" => Self::RequireLogout,
            "RequiredRestart" => Self::RequireRestart,
            "Shutdown" => Self::RequireShutdown,
            _ => Self::None,
        }
    }
}

#[cfg(unix)]
pub use imp::*;

#[cfg(unix)]
mod imp {
    use super::RestartAction;
    use std::io;
    use std::path::Path;

    const MAX_XML: u64 = 16 << 20;

    fn invalid(message: String) -> io::Error {
        io::Error::new(io::ErrorKind::InvalidData, message)
    }

    /// The restart action of a flat package, as
    /// `installer -query RestartAction -pkg package` reports it.
    pub fn restart_action(package: &Path) -> io::Result<RestartAction> {
        let mut archive = russet_xar::Archive::open(package)?;
        if archive.entry("Distribution").is_some() {
            let xml = archive.read("Distribution", MAX_XML)?;
            let xml = String::from_utf8_lossy(&xml);
            let document = roxmltree::Document::parse(&xml)
                .map_err(|e| invalid(format!("Distribution isn't valid XML: {e}")))?;
            return Ok(document
                .descendants()
                .filter(|n| n.has_tag_name("pkg-ref"))
                .filter_map(|n| n.attribute("onConclusion"))
                .map(RestartAction::from_on_conclusion)
                .max()
                .unwrap_or(RestartAction::None));
        }
        let xml = archive.read("PackageInfo", MAX_XML)?;
        let xml = String::from_utf8_lossy(&xml);
        let document = roxmltree::Document::parse(&xml)
            .map_err(|e| invalid(format!("PackageInfo isn't valid XML: {e}")))?;
        Ok(document
            .descendants()
            .find(|n| n.has_tag_name("pkg-info"))
            .and_then(|n| n.attribute("postinstall-action"))
            .map(RestartAction::from_postinstall_action)
            .unwrap_or(RestartAction::None))
    }

    /// `installer -showChoiceChangesXML` evaluates Distribution scripts,
    /// which Russet doesn't run, so this always fails with an explanation.
    pub fn choice_changes(_package: &Path) -> io::Result<Vec<u8>> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Reading installer choices needs macOS, because Distribution files can choose packages with JavaScript",
        ))
    }
}

#[cfg(all(test, unix))]
mod tests;
