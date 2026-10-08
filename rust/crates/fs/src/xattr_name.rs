use std::borrow::Cow;

/// Maps an Apple extended-attribute name, such as `com.apple.ResourceFork`,
/// to the name stored on this host. Linux stores every one in the
/// unprivileged `user.` namespace, as `user.com.apple.ResourceFork`, even a
/// name that looks like another namespace: an archive's
/// `security.capability` must not set file capabilities when Russet runs as
/// root. macOS stores names unchanged.
pub fn host_xattr_name(name: &str) -> Cow<'_, str> {
    #[cfg(target_os = "linux")]
    return Cow::Owned(format!("user.{name}"));
    #[cfg(not(target_os = "linux"))]
    Cow::Borrowed(name)
}

/// Reverses [`host_xattr_name`]. On Linux, attributes outside the `user.`
/// namespace, such as SELinux labels, aren't Apple attributes, so this
/// returns `None` for them.
pub fn apple_xattr_name(name: &str) -> Option<&str> {
    #[cfg(target_os = "linux")]
    return name.strip_prefix("user.");
    #[cfg(not(target_os = "linux"))]
    Some(name)
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn keeps_every_name_in_the_user_namespace() {
        for name in ["com.apple.FinderInfo", "security.capability", "user.x"] {
            let host = host_xattr_name(name);
            assert_eq!(host, format!("user.{name}"));
            assert_eq!(apple_xattr_name(&host), Some(name));
        }
        assert_eq!(apple_xattr_name("security.selinux"), None);
    }
}
