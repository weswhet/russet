//! `ChocolateyPackager`: build a Chocolatey package. The platform crate
//! builds it.
//!
//! Inputs and outputs: run `russet processor-info ChocolateyPackager`, or see
//! `ChocolateyPackager` in `compatibility/reference.json`.
use crate::Result;
use plist::Dictionary;

pub(crate) fn execute(env: &mut Dictionary) -> Result<()> {
    autopkg_platform::chocolatey::execute(env)
}
