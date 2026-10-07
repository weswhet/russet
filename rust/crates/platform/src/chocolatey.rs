//! Chocolatey package configuration and native Windows packaging.
use plist::{Dictionary, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

fn text<'a>(env: &'a Dictionary, key: &str) -> Result<&'a str, String> {
    env.get(key)
        .and_then(Value::as_string)
        .ok_or_else(|| format!("{key} must be a string"))
}
fn component<'a>(env: &'a Dictionary, key: &str) -> Result<&'a str, String> {
    let value = text(env, key)?;
    if value.is_empty()
        || value.contains(['/', '\\'])
        || value == "."
        || value == ".."
        || Path::new(value).is_absolute()
    {
        return Err(format!(
            "Variable `{key}` must be a non-empty string path component."
        ));
    }
    Ok(value)
}
fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
fn quoted(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

pub fn nuspec(env: &Dictionary) -> Result<String, String> {
    for key in ["id", "version", "title", "authors", "description"] {
        text(env, key)?;
    }
    component(env, "id")?;
    component(env, "version")?;
    for key in ["license", "contentFiles"] {
        if env.get(key).is_some_and(|value| !value.is_null()) {
            // The reference forwards these values to generated Python objects
            // without converting recipe dictionaries into those object types.
            return Err(format!(
                "Non-null Nuspec {key} values cannot be represented by supported recipe inputs"
            ));
        }
    }
    let mut xml = "<package xmlns:mstns=\"http://schemas.microsoft.com/packaging/2015/06/nuspec.xsd\" xmlns:None=\"http://schemas.microsoft.com/packaging/2015/06/nuspec.xsd\" >\n    <metadata>\n".to_string();
    for key in [
        "id",
        "version",
        "title",
        "authors",
        "owners",
        "licenseUrl",
        "projectUrl",
        "iconUrl",
        "description",
        "summary",
        "releaseNotes",
        "copyright",
        "tags",
        "icon",
    ] {
        if env.contains_key(key) {
            xml.push_str(&format!(
                "        <{key}>{}</{key}>\n",
                escape(text(env, key)?).replace("&quot;", "\"")
            ));
        }
    }
    if let Some(dependencies) = env.get("dependencies") {
        let dependencies = dependencies
            .as_array()
            .ok_or("dependencies must be an array")?;
        if !dependencies.is_empty() {
            xml.push_str("        <dependencies>\n");
            for dependency in dependencies {
                let dependency = dependency
                    .as_dictionary()
                    .ok_or("Each dependency must be a dictionary")?;
                let id = text(dependency, "id")?;
                let mut attrs = format!("id=\"{}\"", escape(id));
                for key in ["version", "include", "exclude"] {
                    if dependency.contains_key(key) {
                        attrs.push_str(&format!(" {key}=\"{}\"", escape(text(dependency, key)?)));
                    }
                }
                if dependency
                    .keys()
                    .any(|k| !["id", "version", "include", "exclude"].contains(&k.as_str()))
                {
                    return Err("Unknown Nuspec dependency field".into());
                }
                xml.push_str(&format!("            <mstns:dependency {attrs}/>\n"));
            }
            xml.push_str("        </dependencies>\n");
        } else {
            xml.push_str("        <dependencies/>\n");
        }
    } else {
        xml.push_str("        <dependencies/>\n");
    }
    xml.push_str("    </metadata>\n</package>\n");
    Ok(xml)
}

pub fn install_script(env: &Dictionary) -> Result<String, String> {
    let id = component(env, "id")?;
    let kind = text(env, "installer_type")?;
    if !["exe", "msi", "msu", "zip"].contains(&kind) {
        return Err("Invalid installer_type".into());
    }
    let args = match env.get("installer_args") {
        None if kind == "msi" => "/qn /norestart /l*v MsiInstall.log".into(),
        None => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(a)) => a
            .iter()
            .map(|v| {
                v.as_string()
                    .ok_or("installer_args entries must be strings")
            })
            .collect::<Result<Vec<_>, _>>()?
            .join(" "),
        _ => return Err("Variable `installer_args` must have type list or string".into()),
    };
    let mut fields = vec![
        ("packageName", quoted(id)),
        ("fileType", quoted(kind)),
        ("silentArgs", quoted(&args)),
    ];
    let mut preamble = String::new();
    let local = !env.contains_key("installer_url");
    if local {
        let file = Path::new(text(env, "installer_path")?)
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or("Invalid installer filename")?;
        preamble = format!("$file = Join-Path $toolsDir {}", quoted(file));
        fields.push(("file", "$file".into()));
    } else {
        let checksum = text(env, "installer_checksum")?;
        if checksum.is_empty() {
            return Err(
                "Variable `installer_checksum` is required when `installer_url` is provided."
                    .into(),
            );
        }
        let checksum_type = env
            .get("installer_checksum_type")
            .and_then(Value::as_string)
            .unwrap_or("sha512");
        if !["md5", "sha1", "sha256", "sha512"].contains(&checksum_type) {
            return Err("Invalid installer_checksum_type".into());
        }
        fields.extend([
            ("url", quoted(text(env, "installer_url")?)),
            ("checksum", quoted(checksum)),
            ("checksumType", quoted(checksum_type)),
        ]);
    }
    let mut script = format!("$ErrorActionPreference = 'Stop'\n$toolsDir = \"$(Split-Path -Parent $MyInvocation.MyCommand.Definition)\"\n{preamble}\n$packageArgs = @{{\n");
    for (key, value) in fields {
        script.push_str(&format!("  {key} = {value}\n"));
    }
    script.push_str("}\n\n");
    script.push_str(match (local, kind) {
        (true, "zip") => "Get-ChocolateyUnzip @packageArgs -Destination $toolsDir\n\n",
        (false, "zip") => "Install-ChocolateyZipPackage @packageArgs -Destination $toolsDir\n\n",
        (true, _) => "Install-ChocolateyInstallPackage @packageArgs\n\n",
        (false, _) => "Install-ChocolateyPackage @packageArgs\n\n",
    });
    if env.contains_key("additional_install_actions") {
        script.push_str(text(env, "additional_install_actions")?);
    }
    Ok(script)
}

