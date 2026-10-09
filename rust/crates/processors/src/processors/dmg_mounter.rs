//! `DmgMounter`: in AutoPkg, a base class that mounts disk images for other
//! processors, not a processor a recipe runs. Russet mounts images for those
//! processors in `crate::dmg`. Running it directly fails as it does in
//! Python.
use crate::Result;

pub(crate) const ERROR: &str = "'DmgMounter' object has no attribute 'input_variables'";

pub(crate) fn execute() -> Result<()> {
    Err(ERROR.into())
}
