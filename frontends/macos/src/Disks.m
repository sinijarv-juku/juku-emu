#pragma once

#import <AppKit/AppKit.h>
#import <UniformTypeIdentifiers/UniformTypeIdentifiers.h>

#import "Core.m"
#import "RecentDisks.m"
#import "Strings.m"

enum {
    JukuDriveCount = 2,
};

// The disks in the two drives (spec §6.3). A disk lives in memory, in the
// core; its file is read when it goes in and written only when the user
// saves it, always through a save panel. A changed disk asks to be saved
// before it leaves its drive. As the Disks menu's delegate it titles each
// drive's item with the disk in it.
@interface DiskDrives: NSObject<NSMenuDelegate>

- (instancetype)initWithMachine:(JukuMachine *)machine;
- (BOOL)hasDisk:(int)drive;
- (BOOL)isWriteProtected:(int)drive;
// Puts the image in `url` in the drive, once the disk there may go.
- (void)insertURL:(NSURL *)url inDrive:(int)drive;
- (void)chooseDiskForDrive:(int)drive window:(NSWindow *)window;
// New Disk: a formatted, empty disk without a file, once the disk there may
// go.
- (void)insertNewDisk:(int)drive;
- (void)eject:(int)drive;
// Save Disk…: the save panel, on the disk's own file. YES if it was saved.
- (BOOL)saveDisk:(int)drive;
// Asks before the disk in `drive` leaves it, if it is changed: Save…, Don't
// Save or Cancel. YES if it may go.
- (BOOL)mayRemoveDisk:(int)drive;
- (void)toggleWriteProtect:(int)drive;
// Disks → A: or B: → Recent Images, which inserts in that drive.
- (NSMenu *)recentMenuForDrive:(int)drive;
// The drive's item in the Disks menu: "A:" alone for an empty drive, else
// "A: " and the disk's name, in square brackets for a disk that is not saved
// to a file, with a dot for one the running program has written to.
- (NSString *)menuTitleForDrive:(int)drive;
// The first items of the drive's submenu: a line saying that the disk is
// not saved to a file and that it is edited, shown when either is so, and
// a separator under it. Both are kept up to date with the Disks menu.
- (NSArray<NSMenuItem *> *)statusItemsForDrive:(int)drive;
// The files in the drives and their Write Protect are kept in the Drives
// default, so the next launch starts with them.
- (void)restoreDrives;
- (void)saveDrives;
// The disk's file; nil for an empty drive or a disk from a snapshot whose
// file is not on this Mac.
- (NSURL *)urlForDrive:(int)drive;
// The disk's name: its file's, or the one a snapshot gave it.
- (NSString *)nameForDrive:(int)drive;
// After a snapshot is restored, which brings its own disks: per drive,
// `names` holds the snapshot's name for the disk and `paths` the path of
// its file on this Mac, each or NSNull. A path with no file at it is left
// out.
- (void)attachRestoredDisks:(NSArray *)names paths:(NSArray *)paths;

@end

// Keys of each drive's entry in the Drives default. An empty entry is an
// empty drive.
static NSString *const kDriveBookmark = @"Bookmark";
static NSString *const kDriveWriteProtected = @"WriteProtected";

static void showDiskAlert(NSString *format, NSURL *url)
{
    NSAlert *alert = [[NSAlert alloc] init];
    [alert setMessageText:[NSString stringWithFormat:localized(format), [url lastPathComponent]]];
    [alert runModal];
}

// The image in `url`, or nil with `error` set to the alert format that says
// why it cannot go in a drive.
static NSData *readDiskImage(NSURL *url, NSString **error)
{
    NSData *data = [NSData dataWithContentsOfURL:url];

    if (!data) {
        *error = @"The disk image “%@” could not be read.";
    } else if (juku_disk_check([data length]) != 0) {
        *error = @"“%@” is not a Juku disk image (800 KB or 400 KB expected).";
        data = nil;
    }

    return data;
}

