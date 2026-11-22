#pragma once

#import <Foundation/Foundation.h>

#import "Core.m"
#import "Settings.m"
#import "Strings.m"

// The system ROMs in Contents/Resources/roms (resources/roms/README.md), and
// ROM files the user picks. Each is the full 16 KB, both EPROMs, as the core
// expects. A ROM is named by a string: a bundled ROM's file name without .bin,
// or the absolute path of the user's file.
//
// Expansion ROMs (the plug-in memory expander) only come from the user's
// files, named by path; the core checks them (juku_expansion_check).
typedef struct {
    const char *name;     // file name without .bin
    const char *title;    // what it prints at power-on: version and serial
} SystemRom;

static const SystemRom kSystemRoms[] = {
    {"ekta31", "RomBios 3.43 #0031"},
    {"ekta37", "RomBios 3.43m #0037"},
    {"jmon33", "Monitor 3.3 + BASIC"},
};

enum {
    kSystemRomCount = sizeof kSystemRoms / sizeof kSystemRoms[0],
    kSystemRomSize = 0x4000,
};

static NSString *systemRomName(int i)
{
    return @(kSystemRoms[i].name);
}

// For the menu and the Settings pop-up, e.g. "RomBios 3.43m #0037 (Default)".
static NSString *systemRomTitle(int i)
{
    NSString *title = @(kSystemRoms[i].title);

    if ([systemRomName(i) isEqualToString:kDefaultSystemRom]) {
        title = [NSString stringWithFormat:localized(@"%@ (Default)"), title];
    }

    return title;
}

// Index of the bundled ROM called `name`, or -1.
static int systemRomIndex(NSString *name)
{
    for (int i = 0; i < kSystemRomCount; i++) {
        if ([name isEqualToString:systemRomName(i)]) {
            return i;
        }
    }

    return -1;
}

static BOOL isSystemRomFile(NSString *rom)
{
    return [rom isAbsolutePath];
}

// Reads a ROM: a bundled one by name, or the user's file by path. Nil if the
// name is unknown, or the file is missing or not 16 KB.
static NSData *loadSystemRom(NSString *rom)
{
    NSURL *url = nil;

    if (isSystemRomFile(rom)) {
        url = [NSURL fileURLWithPath:rom];
    } else if (systemRomIndex(rom) >= 0) {
        url = [[NSBundle mainBundle] URLForResource:rom withExtension:@"bin" subdirectory:@"roms"];
    }

    NSData *data = url ? [NSData dataWithContentsOfURL:url] : nil;
    return [data length] == kSystemRomSize ? data : nil;
}

// Why juku_expansion_check (or juku_expansion_insert) refused a file, as a
// format for showRomAlert; −3 is a file that could not be read.
static NSString *expansionRomError(int32_t code)
{
    switch (code) {
    case -1:
        return @"“%@” is not an expansion ROM image (32 KB at most).";
    case -2:
        return @"“%@” is not an expansion ROM image: it has no program header the monitor can start.";
    default:
        return @"The ROM file “%@” could not be read.";
    }
}

// Checks the user's expansion ROM file: 0, or an error for expansionRomError.
static int32_t checkExpansionRom(NSString *path, NSData **data)
{
    NSData *image = [NSData dataWithContentsOfFile:path];

    if (!image) {
        return -3;
    }

    if (data) {
        *data = image;
    }

    return juku_expansion_check([image bytes], [image length]);
}
