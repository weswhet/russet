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

#[cfg(test)]
mod tests {
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