static NSArray<UTType *> *diskImageTypes(void)
{
    NSMutableArray<UTType *> *types = [NSMutableArray array];

    for (NSString *ext in @[@"juk", @"img", @"dsk", @"cpm"]) {
        UTType *type = [UTType typeWithFilenameExtension:ext];

        if (type) {
            [types addObject:type];
        }
    }

    return types;
}

@implementation DiskDrives {
    JukuMachine *_machine;
    NSURL *_urls[JukuDriveCount];
    NSString *_names[JukuDriveCount];   // for a disk without a file
    BOOL _notSaved[JukuDriveCount];     // a new disk or one from a snapshot, until it is saved
    NSMenuItem *_status[JukuDriveCount];
    NSMenuItem *_statusSeparator[JukuDriveCount];
    RecentDisks *_recent;
}

- (instancetype)initWithMachine:(JukuMachine *)machine
{
    self = [super init];

    if (self) {
        _machine = machine;
        _recent = [[RecentDisks alloc] initWithTarget:self];
    }

    return self;
}

- (BOOL)hasDisk:(int)drive
{
    size_t len = 0;
    return juku_disk_data(_machine, (uint8_t)drive, &len) != NULL;
}

- (NSURL *)urlForDrive:(int)drive
{
    return _urls[drive];
}

- (NSString *)nameForDrive:(int)drive
{
    if (_urls[drive]) {
        return [_urls[drive] lastPathComponent];
    }

    return _names[drive] ?: [NSString stringWithFormat:localized(@"Disk %@.juk"), drive == 0 ? @"A" : @"B"];
}

- (BOOL)isWriteProtected:(int)drive
{
    return juku_disk_write_protected(_machine, (uint8_t)drive);
}

// Puts an image read from `url` in the drive, in place of the disk there.
- (void)putImage:(NSData *)data from:(NSURL *)url inDrive:(int)drive writeProtected:(BOOL)writeProtected
{
    juku_disk_insert(_machine, (uint8_t)drive, [data bytes], [data length], writeProtected);
    _urls[drive] = url;
    _names[drive] = nil;
    _notSaved[drive] = NO;
}

- (void)insertURL:(NSURL *)url inDrive:(int)drive
{
    NSString *error = nil;
    NSData *data = readDiskImage(url, &error);

    if (!data) {
        showDiskAlert(error, url);
        return;
    }

    if ([self mayRemoveDisk:drive]) {
        [self putImage:data from:url inDrive:drive writeProtected:NO];
        [_recent noteURL:url];
        [self saveDrives];
    }
}

- (void)saveDrives
{
    NSMutableArray<NSDictionary *> *drives = [NSMutableArray array];

    for (int drive = 0; drive < JukuDriveCount; drive++) {
        NSData *bookmark = [_urls[drive] bookmarkDataWithOptions:0 includingResourceValuesForKeys:nil relativeToURL:nil error:nil];
        [drives addObject:bookmark ? @{kDriveBookmark: bookmark, kDriveWriteProtected: @([self isWriteProtected:drive])} : @{}];
    }

    setSetting(kSettingDrives, drives);
}

