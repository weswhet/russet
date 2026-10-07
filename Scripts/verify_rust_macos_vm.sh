#!/bin/sh
# Run only inside a disposable Apple VM or GitHub-hosted macOS runner.
set -eu
if [ "${AUTOPKG_DISPOSABLE_CI:-}" = 1 ]; then
    [ "${GITHUB_ACTIONS:-}" = true ] && [ "${RUNNER_ENVIRONMENT:-}" = github-hosted ] && \
        [ "${RUNNER_OS:-}" = macOS ] || { echo 'Disposable GitHub-hosted macOS runner required' >&2; exit 1; }
else
    [ "${AUTOPKG_DISPOSABLE_VM:-}" = 1 ] || { echo 'Disposable VM opt-in required' >&2; exit 1; }
    case "$(/usr/sbin/sysctl -n hw.model)" in VirtualMac*) ;; *) echo 'This check requires a VirtualMac guest' >&2; exit 1 ;; esac
fi
[ "$(/usr/bin/id -u)" = 0 ] || { echo 'Guest root required' >&2; exit 1; }
[ "$#" = 1 ] || { echo 'Usage: verify_rust_macos_vm.sh EXTRACTED_ARCHIVE_DIRECTORY' >&2; exit 1; }
archive=$1
[ -f "$archive/install.sh" ] || exit 1
[ ! -e /Library/AutoPkg ] || { echo 'Guest already has AutoPkg; refusing to replace it' >&2; exit 1; }
fixture=/usr/local/share/autopkg-rust-vm-fixture
[ ! -e "$fixture" ] || { echo 'Guest already has validation payload' >&2; exit 1; }
work=$(/usr/bin/mktemp -d /private/tmp/autopkg-rust-validation.XXXXXXXX)
/bin/chmod 0755 "$work"
echo "Validation directory: $work"
/usr/bin/sw_vers
/usr/sbin/sysctl -n hw.model
if [ "${AUTOPKG_DISPOSABLE_CI:-}" != 1 ]; then
    for python in /usr/local/bin/python /usr/local/bin/python3 /opt/homebrew/bin/python3 /Library/AutoPkg/Python3; do
        [ ! -e "$python" ] || { echo "Unexpected Python installation: $python" >&2; exit 1; }
    done
fi
/bin/sh "$archive/install.sh" install
/bin/launchctl print system/com.github.autopkgserver > "$work/packaging-launchd.txt"
/bin/launchctl print system/com.github.autopkg.autopkginstalld > "$work/installation-launchd.txt"
/usr/bin/env PATH=/nonexistent /usr/local/bin/autopkg version
/bin/mkdir -p "$work/cache" "$work/payload/usr/local/share" "$work/home"
printf 'native helper payload\n' > "$work/payload/usr/local/share/autopkg-rust-vm-fixture"
printf '<?xml version="1.0"?><plist version="1.0"><dict/></plist>\n' > "$work/preferences.plist"
/usr/sbin/chown -R nobody:nobody "$work/cache" "$work/payload" "$work/home"
cat > "$work/package.input.plist" <<EOF
<?xml version="1.0"?><plist version="1.0"><dict>
<key>RECIPE_CACHE_DIR</key><string>$work/cache</string>
<key>pkg_request</key><dict>
<key>pkgroot</key><string>$work/payload</string>
<key>pkgdir</key><string>$work/cache</string>
<key>pkgname</key><string>NativeFixture</string>
<key>pkgtype</key><string>flat</string>
<key>id</key><string>org.autopkg.rust.vmfixture</string>
<key>version</key><string>1.0</string>
</dict></dict></plist>
EOF
run_processor() {
    /usr/bin/sudo -u nobody /usr/bin/env PATH=/nonexistent HOME="$work/home" \
        AUTOPKG_RS_PREFERENCES_FILE="$work/preferences.plist" \
        /usr/local/bin/autopkg processor-run "$1" < "$2" > "$3"
}
run_processor PkgCreator "$work/package.input.plist" "$work/package.output.plist"
[ -f "$work/cache/NativeFixture.pkg" ]
cat > "$work/install.input.plist" <<EOF
<?xml version="1.0"?><plist version="1.0"><dict>
<key>RECIPE_CACHE_DIR</key><string>$work/cache</string>
<key>pkg_path</key><string>$work/cache/NativeFixture.pkg</string>
</dict></plist>
EOF
run_processor Installer "$work/install.input.plist" "$work/install.output.plist"
[ "$(/usr/bin/plutil -extract install_result raw -o - "$work/install.output.plist")" = DONE ]
/usr/bin/cmp "$fixture" "$work/payload/usr/local/share/autopkg-rust-vm-fixture"
/usr/sbin/pkgutil --pkg-info org.autopkg.rust.vmfixture > "$work/installed-receipt.txt"

