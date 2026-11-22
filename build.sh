#!/bin/zsh

set -e
cd "${0:a:h}"

APP=Juku
MAC=frontends/macos
TARGETDIR=target
APPDIR=$TARGETDIR/$APP.app/Contents
RUST_TARGET=$(rustc -vV | sed -n 's/^host: //p')

# The oldest macOS it runs on (spec §10): CADisplayLink for an NSView, which
# paces the frames, came in macOS 14. The Info.plists' LSMinimumSystemVersion
# says the same. Rust, clang, metal and actool all take it from here.
export MACOSX_DEPLOYMENT_TARGET=14.0

# The version (VERSION, spec §3.3): MAJOR.MINOR.PATCH, with -dev while it is
# being made. macOS takes numbers only, so the bundle gets MAJOR.MINOR.PATCH
# and the build number: 0 for a -dev version, 1 for a release. The app shows
# JukuVersion: the version, and a -dev build's commit (0.5.0-dev+g046bcd2).
VERSION=$(<VERSION)
if [[ ! $VERSION =~ '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-dev)?$' ]]; then
    echo "VERSION: $VERSION is not MAJOR.MINOR.PATCH, or that and -dev" >&2
    exit 1
fi
SHORT_VERSION=${VERSION%-dev}
SHOWN_VERSION=$VERSION
BUILD_NUMBER=1
if [[ $VERSION == *-dev ]]; then
    BUILD_NUMBER=0
    COMMIT=$(git rev-parse --short HEAD 2>/dev/null) && SHOWN_VERSION=$VERSION+g$COMMIT
fi

# Build output is kept out of Time Machine backups.
if [ ! -d $TARGETDIR ]; then
    mkdir $TARGETDIR
    tmutil addexclusion $TARGETDIR
fi

case "$1" in
    debug)
        CFLAGS=(-O0 -g)
        CARGO_PROFILE=()
        CARGO_OUT=debug
        export RUSTFLAGS="-Zlocation-detail=none"
    ;;
    dist)
        # For other Macs: no tuning for this machine's CPU.
        CFLAGS=(-O2 -fno-stack-protector)
        CARGO_PROFILE=(--release)
        CARGO_OUT=release
        export RUSTFLAGS="-Zlocation-detail=none -Zfmt-debug=none"
    ;;
    *)
        CFLAGS=(-O2 -march=native -fno-stack-protector)
        CARGO_PROFILE=(--release)
        CARGO_OUT=release
        export RUSTFLAGS="-Zlocation-detail=none -Zfmt-debug=none"
    ;;
esac

# _metal <out> <source> [<header>...]: rebuilds when any input is newer.
function _metal() {
    local out=$1 dep
    shift
    for dep in "$@"; do
        if [ ! $out -nt $dep ]; then
            xcrun metal -mmacosx-version-min=$MACOSX_DEPLOYMENT_TARGET -o $out $1
            return
        fi
    done
}

# _link <source> <dest>: hard-links unless dest is already the same file. An
# editor that saves by replacing the file gives the source a new inode.
function _link() {
    [ $1 -ef $2 ] || ln -f $1 $2
}

function _plutil() {
    if [ ! $1 -nt $2 ]; then
        plutil -convert binary1 -o $1 $2
    fi
}

# Emulation core and debug server (one Rust static library, std built from
# source; the server's crate holds the core, spec §12.2)
cargo build \
    -p juku-dap \
    --target $RUST_TARGET \
    -Z build-std=std \
    $CARGO_PROFILE

CORELIB=target/cargo/$RUST_TARGET/$CARGO_OUT/libjuku_dap.a

# Application bundle. Resources are checked on every build, so replaced and
# new files are picked up; a deleted one stays until the bundle is removed.
mkdir -p $APPDIR/MacOS $APPDIR/Resources/roms
_link $MAC/resources/PkgInfo $APPDIR/PkgInfo

