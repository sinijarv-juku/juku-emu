#pragma once

#import <Foundation/Foundation.h>

static NSString *const kSettingCRTEffects = @"CRTEffects";
static NSString *const kSettingCurvature = @"CRTCurvature";
static NSString *const kSettingScanlines = @"CRTScanlines";
static NSString *const kSettingGlow = @"CRTGlow";
static NSString *const kSettingBackground = @"CRTBackground";             // unlit face level, 0–1
static NSString *const kSettingFrame = @"CRTFrame";                       // monitor frame level, 0–1
static NSString *const kSettingBezel3D = @"CRTBezel3D";                   // the frame's inner wall in perspective
static NSString *const kSettingBrightness = @"CRTBrightness";
static NSString *const kSettingMouseSensitivity = @"MouseSensitivity";   // Juku counts per point
static NSString *const kSettingKeyClick = @"KeyClick";                   // RomBios key click on/off
static NSString *const kSettingDriveSounds = @"DriveSounds";             // floppy drive sounds on/off
static NSString *const kSettingDriveSoundsVolume = @"DriveSoundsVolume"; // 0–2, 1 = the recordings' level halved
static NSString *const kSettingDriveIdleMute = @"DriveIdleMute";         // the motor fades out 10 s after the last disk command
static NSString *const kSettingT32FastClock = @"T32FastClock";           // timer T3.2 at 1.23 MHz instead of the frame rate
static NSString *const kSettingDebugger = @"AcceptDebugger";             // the debug server listens (spec §12.3)
static NSString *const kSettingDebuggerPort = @"DebuggerPort";           // its TCP port on 127.0.0.1
static NSString *const kSettingSystemRom = @"SystemROM";                 // bundled ROM name or file path (Roms.m)
static NSString *const kSettingSystemRomFile = @"SystemROMFile";         // the user's last ROM file, a path
static NSString *const kSettingExpansionRom = @"ExpansionROM";           // path of the plugged-in expansion ROM; absent = none
static NSString *const kSettingExpansionRomFile = @"ExpansionROMFile";   // the user's last expansion ROM file
static NSString *const kSettingRecentDisks = @"RecentDisks";             // bookmarks, newest first
static NSString *const kSettingDrives = @"Drives";                       // the disks in A: and B: (Disks.m)
static NSString *const kSettingSnapshotKey = @"SnapshotKey";             // 64 random bytes sealing snapshots' file links (Snapshots.m)
static NSString *const kSettingLanguages = @"AppleLanguages";            // macOS per-app language, read at launch

static NSString *const kDefaultSystemRom = @"ekta37";

// The CRT sliders, which Restore CRT Defaults resets.
static NSArray<NSString *> *crtSettingKeys(void)
{
    return @[kSettingCurvature, kSettingScanlines, kSettingGlow, kSettingBackground, kSettingFrame, kSettingBrightness];
}

// User preferences in NSUserDefaults. Read on every frame; the values are
// cached by NSUserDefaults, so this is cheap.
@interface Settings: NSObject

+ (void)registerDefaults;

@end

@implementation Settings

+ (void)registerDefaults
{
    [[NSUserDefaults standardUserDefaults] registerDefaults:@{
        kSettingCRTEffects: @YES,
        kSettingCurvature: @0.022,
        kSettingScanlines: @0.68,
        kSettingGlow: @0.085,
        kSettingBackground: @0.175,
        kSettingFrame: @0.1,
        kSettingBezel3D: @YES,
        kSettingBrightness: @0.91,
        kSettingMouseSensitivity: @0.5,
        kSettingKeyClick: @YES,
        kSettingDriveSounds: @YES,
        kSettingDriveSoundsVolume: @1.0,
        kSettingDriveIdleMute: @NO,
        kSettingT32FastClock: @NO,
        kSettingDebugger: @NO,
        kSettingDebuggerPort: @5104,
        kSettingSystemRom: kDefaultSystemRom,
    }];
}

@end

static BOOL settingBool(NSString *key)
{
    return [[NSUserDefaults standardUserDefaults] boolForKey:key];
}

static NSString *settingString(NSString *key)
{
    return [[NSUserDefaults standardUserDefaults] stringForKey:key];
}

static NSInteger settingInteger(NSString *key)
{
    return [[NSUserDefaults standardUserDefaults] integerForKey:key];
}

static double settingDouble(NSString *key)
{
    return [[NSUserDefaults standardUserDefaults] doubleForKey:key];
}

static void setSetting(NSString *key, id value)
{
    [[NSUserDefaults standardUserDefaults] setObject:value forKey:key];
}

// The UI language chosen for this app ("en", "et"), or nil to follow the
// system. Read from the app's own domain: a plain lookup would return the
// system-wide list. System Settings → Language & Region → Applications writes
// the same key.
static NSString *settingLanguage(void)
{
    NSString *domain = [[NSBundle mainBundle] bundleIdentifier];
    id languages = [[[NSUserDefaults standardUserDefaults] persistentDomainForName:domain] objectForKey:kSettingLanguages];

    if (![languages isKindOfClass:[NSArray class]] || [languages count] == 0) {
        return nil;
    }

    id first = [languages firstObject];
    return [first isKindOfClass:[NSString class]] ? first : nil;
}

static void setSettingLanguage(NSString *language)
{
    NSUserDefaults *defaults = [NSUserDefaults standardUserDefaults];

    if (language) {
        [defaults setObject:@[language] forKey:kSettingLanguages];
    } else {
        [defaults removeObjectForKey:kSettingLanguages];
    }
}
