//! Text-mode console output matching Python's native newline convention.
use std::io::Write;
pub fn write(arguments: std::fmt::Arguments<'_>, stderr: bool) {
    let text = arguments.to_string();
    #[cfg(windows)]
    let text = windows_text(&text);
    if stderr {
        std::io::stderr()
            .lock()
            .write_all(text.as_bytes())
            .expect("write stderr");
    } else {
        std::io::stdout()
            .lock()
            .write_all(text.as_bytes())
            .expect("write stdout");
    }
}
#[macro_export]
macro_rules! text_println {
    () => { $crate::text_output::write(format_args!("\n"), false) };
    ($($arg:tt)*) => { $crate::text_output::write(format_args!("{}\n", format_args!($($arg)*)), false) };
}
#[macro_export]
macro_rules! text_eprintln {
    () => { $crate::text_output::write(format_args!("\n"), true) };
    ($($arg:tt)*) => { $crate::text_output::write(format_args!("{}\n", format_args!($($arg)*)), true) };
}
#[macro_export]
macro_rules! text_print {
    ($($arg:tt)*) => { $crate::text_output::write(format_args!($($arg)*), false) };
}

#[cfg(any(windows, test))]
fn windows_text(text: &str) -> String {
    text.replace('\n', "\r\n")
}
#[cfg(test)]
mod tests {
    #[test]
    fn text_newlines_match_windows_python_translation() {
        assert_eq!(super::windows_text("one\ntwo\n"), "one\r\ntwo\r\n");
        assert_eq!(super::windows_text("explicit\r\n"), "explicit\r\r\n");
    }
}
