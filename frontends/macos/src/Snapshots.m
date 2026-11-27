#pragma once

#import <AppKit/AppKit.h>
#import <CommonCrypto/CommonCrypto.h>
#import <UniformTypeIdentifiers/UniformTypeIdentifiers.h>

#import "Core.m"
#import "Roms.m"
#import "Settings.m"
#import "Strings.m"

// Snapshot files (spec §6.8): the core's file holding the machine (§4.6),
// with a `meta` section from the app, and a `mac ` section with the files
// the snapshot came from, sealed so that only this Mac can read it. ROMs in a snapshot are matched to the
// bundled ROMs and the user's files by their bytes; a ROM this Mac does not
// have is kept in Application Support/Juku/ROMs, so the settings can name it.

static NSString *const kSnapshotExtension = @"jukusnap";
static NSString *const kSnapshotTypeIdentifier = @"io.github.sinijarv-juku.juku-emu.snapshot";

static UTType *snapshotContentType(void)
{
    return [UTType typeWithIdentifier:kSnapshotTypeIdentifier] ?: [UTType typeWithFilenameExtension:kSnapshotExtension];
}

static BOOL isSnapshotURL(NSURL *url)
{
    return [[[url pathExtension] lowercaseString] isEqualToString:kSnapshotExtension];
}

// Application Support/Juku, or a folder in it, made if needed. Nil if it
// cannot be made.
static NSURL *supportFolder(NSString *name)
{
    NSFileManager *fm = [NSFileManager defaultManager];
    NSURL *base = [fm URLForDirectory:NSApplicationSupportDirectory inDomain:NSUserDomainMask appropriateForURL:nil create:YES error:nil];
    NSURL *folder = [base URLByAppendingPathComponent:@"Juku" isDirectory:YES];

    if (name) {
        folder = [folder URLByAppendingPathComponent:name isDirectory:YES];
    }

    return [fm createDirectoryAtURL:folder withIntermediateDirectories:YES attributes:nil error:nil] ? folder : nil;
}

// Machine → Quick Save and Quick Load use one snapshot file.
static NSURL *quickSaveURL(void)
{
    return [supportFolder(nil) URLByAppendingPathComponent:@"Quick Save.jukusnap"];
}

// Why a snapshot could not be opened, as a format for the file name.
static NSString *snapshotError(int32_t code)
{
    switch (code) {
    case -1:
        return @"“%@” is not a Juku snapshot.";
    case -2:
        return @"The snapshot “%@” is damaged.";
    case -3:
        return @"“%@” was made by a newer version of Juku.";
    case -4:
        return @"“%@” was made by an older version of Juku.";
    default:
        return @"The snapshot “%@” could not be read.";
    }
}

// The `meta` section: UTF-8 lines of key=value, readable on any platform.
// Keys: app (name and version), saved (UTC, ISO 8601), rom (a bundled ROM's
// name or the ROM file's name), and expansion, disk0 and disk1 (file
// names). Paths are left out: they are of no use on another computer and
// tell more about the user than a shared file should.
static NSData *snapshotMeta(NSDictionary<NSString *, NSString *> *values)
{
    NSMutableString *text = [NSMutableString string];
    NSCharacterSet *newlines = [NSCharacterSet newlineCharacterSet];

    for (NSString *key in [[values allKeys] sortedArrayUsingSelector:@selector(compare:)]) {
        NSString *value = [[values[key] componentsSeparatedByCharactersInSet:newlines] componentsJoinedByString:@" "];
        [text appendFormat:@"%@=%@\n", key, value];
    }

    return [text dataUsingEncoding:NSUTF8StringEncoding];
}

