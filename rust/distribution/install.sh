#!/bin/sh
# Install a native archive and retain an exact filesystem rollback generation.
set -eu
umask 022

usage() {
    echo 'Usage: install.sh install|rollback [--root EXISTING_DIRECTORY] [--platform Darwin|Linux]'
}
fail() { echo "russet installer: $*" >&2; exit 1; }
action=${1:-}
[ "$#" -gt 0 ] && shift
root=/
platform=$(uname -s)
while [ "$#" -gt 0 ]; do
    case "$1" in
        --root) [ "$#" -ge 2 ] || fail '--root needs a directory'; root=$2; shift 2 ;;
        --platform) [ "$#" -ge 2 ] || fail '--platform needs a value'; platform=$2; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) fail "Unknown argument: $1" ;;
    esac
done
case "$action" in install|rollback) ;; -h|--help) usage; exit 0 ;; *) usage >&2; exit 2 ;; esac
[ -d "$root" ] || fail 'Installation root must already exist'
root=$(cd "$root" && pwd -P)
if [ "$root" = / ]; then
    [ "$(id -u)" = 0 ] || fail 'A system installation requires root'
    [ "$platform" = "$(uname -s)" ] || fail 'Platform override is only allowed with a staging root'
    prefix=
else
    prefix=$root
fi
case "$platform" in Darwin|Linux) ;; *) fail "Unsupported platform: $platform" ;; esac
destination=$prefix/opt/russet
history=$prefix/opt/russet-rollbacks
command=$prefix/usr/local/bin/russet
daemon_dir=$prefix/Library/LaunchDaemons
packaging=$daemon_dir/com.github.weswhet.russet.server.plist
installation=$daemon_dir/com.github.weswhet.russet.installd.plist
source=$(CDPATH= cd "$(dirname "$0")" && pwd -P)

exists() { [ -e "$1" ] || [ -L "$1" ]; }
unload() {
    if [ "$root" = / ] && [ "$platform" = Darwin ]; then
        /bin/launchctl bootout system "$packaging" >/dev/null 2>&1 || :
        /bin/launchctl bootout system "$installation" >/dev/null 2>&1 || :
    fi
}
load() {
    if [ "$root" = / ] && [ "$platform" = Darwin ]; then
        [ ! -f "$packaging" ] || /bin/launchctl bootstrap system "$packaging"
        [ ! -f "$installation" ] || /bin/launchctl bootstrap system "$installation"
    fi
}
safe_parent() {
    # Do not install through a preexisting symlink to another filesystem tree.
    current=$1
    while [ "$current" != "$root" ] && [ "$current" != / ]; do
        [ ! -L "$current" ] || fail "Installation parent is a symlink: $current"
        current=$(dirname "$current")
    done
}
safe_parent "$(dirname "$destination")"
safe_parent "$(dirname "$command")"
safe_parent "$history"
if [ "$platform" = Darwin ]; then safe_parent "$daemon_dir"; fi

# Move an entry rather than resolving symlinks or changing its metadata.
save_entry() {
    if exists "$1"; then mv "$1" "$generation/$2"; fi
}
restore_entry() {
    if exists "$generation/$2"; then mv "$generation/$2" "$1"; fi
}

if [ "$action" = rollback ]; then
    [ ! -L "$destination" ] || fail 'Installed directory must not be a symlink'
    marker=$destination/.russet-rollback
    [ -f "$marker" ] && [ ! -L "$marker" ] || fail 'No native installation rollback record'
    generation=$(cat "$marker")
    case "$generation" in "$history"/generation.*) ;; *) fail 'Invalid rollback record' ;; esac
    [ "$(dirname "$generation")" = "$history" ] || fail 'Rollback record escapes history directory'
    [ -d "$generation" ] && [ ! -L "$generation" ] || fail 'Rollback generation is missing'
    [ -f "$generation/committed" ] || fail 'Rollback generation was not committed'
    [ ! -e "$generation/retired" ] || fail 'Rollback generation was already used'
    previous_installation=0; previous_command=0; previous_packaging=0; previous_installd=0
    if exists "$generation/previous-installation"; then previous_installation=1; fi
    if exists "$generation/previous-command"; then previous_command=1; fi
    if exists "$generation/previous-packaging.plist"; then previous_packaging=1; fi
    if exists "$generation/previous-installation.plist"; then previous_installd=1; fi
    undo_restore() {
        if [ "$3" = 1 ] && ! exists "$generation/$2" && exists "$1"; then
            mv "$1" "$generation/$2"
        fi
    }
    return_retired() {
        if exists "$generation/retired/$2"; then mv "$generation/retired/$2" "$1"; fi
    }
    recover_rollback() {
        status=${1:-$?}
        trap - EXIT HUP INT TERM
        set +e
        unload
        undo_restore "$destination" previous-installation "$previous_installation"
        undo_restore "$command" previous-command "$previous_command"
        if [ "$platform" = Darwin ]; then
            undo_restore "$packaging" previous-packaging.plist "$previous_packaging"
            undo_restore "$installation" previous-installation.plist "$previous_installd"
        fi
        return_retired "$destination" installation
        return_retired "$command" command
        if [ "$platform" = Darwin ]; then
            return_retired "$packaging" packaging.plist
            return_retired "$installation" installation.plist
        fi
        rmdir "$generation/retired" 2>/dev/null || :
        load
        exit "$status"
    }
    trap recover_rollback EXIT
    trap 'recover_rollback 129' HUP
    trap 'recover_rollback 130' INT
    trap 'recover_rollback 143' TERM
    unload
    mkdir -m 0755 "$generation/retired"
    mv "$destination" "$generation/retired/installation"
    if exists "$command"; then mv "$command" "$generation/retired/command"; fi
    if [ "$platform" = Darwin ]; then
        if exists "$packaging"; then mv "$packaging" "$generation/retired/packaging.plist"; fi
        if exists "$installation"; then mv "$installation" "$generation/retired/installation.plist"; fi
    fi
    restore_entry "$destination" previous-installation
    restore_entry "$command" previous-command
    if [ "$platform" = Darwin ]; then
        restore_entry "$packaging" previous-packaging.plist
        restore_entry "$installation" previous-installation.plist
    fi
    load
    trap - EXIT HUP INT TERM
    echo "Restored previous installation; retired native files retained in $generation/retired"
    exit 0
