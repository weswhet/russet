use crate::{restart_action, RestartAction};
use russet_xar::{Builder, Content, Encoding};
use std::path::Path;

fn component(dir: &Path, name: &str, action: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    let mut builder = Builder::new();
    let info = format!(
        "<pkg-info identifier=\"com.example\" version=\"1\" postinstall-action=\"{action}\"/>"
    );
    builder
        .add_file(
            Path::new("PackageInfo"),
            0o644,
            Content::Bytes(info.into_bytes()),
            Encoding::Zlib,
        )
        .unwrap();
    builder.write(&path).unwrap();
    path
}

fn product(dir: &Path, name: &str, conclusions: &[&str]) -> std::path::PathBuf {
    let path = dir.join(name);
    let refs: String = conclusions
        .iter()
        .enumerate()
        .map(|(i, c)| format!("<pkg-ref id=\"p{i}\" onConclusion=\"{c}\">p{i}.pkg</pkg-ref>"))
        .collect();
    let xml = format!("<installer-gui-script minSpecVersion=\"2\">{refs}</installer-gui-script>");
    let mut builder = Builder::new();
    builder
        .add_file(
            Path::new("Distribution"),
            0o644,
            Content::Bytes(xml.into_bytes()),
            Encoding::Bzip2,
        )
        .unwrap();
    builder.add_directory(Path::new("p0.pkg"), 0o755).unwrap();
    builder
        .add_file(
            Path::new("p0.pkg/PackageInfo"),
            0o644,
            Content::Bytes(b"<pkg-info postinstall-action=\"shutdown\"/>".to_vec()),
            Encoding::Zlib,
        )
        .unwrap();
    builder.write(&path).unwrap();
    path
}

/// The cases checked against `installer` on macOS.
#[test]
fn matches_installer_rules() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    for (action, expected) in [
        ("none", RestartAction::None),
        ("logout", RestartAction::RequireLogout),
        ("restart", RestartAction::RequireRestart),
        ("shutdown", RestartAction::RequireShutdown),
    ] {
        assert_eq!(
            restart_action(&component(dir, &format!("c-{action}.pkg"), action)).unwrap(),
            expected
        );
    }
    for (conclusions, expected) in [
        (&["None"][..], RestartAction::None),
        (
            &["RequireLogout", "RequireRestart"],
            RestartAction::RequireRestart,
        ),
        (
            &["RecommendRestart", "RequireLogout"],
            RestartAction::RequireLogout,
        ),
        (
            &["RequireShutdown", "RequireRestart"],
            RestartAction::RequireShutdown,
        ),
        (&["RecommendRestart"], RestartAction::RecommendRestart),
        (&["RequireLogOut"], RestartAction::RequireLogout),
        // The component asks for shutdown, but a product reports only its
        // Distribution's onConclusion.
        (&[][..], RestartAction::None),
    ] {
        let name = format!("p-{}.pkg", conclusions.join("-"));
        assert_eq!(
            restart_action(&product(dir, &name, conclusions)).unwrap(),
            expected,
            "{conclusions:?}"
        );
    }
}

/// Compares with `installer -query RestartAction` on packages that
/// `pkgbuild` and `productbuild` create.
#[cfg(target_os = "macos")]
#[test]
fn matches_apple_installer() {
    use std::process::Command;
    let temp = tempfile::tempdir().unwrap();
    let t = |p: &str| temp.path().join(p);
    std::fs::create_dir_all(t("root/tmp")).unwrap();
    std::fs::write(t("root/tmp/f"), "x").unwrap();
    let query = |pkg: &Path| -> String {
        let out = Command::new("/usr/sbin/installer")
            .args(["-query", "RestartAction", "-pkg"])
            .arg(pkg)
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    };
    for action in ["none", "logout", "restart", "shutdown"] {
        std::fs::write(
            t(&format!("{action}.xml")),
            format!("<pkg-info postinstall-action=\"{action}\"/>"),
        )
        .unwrap();
        let pkg = t(&format!("c-{action}.pkg"));
        assert!(Command::new("/usr/bin/pkgbuild")
            .args(["--quiet", "--root"])
            .arg(t("root"))
            .args([
                "--identifier",
                &format!("com.example.{action}"),
                "--version",
                "1",
                "--info"
            ])
            .arg(t(&format!("{action}.xml")))
            .arg(&pkg)
            .status()
            .unwrap()
            .success());
        assert_eq!(
            restart_action(&pkg).unwrap().as_str(),
            query(&pkg),
            "{action}"
        );
        let product = t(&format!("p-{action}.pkg"));
        assert!(Command::new("/usr/bin/productbuild")
            .args(["--quiet", "--package"])
            .arg(&pkg)
            .arg(&product)
            .status()
            .unwrap()
            .success());
        assert_eq!(
            restart_action(&product).unwrap().as_str(),
            query(&product),
            "product {action}"
        );
    }
}
