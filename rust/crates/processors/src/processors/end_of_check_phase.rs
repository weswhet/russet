//! `EndOfCheckPhase`: marks where `autopkg run --check` stops. The recipe
//! engine acts on it, so the processor itself does nothing.
//!
//! Inputs and outputs: run `russet processor-info EndOfCheckPhase`, or see
//! `EndOfCheckPhase` in `compatibility/reference.json`.
use crate::Result;

pub(crate) fn execute() -> Result<()> {
    Ok(())
}