// Called at launch, before the machine runs, so it boots from the disk in A:.
// An image that has gone or no longer loads is reported once the app is up,
// and its drive is left empty.
- (void)restoreDrives
{
    NSArray *drives = [[NSUserDefaults standardUserDefaults] arrayForKey:kSettingDrives];

    for (int drive = 0; drive < JukuDriveCount && drive < (int)[drives count]; drive++) {
        NSDictionary *saved = drives[drive];

        if (![saved isKindOfClass:[NSDictionary class]] || ![saved[kDriveBookmark] isKindOfClass:[NSData class]]) {
            continue;
        }

        NSData *bookmark = saved[kDriveBookmark];
        id writeProtected = saved[kDriveWriteProtected];
        NSURL *url = resolveBookmark(bookmark);
        NSString *error = @"The disk image “%@” could not be read.";
        NSData *data = url ? readDiskImage(url, &error) : nil;

        if (data) {
            [self putImage:data
                      from:url
                   inDrive:drive
            writeProtected:[writeProtected isKindOfClass:[NSNumber class]] && [writeProtected boolValue]];
            continue;
        }

        if (!url) {
            NSString *path = [NSURL resourceValuesForKeys:@[NSURLPathKey] fromBookmarkData:bookmark][NSURLPathKey];
            url = path ? [NSURL fileURLWithPath:path] : nil;
        }

        if (url) {
            dispatch_async(dispatch_get_main_queue(), ^{
                showDiskAlert(error, url);
            });
        }
    }

    [self saveDrives];
}

- (NSMenu *)recentMenuForDrive:(int)drive
{
    return [_recent menuForDrive:drive];
}

- (NSString *)menuTitleForDrive:(int)drive
{
    if (![self hasDisk:drive]) {
        return [NSString stringWithFormat:@"%c:", 'A' + drive];
    }

    NSString *name = _urls[drive] ? [[NSFileManager defaultManager] displayNameAtPath:[_urls[drive] path]] : [self nameForDrive:drive];

    if (_notSaved[drive]) {
        name = [NSString stringWithFormat:@"[%@]", name];
    }

    NSString *edited = juku_disk_dirty(_machine, (uint8_t)drive) ? @" •" : @"";
    return [NSString stringWithFormat:@"%c: %@%@", 'A' + drive, name, edited];
}

- (NSArray<NSMenuItem *> *)statusItemsForDrive:(int)drive
{
    if (!_status[drive]) {
        _status[drive] = [[NSMenuItem alloc] initWithTitle:@"" action:NULL keyEquivalent:@""];
        _statusSeparator[drive] = [NSMenuItem separatorItem];
        [self updateStatus:drive];
    }

    return @[_status[drive], _statusSeparator[drive]];
}

// The status line: Not saved to a file, Edited, or both.
- (void)updateStatus:(int)drive
{
    NSMutableArray<NSString *> *parts = [NSMutableArray array];

    if (_notSaved[drive]) {
        [parts addObject:localized(@"Not saved to a file")];
    }

    if (juku_disk_dirty(_machine, (uint8_t)drive)) {
        [parts addObject:localized(@"Edited")];
    }

    [_status[drive] setTitle:[parts componentsJoinedByString:@" · "]];
    [_status[drive] setHidden:[parts count] == 0];
    [_statusSeparator[drive] setHidden:[parts count] == 0];
}

- (void)menuNeedsUpdate:(NSMenu *)menu
{
    for (NSMenuItem *item in [menu itemArray]) {
        int drive = (int)[item tag];
        [item setTitle:[self menuTitleForDrive:drive]];
        [item setToolTip:[_urls[drive] path]];
        [self updateStatus:drive];
    }
}

- (void)insertRecentDisk:(NSMenuItem *)sender
{
    [self insertURL:[sender representedObject] inDrive:(int)[sender tag]];
}

- (void)chooseDiskForDrive:(int)drive window:(NSWindow *)window
{
    NSOpenPanel *panel = [NSOpenPanel openPanel];
    [panel setAllowedContentTypes:diskImageTypes()];
    [panel beginSheetModalForWindow:window completionHandler:^(NSModalResponse result) {
        if (result == NSModalResponseOK) {
            [self insertURL:[panel URL] inDrive:drive];
        }
    }];
}

- (void)insertNewDisk:(int)drive
{
    if (![self mayRemoveDisk:drive]) {
        return;
    }

    juku_disk_insert_blank(_machine, (uint8_t)drive);
    _urls[drive] = nil;
    _names[drive] = localized(@"Untitled.juk");
    _notSaved[drive] = YES;
    [self saveDrives];
}

