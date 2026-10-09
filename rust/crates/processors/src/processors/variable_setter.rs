//! `VariableSetter`: the recipe engine sets its arguments in the environment,
//! so the processor itself does nothing.
//!
//! Inputs and outputs: run `russet processor-info VariableSetter`, or see
//! `VariableSetter` in `compatibility/reference.json`.
use crate::Result;

pub(crate) fn execute() -> Result<()> {
    Ok(())
}
