//! `ChocolateyPackager`: build a Chocolatey package. The platform crate
//! builds it.
use crate::Result;
use plist::Dictionary;

pub(crate) fn execute(env: &mut Dictionary) -> Result<()> {
    autopkg_platform::chocolatey::execute(env)
}
