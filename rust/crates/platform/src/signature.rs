//! Native signature verification with explicit signer requirements.
use plist::{Dictionary, Value};
use std::{
    path::{Path, PathBuf},
    process::Command,
};

fn enabled(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Boolean(v)) => *v,
        Some(Value::String(v)) => !v.is_empty(),
        Some(Value::Integer(v)) => v.as_signed() != Some(0),
        Some(Value::Array(v)) => !v.is_empty(),
        _ => true,
    }
}
fn strings(env: &Dictionary, key: &str) -> Result<Vec<String>, String> {
    match env.get(key) {
        None | Some(Value::Null) => Ok(vec![]),
        Some(Value::Array(values)) => values
            .iter()
            .map(|v| {
                v.as_string()
                    .map(str::to_owned)
                    .ok_or_else(|| format!("'{key}' must be a list of strings."))
            })
            .collect(),
        _ => Err(format!("'{key}' must be a list of strings.")),
    }
}
fn input(env: &Dictionary) -> Result<&str, String> {
    env.get("input_path")
        .and_then(Value::as_string)
        .ok_or_else(|| "input_path must be a string".into())
}

const VERIFICATION_FAILED: &str = "Code signature verification failed. Note that all verification can be disabled by setting the variable DISABLE_CODE_SIGNATURE_VERIFICATION to a non-empty value.";

