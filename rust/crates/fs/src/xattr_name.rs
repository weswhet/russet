use std::borrow::Cow;

/// Linux keeps unprivileged attributes in the `user.` namespace.
#[cfg(target_os = "linux")]
const NAMESPACES: [&str; 4] = ["user.", "trusted.", "security.", "system."];

/// Maps an Apple extended-attribute name, such as `com.apple.ResourceFork`,
/// to the name stored on this host. Linux stores it as
/// `user.com.apple.ResourceFork`; macOS stores it unchanged.
pub fn host_xattr_name(name: &str) -> Cow<'_, str> {
    #[cfg(target_os = "linux")]
    if !NAMESPACES.iter().any(|ns| name.starts_with(ns)) {
        return Cow::Owned(format!("user.{name}"));
    }
    Cow::Borrowed(name)
}

/// Reverses [`host_xattr_name`].
pub fn apple_xattr_name(name: &str) -> &str {
    #[cfg(target_os = "linux")]
    if let Some(rest) = name.strip_prefix("user.") {
        return rest;
    }
    name
}
