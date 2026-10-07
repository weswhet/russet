//! Native subprocess services. No shell interpolation or Python runtime.
use std::ffi::OsStr;
use std::process::{Command, Output};
pub mod backend;
pub mod chocolatey;
pub mod downloads;
pub mod github;
pub mod signature;

/// Read an application preference using the user's native macOS preference domain.
#[cfg(target_os = "macos")]
pub fn preference(domain: &str, key: &str) -> Result<Option<plist::Value>, String> {
    use core_foundation_sys::{
        base::{CFRelease, CFTypeRef},
        data::{CFDataGetBytePtr, CFDataGetLength},
        preferences::CFPreferencesCopyAppValue,
        propertylist::{kCFPropertyListXMLFormat_v1_0, CFPropertyListCreateData},
        string::{kCFStringEncodingUTF8, CFStringCreateWithCString},
    };
    use std::{ffi::CString, ptr};
    struct Owned(CFTypeRef);
    impl Drop for Owned {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { CFRelease(self.0) }
            }
        }
    }
    let domain = CString::new(domain).map_err(|e| e.to_string())?;
    let key = CString::new(key).map_err(|e| e.to_string())?;
    // Every Create/Copy object is released once. The borrowed CFData bytes are
    // parsed before their owner is dropped.
    unsafe {
        let domain = Owned(
            CFStringCreateWithCString(ptr::null(), domain.as_ptr(), kCFStringEncodingUTF8).cast(),
        );
        let key = Owned(
            CFStringCreateWithCString(ptr::null(), key.as_ptr(), kCFStringEncodingUTF8).cast(),
        );
        if domain.0.is_null() || key.0.is_null() {
            return Err("Cannot allocate preference key".into());
        }
        let value = CFPreferencesCopyAppValue(key.0.cast(), domain.0.cast());
        if value.is_null() {
            return Ok(None);
        }
        let value = Owned(value);
        let mut error = ptr::null_mut();
        let bytes = CFPropertyListCreateData(
            ptr::null(),
            value.0,
            kCFPropertyListXMLFormat_v1_0,
            0,
            &mut error,
        );
        if !error.is_null() {
            CFRelease(error.cast());
        }
        if bytes.is_null() {
            return Err("Cannot serialize native preference".into());
        }
        let owner = Owned(bytes.cast());
        let data =
            std::slice::from_raw_parts(CFDataGetBytePtr(bytes), CFDataGetLength(bytes) as usize);
        let result =
            plist::Value::from_reader(std::io::Cursor::new(data)).map_err(|e| e.to_string());
        drop(owner);
        result.map(Some)
    }
}

#[cfg(not(target_os = "macos"))]
pub fn preference(_domain: &str, _key: &str) -> Result<Option<plist::Value>, String> {
    // Other platforms have no CFPreferences domains. Explicit preference files
    // are handled by the recipe engine.
    Ok(None)
}

