#!/bin/sh
# Execute inside a fresh disposable Linux container with no Python installed.
set -eu
[ "${AUTOPKG_DISPOSABLE_CONTAINER:-}" = 1 ] && [ -f /.dockerenv ] || {
    echo 'Disposable container opt-in required' >&2; exit 1;
}
[ "$(id -u)" = 0 ] || { echo 'Container root required' >&2; exit 1; }
[ "$#" = 1 ] || { echo 'Usage: verify_rust_linux.sh EXTRACTED_ARCHIVE_DIRECTORY' >&2; exit 1; }
[ ! -e /usr/local/lib/autopkg ] && [ ! -e /usr/local/bin/autopkg ] || exit 1
if command -v python || command -v python3; then
    echo 'Clean runtime check requires Python to be absent' >&2; exit 1
fi
archive=$1
work=$(mktemp -d)
printf '<?xml version="1.0"?><plist version="1.0"><dict/></plist>\n' > "$work/preferences.plist"
sh "$archive/install.sh" install
env PATH=/nonexistent /usr/local/bin/autopkg version
cat > "$work/create.plist" <<EOF
<?xml version="1.0"?><plist version="1.0"><dict>
<key>file_path</key><string>$work/native.txt</string>
<key>file_content</key><string>native Linux fixture</string>
</dict></plist>
EOF
env PATH=/nonexistent AUTOPKG_RS_PREFERENCES_FILE="$work/preferences.plist" \
    /usr/local/bin/autopkg processor-run FileCreator < "$work/create.plist" > "$work/result.plist"
[ "$(cat "$work/native.txt")" = 'native Linux fixture' ]
grep -q '<plist' "$work/result.plist"
cat > "$work/native.recipe" <<EOF
<?xml version="1.0"?><plist version="1.0"><dict>
<key>Identifier</key><string>org.autopkg.linux.native-gate</string>
<key>Input</key><dict><key>CACHE_DIR</key><string>$work/cache</string></dict>
<key>Process</key><array>
<dict><key>Processor</key><string>FileCreator</string><key>Arguments</key><dict>
<key>file_path</key><string>$work/recipe.txt</string>
<key>file_content</key><string>native Linux recipe</string>
</dict></dict></array></dict></plist>
EOF
env PATH=/nonexistent AUTOPKG_RS_CACHE_DIR="$work/cache" \
    /usr/local/bin/autopkg run --prefs "$work/preferences.plist" "$work/native.recipe"
[ "$(cat "$work/recipe.txt")" = 'native Linux recipe' ]
[ -f "$work/cache/autopkg_results.plist" ]
sh "$archive/install.sh" install
sh /usr/local/lib/autopkg/install.sh rollback
env PATH=/nonexistent /usr/local/bin/autopkg version
sh /usr/local/lib/autopkg/install.sh rollback
[ ! -e /usr/local/lib/autopkg ] && [ ! -L /usr/local/bin/autopkg ]
echo 'Clean Linux installation, processor, recipe, upgrade, and rollback checks passed.'
