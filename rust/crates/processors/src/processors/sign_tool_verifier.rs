//! `SignToolVerifier`: verify a Windows Authenticode signature with
//! `signtool.exe`. The platform crate does the verification.
//!
//! Inputs and outputs: run `russet processor-info SignToolVerifier`, or see
//! `SignToolVerifier` in `compatibility/reference.json`.
use crate::Result;
use plist::Dictionary;

pub(crate) fn execute(env: &mut Dictionary) -> Result<()> {
    autopkg_platform::signature::verify_authenticode(env)
}