pub fn run(program: &OsStr, arguments: &[&OsStr]) -> Result<Output, String> {
    let output = Command::new(program)
        .args(arguments)
        .output()
        .map_err(|e| format!("Cannot execute {}: {e}", program.to_string_lossy()))?;
    if !output.status.success() {
        return Err(format!(
            "{} exited with {}: {}",
            program.to_string_lossy(),
            output.status,
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(output)
}

/// Evaluate through Foundation, retaining the reference's complete macOS
/// NSPredicate language. Objective-C exceptions become processor errors.
#[cfg(target_os = "macos")]
pub fn predicate(source: &str, env: &plist::Dictionary) -> Result<bool, String> {
    use objc2::{class, msg_send, rc::autoreleasepool, runtime::AnyObject};
    use std::ffi::c_void;
    #[link(name = "Foundation", kind = "framework")]
    extern "C" {}
    // Use Foundation objects directly: property lists cannot represent NSNull,
    // while Python recipe environments can contain None at any depth.
    unsafe fn object(value: &plist::Value, depth: usize) -> Result<*mut AnyObject, String> {
        if depth > 128 {
            return Err("Predicate environment nesting exceeds 128 levels".into());
        }
        use plist::Value;
        let result: *mut AnyObject = match value {
            Value::Null => msg_send![class!(NSNull), null],
            Value::String(s) => {
                msg_send![class!(NSString), stringWithBytes: s.as_ptr().cast::<c_void>(), length: s.len(), encoding: 4usize]
            }
            Value::Boolean(b) => msg_send![class!(NSNumber), numberWithBool: *b],
            Value::Integer(n) => {
                if let Some(signed) = n.as_signed() {
                    msg_send![class!(NSNumber), numberWithLongLong: signed]
                } else {
                    msg_send![class!(NSNumber), numberWithUnsignedLongLong: n.as_unsigned().ok_or("Invalid integer")?]
                }
            }
            Value::Real(n) => msg_send![class!(NSNumber), numberWithDouble: *n],
            Value::Data(bytes) => {
                msg_send![class!(NSData), dataWithBytes: bytes.as_ptr().cast::<c_void>(), length: bytes.len()]
            }
            Value::Date(date) => {
                let time: std::time::SystemTime = (*date).into();
                let seconds = match time.duration_since(std::time::UNIX_EPOCH) {
                    Ok(d) => d.as_secs_f64(),
                    Err(e) => -e.duration().as_secs_f64(),
                };
                msg_send![class!(NSDate), dateWithTimeIntervalSince1970: seconds]
            }
            Value::Array(values) => {
                let array: *mut AnyObject =
                    msg_send![class!(NSMutableArray), arrayWithCapacity: values.len()];
                for value in values {
                    let value = object(value, depth + 1)?;
                    let _: () = msg_send![array, addObject: value];
                }
                array
            }
            Value::Dictionary(values) => {
                let dictionary: *mut AnyObject =
                    msg_send![class!(NSMutableDictionary), dictionaryWithCapacity: values.len()];
                for (key, value) in values {
                    let key = object(&Value::String(key.clone()), depth + 1)?;
                    let value = object(value, depth + 1)?;
                    let _: () = msg_send![dictionary, setObject: value, forKey: key];
                }
                dictionary
            }
            _ => return Err("Unsupported value in native predicate environment".into()),
        };
        if result.is_null() {
            Err("Cannot create native predicate value".into())
        } else {
            Ok(result)
        }
    }
    autoreleasepool(|_| {
        objc2::exception::catch(|| {
            // NSData and NSString copy their input bytes. All returned objects
            // are autoreleased and used only inside this pool. Method argument
            // types match Foundation's NSUInteger and NSError** declarations.
            unsafe {
                let dictionary = object(&plist::Value::Dictionary(env.clone()), 0)?;
                let format: *mut AnyObject = msg_send![class!(NSString), stringWithBytes: source.as_ptr().cast::<c_void>(), length: source.len(), encoding: 4usize];
                let arguments: *mut AnyObject = msg_send![class!(NSArray), array];
                let predicate: *mut AnyObject = msg_send![class!(NSPredicate), predicateWithFormat: format, argumentArray: arguments];
                let result: bool = msg_send![predicate, evaluateWithObject: dictionary];
                Ok(result)
            }
        }).map_err(|error| error.map(|e| e.to_string()).unwrap_or_else(|| "Native predicate exception".into()))?
    })
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    #[test]
    fn native_predicate_and_exception_handling() {
        let environment =
            plist::Dictionary::from_iter([("name", plist::Value::String("CAFÉ".into()))]);
        assert!(predicate("name ==[cd] 'cafe'", &environment).unwrap());
        assert!(predicate("name MATCHES 'CAF(?=É)É'", &environment).unwrap());
        assert!(predicate("invalid syntax (!", &environment).is_err());
    }
    #[test]
    fn native_predicate_preserves_nulls() {
        use plist::{Dictionary, Value};
        let environment = Dictionary::from_iter([
            ("nothing", Value::Null),
            (
                "nested",
                Value::Dictionary(Dictionary::from_iter([("value", Value::Null)])),
            ),
            ("items", Value::Array(vec![Value::Null, "x".into()])),
        ]);
        // Expected results captured from Foundation through the pinned Python
        // environment, including its distinction between nil and NSNull in IN.
        assert!(predicate("nothing == nil", &environment).unwrap());
        assert!(predicate("nested.value == nil", &environment).unwrap());
        assert!(!predicate("nothing == 0", &environment).unwrap());
        assert!(!predicate("nothing == ''", &environment).unwrap());
        assert!(!predicate("items CONTAINS nil", &environment).unwrap());
    }
    #[test]
    fn missing_preference_is_absent() {
        assert!(preference("org.autopkg.rust.tests.nonexistent", "missing")
            .unwrap()
            .is_none());
    }
}

pub mod dmg;

pub mod processor_output;
pub mod text_output;
pub use processor_output::emit as processor_output;
