//! Python glob order follows scandir, unlike the sorted glob crate iterator.
use super::Result;
use std::{fs, path::PathBuf};
fn magic(s: &str) -> bool {
    s.contains(['*', '?', '['])
}
// fnmatch treats an unmatched opening bracket as a literal character.
fn basename_pattern(pattern: &str) -> String {
    let chars: Vec<char> = pattern.chars().collect();
    let mut result = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '[' {
            let mut end = i + 1;
            if chars.get(end) == Some(&'!') {
                end += 1;
            }
            if chars.get(end) == Some(&']') {
                end += 1;
            }
            while end < chars.len() && chars[end] != ']' {
                end += 1;
            }
            if end == chars.len() {
                result.push_str("[[]");
            } else {
                result.extend(chars[i..=end].iter());
                i = end;
            }
        } else if chars[i] != '*' || !result.ends_with('*') {
            result.push(chars[i]);
        }
        i += 1;
    }
    result
}
fn join(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.into()
    } else if dir.ends_with(std::path::MAIN_SEPARATOR)
        || (cfg!(windows) && (dir.ends_with('/') || dir.ends_with(':')))
    {
        format!("{dir}{name}")
    } else {
        format!("{dir}{}{name}", std::path::MAIN_SEPARATOR)
    }
}
fn entries(dir: &str, only_dirs: bool) -> Vec<String> {
    fs::read_dir(if dir.is_empty() { "." } else { dir })
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let entry = entry.ok()?;
            if only_dirs && !entry.path().is_dir() {
                return None;
            }
            entry.file_name().into_string().ok()
        })
        .collect()
}
fn descendants(dir: &str, only_dirs: bool, depth: usize) -> Vec<String> {
    if depth > 128 {
        return Vec::new();
    }
    let mut result = Vec::new();
    for name in entries(dir, only_dirs)
        .into_iter()
        .filter(|n| !n.starts_with('.'))
    {
        result.push(name.clone());
        for child in descendants(&join(dir, &name), only_dirs, depth + 1) {
            result.push(join(&name, &child));
        }
    }
    result
}
fn expand(pattern: &str, only_dirs: bool, depth: usize, recursive: bool) -> Result<Vec<String>> {
    if depth > 128 {
        return Err("Glob nesting exceeds 128 levels".into());
    }
    let (dirname, basename) = pattern
        .rsplit_once(|c| c == std::path::MAIN_SEPARATOR || (cfg!(windows) && c == '/'))
        .map(|(d, b)| {
            (
                if d.is_empty() {
                    &pattern[..1]
                } else if cfg!(windows) && d.ends_with(':') {
                    &pattern[..d.len() + 1]
                } else {
                    d
                },
                b,
            )
        })
        .unwrap_or_else(|| {
            if cfg!(windows) && pattern.as_bytes().get(1) == Some(&b':') {
                (&pattern[..2], &pattern[2..])
            } else {
                ("", pattern)
            }
        });
    if !magic(pattern) {
        return Ok(
            if (!basename.is_empty() && fs::symlink_metadata(pattern).is_ok())
                || (basename.is_empty() && std::path::Path::new(dirname).is_dir())
            {
                vec![pattern.into()]
            } else {
                vec![]
            },
        );
    }
    let directories = if magic(dirname) {
        expand(dirname, true, depth + 1, recursive)?
    } else {
        vec![dirname.into()]
    };
    let mut result = Vec::new();
    for dir in directories {
        let names = if basename == "**" && recursive {
            let mut names = vec![String::new()];
            names.extend(descendants(&dir, only_dirs, 0));
            names
        } else if magic(basename) {
            let matcher =
                glob::Pattern::new(&basename_pattern(basename)).map_err(|e| e.to_string())?;
            entries(&dir, only_dirs)
                .into_iter()
                .filter(|n| {
                    (!n.starts_with('.') || basename.starts_with('.'))
                        && matcher.matches_with(
                            n,
                            glob::MatchOptions {
                                case_sensitive: !cfg!(windows),
                                require_literal_separator: true,
                                require_literal_leading_dot: false,
                            },
                        )
                })
                .collect()
        } else if basename.is_empty() {
            if std::path::Path::new(&dir).is_dir() {
                vec![String::new()]
            } else {
                vec![]
            }
        } else if fs::symlink_metadata(join(&dir, basename)).is_ok() {
            vec![basename.into()]
        } else {
            vec![]
        };
        result.extend(names.into_iter().map(|name| join(&dir, &name)));
    }
    Ok(result)
}
pub(super) fn paths(pattern: &str) -> Result<Vec<PathBuf>> {
    paths_with_recursion(pattern, true)
}
pub(super) fn paths_with_recursion(pattern: &str, recursive: bool) -> Result<Vec<PathBuf>> {
    let mut paths = expand(pattern, false, 0, recursive)?;
    if paths.is_empty() && cfg!(target_os = "linux") {
        // Recipes are written for case-insensitive macOS volumes.
        paths = autopkg_platform::case_fold::glob(pattern)
            .into_iter()
            .filter_map(|p| p.into_os_string().into_string().ok())
            .collect();
    }
    // Python removes the empty first result for a bare recursive ** pattern.
    Ok(paths
        .into_iter()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .collect())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    #[test]
    fn preserves_mixed_directory_separators_like_ntpath_glob() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir(temp.path().join("child")).unwrap();
        fs::write(temp.path().join("child/file.txt"), "fixture").unwrap();
        let directory = format!("{}/child", temp.path().display());
        let matched = paths(&format!("{directory}/*.txt")).unwrap();
        assert_eq!(matched.len(), 1);
        assert_eq!(
            matched[0].to_string_lossy(),
            format!("{directory}\\file.txt")
        );
        let literal = format!("{directory}/file.txt");
        assert_eq!(paths(&literal).unwrap()[0].to_string_lossy(), literal);
    }
    #[test]
    fn follows_filesystem_order_and_excludes_hidden_recursion() {
        let temp = tempfile::tempdir().unwrap();
        for name in ["z.txt", "a.txt", "b.txt", ".hidden.txt"] {
            fs::write(temp.path().join(name), name).unwrap();
        }
        let expected: Vec<_> = fs::read_dir(temp.path())
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| !p.file_name().unwrap().to_string_lossy().starts_with('.'))
            .collect();
        assert_eq!(
            paths(&format!("{}/*.txt", temp.path().display())).unwrap(),
            expected
        );
        fs::create_dir(temp.path().join("child")).unwrap();
        fs::write(temp.path().join("child/n.txt"), "").unwrap();
        fs::create_dir(temp.path().join(".hidden")).unwrap();
        fs::write(temp.path().join(".hidden/n.txt"), "").unwrap();
        let paths = paths(&format!("{}/**/*.txt", temp.path().display())).unwrap();
        assert_eq!(paths.len(), 4);
    }
}