fn default_installer_path(env: &mut Dictionary) -> Result<(), String> {
    if !env.contains_key("installer_url") && !env.contains_key("installer_path") {
        let path = env
            .get("pathname")
            .and_then(Value::as_string)
            .ok_or("ChocolateyPackager requires an installer_path or pathname variable")?;
        let path = std::path::absolute(path).map_err(|e| e.to_string())?;
        if !path.is_file() {
            return Err(format!("installer_path not found at: {}", path.display()));
        }
        env.insert(
            "installer_path".into(),
            path.to_string_lossy().into_owned().into(),
        );
    }
    Ok(())
}

pub fn execute(env: &mut Dictionary) -> Result<(), String> {
    if !cfg!(windows) {
        return Err("Chocolatey packaging is only supported on Windows.".into());
    }
    let id = component(env, "id")?.to_owned();
    let version = component(env, "version")?.to_owned();
    let executable = std::path::absolute(text(env, "chocoexe_path")?).map_err(|e| e.to_string())?;
    if !executable.exists() {
        return Err(format!(
            "chocoexe_path not found at: {}",
            executable.display()
        ));
    }
    env.insert(
        "chocoexe_path".into(),
        executable.to_string_lossy().into_owned().into(),
    );
    if env.contains_key("installer_url") && env.contains_key("installer_path") {
        return Err("Variables `installer_url`, `installer_path` conflict. Provide one.".into());
    }
    default_installer_path(env)?;
    let specification = nuspec(env)?;
    let script = install_script(env)?;
    let cache = PathBuf::from(text(env, "RECIPE_CACHE_DIR")?);
    let output = env
        .get("output_directory")
        .and_then(Value::as_string)
        .map(PathBuf::from)
        .unwrap_or_else(|| cache.join("nupkgs"));
    let output = std::path::absolute(output).map_err(|e| e.to_string())?;
    let builds = std::path::absolute(cache.join("builds")).map_err(|e| e.to_string())?;
    fs::create_dir_all(&output).map_err(|e| e.to_string())?;
    fs::create_dir_all(&builds).map_err(|e| e.to_string())?;
    let build = tempfile::Builder::new()
        .prefix(&format!("{id}."))
        .tempdir_in(builds)
        .map_err(|e| e.to_string())?;
    let path = build.path().to_owned();
    let keep = env.get("KEEP_BUILD_DIRECTORY").is_some_and(|v| match v {
        Value::Null => false,
        Value::Boolean(b) => *b,
        Value::String(s) => !s.is_empty(),
        _ => true,
    });
    let retained = if keep { Some(build.keep()) } else { None };
    // Retain a cleanup guard when requested; otherwise the original TempDir
    // lives until all writes and the native pack command have completed.
    let _ = &retained;
    let result = execute_build(
        env,
        &id,
        &version,
        &executable,
        &output,
        &path,
        &specification,
        &script,
    );
    if keep {
        super::processor_output(0, format!("Preserved build directory: {}", path.display()));
    } else {
        fs::remove_dir_all(&path).map_err(|e| e.to_string())?;
        super::processor_output(0, "Cleaned up build directory.");
    }
    result
}

