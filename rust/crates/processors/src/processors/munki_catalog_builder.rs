//! `MunkiCatalogBuilder`: deprecated in AutoPkg 2.7.5. It only warns.
//!
//! Inputs and outputs: run `russet processor-info MunkiCatalogBuilder`, or see
//! `MunkiCatalogBuilder` in `compatibility/reference.json`.
use super::deprecation_warning::warn;
use crate::Result;
use plist::Dictionary;

pub(crate) fn execute(env: &mut Dictionary) -> Result<()> {
    warn(
        env,
        "MunkiCatalogBuilder was deprecated in AutoPkg version 2.7.5 and may be removed in a future release.".into(),
    )
}
