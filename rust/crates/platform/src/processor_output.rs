//! Scoped processor logging without adding private keys to recipe environments.
use std::{cell::RefCell, marker::PhantomData, rc::Rc};

#[derive(Clone)]
struct Context {
    name: String,
    verbosity: i64,
    standalone: bool,
}
thread_local! {
    static CONTEXT: RefCell<Option<Context>> = const { RefCell::new(None) };
}

/// Restores the previous processor context on success, error, or unwind.
pub struct OutputScope {
    previous: Option<Context>,
    _thread_bound: PhantomData<Rc<()>>,
}
impl Drop for OutputScope {
    fn drop(&mut self) {
        CONTEXT.with(|context| *context.borrow_mut() = self.previous.take());
    }
}

pub fn scope(name: &str, env: &plist::Dictionary, standalone: bool) -> OutputScope {
    let verbosity = env
        .get("verbose")
        .and_then(|value| {
            value
                .as_signed_integer()
                .or_else(|| value.as_boolean().map(i64::from))
                .or_else(|| value.as_string().and_then(|text| text.parse().ok()))
        })
        .unwrap_or(0);
    let current = Context {
        name: name.into(),
        verbosity,
        standalone,
    };
    let previous = CONTEXT.with(|context| context.replace(Some(current)));
    OutputScope {
        previous,
        _thread_bound: PhantomData,
    }
}

pub fn emit(level: i64, message: impl std::fmt::Display) {
    let context = CONTEXT.with(|context| context.borrow().clone());
    if let Some(context) = context.filter(|context| context.verbosity >= level) {
        super::text_output::write(
            format_args!("{}: {}\n", context.name, message),
            context.standalone,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nested_scopes_restore_without_mutating_environment() {
        let env = plist::Dictionary::from_iter([("verbose", plist::Value::Integer(2.into()))]);
        let before = env.clone();
        let outer = scope("outer", &env, false);
        {
            let _inner = scope("inner", &env, true);
            CONTEXT.with(|context| assert_eq!(context.borrow().as_ref().unwrap().name, "inner"));
        }
        CONTEXT.with(|context| assert_eq!(context.borrow().as_ref().unwrap().name, "outer"));
        drop(outer);
        CONTEXT.with(|context| assert!(context.borrow().is_none()));
        assert_eq!(env, before);
    }
}
