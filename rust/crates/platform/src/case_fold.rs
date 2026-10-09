//! Case-insensitive path matching for recipes written for macOS.
//!
//! macOS volumes are usually case-insensitive, so recipes can name
//! `CyberDuck.app` when the app is `Cyberduck.app`. Linux file systems are
//! case-sensitive. When a path pattern matches nothing on Linux, Russet
//! retries it here, matching each component regardless of case, the way the
//! path would resolve on macOS.

use std::fs;
use std::path::{Component, Path, PathBuf};

const MAX_MATCHES: usize = 10_000;

fn same(a: &str, b: &str) -> bool {
    a == b || a.to_lowercase() == b.to_lowercase()
}

fn is_pattern(component: &str) -> bool {
    component.contains(['*', '?', '['])
}

/// Matches `pattern` (a path, possibly with `*`, `?`, and `[...]` in its
/// components) regardless of case, sorted. Recursive `**` isn't supported.
pub fn glob(pattern: &str) -> Vec<PathBuf> {
    let path = Path::new(pattern);
    let mut candidates = vec![PathBuf::new()];
    for component in path.components() {
        let part = match component {
            Component::Normal(part) => match part.to_str() {
                Some(part) => part,
                None => return Vec::new(),
            },
            other => {
                for candidate in &mut candidates {
                    candidate.push(other.as_os_str());
                }
                continue;
            }
        };
        if part == "**" {
            return Vec::new();
        }
        let matcher = is_pattern(part)
            .then(|| glob::Pattern::new(part).ok())
            .flatten();
        let options = glob::MatchOptions {
            case_sensitive: false,
            require_literal_separator: true,
            require_literal_leading_dot: true,
        };
        let mut next = Vec::new();
        for candidate in candidates {
            let directory = if candidate.as_os_str().is_empty() {
                Path::new(".")
            } else {
                candidate.as_path()
            };
            let Ok(entries) = fs::read_dir(directory) else {
                continue;
            };
            let names: Vec<String> = entries
                .flatten()
                .filter_map(|e| e.file_name().into_string().ok())
                .collect();
            match &matcher {
                Some(matcher) => next.extend(
                    names
                        .iter()
                        .filter(|n| matcher.matches_with(n, options))
                        .map(|n| candidate.join(n)),
                ),
                // A name that matches exactly wins over ones that differ
                // only in case.
                None => match names.iter().find(|n| n.as_str() == part) {
                    Some(name) => next.push(candidate.join(name)),
                    None => next.extend(
                        names
                            .iter()
                            .filter(|n| same(n, part))
                            .map(|n| candidate.join(n)),
                    ),
                },
            }
            if next.len() > MAX_MATCHES {
                return Vec::new();
            }
        }
        candidates = next;
        if candidates.is_empty() {
            break;
        }
    }
    candidates.sort();
    candidates
}

/// Resolves a literal path the way a case-insensitive macOS volume would:
/// each component that doesn't exist as written matches the one entry whose
/// name differs only in case. Returns `None` when a component is missing or
/// more than one entry matches it, which a macOS volume can't hold.
pub fn resolve(path: &Path) -> Option<PathBuf> {
    if path.symlink_metadata().is_ok() {
        return Some(path.to_path_buf());
    }
    let mut resolved = PathBuf::new();
    for component in path.components() {
        let part = match component {
            Component::Normal(part) => part,
            other => {
                resolved.push(other.as_os_str());
                continue;
            }
        };
        let exact = resolved.join(part);
        if exact.symlink_metadata().is_ok() {
            resolved = exact;
            continue;
        }
        let part = part.to_str()?;
        let directory = if resolved.as_os_str().is_empty() {
            Path::new(".")
        } else {
            resolved.as_path()
        };
        let mut matches = fs::read_dir(directory)
            .ok()?
            .flatten()
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|name| same(name, part));
        let name = matches.next()?;
        if matches.next().is_some() {
            return None;
        }
        resolved.push(name);
    }
    Some(resolved)
}

#[cfg(all(test, unix))]
mod tests {
    #[test]
    fn resolves_literal_paths_regardless_of_case() {
        let temp = tempfile::tempdir().unwrap();
        let package = temp.path().join("unpack/TempPackage.pkg");
        std::fs::create_dir_all(&package).unwrap();
        std::fs::write(package.join("Payload"), b"").unwrap();
        std::fs::write(package.join("[v2] Read Me.rtf"), b"").unwrap();
        if temp.path().join("UNPACK").exists() {
            // A case-insensitive volume, such as the default on macOS,
            // resolves these paths itself.
            return;
        }
        let resolve = |p: &str| super::resolve(&temp.path().join(p));
        assert_eq!(
            resolve("unpack/tempPACKAGE.pkg/payload"),
            Some(package.join("Payload"))
        );
        // Glob characters are literal here.
        assert_eq!(
            resolve("unpack/TempPackage.pkg/[V2] read me.rtf"),
            Some(package.join("[v2] Read Me.rtf"))
        );
        assert_eq!(resolve("unpack/TempPackage.pkg/Missing"), None);
        // Two names that differ only in case are ambiguous.
        std::fs::write(package.join("PAYLOAD"), b"").unwrap();
        assert_eq!(resolve("unpack/TempPackage.pkg/payload"), None);
        assert_eq!(
            resolve("unpack/TempPackage.pkg/Payload"),
            Some(package.join("Payload"))
        );
    }

    #[test]
    fn matches_components_regardless_of_case() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(temp.path().join("Cyberduck/Cyberduck.app/Contents")).unwrap();
        let root = temp.path().display();
        let found = super::glob(&format!("{root}/cyberduck/CyberDuck.app"));
        assert_eq!(found, [temp.path().join("Cyberduck/Cyberduck.app")]);
        let found = super::glob(&format!("{root}/CYBERDUCK/*.APP/contents"));
        assert_eq!(
            found,
            [temp.path().join("Cyberduck/Cyberduck.app/Contents")]
        );
        assert!(super::glob(&format!("{root}/Missing.app")).is_empty());
    }
}
