//! `VariableSetter`: the recipe engine sets its arguments in the environment,
//! so the processor itself does nothing.
use crate::Result;

pub(crate) fn execute() -> Result<()> {
    Ok(())
}
