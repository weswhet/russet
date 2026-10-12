//! Text-mode console output matching Python's native newline convention.
//!
//! When recipes run in parallel, each recipe's thread sets a prefix with
//! [`prefix_scope`]. Every line that thread writes then starts with
//! `[name] `, and is written whole, so lines from different recipes never
//! mix.
use std::{cell::RefCell, io::Write, marker::PhantomData, rc::Rc};

thread_local! {
    static PREFIX: RefCell<Option<String>> = const { RefCell::new(None) };
    /// The unfinished line on stdout and stderr, waiting for its newline.
    static PENDING: RefCell<[String; 2]> = const { RefCell::new([String::new(), String::new()]) };
}

pub fn write(arguments: std::fmt::Arguments<'_>, stderr: bool) {
    emit(&prefixed(arguments.to_string(), stderr), stderr);
}

/// The text to write now: unchanged without a prefix, otherwise the lines
/// it completes.
fn prefixed(text: String, stderr: bool) -> String {
    match PREFIX.with(|prefix| prefix.borrow().clone()) {
        Some(prefix) => PENDING.with(|pending| {
            complete_lines(
                &prefix,
                &mut pending.borrow_mut()[usize::from(stderr)],
                &text,
            )
        }),
        None => text,
    }
}

fn emit(text: &str, stderr: bool) {
    if text.is_empty() {
        return;
    }
    #[cfg(windows)]
    let text = &windows_text(text);
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

/// Adds `text` to the unfinished line in `pending`, and returns every line
/// that is now complete, each starting with `[prefix] `.
fn complete_lines(prefix: &str, pending: &mut String, text: &str) -> String {
    pending.push_str(text);
    let Some(end) = pending.rfind('\n') else {
        return String::new();
    };
    let rest = pending.split_off(end + 1);
    let lines = std::mem::replace(pending, rest);
    lines
        .split_inclusive('\n')
        .map(|line| format!("[{prefix}] {line}"))
        .collect()
}

/// Starts each line this thread writes with `[name] ` until the guard drops.
/// The guard then writes any unfinished line and restores the previous
/// prefix.
pub fn prefix_scope(name: &str) -> PrefixScope {
    let previous = PREFIX.with(|prefix| prefix.replace(Some(name.into())));
    PrefixScope {
        previous,
        _thread_bound: PhantomData,
    }
}

pub struct PrefixScope {
    previous: Option<String>,
    _thread_bound: PhantomData<Rc<()>>,
}

impl Drop for PrefixScope {
    fn drop(&mut self) {
        let prefix = PREFIX.with(|prefix| prefix.replace(self.previous.take()));
        if let Some(prefix) = prefix {
            for stderr in [false, true] {
                let line = PENDING
                    .with(|pending| std::mem::take(&mut pending.borrow_mut()[usize::from(stderr)]));
                if !line.is_empty() {
                    emit(&format!("[{prefix}] {line}\n"), stderr);
                }
            }
        }
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
    use super::*;
    #[test]
    fn text_newlines_match_windows_python_translation() {
        assert_eq!(super::windows_text("one\ntwo\n"), "one\r\ntwo\r\n");
        assert_eq!(super::windows_text("explicit\r\n"), "explicit\r\r\n");
    }

    #[test]
    fn every_line_carries_the_prefix() {
        let mut pending = String::new();
        assert_eq!(
            complete_lines("Firefox.munki", &mut pending, "one\ntwo\n"),
            "[Firefox.munki] one\n[Firefox.munki] two\n"
        );
        assert_eq!(
            complete_lines("Firefox.munki", &mut pending, "\n"),
            "[Firefox.munki] \n"
        );
        assert!(pending.is_empty());
    }

    #[test]
    fn fragments_wait_for_their_newline() {
        let mut pending = String::new();
        assert_eq!(complete_lines("A", &mut pending, "    "), "");
        assert_eq!(complete_lines("A", &mut pending, "cell"), "");
        assert_eq!(
            complete_lines("A", &mut pending, "\nnext"),
            "[A]     cell\n"
        );
        assert_eq!(pending, "next");
    }

    #[test]
    fn output_is_unchanged_without_a_prefix() {
        assert_eq!(prefixed("    ".into(), false), "    ");
        assert_eq!(prefixed("one\ntwo".into(), true), "one\ntwo");
        let scope = prefix_scope("A");
        assert_eq!(prefixed("one\ntwo".into(), true), "[A] one\n");
        PENDING.with(|pending| assert_eq!(pending.borrow()[1], "two"));
        PENDING.with(|pending| pending.borrow_mut()[1].clear());
        drop(scope);
    }

    #[test]
    fn scopes_nest_and_restore() {
        assert!(PREFIX.with(|p| p.borrow().is_none()));
        let outer = prefix_scope("outer");
        {
            let _inner = prefix_scope("inner");
            assert_eq!(
                PREFIX.with(|p| p.borrow().clone()).as_deref(),
                Some("inner")
            );
        }
        assert_eq!(
            PREFIX.with(|p| p.borrow().clone()).as_deref(),
            Some("outer")
        );
        drop(outer);
        assert!(PREFIX.with(|p| p.borrow().is_none()));
    }
}
