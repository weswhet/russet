//! `EndOfCheckPhase`: marks where `autopkg run --check` stops. The recipe
//! engine acts on it, so the processor itself does nothing.
use crate::Result;

pub(crate) fn execute() -> Result<()> {
    Ok(())
}