fi

for binary in russet; do
    [ -f "$source/bin/$binary" ] && [ ! -L "$source/bin/$binary" ] && [ -x "$source/bin/$binary" ] || fail "Missing native executable: $binary"
done
if [ "$platform" = Darwin ]; then
    for plist in russet-server.plist russet-installd.plist; do
        [ -f "$source/launchd/$plist" ] && [ ! -L "$source/launchd/$plist" ] || fail "Missing launchd configuration: $plist"
    done
fi
mkdir -p "$(dirname "$destination")" "$(dirname "$command")" "$history"
[ "$platform" != Darwin ] || mkdir -p "$daemon_dir"
generation=$(mktemp -d "$history/generation.XXXXXXXX")
chmod 0755 "$generation"
stage=$generation/candidate
mkdir -m 0755 "$stage"
cp "$source/bin/russet" "$stage/russet"
cp "$source/install.sh" "$stage/install.sh"
chmod 0755 "$stage/russet" "$stage/install.sh"
printf '%s\n' "$generation" > "$stage/.russet-rollback"
if [ "$platform" = Darwin ]; then
    cp "$source/launchd/russet-server.plist" "$generation/next-packaging.plist"
    cp "$source/launchd/russet-installd.plist" "$generation/next-installation.plist"
    chmod 0644 "$generation/next-"*.plist
fi

changed=0
recover() {
    status=${1:-$?}
    trap - EXIT HUP INT TERM
    if [ "$changed" = 1 ]; then
        unload
        # Preserve the failed candidate for diagnostics; never delete the old release.
        if [ -f "$destination/.russet-rollback" ] && [ "$(cat "$destination/.russet-rollback")" = "$generation" ]; then
            mv "$destination" "$generation/failed-candidate"
        fi
        if [ -f "$generation/command-installed" ] && exists "$command"; then mv "$command" "$generation/failed-command"; fi
        if [ -f "$generation/daemons-installed" ]; then
            if exists "$packaging"; then mv "$packaging" "$generation/failed-packaging.plist"; fi
            if exists "$installation"; then mv "$installation" "$generation/failed-installation.plist"; fi
        fi
        restore_entry "$destination" previous-installation
        restore_entry "$command" previous-command
        if [ "$platform" = Darwin ]; then
            restore_entry "$packaging" previous-packaging.plist
            restore_entry "$installation" previous-installation.plist
        fi
        load || :
    fi
    exit "$status"
}
trap recover EXIT
trap 'recover 129' HUP
trap 'recover 130' INT
trap 'recover 143' TERM
changed=1
unload
save_entry "$destination" previous-installation
save_entry "$command" previous-command
if [ "$platform" = Darwin ]; then
    save_entry "$packaging" previous-packaging.plist
    save_entry "$installation" previous-installation.plist
fi
mv "$stage" "$destination"
if [ "$platform" = Darwin ]; then
    : > "$generation/daemons-installed"
    mv "$generation/next-packaging.plist" "$packaging"
    mv "$generation/next-installation.plist" "$installation"
fi
: > "$generation/command-installed"
ln -s "$destination/russet" "$command"
load
: > "$generation/committed"
changed=0
trap - EXIT HUP INT TERM
echo "Installed Russet; rollback: $destination/install.sh rollback"