/bin/mkdir -p "$work/NativeFixture.app/Contents"
cat > "$work/NativeFixture.app/Contents/Info.plist" <<EOF
<?xml version="1.0"?><plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>org.autopkg.rust.vmappfixture</string>
<key>CFBundleName</key><string>NativeFixture</string>
<key>CFBundleShortVersionString</key><string>2.0</string>
<key>CFBundleVersion</key><string>2.0</string>
<key>CFBundlePackageType</key><string>APPL</string>
</dict></plist>
EOF
/usr/sbin/chown -R nobody:nobody "$work/NativeFixture.app"
cat > "$work/app.input.plist" <<EOF
<?xml version="1.0"?><plist version="1.0"><dict>
<key>RECIPE_CACHE_DIR</key><string>$work/cache</string>
<key>app_path</key><string>$work/NativeFixture.app</string>
</dict></plist>
EOF
run_processor AppPkgCreator "$work/app.input.plist" "$work/app.output.plist"
app_package=$(/usr/bin/plutil -extract pkg_path raw -o - "$work/app.output.plist")
[ -f "$app_package" ]
/usr/sbin/pkgutil --expand "$app_package" "$work/app-package-expanded"
/usr/bin/grep -q 'org.autopkg.rust.vmappfixture' "$work/app-package-expanded/PackageInfo"

# Kernel peer credentials must reject a request for a payload owned by another UID.
/usr/sbin/chown root:wheel "$work/payload"
if run_processor PkgCreator "$work/package.input.plist" "$work/rejected.output.plist" 2> "$work/rejected.stderr"; then
    # Cache reuse is a valid shortcut; force a new package before this rejection check.
    /bin/rm "$work/cache/NativeFixture.pkg"
fi
if run_processor PkgCreator "$work/package.input.plist" "$work/rejected.output.plist" 2> "$work/rejected.stderr"; then
    echo 'Packaging helper accepted another user payload' >&2; exit 1
fi
/usr/bin/grep -q "isn't owned by" "$work/rejected.stderr"
/usr/sbin/chown nobody:nobody "$work/payload"
printf 'not a property list' | /usr/bin/nc -w 5 -U /var/run/autopkgserver > "$work/malformed.reply"
/usr/bin/grep -q 'ERROR:Malformed request' "$work/malformed.reply"

# Exercise the privileged image copier with an actual mounted disk image.
/bin/mkdir "$work/image-source"
printf 'native image copy\n' > "$work/image-source/copied.txt"
/usr/bin/hdiutil create -srcfolder "$work/image-source" -format UDZO "$work/fixture.dmg"
cat > "$work/dmg.input.plist" <<EOF
<?xml version="1.0"?><plist version="1.0"><dict>
<key>dmg_path</key><string>$work/fixture.dmg</string>
<key>items_to_copy</key><array><dict>
<key>source_item</key><string>copied.txt</string>
<key>destination_path</key><string>$work/copied</string>
</dict></array></dict></plist>
EOF
/usr/bin/env PATH=/usr/bin:/bin:/usr/sbin:/sbin HOME="$work/home" \
    AUTOPKG_RS_PREFERENCES_FILE="$work/preferences.plist" \
    /usr/local/bin/autopkg processor-run InstallFromDMG < "$work/dmg.input.plist" > "$work/dmg.output.plist"
[ "$(/usr/bin/plutil -extract install_result raw -o - "$work/dmg.output.plist")" = DONE ]
/usr/bin/cmp "$work/image-source/copied.txt" "$work/copied/copied.txt"

# This preference is confined to the disposable VM; never write it on the host.
/usr/bin/defaults write com.googlecode.munki.munkiimport default_catalog -string FixtureStable
/usr/bin/env PATH=/nonexistent HOME="$work/home" \
    AUTOPKG_RS_PREFERENCES_FILE="$work/preferences.plist" \
    /usr/local/bin/autopkg processor-run MunkiSetDefaultCatalog < "$work/preferences.plist" > "$work/catalog.output.plist"
[ "$(/usr/bin/plutil -extract pkginfo.catalogs.0 raw -o - "$work/catalog.output.plist")" = FixtureStable ]
/usr/bin/defaults delete com.googlecode.munki.munkiimport default_catalog

# Upgrade and restore the preceding native generation before undoing the clean install.
/bin/sh "$archive/install.sh" install
/usr/bin/env PATH=/nonexistent /usr/local/bin/autopkg version
/bin/sh /Library/AutoPkg/install.sh rollback
/usr/bin/env PATH=/nonexistent /usr/local/bin/autopkg version
/bin/sh /Library/AutoPkg/install.sh rollback
[ ! -e /Library/AutoPkg ] && [ ! -L /usr/local/bin/autopkg ]
if /bin/launchctl print system/com.github.autopkgserver >/dev/null 2>&1; then exit 1; fi
if /bin/launchctl print system/com.github.autopkg.autopkginstalld >/dev/null 2>&1; then exit 1; fi
/bin/rm "$fixture"
/usr/sbin/pkgutil --forget org.autopkg.rust.vmfixture
echo "PASS: clean install, launchd sockets, non-root package and app packaging, installation, ownership rejection, malformed protocol, DMG copying, Munki catalog preferences, upgrade, and rollback"
echo "Evidence retained at $work"
