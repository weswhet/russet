use crate::invalid;
use std::io;
use std::path::{Component, Path, PathBuf};

/// Cleans an archive member path into a relative path below the destination.
///
/// `.` components are dropped and `..` removes the previous component.
/// Absolute paths, paths that climb above the destination, and names that
/// contain NUL are rejected. The result may be empty, which names the
/// destination itself.
pub fn clean_relative(path: &Path) -> io::Result<PathBuf> {
    let mut clean = PathBuf::new();
    for part in path.components() {
        match part {
            Component::Normal(name) => {
                if name.as_encoded_bytes().contains(&0) {
                    return Err(invalid(format!(
                        "Archive contains a name with a NUL byte: '{}'",
                        path.display()
                    )));
                }
                clean.push(name);
            }
            Component::CurDir => {}
            Component::ParentDir => {
                if !clean.pop() {
                    return Err(invalid(format!(
                        "Archive contains path '{}' outside destination",
                        path.display()
                    )));
                }
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(invalid(format!(
                    "Archive contains absolute path '{}'",
                    path.display()
                )))
            }
        }
    }
    Ok(clean)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleans_and_rejects() {
        assert_eq!(
            clean_relative(Path::new("./a/b/../c")).unwrap(),
            Path::new("a/c")
        );
        assert_eq!(clean_relative(Path::new(".")).unwrap(), Path::new(""));
        assert!(clean_relative(Path::new("../a")).is_err());
        assert!(clean_relative(Path::new("a/../../b")).is_err());
        assert!(clean_relative(Path::new("/etc/passwd")).is_err());
    }
}
