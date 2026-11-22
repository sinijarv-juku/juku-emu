#!/bin/zsh
#
# Builds Juku for distribution, signs it and packages it as a disk image:
# target/Juku-<version>.dmg, holding Juku.app and a link to /Applications.
#
# Environment:
#   CODE_SIGN_IDENTITY  Code signing identity (required): the SHA-1 hash of a
#                       keychain certificate, as listed by
#                       `security find-identity -v -p codesigning`, or its
#                       name. Use the hash if the name has non-ASCII
#                       characters: codesign does not match those reliably.
#   NOTARY_PROFILE      notarytool keychain profile (optional). When set, the
#                       disk image is notarized and the ticket stapled; this
#                       needs a Developer ID Application certificate.

set -e
cd "${0:a:h}"

APP=Juku
BUNDLE=target/$APP.app
STAGING=target/dmg

if [ -z "$CODE_SIGN_IDENTITY" ]; then
    echo "package.sh: CODE_SIGN_IDENTITY is not set; the keychain has these identities:" >&2
    security find-identity -v -p codesigning >&2
    exit 1
fi

# Name of the identity, from the keychain's list of valid identities.
IDENTITY_NAME=$(security find-identity -v -p codesigning |
    awk -v id="$CODE_SIGN_IDENTITY" -F '"' '{ split($1, f, " ") } f[2] == toupper(id) || $2 == id { print $2; exit }')

if [ -z "$IDENTITY_NAME" ]; then
    echo "package.sh: \"$CODE_SIGN_IDENTITY\" is not a valid code signing identity in the keychain" >&2
    exit 1
fi

if [[ "$IDENTITY_NAME" != "Developer ID Application:"* ]]; then
    echo "package.sh: warning: \"$IDENTITY_NAME\" is not a Developer ID certificate;" >&2
    echo "            Gatekeeper will block the app on other Macs" >&2
fi

VERSION=$(<VERSION)
DMG=target/$APP-$VERSION.dmg

# Fresh bundle, so nothing left over from development builds is signed.
rm -rf $BUNDLE
./build.sh dist

echo "Signing with \"$IDENTITY_NAME\""
# The Quick Look extension first, with its sandbox entitlements: signing the
# app does not sign what is nested in it.
codesign --force --timestamp --options runtime --sign "$CODE_SIGN_IDENTITY" \
    --entitlements frontends/macos/src/thumbnail/Thumbnail.entitlements $BUNDLE/Contents/PlugIns/JukuThumbnail.appex
codesign --force --timestamp --options runtime --sign "$CODE_SIGN_IDENTITY" $BUNDLE
codesign --verify --deep --strict --verbose=2 $BUNDLE

# The disk image opens as a window with the app and Applications side by side,
# large, on a background with a chevron between them, and the volume has
# the app's icon on a drive (spec §3.4).
VOLUME="$APP $VERSION"
ART=target/dmg-art
RW=target/dmg-rw.dmg
if [ -e "/Volumes/$VOLUME" ]; then
    echo "package.sh: a volume named \"$VOLUME\" is mounted; eject it first" >&2
    exit 1
fi
rm -rf $STAGING $ART $RW $DMG
mkdir -p $STAGING/.background $ART
ditto $BUNDLE $STAGING/$APP.app
ln -s /Applications $STAGING/Applications

# The artwork (frontends/macos/dmg/DiskImageArt.m): the system's white
# removable-disk icon, as mounted disk images have, with the app's icon on it.
DRIVE=/System/Library/Extensions/IOStorageFamily.kext/Contents/Resources/Removable.icns
[ -f $DRIVE ] || DRIVE=
clang -fobjc-arc -fmodules -framework AppKit frontends/macos/dmg/DiskImageArt.m -o $ART/DiskImageArt
$ART/DiskImageArt $ART $BUNDLE/Contents/Resources/$APP.icns $DRIVE
tiffutil -cathidpicheck $ART/background.png $ART/background@2x.png -out $STAGING/.background/background.tiff >/dev/null
iconutil -c icns -o $ART/VolumeIcon.icns $ART/volume.iconset

# Finder lays out the window, in a writable image: it keeps the layout in the
# volume's .DS_Store. The first run asks to let the terminal control Finder.
hdiutil create -quiet -volname "$VOLUME" -srcfolder $STAGING -fs APFS -format UDRW -o $RW
hdiutil attach -quiet -readwrite -noverify -noautoopen $RW
osascript - "$VOLUME" "$APP.app" <<'END'
on run {volumeName, appName}
    tell application "Finder"
        tell disk volumeName
            open
            set current view of container window to icon view
            set toolbar visible of container window to false
            set statusbar visible of container window to false
            set sidebar width of container window to 0
            -- 660 × 400 points inside, the background's size.
            set the bounds of container window to {200, 120, 860, 548}
            set viewOptions to the icon view options of container window
            set arrangement of viewOptions to not arranged
            set icon size of viewOptions to 160
            set text size of viewOptions to 13
            set shows item info of viewOptions to false
            set background picture of viewOptions to file ".background:background.tiff"
            -- The icons' centres, as DiskImageArt.m draws the chevron between them.
            set position of item appName of container window to {180, 170}
            set position of item "Applications" of container window to {480, 170}
            close
            open
            update without registering applications
            delay 1
            close
        end tell
    end tell
end run
END
# Finder writes .DS_Store a moment after the window closes.
for i in {1..20}; do
    [ -f "/Volumes/$VOLUME/.DS_Store" ] && break
    sleep 0.5
done
if [ ! -f "/Volumes/$VOLUME/.DS_Store" ]; then
    echo "package.sh: Finder did not save the window's layout" >&2
    exit 1
fi
# The volume's icon goes in after Finder's work: its update deletes it.
cp $ART/VolumeIcon.icns "/Volumes/$VOLUME/.VolumeIcon.icns"
SetFile -a C "/Volumes/$VOLUME"
rm -rf "/Volumes/$VOLUME/.fseventsd"
hdiutil detach -quiet "/Volumes/$VOLUME"
hdiutil convert -quiet $RW -format ULFO -o $DMG
rm -rf $STAGING $ART $RW

codesign --force --timestamp --sign "$CODE_SIGN_IDENTITY" $DMG
codesign --verify --strict $DMG

if [ -n "$NOTARY_PROFILE" ]; then
    xcrun notarytool submit $DMG --keychain-profile "$NOTARY_PROFILE" --wait
    xcrun stapler staple $DMG
fi

echo "Created $DMG"
