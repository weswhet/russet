//! `cargo xtask formula`: render the Homebrew formula for a published release
//! from its `SHA256SUMS`.

use crate::promote::{is_lower_hex, is_version, REPOSITORY};
use std::collections::BTreeMap;

/// The archives the formula installs, by Homebrew platform block.
const ARCHIVES: [(&str, &str, &str); 3] = [
    ("macos", "arm", "aarch64-apple-darwin"),
    ("macos", "intel", "x86_64-apple-darwin"),
    ("linux", "intel", "x86_64-unknown-linux-gnu"),
];

/// Read `sha256sum` output: a digest, a space, a space or `*`, and a name.
fn checksums(text: &str) -> Result<BTreeMap<&str, &str>, String> {
    let mut sums = BTreeMap::new();
    for line in text.lines().filter(|line| !line.is_empty()) {
        let (digest, name) = line
            .split_once(' ')
            .ok_or_else(|| format!("Malformed checksum line: {line}"))?;
        let name = name
            .strip_prefix(' ')
            .or_else(|| name.strip_prefix('*'))
            .ok_or_else(|| format!("Malformed checksum line: {line}"))?;
        if !is_lower_hex(digest, 64) {
            return Err(format!("Malformed SHA-256 digest for {name}"));
        }
        if sums.insert(name, digest).is_some() {
            return Err(format!("Duplicate checksum for {name}"));
        }
    }
    Ok(sums)
}

/// Render `Formula/russet.rb` for release `version` from its `SHA256SUMS`.
pub fn render(version: &str, sums: &str) -> Result<String, String> {
    if !is_version(version) {
        return Err(format!("Version must be MAJOR.MINOR.PATCH, not {version}"));
    }
    let sums = checksums(sums)?;
    let base = format!("https://github.com/{REPOSITORY}/releases/download/rust-v{version}");
    let mut blocks = String::new();
    for os in ["macos", "linux"] {
        blocks.push_str(&format!("  on_{os} do\n"));
        for (_, cpu, target) in ARCHIVES.iter().filter(|(name, _, _)| *name == os) {
            let archive = format!("russet-{version}-{target}.tar.gz");
            let digest = sums
                .get(archive.as_str())
                .ok_or_else(|| format!("SHA256SUMS has no entry for {archive}"))?;
            blocks.push_str(&format!(
                "    on_{cpu} do\n      url \"{base}/{archive}\"\n      sha256 \"{digest}\"\n    end\n"
            ));
        }
        blocks.push_str("  end\n");
    }
    Ok(format!(
        r##"class Russet < Formula
  desc "Rust implementation of the AutoPkg recipe interface"
  homepage "https://weswhet.github.io/russet/"
  version "{version}"
  license "Apache-2.0"

  livecheck do
    url :stable
    regex(/^rust-v?(\d+(?:\.\d+)+)$/i)
    strategy :github_latest
  end

{blocks}
  def install
    bin.install "bin/russet"
    prefix.install "LICENSE.txt", "licenses"
  end

  def caveats
    on_macos do
      <<~EOS
        Homebrew installs only the russet command. On macOS, the PkgCreator
        and Installer processors also need the russet-server and
        russet-installd launchd helpers, which run as root. To get them,
        install the Russet package from the GitHub release instead.
      EOS
    end
  end

  test do
    assert_equal "3.0.0", shell_output("#{{bin}}/russet version").strip
  end
end
"##
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sums() -> String {
        [
            ("a", "gate-evidence.json"),
            ("b", "russet-1.2.3-aarch64-apple-darwin.tar.gz"),
            ("c", "russet-1.2.3-x86_64-apple-darwin.tar.gz"),
            ("d", "russet-1.2.3-x86_64-pc-windows-msvc.zip"),
            ("e", "russet-1.2.3-x86_64-unknown-linux-gnu.tar.gz"),
        ]
        .iter()
        .map(|(digit, name)| format!("{}  {name}\n", digit.repeat(64)))
        .collect()
    }

    #[test]
    fn formula_names_each_archive_with_its_digest() {
        let formula = render("1.2.3", &sums()).unwrap();
        let base = "https://github.com/weswhet/russet/releases/download/rust-v1.2.3";
        for (target, digit) in [
            ("aarch64-apple-darwin", "b"),
            ("x86_64-apple-darwin", "c"),
            ("x86_64-unknown-linux-gnu", "e"),
        ] {
            let entry = format!(
                "url \"{base}/russet-1.2.3-{target}.tar.gz\"\n      sha256 \"{}\"",
                digit.repeat(64)
            );
            assert!(formula.contains(&entry), "{target}");
        }
        assert!(formula.contains("version \"1.2.3\""));
        assert!(formula.contains("\"#{bin}/russet version\""));
        assert!(!formula.contains("windows"));
    }

    #[test]
    fn formula_rejects_missing_or_malformed_checksums() {
        let missing: String = sums()
            .lines()
            .filter(|line| !line.contains("linux"))
            .map(|line| format!("{line}\n"))
            .collect();
        assert!(render("1.2.3", &missing)
            .unwrap_err()
            .contains("x86_64-unknown-linux-gnu"));
        let short = sums().replacen(&"b".repeat(64), "bb", 1);
        assert!(render("1.2.3", &short).unwrap_err().contains("Malformed"));
        let duplicate = format!("{}{}", sums(), sums().lines().next().unwrap());
        assert!(render("1.2.3", &duplicate)
            .unwrap_err()
            .contains("Duplicate"));
        assert!(render("1.2", &sums()).is_err());
    }
}
