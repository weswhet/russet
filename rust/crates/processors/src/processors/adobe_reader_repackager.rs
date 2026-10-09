//! `AdobeReaderRepackager`: repackage Adobe Reader with its preinstall and
//! distribution changes. A native port of the autopkg/recipes processor
//! (Apache-2.0).
//!
//! Inputs and outputs: run `russet processor-info AdobeReaderRepackager`, or see
//! `AdobeReaderRepackager` in `compatibility/community-processors.json`.
use crate::community_legacy::{output, Result};
use plist::Dictionary;
use std::{fs, path::Path, process::Command};

pub(crate) fn modify_distribution(path: &Path) -> Result<()> {
    if !path.exists() {
        return Err("%s not found".into());
    }
    let bytes = fs::read(path).map_err(|e| format!("Can't read {}: {e}", path.display()))?;
    let mut root = xmltree::Element::parse(bytes.as_slice())
        .map_err(|e| format!("Can't read {}: {e}", path.display()))?;
    if !["installer-script", "installer-gui-script"].contains(&root.name.as_str()) {
        return Err("Distribution file is not in the expected format.".into());
    }
    if let Some(index) = root
        .children
        .iter()
        .position(|n| n.as_element().is_some_and(|e| e.name == "domains"))
    {
        root.children.remove(index);
        let output = fs::File::create(path).map_err(|e| e.to_string())?;
        root.write_with_config(
            output,
            xmltree::EmitterConfig::new().write_document_declaration(false),
        )
        .map_err(|e| format!("Could not write {}: {e}", path.display()))?;
    }
    Ok(())
}
pub(crate) fn replace_preinstall(expanded: &Path) -> Result<()> {
    let app = expanded.join("application_mini_7z.pkg");
    if !app.exists() {
        return Err("application_mini_7z.pkg not found!".into());
    }
    let script = app.join("Scripts/preinstall");
    fs::remove_file(&script).map_err(|e| format!("{e} removing {}", script.display()))?;
    let dc = expanded
        .file_name()
        .is_some_and(|n| n.to_string_lossy().starts_with("AcroRdrDC"));
    fs::write(
        &script,
        if dc {
            include_bytes!("../community_readerdc_preinstall").as_slice()
        } else {
            include_bytes!("../community_reader_preinstall").as_slice()
        },
    )
    .map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755))
            .map_err(|e| e.to_string())?;
    }
    let resource = if dc {
        "readerdc_preinstall"
    } else {
        "reader_preinstall"
    };
    output(1, format!("Replaced pkg preinstall script with our custom script at embedded:AdobeReader/package_resources/scripts/{resource}"));
    Ok(())
}
fn pkgutil(args: &[&std::ffi::OsStr]) -> Result<()> {
    let result = Command::new("/usr/sbin/pkgutil")
        .args(args)
        .output()
        .map_err(|e| e.to_string())?;
    if result.status.success() {
        Ok(())
    } else {
        Err(format!(
            "pkgutil failed: {}",
            String::from_utf8_lossy(&result.stderr)
        ))
    }
}
pub(crate) fn execute(env: &mut Dictionary) -> Result<()> {
    let mut mount = crate::dmg::Mount::new(crate::string(env, "dmg_path")?)?;
    let result = (|| {
        let pkg = fs::read_dir(mount.path())
            .map_err(|e| e.to_string())?
            .filter_map(|p| p.ok())
            .map(|p| p.path())
            .find(|p| {
                p.file_name()
                    .is_some_and(|n| n.to_string_lossy().ends_with(".pkg"))
            })
            .ok_or_else(|| format!("No package found in {}", mount.path().display()))?;
        let cache = Path::new(crate::string(env, "RECIPE_CACHE_DIR")?);
        let expanded = cache.join(pkg.file_stem().ok_or("Invalid package name")?);
        let output = cache.join(pkg.file_name().ok_or("Invalid package name")?);
        if expanded.is_dir() {
            fs::remove_dir_all(&expanded).map_err(|e| e.to_string())?;
        }
        pkgutil(&["--expand".as_ref(), pkg.as_os_str(), expanded.as_os_str()])?;
        modify_distribution(&expanded.join("Distribution"))?;
        replace_preinstall(&expanded)?;
        if output.exists() {
            fs::remove_file(&output).map_err(|e| e.to_string())?;
        }
        pkgutil(&[
            "--flatten".as_ref(),
            expanded.as_os_str(),
            output.as_os_str(),
        ])?;
        env.insert(
            "pkg_path".into(),
            output.to_string_lossy().into_owned().into(),
        );
        Ok(())
    })();
    mount.detach().and(result)
}