/// Runs `pkgutil --check-signature`, logs its output, and returns the
/// certificate chain's names.
fn apple_package_chain(path: &std::path::Path) -> Result<Vec<String>, String> {
    let output = Command::new("/usr/sbin/pkgutil")
        .arg("--check-signature")
        .arg(path)
        .output()
        .map_err(|e| e.to_string())?;
    for line in String::from_utf8_lossy(&output.stderr)
        .lines()
        .chain(String::from_utf8_lossy(&output.stdout).lines())
    {
        super::processor_output(1, line);
    }
    if !output.status.success() {
        return Err(VERIFICATION_FAILED.into());
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let pattern = regex::Regex::new(r"\s+[1-9]+\. (?P<authority>.*)\n").unwrap();
    Ok(pattern
        .captures_iter(&text)
        .map(|c| c["authority"].to_owned())
        .collect())
}

/// Checks the signature with Russet's replacement for
/// `pkgutil --check-signature`, logs a report in the same layout, and
/// returns the certificate chain's names.
#[cfg(unix)]
fn native_package_chain(path: &std::path::Path) -> Result<Vec<String>, String> {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    match russet_pkgutil::check_signature(path, std::time::SystemTime::now()) {
        Ok(signature) => {
            for line in signature.report(&name) {
                super::processor_output(1, line);
            }
            Ok(signature.chain.names())
        }
        Err(error) => {
            super::processor_output(1, format!("Package \"{name}\":"));
            super::processor_output(1, format!("   {error}"));
            Err(VERIFICATION_FAILED.into())
        }
    }
}

pub fn verify_code_signature(env: &Dictionary) -> Result<(), String> {
    if enabled(env.get("DISABLE_CODE_SIGNATURE_VERIFICATION")) {
        eprintln!("WARNING: Code signature verification disabled for this recipe run.");
        return Ok(());
    }
    let requirement = match env.get("requirement") {
        None | Some(Value::Null) => "",
        Some(Value::String(s)) => s.as_str(),
        _ => return Err("'requirement' must be a string.".into()),
    };
    let extra = strings(env, "codesign_additional_arguments")?;
    let authorities = strings(env, "expected_authority_names")?;
    let matches = glob::glob(input(env)?)
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    let path = matches.first().ok_or_else(|| {
        format!(
            "Error processing path '{}' with glob.",
            input(env).unwrap_or_default()
        )
    })?;
    if matches.len() > 1 {
        super::processor_output(
            1,
            format!(
                "WARNING: Multiple paths match 'input_path' glob '{}':",
                input(env)?
            ),
        );
        for item in &matches {
            super::processor_output(1, format!("  - {}", item.display()));
        }
    }
    if input(env)?.contains(['*', '?', '[', ']', '!']) {
        super::processor_output(
            1,
            format!(
                "Using path '{}' matched from globbed '{}'.",
                path.display(),
                input(env)?
            ),
        );
    }
    let codesign_pinning = !requirement.is_empty()
        || extra.iter().any(|a| {
            a.starts_with("-R") || a == "--test-requirement" || a.starts_with("--test-requirement=")
        });
    if matches!(
        path.extension().and_then(|v| v.to_str()),
        Some("pkg" | "mpkg" | "xip")
    ) {
        super::processor_output(1, "Verifying installer package signature...");
        if env.contains_key("expected_authorities") {
            return Err("Use 'expected_authority_names' instead of 'expected_authorities'.".into());
        }
        if env.contains_key("expected_authority_names") && authorities.is_empty() {
            return Err("'expected_authority_names' is set but empty. Provide the full certificate authority chain or remove the key. Note that all verification can be disabled by setting the variable DISABLE_CODE_SIGNATURE_VERIFICATION to a non-empty value.".into());
        }
        if authorities.is_empty() {
            return Err(if codesign_pinning { "'requirement' cannot verify an installer package signature; use 'expected_authority_names' to pin the signer. Note that verification can be disabled by setting the variable DISABLE_CODE_SIGNATURE_VERIFICATION to a non-empty value." } else { "No 'expected_authority_names' set. A valid package signature alone does not verify the expected signer. Set 'expected_authority_names' to the certificate authority chain from 'pkgutil --check-signature <path>'. Note that verification can be disabled by setting the variable DISABLE_CODE_SIGNATURE_VERIFICATION to a non-empty value." }.into());
        }
        if codesign_pinning {
            super::processor_output(1, "WARNING: Ignoring 'requirement'/'-R' on installer packages; 'expected_authority_names' is pinning the signer.");
        }
        let actual = match crate::backend::select(crate::backend::Tool::Pkgutil) {
            crate::backend::Backend::Apple => apple_package_chain(path)?,
            #[cfg(unix)]
            crate::backend::Backend::Native => native_package_chain(path)?,
            _ => {
                return Err(
                    "Code signature verification is only supported on macOS and Linux.".into(),
                )
            }
        };
        super::processor_output(1, "Signature is valid");
        if actual != authorities {
            super::processor_output(1, "Mismatch in authority names");
            super::processor_output(1, format!("Expected: {}", authorities.join(" -> ")));
            super::processor_output(1, format!("Found:    {}", actual.join(" -> ")));
            return Err("Mismatch in authority names. Note that all verification can be disabled by setting the variable DISABLE_CODE_SIGNATURE_VERIFICATION to a non-empty value.".into());
        }
        super::processor_output(1, "Authority name chain is valid");
        return Ok(());
    }
    if !cfg!(target_os = "macos") {
        return Err("Code signature verification of apps is only supported on macOS.".into());
    }

    super::processor_output(1, "Verifying code signature...");
    if env.contains_key("requirements") {
        return Err("Use 'requirement' instead of 'requirements'.".into());
    }
    if codesign_pinning && !authorities.is_empty() {
        super::processor_output(1, "WARNING: Ignoring 'expected_authority_names' on the codesign path; 'requirement' is verifying the signature.");
    }
    if !codesign_pinning {
        if !authorities.is_empty() {
            super::processor_output(1, "ERROR: 'expected_authority_names' cannot verify an application signature; use 'requirement' instead.");
            super::processor_output(1, "See https://github.com/autopkg/autopkg/wiki/Using-CodeSignatureVerifier for more information.");
        }
        return Err(if !authorities.is_empty() { "Using 'expected_authority_names' to verify an application signature is not supported; use 'requirement' instead. Note that all verifications can be disabled by setting the variable DISABLE_CODE_SIGNATURE_VERIFICATION to a non-empty value." } else { "No 'requirement' set. Confirming only that the code is signed by some valid Developer ID does not verify the expected signer. Set 'requirement' to the app's designated requirement from 'codesign --display -r- <path>'. Note that verification can be disabled by setting the variable DISABLE_CODE_SIGNATURE_VERIFICATION to a non-empty value." }.into());
    }
    let mut command = Command::new("/usr/bin/codesign");
    command.args(["--verify", "--verbose=1"]);
    if env.get("deep_verification").is_none() || enabled(env.get("deep_verification")) {
        super::processor_output(1, "Deep verification enabled...");
        command.arg("--deep");
    } else {
        super::processor_output(1, "Deep verification disabled...");
    }
    match env.get("strict_verification") {
        Some(Value::Null) => super::processor_output(
            1,
            "Strict verification not defined. Using codesign defaults...",
        ),
        value if value.is_none() || enabled(value) => {
            super::processor_output(1, "Strict verification enabled...");
            command.arg("--strict");
        }
        _ => {
            super::processor_output(1, "Strict verification disabled...");
            command.arg("--no-strict");
        }
    }
    command.args(extra);
    if !requirement.is_empty() {
        if enabled(env.get("CODE_SIGNATURE_VERIFICATION_DEBUG")) {
            super::processor_output(1, format!("Requirement: {requirement}"));
        }
        command
            .arg("--test-requirement")
            .arg(format!("={requirement}"));
    }
    command.arg(path);
    if enabled(env.get("CODE_SIGNATURE_VERIFICATION_DEBUG")) {
        super::processor_output(
            1,
            format!(
                "/usr/bin/codesign {}",
                command
                    .get_args()
                    .map(|a| a.to_string_lossy())
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
        );
    }
    let output = command.output().map_err(|e| e.to_string())?;
    for line in String::from_utf8_lossy(&output.stderr)
        .lines()
        .chain(String::from_utf8_lossy(&output.stdout).lines())
    {
        super::processor_output(1, line);
    }
    if output.status.success() {
        super::processor_output(1, "Signature is valid");
        return Ok(());
    }
    let reason = match output.status.code() {
        Some(3) => "signed by an unexpected identity.",
        Some(2) => "codesign rejected its arguments (exit 2). Check the 'requirement' string and 'codesign_additional_arguments'.",
        _ => "the code is unsigned or has an invalid signature.",
    };
    Err(format!("Code signature verification failed: {reason} Note that all verifications can be disabled by setting the variable DISABLE_CODE_SIGNATURE_VERIFICATION to a non-empty value."))
}

pub fn signtool_default_path() -> Option<PathBuf> {
    for key in ["ProgramFiles(x86)", "ProgramFiles"] {
        if let Some(root) = std::env::var_os(key) {
            for arch in ["x64", "x86"] {
                let path = Path::new(&root)
                    .join("Windows Kits/10/bin")
                    .join(arch)
                    .join("signtool.exe");
                if path.exists() {
                    return Some(path);
                }
            }
        }
    }
    let path =
        PathBuf::from(r"C:\Program Files (x86)\Windows Kits\10\App Certification Kit\signtool.exe");
    path.exists().then_some(path)
}

pub fn verify_authenticode(env: &Dictionary) -> Result<(), String> {
    if enabled(env.get("DISABLE_CODE_SIGNATURE_VERIFICATION")) {
        eprintln!("WARNING: Authenticode verification disabled for this recipe run.");
        return Ok(());
    }
    if !cfg!(windows) {
        return Err("Authenticode verification is only supported on Windows.".into());
    }
    let executable = env
        .get("signtool_path")
        .and_then(Value::as_string)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(signtool_default_path)
        .ok_or("No signtool_path configured. Set signtool_path to the path to signtool.exe.")?;
    let path = std::path::absolute(input(env)?).map_err(|e| e.to_string())?;
    let combined = tempfile::tempfile().map_err(|e| e.to_string())?;
    let output = Command::new(executable)
        .args(["verify", "/v", "/pa"])
        .args(strings(env, "additional_arguments")?)
        .arg(path)
        .stdout(combined.try_clone().map_err(|e| e.to_string())?)
        .stderr(combined.try_clone().map_err(|e| e.to_string())?)
        .output()
        .map_err(|e| e.to_string())?;
    use std::io::{Read, Seek};
    let mut combined = combined;
    combined.rewind().map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    combined
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    let text = signtool_text(&bytes);
    for line in text.lines() {
        super::processor_output(1, line);
    }
    match output.status.code() {
        Some(0) => Ok(()),
        Some(2) => {
            super::processor_output(1, "WARNING: Verification had warnings. Check output above.");
            Ok(())
        }
        _ => Err("Authenticode verification failed. Note that all verification can be disabled by setting the variable DISABLE_CODE_SIGNATURE_VERIFICATION to a non-empty value.".into()),
    }
}

fn signtool_text(bytes: &[u8]) -> String {
    // subprocess text=True applies universal newline conversion before the
    // processor collapses blank lines. SignTool can emit CR CR LF on Windows.
    String::from_utf8_lossy(bytes)
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace("\n\n", "\n")
        .replace("\n\n\n", "\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn signtool_universal_newlines_precede_blank_line_collapse() {
        // Matches CPython TextIOWrapper(newline=None), then the reference's
        // two replace calls. In particular a doubled CR must not lose blanks.
        let raw = b"\r\r\nVerifying: input.exe\r\r\n\r\r\nNumber of errors: 0\r\r\n";
        assert_eq!(
            signtool_text(raw),
            "\nVerifying: input.exe\n\nNumber of errors: 0\n"
        );
        assert_eq!(signtool_text(b"a\rb\r\nc\n"), "a\nb\nc\n");
    }
    #[test]
    fn explicit_disable_short_circuits() {
        let env =
            Dictionary::from_iter([("DISABLE_CODE_SIGNATURE_VERIFICATION", Value::Boolean(true))]);
        verify_code_signature(&env).unwrap();
        verify_authenticode(&env).unwrap();
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn verifies_native_signed_binary_and_rejects_wrong_signer() {
        let mut env = Dictionary::from_iter([
            ("input_path", Value::String("/usr/bin/true".into())),
            ("requirement", Value::String("anchor apple".into())),
        ]);
        verify_code_signature(&env).unwrap();
        env.insert(
            "requirement".into(),
            "identifier \"org.autopkg.impossible\"".into(),
        );
        assert!(verify_code_signature(&env)
            .unwrap_err()
            .contains("unexpected identity"));
        env.remove("requirement");
        assert!(verify_code_signature(&env)
            .unwrap_err()
            .contains("No 'requirement'"));
    }
}
