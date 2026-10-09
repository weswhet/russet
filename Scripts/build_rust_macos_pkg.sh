#!/bin/bash
# Build Russet's macOS installer package from the two verified release
# archives. The package installs only the executable, at
# /usr/local/bin/russet. It doesn't set up the launchd helpers;
# `sudo russet --install-helpers` does that. The executable is the two signed
# archive slices joined with lipo, which keeps each slice's signature.
set -euo pipefail

usage() {
    echo 'Usage: build_rust_macos_pkg.sh --version VERSION --arm64 ARCHIVE --x86_64 ARCHIVE --output PKG [--sign IDENTITY]'
}
fail() { echo "build_rust_macos_pkg: $*" >&2; exit 1; }

version='' arm64='' x86_64='' output='' identity=''
while [[ $# -gt 0 ]]; do
    [[ $# -ge 2 ]] || { usage >&2; exit 64; }
    case "$1" in
        --version) version=$2 ;;
        --arm64) arm64=$2 ;;
        --x86_64) x86_64=$2 ;;
        --output) output=$2 ;;
        --sign) identity=$2 ;;
        *) usage >&2; exit 64 ;;
    esac
    shift 2
done
[[ -n "$version" && -n "$arm64" && -n "$x86_64" && -n "$output" ]] || { usage >&2; exit 64; }
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || fail "version must be MAJOR.MINOR.PATCH: $version"
[[ ! -e "$output" ]] || fail "$output already exists"
mkdir -p "$(dirname "$output")"

readonly identifier=com.github.weswhet.russet
readonly daemons=(server installd)
work=$(mktemp -d "${TMPDIR:-/tmp}/russet-pkg.XXXXXX")
trap 'rm -rf "$work"' EXIT

# Each archive has one top-level folder named after the release and target.
unpack() {
    local archive=$1 target=$2 folder="russet-$version-$2"
    [[ -f "$archive" ]] || fail "archive not found: $archive"
    mkdir "$work/$target"
    tar -xzf "$archive" -C "$work/$target"
    [[ -f "$work/$target/$folder/bin/russet" && ! -L "$work/$target/$folder/bin/russet" ]] ||
        fail "$archive has no $folder/bin/russet"
    echo "$work/$target/$folder"
}
arm=$(unpack "$arm64" aarch64-apple-darwin)
intel=$(unpack "$x86_64" x86_64-apple-darwin)
[[ "$(lipo -archs "$arm/bin/russet")" == arm64 ]] || fail "$arm64 doesn't hold an arm64 executable"
[[ "$(lipo -archs "$intel/bin/russet")" == x86_64 ]] || fail "$x86_64 doesn't hold an x86_64 executable"

payload=$work/payload
mkdir -p "$payload/usr/local/bin"
lipo -create -output "$payload/usr/local/bin/russet" "$arm/bin/russet" "$intel/bin/russet"
chmod 0755 "$payload/usr/local/bin/russet"
if [[ -n "$identity" ]]; then
    # The archives' slices are signed; joining them must not break that.
    codesign --verify --strict --verbose=2 "$payload/usr/local/bin/russet"
fi

# If an administrator already set up the helpers, stop them before replacing
# their executable and start them again afterward. The package never sets
# them up itself, and installing to another volume leaves launchd alone.
scripts=$work/scripts
mkdir "$scripts"
cat > "$scripts/preinstall" <<SH
#!/bin/sh
[ "\$3" = / ] || exit 0
for daemon in ${daemons[*]}; do
    /bin/launchctl bootout "system/$identifier.\$daemon" 2>/dev/null || :
done
exit 0
SH
cat > "$scripts/postinstall" <<SH
#!/bin/sh
[ "\$3" = / ] || exit 0
for daemon in ${daemons[*]}; do
    plist="/Library/LaunchDaemons/$identifier.\$daemon.plist"
    [ ! -f "\$plist" ] || /bin/launchctl bootstrap system "\$plist" || exit 1
done
exit 0
SH
chmod 0755 "$scripts/preinstall" "$scripts/postinstall"

mkdir "$work/component"
pkgbuild --quiet --root "$payload" --identifier "$identifier" --version "$version" \
    --install-location / --scripts "$scripts" --ownership recommended \
    "$work/component/russet.pkg"

# A distribution that names both architectures, so Installer doesn't ask for
# Rosetta on Apple silicon.
cat > "$work/distribution.xml" <<XML
<?xml version="1.0" encoding="utf-8"?>
<installer-gui-script minSpecVersion="2">
    <title>Russet $version</title>
    <options customize="never" require-scripts="false" hostArchitectures="x86_64,arm64"/>
    <domains enable_localSystem="true" enable_anywhere="false" enable_currentUserHome="false"/>
    <choices-outline>
        <line choice="$identifier"/>
    </choices-outline>
    <choice id="$identifier" visible="false">
        <pkg-ref id="$identifier"/>
    </choice>
    <pkg-ref id="$identifier" version="$version" onConclusion="none">russet.pkg</pkg-ref>
</installer-gui-script>
XML
signing=()
[[ -z "$identity" ]] || signing=(--sign "$identity" --timestamp)
productbuild --quiet --distribution "$work/distribution.xml" --package-path "$work/component" \
    --version "$version" ${signing[@]+"${signing[@]}"} "$output"

# Check what the package installs.
pkgutil --expand-full "$output" "$work/expanded"
installed=$work/expanded/russet.pkg/Payload
files="$(cd "$installed" && find . \( -type f -o -type l \) -print)"
[[ "$files" == ./usr/local/bin/russet ]] ||
    fail "the package must install only usr/local/bin/russet, not: $(tr '\n' ' ' <<<"$files")"
[[ "$(lipo -archs "$installed/usr/local/bin/russet")" == "x86_64 arm64" ]] ||
    fail "the packaged executable isn't the universal binary"
if [[ -n "$identity" ]]; then
    pkgutil --check-signature "$output" | grep -Fq "$identity" ||
        fail "$output isn't signed by $identity"
fi
echo "Built $output"
