//! Chooses between Apple's command-line tools and Russet's native
//! replacements.
//!
//! macOS calls Apple's tools by default, and Linux uses the native
//! replacements. `RUSSET_NATIVE` forces native replacements on macOS for
//! side-by-side testing: set it to `all` or to a comma-separated list of tool
//! names, such as `ditto`. It's read only from the process environment, never
//! from preferences or recipes, so a recipe can't change which verifier runs.

use std::sync::{Mutex, OnceLock};

/// The environment variable that forces native replacements on macOS.
pub const VARIABLE: &str = "RUSSET_NATIVE";

/// An Apple tool Russet can replace.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tool {
    Hdiutil,
    Ditto,
    Aa,
    Xar,
    Mkbom,
    Codesign,
    Pkgutil,
    Pkgbuild,
    Installer,
    Icons,
}

impl Tool {
    pub const ALL: [Tool; 10] = [
        Tool::Hdiutil,
        Tool::Ditto,
        Tool::Aa,
        Tool::Xar,
        Tool::Mkbom,
        Tool::Codesign,
        Tool::Pkgutil,
        Tool::Pkgbuild,
        Tool::Installer,
        Tool::Icons,
    ];

    /// The name used in `RUSSET_NATIVE`.
    pub fn name(self) -> &'static str {
        match self {
            Tool::Hdiutil => "hdiutil",
            Tool::Ditto => "ditto",
            Tool::Aa => "aa",
            Tool::Xar => "xar",
            Tool::Mkbom => "mkbom",
            Tool::Codesign => "codesign",
            Tool::Pkgutil => "pkgutil",
            Tool::Pkgbuild => "pkgbuild",
            Tool::Installer => "installer",
            Tool::Icons => "icons",
        }
    }

    /// Whether Russet has a native replacement yet.
    pub fn implemented(self) -> bool {
        matches!(
            self,
            Tool::Ditto | Tool::Hdiutil | Tool::Xar | Tool::Pkgutil
        )
    }
}

/// Which implementation runs for a tool.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    /// Call Apple's tool.
    Apple,
    /// Use Russet's native replacement.
    Native,
    /// Neither is available on this platform.
    Unsupported,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Platform {
    Mac,
    Linux,
    Other,
}

const OS: Platform = if cfg!(target_os = "macos") {
    Platform::Mac
} else if cfg!(target_os = "linux") {
    Platform::Linux
} else {
    Platform::Other
};

fn parse(value: &str) -> Result<Vec<Tool>, String> {
    let mut tools = Vec::new();
    for item in value.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        if item == "all" {
            tools.extend(Tool::ALL.into_iter().filter(|t| t.implemented()));
            continue;
        }
        let tool = Tool::ALL
            .into_iter()
            .find(|t| t.name() == item)
            .ok_or_else(|| {
                format!(
                    "{VARIABLE} contains unknown tool '{item}'. Use 'all' or a comma-separated list of: {}",
                    Tool::ALL.map(Tool::name).join(", ")
                )
            })?;
        if !tool.implemented() {
            return Err(format!(
                "{VARIABLE} names '{item}', which has no native replacement yet"
            ));
        }
        tools.push(tool);
    }
    Ok(tools)
}

fn forced() -> &'static Result<Vec<Tool>, String> {
    static FORCED: OnceLock<Result<Vec<Tool>, String>> = OnceLock::new();
    FORCED.get_or_init(|| match std::env::var(VARIABLE) {
        Ok(value) => parse(&value),
        Err(std::env::VarError::NotPresent) => Ok(Vec::new()),
        Err(std::env::VarError::NotUnicode(_)) => Err(format!("{VARIABLE} isn't valid UTF-8")),
    })
}

/// Checks `RUSSET_NATIVE`. The CLI calls this before running anything, so a
/// mistyped value stops the run instead of silently using Apple's tools.
pub fn validate() -> Result<(), String> {
    forced().as_ref().map(|_| ()).map_err(Clone::clone)
}

fn decide(os: Platform, forced: bool, tool: Tool) -> Backend {
    match os {
        Platform::Mac if forced && tool.implemented() => Backend::Native,
        Platform::Mac => Backend::Apple,
        Platform::Linux if tool.implemented() => Backend::Native,
        _ => Backend::Unsupported,
    }
}

/// Selects the implementation for `tool` on this platform. When native code
/// is forced on macOS, the first use of each tool is noted at verbosity 1.
pub fn select(tool: Tool) -> Backend {
    let forced = matches!(forced(), Ok(tools) if tools.contains(&tool));
    let backend = decide(OS, forced, tool);
    if forced && backend == Backend::Native && OS == Platform::Mac {
        static ANNOUNCED: Mutex<Vec<Tool>> = Mutex::new(Vec::new());
        let mut announced = ANNOUNCED.lock().unwrap_or_else(|e| e.into_inner());
        if !announced.contains(&tool) {
            announced.push(tool);
            crate::processor_output(
                1,
                format!("Using Russet's native {} ({VARIABLE})", tool.name()),
            );
        }
    }
    backend
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_names_and_rejects_unknown_or_unimplemented() {
        assert_eq!(parse("").unwrap(), vec![]);
        assert_eq!(parse(" ditto ,").unwrap(), vec![Tool::Ditto]);
        assert_eq!(
            parse("all").unwrap(),
            vec![Tool::Hdiutil, Tool::Ditto, Tool::Xar, Tool::Pkgutil]
        );
        assert!(parse("dito").unwrap_err().contains("unknown tool 'dito'"));
        assert!(parse("codesign")
            .unwrap_err()
            .contains("no native replacement yet"));
    }

    #[test]
    fn platform_defaults() {
        assert_eq!(decide(Platform::Mac, false, Tool::Ditto), Backend::Apple);
        assert_eq!(decide(Platform::Mac, true, Tool::Ditto), Backend::Native);
        assert_eq!(decide(Platform::Mac, true, Tool::Codesign), Backend::Apple);
        assert_eq!(decide(Platform::Linux, false, Tool::Ditto), Backend::Native);
        assert_eq!(
            decide(Platform::Linux, false, Tool::Codesign),
            Backend::Unsupported
        );
        assert_eq!(
            decide(Platform::Other, true, Tool::Ditto),
            Backend::Unsupported
        );
    }
}