static NSDictionary<NSString *, NSString *> *parseSnapshotMeta(NSData *data)
{
    NSMutableDictionary<NSString *, NSString *> *values = [NSMutableDictionary dictionary];
    NSString *text = data ? [[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding] : nil;

    for (NSString *line in [text componentsSeparatedByString:@"\n"]) {
        NSRange equals = [line rangeOfString:@"="];

        if (equals.location != NSNotFound) {
            values[[line substringToIndex:equals.location]] = [line substringFromIndex:NSMaxRange(equals)];
        }
    }

    return values;
}

// Section `tag` of a snapshot file, or nil.
static NSData *snapshotSection(NSData *file, const char *tag)
{
    const uint8_t *out = NULL;
    size_t len = 0;

    if (juku_snapshot_section([file bytes], [file length], (const uint8_t *)tag, &out, &len) != 0 || !out) {
        return nil;
    }

    return [NSData dataWithBytes:out length:len];
}

// A ROM's name for `meta`: a bundled ROM's own name, or the file's name.
static NSString *romMetaName(NSString *setting)
{
    return isSystemRomFile(setting) ? [setting lastPathComponent] : setting;
}

// An expansion ROM as a snapshot stores it: without its trailing FFh.
static NSData *trimmedExpansion(NSData *image)
{
    const uint8_t *bytes = [image bytes];
    NSUInteger len = [image length];

    while (len > 0 && bytes[len - 1] == 0xFF) {
        len--;
    }

    return [image subdataWithRange:NSMakeRange(0, len)];
}

// The path of a ROM file that `matches`: one of `candidates`, one kept
// before, or else `contents` written to a new file in Application
// Support/Juku/ROMs named after `name`. Nil if that cannot be written.
static NSString *keptRomFile(NSData *contents, NSString *name, NSArray<NSString *> *candidates, BOOL (^matches)(NSData *file))
{
    NSFileManager *fm = [NSFileManager defaultManager];
    NSURL *folder = supportFolder(@"ROMs");
    NSMutableArray<NSString *> *paths = [NSMutableArray array];

    for (NSString *path in candidates) {
        if (isSystemRomFile(path)) {
            [paths addObject:path];
        }
    }

    for (NSURL *url in [fm contentsOfDirectoryAtURL:folder includingPropertiesForKeys:nil options:NSDirectoryEnumerationSkipsHiddenFiles error:nil]) {
        [paths addObject:[url path]];
    }

    for (NSString *path in paths) {
        NSData *file = [NSData dataWithContentsOfFile:path];

        if (file && matches(file)) {
            return path;
        }
    }

    if (!folder) {
        return nil;
    }

    NSString *base = [[[name lastPathComponent] stringByDeletingPathExtension] stringByReplacingOccurrencesOfString:@":" withString:@"-"];
    base = [base stringByTrimmingCharactersInSet:[NSCharacterSet characterSetWithCharactersInString:@". "]];

    if ([base length] == 0) {
        base = @"ROM";
    }

    for (int n = 1; n < 100; n++) {
        NSString *file = n == 1 ? base : [NSString stringWithFormat:@"%@ %d", base, n];
        NSURL *url = [folder URLByAppendingPathComponent:[file stringByAppendingPathExtension:@"bin"]];

        if ([fm fileExistsAtPath:[url path]]) {
            continue;
        }

        return [contents writeToURL:url options:NSDataWritingWithoutOverwriting error:nil] ? [url path] : nil;
    }

    return nil;
}

// The SystemROM setting for a snapshot's ROM: the bundled ROM or the
// user's file with the same bytes, or a kept copy. `name` is the ROM's name
// in the snapshot, and `path` the file it came from, if this Mac knows it.
static NSString *systemRomSetting(NSData *rom, NSString *name, NSString *path)
{
    for (int i = 0; i < kSystemRomCount; i++) {
        if ([loadSystemRom(systemRomName(i)) isEqualToData:rom]) {
            return systemRomName(i);
        }
    }

    NSArray *candidates = @[path ?: @"", settingString(kSettingSystemRom) ?: @"", settingString(kSettingSystemRomFile) ?: @""];
    return keptRomFile(rom, name, candidates, ^BOOL(NSData *file) {
        return [file isEqualToData:rom];
    });
}

// The ExpansionROM setting for a snapshot's expansion window (not empty):
// the user's file with the same program, or a kept copy. A copy is padded
// with FFh to the length in its program header, so that it passes the check
// again. `path` is the file it came from, if this Mac knows it.
static NSString *expansionRomSetting(NSData *window, NSString *name, NSString *path)
{
    NSMutableData *image = [window mutableCopy];

    if ([image length] >= 7) {
        const uint8_t *header = [image bytes];
        NSUInteger declared = header[5] | (NSUInteger)header[6] << 8;

        while ([image length] < declared) {
            uint8_t ff = 0xFF;
            [image appendBytes:&ff length:1];
        }
    }

    NSArray *candidates = @[path ?: @"", settingString(kSettingExpansionRom) ?: @"", settingString(kSettingExpansionRomFile) ?: @""];
    return keptRomFile(image, name ?: @"Expansion ROM", candidates, ^BOOL(NSData *file) {
        return [trimmedExpansion(file) isEqualToData:window];
    });
}

// ---- The sealed `mac ` section --------------------------------------------
// The files a snapshot came from, as a binary plist: `Disks`, one entry per
// drive with the image's `Bookmark` and `Path` (empty for none), and the
// paths of ROM files in `SystemROM` and `ExpansionROM`. They are kept as a
// binary plist,
// encrypted with AES-256-CBC and authenticated with HMAC-SHA256, both from
// CommonCrypto. The keys are 64 random bytes in the SnapshotKey default:
// the first half encrypts, the second authenticates. Another Mac, or this
// one after its preferences were deleted, cannot read the section, and it
// cannot even tell whose it is: the file holds no identifier, only
// `version, IV, ciphertext, MAC`.

enum {
    kSealVersion = 1,
    kSealKeySize = kCCKeySizeAES256,
    kSealIVSize = kCCBlockSizeAES128,
    kSealMACSize = CC_SHA256_DIGEST_LENGTH,
};

// The installation's keys, made on first use.
static NSData *snapshotKey(void)
{
    NSData *key = [[NSUserDefaults standardUserDefaults] dataForKey:kSettingSnapshotKey];

    if ([key length] != 2 * kSealKeySize) {
        NSMutableData *fresh = [NSMutableData dataWithLength:2 * kSealKeySize];
        arc4random_buf([fresh mutableBytes], [fresh length]);
        key = fresh;
        setSetting(kSettingSnapshotKey, key);
    }

    return key;
}

static NSData *sealMAC(NSData *key, const void *bytes, size_t len)
{
    NSMutableData *mac = [NSMutableData dataWithLength:kSealMACSize];
    CCHmac(kCCHmacAlgSHA256, (const uint8_t *)[key bytes] + kSealKeySize, kSealKeySize, bytes, len, [mac mutableBytes]);
    return mac;
}

// The `mac ` section for `links`, or nil if it cannot be made.
static NSData *sealSnapshotLinks(NSDictionary *links)
{
    NSData *plain = [NSPropertyListSerialization dataWithPropertyList:links format:NSPropertyListBinaryFormat_v1_0 options:0 error:nil];

    if (!plain) {
        return nil;
    }

    NSData *key = snapshotKey();
    NSMutableData *sealed = [NSMutableData dataWithLength:1 + kSealIVSize + [plain length] + kCCBlockSizeAES128];
    uint8_t *bytes = [sealed mutableBytes];
    bytes[0] = kSealVersion;
    arc4random_buf(bytes + 1, kSealIVSize);
    size_t moved = 0;
    CCCryptorStatus status = CCCrypt(kCCEncrypt, kCCAlgorithmAES, kCCOptionPKCS7Padding, [key bytes], kSealKeySize,
                                     bytes + 1, [plain bytes], [plain length],
                                     bytes + 1 + kSealIVSize, [plain length] + kCCBlockSizeAES128, &moved);

    if (status != kCCSuccess) {
        return nil;
    }

    [sealed setLength:1 + kSealIVSize + moved];
    [sealed appendData:sealMAC(key, [sealed bytes], [sealed length])];
    return sealed;
}

// The links in a `mac ` section, or nil if it is missing, damaged or was
// sealed by another installation.
static NSDictionary *openSnapshotLinks(NSData *sealed)
{
    NSUInteger len = [sealed length];

    if (len < 1 + kSealIVSize + kCCBlockSizeAES128 + kSealMACSize || ((const uint8_t *)[sealed bytes])[0] != kSealVersion) {
        return nil;
    }

    NSData *key = snapshotKey();
    const uint8_t *bytes = [sealed bytes];
    size_t body = len - kSealMACSize;

    if (timingsafe_bcmp([sealMAC(key, bytes, body) bytes], bytes + body, kSealMACSize) != 0) {
        return nil;
    }

    size_t cipherLen = body - 1 - kSealIVSize;
    NSMutableData *plain = [NSMutableData dataWithLength:cipherLen];
    size_t moved = 0;
    CCCryptorStatus status = CCCrypt(kCCDecrypt, kCCAlgorithmAES, kCCOptionPKCS7Padding, [key bytes], kSealKeySize,
                                     bytes + 1, bytes + 1 + kSealIVSize, cipherLen,
                                     [plain mutableBytes], cipherLen, &moved);

    if (status != kCCSuccess) {
        return nil;
    }

    [plain setLength:moved];
    id links = [NSPropertyListSerialization propertyListWithData:plain options:NSPropertyListImmutable format:NULL error:nil];
    return [links isKindOfClass:[NSDictionary class]] ? links : nil;
}