- (BOOL)saveDisk:(int)drive
{
    if (![self hasDisk:drive]) {
        return NO;
    }

    NSURL *url = _urls[drive];
    NSURL *folder = [url URLByDeletingLastPathComponent];

    if (!folder) {
        folder = [[NSFileManager defaultManager] URLForDirectory:NSDocumentDirectory
                                                        inDomain:NSUserDomainMask
                                               appropriateForURL:nil
                                                          create:NO
                                                           error:nil];
    }

    NSSavePanel *panel = [NSSavePanel savePanel];
    [panel setAllowedContentTypes:diskImageTypes()];
    [panel setDirectoryURL:folder];
    [panel setNameFieldStringValue:[self nameForDrive:drive]];

    if ([panel runModal] != NSModalResponseOK) {
        return NO;
    }

    size_t len = 0;
    const uint8_t *bytes = juku_disk_data(_machine, (uint8_t)drive, &len);

    if (!bytes || ![[NSData dataWithBytes:bytes length:len] writeToURL:[panel URL] options:NSDataWritingAtomic error:nil]) {
        showDiskAlert(@"Changes to “%@” could not be saved.", [panel URL]);
        return NO;
    }

    juku_disk_clear_dirty(_machine, (uint8_t)drive);
    _urls[drive] = [panel URL];
    _names[drive] = nil;
    _notSaved[drive] = NO;
    [_recent noteURL:[panel URL]];
    [self saveDrives];
    return YES;
}

- (BOOL)mayRemoveDisk:(int)drive
{
    while (juku_disk_dirty(_machine, (uint8_t)drive)) {
        NSAlert *alert = [[NSAlert alloc] init];
        NSString *letter = drive == 0 ? @"A:" : @"B:";
        [alert setMessageText:[NSString stringWithFormat:localized(@"Do you want to save the changes to “%@” in %@?"), [self nameForDrive:drive], letter]];
        [alert setInformativeText:localized(@"Your changes are lost if you don't save them.")];
        [alert addButtonWithTitle:localized(@"Save…")];
        [alert addButtonWithTitle:localized(@"Cancel")];
        NSButton *discard = [alert addButtonWithTitle:localized(@"Don't Save")];
        [discard setKeyEquivalent:@"d"];
        [discard setKeyEquivalentModifierMask:NSEventModifierFlagCommand];

        switch ([alert runModal]) {
        case NSAlertFirstButtonReturn:
            // Saved, or asked again.
            [self saveDisk:drive];
            break;
        case NSAlertThirdButtonReturn:
            return YES;
        default:
            return NO;
        }
    }

    return YES;
}

- (void)eject:(int)drive
{
    if (![self hasDisk:drive] || ![self mayRemoveDisk:drive]) {
        return;
    }

    juku_disk_eject(_machine, (uint8_t)drive);
    _urls[drive] = nil;
    _names[drive] = nil;
    _notSaved[drive] = NO;
    [self saveDrives];
}

- (void)attachRestoredDisks:(NSArray *)names paths:(NSArray *)paths
{
    for (int drive = 0; drive < JukuDriveCount; drive++) {
        id name = drive < (int)[names count] ? names[drive] : nil;
        id path = drive < (int)[paths count] ? paths[drive] : nil;
        BOOL found = [path isKindOfClass:[NSString class]] && [[NSFileManager defaultManager] fileExistsAtPath:path];
        BOOL disk = [self hasDisk:drive];
        _urls[drive] = disk && found ? [NSURL fileURLWithPath:path] : nil;
        _names[drive] = disk && [name isKindOfClass:[NSString class]] ? name : nil;
        _notSaved[drive] = disk;
    }

    [self saveDrives];
}

- (void)toggleWriteProtect:(int)drive
{
    juku_disk_set_write_protect(_machine, (uint8_t)drive, ![self isWriteProtected:drive]);
    [self saveDrives];
}

@end
