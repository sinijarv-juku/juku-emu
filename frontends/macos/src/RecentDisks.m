#pragma once

#import <AppKit/AppKit.h>

#import "Settings.m"
#import "Strings.m"

// Longest list in Recent Images.
static const NSUInteger kMaxRecentDisks = 10;

// Disk images inserted lately, newest first, kept in NSUserDefaults as
// bookmarks so they survive renames and moves. The images are also reported
// to NSDocumentController for the Dock menu.
@interface RecentDisks: NSObject<NSMenuDelegate>

// Target for the menus' items: -insertRecentDisk:(NSMenuItem *) with the URL
// as representedObject and the drive as tag.
- (instancetype)initWithTarget:(id)target;
- (void)noteURL:(NSURL *)url;
// The list for each drive's submenu; its items insert in that drive.
- (NSMenu *)menuForDrive:(int)drive;

@end

@implementation RecentDisks {
    id __weak _target;
    NSArray<NSMenu *> *_menus;  // one per drive
}

- (instancetype)initWithTarget:(id)target
{
    self = [super init];

    if (self) {
        _target = target;
        NSMutableArray<NSMenu *> *menus = [NSMutableArray array];

        for (int drive = 0; drive < 2; drive++) {
            NSMenu *menu = [[NSMenu alloc] initWithTitle:localized(@"Recent Images")];
            [menu setDelegate:self];
            [menus addObject:menu];
        }

        _menus = menus;
    }

    return self;
}

- (NSMenu *)menuForDrive:(int)drive
{
    return _menus[drive];
}

static NSArray<NSData *> *storedBookmarks(void)
{
    NSArray *list = [[NSUserDefaults standardUserDefaults] arrayForKey:kSettingRecentDisks];
    NSMutableArray<NSData *> *bookmarks = [NSMutableArray array];

    for (id item in list) {
        if ([item isKindOfClass:[NSData class]]) {
            [bookmarks addObject:item];
        }
    }

    return bookmarks;
}

static NSURL *resolveBookmark(NSData *bookmark)
{
    BOOL stale = NO;
    NSURL *url = [NSURL URLByResolvingBookmarkData:bookmark
                                           options:NSURLBookmarkResolutionWithoutUI | NSURLBookmarkResolutionWithoutMounting
                                     relativeToURL:nil
                               bookmarkDataIsStale:&stale
                                             error:nil];
    return [url checkResourceIsReachableAndReturnError:nil] ? url : nil;
}

- (void)noteURL:(NSURL *)url
{
    NSData *bookmark = [url bookmarkDataWithOptions:0 includingResourceValuesForKeys:nil relativeToURL:nil error:nil];

    if (!bookmark) {
        return;
    }

    NSMutableArray<NSData *> *list = [NSMutableArray arrayWithObject:bookmark];
    NSURL *added = [url URLByStandardizingPath];

    for (NSData *old in storedBookmarks()) {
        NSURL *oldURL = resolveBookmark(old);

        if (oldURL && ![[oldURL URLByStandardizingPath] isEqual:added] && [list count] < kMaxRecentDisks) {
            [list addObject:old];
        }
    }

    setSetting(kSettingRecentDisks, list);
    [[NSDocumentController sharedDocumentController] noteNewRecentDocumentURL:url];
}

- (void)clearRecentDisks:(id)sender
{
    setSetting(kSettingRecentDisks, @[]);
    [[NSDocumentController sharedDocumentController] clearRecentDocuments:sender];
}

// Each image is an item that inserts it in the menu's drive. Images that
// can no longer be found are left out.
- (void)menuNeedsUpdate:(NSMenu *)menu
{
    NSInteger drive = [_menus indexOfObject:menu];
    [menu removeAllItems];

    for (NSData *bookmark in storedBookmarks()) {
        NSURL *url = resolveBookmark(bookmark);

        if (!url) {
            continue;
        }

        NSString *name = [[NSFileManager defaultManager] displayNameAtPath:[url path]];
        NSMenuItem *item = [menu addItemWithTitle:name action:@selector(insertRecentDisk:) keyEquivalent:@""];
        [item setTarget:_target];
        [item setRepresentedObject:url];
        [item setTag:drive];
        [item setToolTip:[url path]];
    }

    if ([menu numberOfItems] > 0) {
        [menu addItem:[NSMenuItem separatorItem]];
    }

    NSMenuItem *clear = [menu addItemWithTitle:localized(@"Clear Menu") action:@selector(clearRecentDisks:) keyEquivalent:@""];
    [clear setTarget:self];
}

- (BOOL)validateMenuItem:(NSMenuItem *)item
{
    if ([item action] == @selector(clearRecentDisks:)) {
        return [storedBookmarks() count] > 0;
    }

    return YES;
}

@end