#[allow(clippy::too_many_arguments)]
fn execute_build(
    env: &mut Dictionary,
    id: &str,
    version: &str,
    executable: &Path,
    output: &Path,
    build: &Path,
    specification: &str,
    script: &str,
) -> Result<(), String> {
    let tools = build.join("tools");
    fs::create_dir(&tools).map_err(|e| e.to_string())?;
    let specification_path = build.join(format!("{id}.nuspec"));
    fs::write(&specification_path, package_text(specification)).map_err(|e| e.to_string())?;
    fs::write(tools.join("chocolateyInstall.ps1"), package_text(script))
        .map_err(|e| e.to_string())?;
    if let Some(file) = env.get("installer_path").and_then(Value::as_string) {
        let name = Path::new(file)
            .file_name()
            .ok_or("Installer has no filename")?;
        fs::copy(file, tools.join(name)).map_err(|e| e.to_string())?;
        fs::write(tools.join(format!("{}.ignore", name.to_string_lossy())), [])
            .map_err(|e| e.to_string())?;
    }
    env.insert(
        "chocolatey_packager_summary_result".into(),
        Value::Dictionary(Dictionary::new()),
    );
    super::processor_output(
        0,
        format!(
            "Building package {} version {}",
            plist::python_str(&env["id"]),
            plist::python_str(&env["version"])
        ),
    );
    let mut command = Command::new(executable);
    command
        .arg("pack")
        .arg(specification_path)
        .arg(format!("--output-directory={}", output.display()))
        .arg(format!(
            "--log-file={}",
            output.join(format!("{id}.{version}.log")).display()
        ));
    super::processor_output(
        1,
        format!(
            "Running: {} {}",
            executable.display(),
            command
                .get_args()
                .map(|a| a.to_string_lossy())
                .collect::<Vec<_>>()
                .join(" ")
        ),
    );
    let mut combined = tempfile::tempfile().map_err(|e| e.to_string())?;
    command
        .stdout(combined.try_clone().map_err(|e| e.to_string())?)
        .stderr(combined.try_clone().map_err(|e| e.to_string())?);
    let result = command.output().map_err(|e| e.to_string())?;
    use std::io::{Read, Seek};
    combined.rewind().map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    combined
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    let tool_output = String::from_utf8_lossy(&bytes);
    for line in tool_output.lines() {
        super::processor_output(1, line);
    }
    if !result.status.success() {
        return Err(format!("Chocolatey packaging failed: {tool_output}"));
    }
    let package = output.join(format!("{id}.{version}.nupkg"));
    if !package.is_file() {
        return Err(format!("Expected package not found: {}", package.display()));
    }
    let package = package.to_string_lossy().into_owned();
    super::processor_output(0, format!("Wrote Nuget package to: {package}"));
    env.insert("nuget_package_path".into(), package.clone().into());
    env.insert(
        "choco_build_directory".into(),
        build.to_string_lossy().into_owned().into(),
    );
    env.insert(
        "chocolatey_packager_summary_result".into(),
        Value::Dictionary(Dictionary::from_iter([
            (
                "summary_text",
                Value::String("The following packages were built:".into()),
            ),
            (
                "report_fields",
                Value::Array(vec![
                    "identifier".into(),
                    "version".into(),
                    "pkg_path".into(),
                ]),
            ),
            (
                "data",
                Value::Dictionary(Dictionary::from_iter([
                    ("identifier", Value::String(id.into())),
                    ("version", version.into()),
                    ("pkg_path", package.into()),
                ])),
            ),
        ])),
    );
    Ok(())
}