for i in resources/roms/*.bin; do
    _link $i $APPDIR/Resources/roms/${i:t}
done

# The strings shared with the other front-ends become each language's
# Localizable.strings, next to the Mac's own .lproj files.
for i in resources/strings/*.strings; do
    mkdir -p $APPDIR/Resources/${i:t:r}.lproj
    _link $i $APPDIR/Resources/${i:t:r}.lproj/Localizable.strings
done

for d in $MAC/resources/*.lproj; do
    mkdir -p $APPDIR/Resources/${d:t}
    for i in $d/*; do
        _link $i $APPDIR/Resources/${d:t}/${i:t}
    done
done

# All Objective-C code is a single translation unit rooted at Main.m
rm -rf $APPDIR/MacOS/$APP.dSYM
clang $CFLAGS -Wl,-dead_strip $MAC/src/Main.m $CORELIB \
    -fobjc-arc -fmodules -mmacosx-version-min=$MACOSX_DEPLOYMENT_TARGET \
    -framework AppKit -framework AudioToolbox -framework Metal -framework QuartzCore \
    -o $APPDIR/MacOS/$APP

# Release builds leave out the symbol table, a sixth of the executable. A
# copy with it, and the same UUID, stays in target/ for symbolicating crash
# reports: atos -o target/Juku.symbols -l <load address> <addresses>.
if [ "$1" != debug ]; then
    cp $APPDIR/MacOS/$APP $TARGETDIR/$APP.symbols
    strip $APPDIR/MacOS/$APP
fi

# Quick Look thumbnail extension for snapshots: a binary of its own, outside
# the single translation unit. macOS loads only a signed, sandboxed
# extension, so it is signed here (ad hoc); package.sh signs it again with
# the real identity. It takes the app's version.
APPEX=$APPDIR/PlugIns/JukuThumbnail.appex
mkdir -p $APPEX/Contents/MacOS
clang $CFLAGS -fobjc-arc -fmodules -fapplication-extension -mmacosx-version-min=$MACOSX_DEPLOYMENT_TARGET \
    -e _NSExtensionMain $MAC/src/thumbnail/Thumbnail.m \
    -framework Foundation -framework ImageIO -framework QuickLookThumbnailing \
    -o $APPEX/Contents/MacOS/JukuThumbnail

if [ "$1" != debug ]; then
    strip $APPEX/Contents/MacOS/JukuThumbnail
fi

plutil -convert binary1 -o $APPEX/Contents/Info.plist $MAC/src/thumbnail/Info.plist
plutil -replace CFBundleShortVersionString -string $SHORT_VERSION $APPEX/Contents/Info.plist
plutil -replace CFBundleVersion -string $BUILD_NUMBER $APPEX/Contents/Info.plist
codesign --force --sign - --entitlements $MAC/src/thumbnail/Thumbnail.entitlements $APPEX

_metal $APPDIR/Resources/default.metallib $MAC/src/Shaders.metal $MAC/src/ShaderTypes.h build.sh

# App icon: actool compiles the Icon Composer bundle into Assets.car and a
# fallback Juku.icns, only when one of its files is newer. The paths are
# absolute because actool hands them to a long-lived service with its own cwd.
for i in $MAC/resources/Juku.icon/**/*(.) build.sh; do
    if [ ! $APPDIR/Resources/Assets.car -nt $i ]; then
        xcrun actool $PWD/$MAC/resources/Juku.icon --compile $PWD/$APPDIR/Resources \
            --app-icon Juku --platform macosx --minimum-deployment-target $MACOSX_DEPLOYMENT_TARGET \
            --output-partial-info-plist $PWD/$TARGETDIR/icon-info.plist \
            --output-format human-readable-text --errors --warnings
        break
    fi
done

_plutil $APPDIR/Info.plist $MAC/resources/Info.plist
plutil -replace CFBundleShortVersionString -string $SHORT_VERSION $APPDIR/Info.plist
plutil -replace CFBundleVersion -string $BUILD_NUMBER $APPDIR/Info.plist
plutil -replace JukuVersion -string $SHOWN_VERSION $APPDIR/Info.plist
