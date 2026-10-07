//! Native code-signature verification for Russet.
//!
//! [`trust`] builds certificate chains to Apple's root certificates, which
//! are the only roots it trusts, and [`cms`] verifies detached CMS
//! signatures and their timestamps. [`package`] checks an installer
//! package's signature the way `pkgutil --check-signature` does.
//!
//! Verification fails closed: anything the verifier doesn't understand, such
//! as an unknown critical certificate extension, is a failure. It doesn't
//! check notarization or certificate revocation.
#![forbid(unsafe_code)]

pub mod cms {
    pub use crate::cms_verify::{verify_detached, VerifiedCms};
}
mod ber;
pub mod bundle;
mod cms_verify;
pub mod code;
pub mod macho;
pub mod package;
pub mod requirement;
pub mod trust;
