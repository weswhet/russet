//! Whether the live display can draw, and how big the window is.

/// The window's columns and rows, or `None` when stdout isn't a terminal
/// that reports a size.
#[cfg(unix)]
pub fn size() -> Option<(usize, usize)> {
    // SAFETY: TIOCGWINSZ fills the zeroed `winsize` and touches nothing else.
    let mut size: libc::winsize = unsafe { std::mem::zeroed() };
    let result = unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut size) };
    (result == 0 && size.ws_col > 0 && size.ws_row > 0)
        .then(|| (usize::from(size.ws_col), usize::from(size.ws_row)))
}

#[cfg(not(unix))]
pub fn size() -> Option<(usize, usize)> {
    None
}

/// Whether stdout and stderr are the same terminal.
#[cfg(unix)]
fn same_terminal() -> bool {
    fn device(fd: libc::c_int) -> Option<libc::dev_t> {
        // SAFETY: isatty and fstat only read the descriptor and fill `stat`.
        unsafe {
            if libc::isatty(fd) != 1 {
                return None;
            }
            let mut stat: libc::stat = std::mem::zeroed();
            (libc::fstat(fd, &mut stat) == 0).then_some(stat.st_rdev)
        }
    }
    matches!(
        (device(libc::STDOUT_FILENO), device(libc::STDERR_FILENO)),
        (Some(out), Some(err)) if out == err
    )
}

#[cfg(not(unix))]
fn same_terminal() -> bool {
    false
}

/// Whether the live display can draw here.
pub fn available() -> bool {
    let term = std::env::var("TERM").ok();
    decide(
        cfg!(unix),
        same_terminal(),
        term.as_deref(),
        size().map(|(width, _)| width),
    )
}

/// The display needs macOS or Linux, stdout and stderr on the same
/// terminal, a `TERM` that isn't `dumb`, and a window at least 40 columns
/// wide.
fn decide(unix: bool, same_terminal: bool, term: Option<&str>, width: Option<usize>) -> bool {
    unix && same_terminal
        && term.is_some_and(|term| !term.is_empty() && term != "dumb")
        && width.is_some_and(|width| width >= 40)
}

#[cfg(test)]
mod tests {
    use super::decide;

    #[test]
    fn draws_only_on_a_shared_capable_terminal() {
        assert!(decide(true, true, Some("xterm-256color"), Some(80)));
        assert!(!decide(false, true, Some("xterm-256color"), Some(80)));
        assert!(!decide(true, false, Some("xterm-256color"), Some(80)));
        assert!(!decide(true, true, Some("dumb"), Some(80)));
        assert!(!decide(true, true, Some(""), Some(80)));
        assert!(!decide(true, true, None, Some(80)));
        assert!(!decide(true, true, Some("xterm"), Some(39)));
        assert!(!decide(true, true, Some("xterm"), None));
        assert!(decide(true, true, Some("xterm"), Some(40)));
    }
}