fn package_text(text: &str) -> String {
    // Python writes both generated files using open(..., "w").
    if cfg!(windows) {
        text.replace('\n', "\r\n")
    } else {
        text.to_owned()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_fallback_installer_paths_are_normalized() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("payload.zip");
        std::fs::write(&source, b"payload").unwrap();
        let spelling = source.to_string_lossy().replace('\\', "/");
        let mut explicit = plist::Dictionary::from_iter([(
            "installer_path",
            plist::Value::String(spelling.clone()),
        )]);
        super::default_installer_path(&mut explicit).unwrap();
        assert_eq!(
            explicit["installer_path"].as_string(),
            Some(spelling.as_str())
        );
        let mut fallback =
            plist::Dictionary::from_iter([("pathname", plist::Value::String(spelling.clone()))]);
        super::default_installer_path(&mut fallback).unwrap();
        assert_eq!(
            fallback["installer_path"].as_string(),
            Some(std::path::absolute(&spelling).unwrap().to_str().unwrap())
        );
    }
    #[test]
    fn generated_file_text_matches_python_newlines() {
        let input = "generated\nexplicit\r\n";
        assert_eq!(
            super::package_text(input),
            if cfg!(windows) {
                "generated\r\nexplicit\r\r\n"
            } else {
                input
            }
        );
    }
    use super::*;
    fn environment() -> Dictionary {
        Dictionary::from_iter([
            ("id", Value::String("demo".into())),
            ("version", "1.0".into()),
            ("title", "Demo & test".into()),
            ("authors", "Author".into()),
            ("description", "A <package>".into()),
            ("installer_type", "msi".into()),
            ("installer_path", "/tmp/setup's.msi".into()),
        ])
    }
    #[test]
    fn quotes_powershell_and_escapes_xml() {
        let env = environment();
        let script = install_script(&env).unwrap();
        assert!(script.contains("$file = Join-Path $toolsDir 'setup''s.msi'"));
        assert!(script.contains("silentArgs = '/qn /norestart /l*v MsiInstall.log'"));
        let xml = nuspec(&env).unwrap();
        assert!(xml.contains("Demo &amp; test"));
        assert!(xml.contains("A &lt;package&gt;"));
    }
    #[test]
    fn rejects_path_components_before_writing() {
        let mut env = environment();
        env.insert("id".into(), "../escape".into());
        assert!(nuspec(&env).is_err());
    }
    #[test]
    fn structured_fields_match_pinned_python_recipe_value_boundary() {
        // Captured by invoking ChocolateyPackager.nuspec_definition().render_str()
        // from git-archived reference c36e58f8d3d8ddb70b6c2d848d2ceca7f767ce5c.
        // None is omitted. Every non-null case below raises AttributeError on
        // the Python value's missing validate_ method; only custom Python
        // generator objects can supply that method, outside the recipe contract.
        let expected = concat!(
            "<package xmlns:mstns=\"http://schemas.microsoft.com/packaging/2015/06/nuspec.xsd\" xmlns:None=\"http://schemas.microsoft.com/packaging/2015/06/nuspec.xsd\" >\n",
            "    <metadata>\n",
            "        <id>demo</id>\n",
            "        <version>1.0</version>\n",
            "        <title>Demo &amp; test</title>\n",
            "        <authors>Author</authors>\n",
            "        <description>A &lt;package&gt;</description>\n",
            "        <dependencies/>\n",
            "    </metadata>\n",
            "</package>\n",
        );
        let cases = serde_json::json!([
            ["license", null],
            ["contentFiles", null],
            ["license", {"type": "expression", "value": "MIT"}],
            ["license", {"type_": "expression", "valueOf_": "MIT"}],
            ["license", "MIT"],
            ["license", {}],
            ["contentFiles", {"files": [{"include": "**/*"}]}],
            ["contentFiles", [{"include": "**/*"}]],
            ["contentFiles", {}],
        ]);
        for case in cases.as_array().unwrap() {
            let key = case[0].as_str().unwrap();
            let value: Value = serde_json::from_value(case[1].clone()).unwrap();
            let is_null = value.is_null();
            let mut env = environment();
            env.insert(key.into(), value);
            if is_null {
                assert_eq!(nuspec(&env).unwrap(), expected, "{case}");
            } else {
                assert!(nuspec(&env).unwrap_err().contains(key), "{case}");
            }
        }
        let mut env = environment();
        env.insert("license".into(), Value::Null);
        env.insert("contentFiles".into(), Value::Null);
        assert_eq!(nuspec(&env).unwrap(), expected);
    }
    #[test]
    fn matches_frozen_python_rendering() {
        let reference: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../compatibility/chocolatey-render-reference.json"
        ))
        .unwrap();
        for case in reference["cases"].as_array().unwrap() {
            let env: Dictionary = case["environment"]
                .as_object()
                .unwrap()
                .iter()
                .map(|(k, v)| (k.clone(), Value::String(v.as_str().unwrap().into())))
                .collect();
            assert_eq!(nuspec(&env).unwrap(), case["nuspec"].as_str().unwrap());
            assert_eq!(
                install_script(&env).unwrap(),
                case["script"].as_str().unwrap()
            );
        }
    }
}
