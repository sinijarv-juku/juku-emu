#import <AppKit/AppKit.h>

// The whole Objective-C part of the app is one translation unit.
#import "Audio.m"
#import "Core.m"
#import "Disks.m"
#import "KeyboardWindow.m"
#import "Renderer.m"
#import "Roms.m"
#import "Settings.m"
#import "SettingsWindow.m"
#import "Snapshots.m"
#import "Strings.m"
#import "EmuView.m"

@interface AppDelegate: NSObject<NSApplicationDelegate, NSWindowDelegate>
@end

@implementation AppDelegate {
    NSWindow *_window;
    JukuMachine *_machine;
    const JukuAudio *_audio;
    AudioOutput *_audioOutput;
    DiskDrives *_disks;
    NSString *_romName;     // the ROM in the machine (Roms.m)
    BOOL _keyClickAvailable;    // the key click patch applies to that ROM
    NSString *_expansionPath;   // the expansion ROM plugged in, or nil
    SettingsWindow *_settingsWindow;
    KeyboardWindow *_keyboardWindow;
    JukuDap *_dap;              // the debug server, or NULL (spec §12.3)
    uint16_t _dapPort;          // its port
    uint16_t _dapArgPort;       // --debug-server[=port] on the command line, or 0
    uint16_t _dapFailedPort;    // the port last reported as taken, or 0
}

static NSMenuItem *addMenuItem(NSMenu *menu, NSString *title, SEL action, NSString *keyEquivalent)
{
    return [menu addItemWithTitle:localized(title) action:action keyEquivalent:keyEquivalent];
}

// The View menu's window sizes, 1× to 3× in steps of 0.5×, counted in
// half steps.
static const int kMinWindowHalves = 2;
static const int kMaxWindowHalves = 6;

static void createMenubar(DiskDrives *disks)
{
    NSMenu *mainMenu = [[NSMenu alloc] initWithTitle:@"Main Menu"];

    NSMenuItem *appMenuItem = [mainMenu addItemWithTitle:@"" action:NULL keyEquivalent:@""];
    NSMenu *appMenu = [[NSMenu alloc] initWithTitle:@"Juku"];
    addMenuItem(appMenu, @"About Juku", @selector(showAboutPanel:), @"");
    [appMenu addItem:[NSMenuItem separatorItem]];
    addMenuItem(appMenu, @"Settings…", @selector(showSettings:), @",");
    [appMenu addItem:[NSMenuItem separatorItem]];
    addMenuItem(appMenu, @"Hide Juku", @selector(hide:), @"h");
    NSMenuItem *hideOthers = addMenuItem(appMenu, @"Hide Others", @selector(hideOtherApplications:), @"h");
    [hideOthers setKeyEquivalentModifierMask:NSEventModifierFlagCommand | NSEventModifierFlagOption];
    addMenuItem(appMenu, @"Show All", @selector(unhideAllApplications:), @"");
    [appMenu addItem:[NSMenuItem separatorItem]];
    addMenuItem(appMenu, @"Quit Juku", @selector(terminate:), @"q");
    [mainMenu setSubmenu:appMenu forItem:appMenuItem];

    NSMenuItem *machineMenuItem = [mainMenu addItemWithTitle:@"" action:NULL keyEquivalent:@""];
    NSMenu *machineMenu = [[NSMenu alloc] initWithTitle:localized(@"Machine")];
    addMenuItem(machineMenu, @"Reset", @selector(resetMachine:), @"r");
    NSMenuItem *powerCycle = addMenuItem(machineMenu, @"Power Cycle", @selector(powerCycleMachine:), @"r");
    [powerCycle setKeyEquivalentModifierMask:NSEventModifierFlagCommand | NSEventModifierFlagShift];
    NSMenuItem *romItem = addMenuItem(machineMenu, @"System ROM", NULL, @"");
    NSMenu *romMenu = [[NSMenu alloc] initWithTitle:localized(@"System ROM")];

    for (int i = 0; i < kSystemRomCount; i++) {
        NSMenuItem *item = [romMenu addItemWithTitle:systemRomTitle(i) action:@selector(selectSystemRom:) keyEquivalent:@""];
        [item setTag:i];
    }

    // The user's last ROM file, hidden until there is one, and the panel.
    [romMenu addItem:[NSMenuItem separatorItem]];
    [romMenu addItemWithTitle:@"" action:@selector(selectSystemRomFile:) keyEquivalent:@""];
    addMenuItem(romMenu, @"Other…", @selector(chooseSystemRomFile:), @"");
    [machineMenu setSubmenu:romMenu forItem:romItem];

    // None, the user's last expansion ROM file (hidden until there is one),
    // and the panel.
    NSMenuItem *expansionItem = addMenuItem(machineMenu, @"Expansion ROM", NULL, @"");
    NSMenu *expansionMenu = [[NSMenu alloc] initWithTitle:localized(@"Expansion ROM")];
    addMenuItem(expansionMenu, @"None", @selector(selectNoExpansionRom:), @"");
    [expansionMenu addItem:[NSMenuItem separatorItem]];
    [expansionMenu addItemWithTitle:@"" action:@selector(selectExpansionRomFile:) keyEquivalent:@""];
    addMenuItem(expansionMenu, @"Other…", @selector(chooseExpansionRomFile:), @"");
    [machineMenu setSubmenu:expansionMenu forItem:expansionItem];
    [machineMenu addItem:[NSMenuItem separatorItem]];
    addMenuItem(machineMenu, @"Open Snapshot…", @selector(openSnapshot:), @"o");
    NSMenuItem *saveSnapshot = addMenuItem(machineMenu, @"Save Snapshot…", @selector(saveSnapshot:), @"s");
    [saveSnapshot setKeyEquivalentModifierMask:NSEventModifierFlagCommand | NSEventModifierFlagShift];
    addMenuItem(machineMenu, @"Quick Save", @selector(quickSave:), @"s");
    addMenuItem(machineMenu, @"Quick Load", @selector(quickLoad:), @"l");
    addMenuItem(machineMenu, @"Save Quick Save As…", @selector(saveQuickSaveAs:), @"");
    [machineMenu addItem:[NSMenuItem separatorItem]];
    addMenuItem(machineMenu, @"Pause", @selector(togglePause:), @"p");
    addMenuItem(machineMenu, @"Warp Speed", @selector(toggleWarp:), @"w");
    [mainMenu setSubmenu:machineMenu forItem:machineMenuItem];

    // One submenu per drive, titled with the image in it (DiskDrives).
    NSMenuItem *disksMenuItem = [mainMenu addItemWithTitle:@"" action:NULL keyEquivalent:@""];
    NSMenu *disksMenu = [[NSMenu alloc] initWithTitle:localized(@"Disks")];
    [disksMenu setDelegate:disks];

    for (int drive = 0; drive < JukuDriveCount; drive++) {
        NSString *key = [NSString stringWithFormat:@"%d", drive + 1];
        NSMenuItem *driveItem = [disksMenu addItemWithTitle:[disks menuTitleForDrive:drive] action:NULL keyEquivalent:@""];
        [driveItem setTag:drive];
        NSMenu *driveMenu = [[NSMenu alloc] initWithTitle:[driveItem title]];

        for (NSMenuItem *item in [disks statusItemsForDrive:drive]) {
            [driveMenu addItem:item];
        }

        [addMenuItem(driveMenu, @"Insert…", @selector(insertDisk:), key) setTag:drive];
        [addMenuItem(driveMenu, @"New Disk", @selector(newDisk:), @"") setTag:drive];
        NSMenuItem *recent = addMenuItem(driveMenu, @"Recent Images", NULL, @"");
        [driveMenu setSubmenu:[disks recentMenuForDrive:drive] forItem:recent];
        [driveMenu addItem:[NSMenuItem separatorItem]];
        [addMenuItem(driveMenu, @"Save Disk…", @selector(saveDisk:), @"") setTag:drive];
        NSMenuItem *eject = addMenuItem(driveMenu, @"Eject", @selector(ejectDisk:), key);
        [eject setKeyEquivalentModifierMask:NSEventModifierFlagCommand | NSEventModifierFlagShift];
        [eject setTag:drive];
        [driveMenu addItem:[NSMenuItem separatorItem]];
        [addMenuItem(driveMenu, @"Write Protect", @selector(toggleWriteProtect:), @"") setTag:drive];
        [disksMenu setSubmenu:driveMenu forItem:driveItem];
    }

    [mainMenu setSubmenu:disksMenu forItem:disksMenuItem];

    NSMenuItem *inputMenuItem = [mainMenu addItemWithTitle:@"" action:NULL keyEquivalent:@""];
    NSMenu *inputMenu = [[NSMenu alloc] initWithTitle:localized(@"Input")];
    addMenuItem(inputMenu, @"Capture Mouse", @selector(toggleMouseCapture:), @"m");
    [inputMenu addItem:[NSMenuItem separatorItem]];
    addMenuItem(inputMenu, @"Keyboard", @selector(toggleKeyboard:), @"k");
    [mainMenu setSubmenu:inputMenu forItem:inputMenuItem];

    NSMenuItem *viewMenuItem = [mainMenu addItemWithTitle:@"" action:NULL keyEquivalent:@""];
    NSMenu *viewMenu = [[NSMenu alloc] initWithTitle:localized(@"View")];
    addMenuItem(viewMenu, @"CRT Effects", @selector(toggleCRTEffects:), @"e");
    [viewMenu addItem:[NSMenuItem separatorItem]];

    // 1.5× in English, 1,5× in Estonian: the decimal mark of the user's region.
    NSNumberFormatter *number = [[NSNumberFormatter alloc] init];
    [number setNumberStyle:NSNumberFormatterDecimalStyle];
    [number setMaximumFractionDigits:1];

    for (int halves = kMinWindowHalves; halves <= kMaxWindowHalves; halves++) {
        NSString *title = [[number stringFromNumber:@(halves * 0.5)] stringByAppendingString:@"×"];
        NSMenuItem *item = [viewMenu addItemWithTitle:title action:@selector(selectWindowScale:) keyEquivalent:@""];
        [item setTag:halves];
    }

    [viewMenu addItem:[NSMenuItem separatorItem]];
    NSMenuItem *fullScreen = addMenuItem(viewMenu, @"Enter Full Screen", @selector(toggleFullScreen:), @"f");
    [fullScreen setKeyEquivalentModifierMask:NSEventModifierFlagCommand | NSEventModifierFlagControl];
    [mainMenu setSubmenu:viewMenu forItem:viewMenuItem];

    // Cmd-M is Capture Mouse, so Minimize has no shortcut.
    NSMenuItem *windowMenuItem = [mainMenu addItemWithTitle:@"" action:NULL keyEquivalent:@""];
    NSMenu *windowMenu = [[NSMenu alloc] initWithTitle:localized(@"Window Menu")];
    addMenuItem(windowMenu, @"Minimize", @selector(performMiniaturize:), @"");
    addMenuItem(windowMenu, @"Zoom", @selector(performZoom:), @"");
    [windowMenu addItem:[NSMenuItem separatorItem]];
    addMenuItem(windowMenu, @"Bring All to Front", @selector(arrangeInFront:), @"");
    [mainMenu setSubmenu:windowMenu forItem:windowMenuItem];
    [NSApp setWindowsMenu:windowMenu];

    [NSApp setMainMenu:mainMenu];
}

// View → Size: at N× the tube is N points per raster line (287 visible
// lines), so 2N pixels on a retina display. The window around it depends on
// the bezel margin (Renderer.m), which is set by CRT Effects and 3D Frame.
// The size is given in half steps: 3 is 1.5×.
static NSSize contentSizeForHalves(int halves, float margin)
{
    CGFloat height = round(kCanvasHeight * halves * 0.5 * (1.0 + margin));
    return NSMakeSize(round(height * kTubeAspect), height);
}

// The size, in half steps, a content view of `size` draws the tube at in a
// window, or 0 if it is not a whole half step.
static int halvesForContentSize(NSSize size)
{
    simd_float4 tube = tubeRect((float)size.width, (float)size.height, tubeMargin(NO));
    float half = kCanvasHeight * 0.5f;
    int halves = (int)lroundf(tube.w / half);
    return halves > 0 && fabsf(tube.w - halves * half) <= 1.0f ? halves : 0;
}

// `window` with a content view of `size` fits on its screen, or on the main
// screen if it is on none yet.
static BOOL windowFits(NSWindow *window, NSSize size)
{
    NSRect frame = [window frameRectForContentRect:(NSRect){ NSZeroPoint, size }];
    NSSize visible = [([window screen] ?: [NSScreen mainScreen]) visibleFrame].size;
    return frame.size.width <= visible.width && frame.size.height <= visible.height;
}

static NSWindow *createWindow(JukuMachine *machine, const JukuAudio *audio, DiskDrives *disks)
{
    NSWindowStyleMask styleMask = NSWindowStyleMaskTitled | NSWindowStyleMaskClosable |
                                  NSWindowStyleMaskMiniaturizable | NSWindowStyleMaskResizable;
    NSRect contentRect = { NSZeroPoint, contentSizeForHalves(kMinWindowHalves, tubeMargin(NO)) };
    NSWindow *window = [[NSWindow alloc] initWithContentRect:contentRect styleMask:styleMask backing:NSBackingStoreBuffered defer:YES];

    // It opens at the largest size that fits on the screen.
    for (int halves = kMaxWindowHalves; halves > kMinWindowHalves; halves--) {
        NSSize size = contentSizeForHalves(halves, tubeMargin(NO));

        if (windowFits(window, size)) {
            contentRect.size = size;
            [window setContentSize:size];
            break;
        }
    }

    [window setTitle:localized(@"Window")];
    [window setContentAspectRatio:NSMakeSize(4.0, 3.0)];
    [window setContentMinSize:contentSizeForHalves(kMinWindowHalves, 0.0f)];
    [window setCollectionBehavior:NSWindowCollectionBehaviorFullScreenPrimary];
    [window center];

    Renderer *renderer = [[Renderer alloc] init];

    if (!renderer) {
        return nil;
    }

    EmuView *view = [[EmuView alloc] initWithFrame:contentRect machine:machine audio:audio renderer:renderer disks:disks];
    [window setContentView:view];
    [window makeFirstResponder:view];

    // AppKit keeps the frame in NSUserDefaults and puts it back here, if it
    // has one, in place of the size and position above.
    [window setFrameAutosaveName:@"Juku"];
    return window;
}

static void showRomAlert(NSString *format, NSString *path)
{
    NSAlert *alert = [[NSAlert alloc] init];
    [alert setMessageText:[NSString stringWithFormat:localized(format), [path lastPathComponent]]];
    [alert runModal];
}

// `name` is a file name or a drive.
static void showSnapshotAlert(NSString *format, NSString *name)
{
    NSAlert *alert = [[NSAlert alloc] init];
    [alert setMessageText:[NSString stringWithFormat:localized(format), name ?: @""]];
    [alert runModal];
}

// The port of `--debug-server` or `--debug-server=port`, or 0.
static uint16_t debugServerArgument(void)
{
    for (NSString *arg in [[NSProcessInfo processInfo] arguments]) {
        if ([arg isEqualToString:@"--debug-server"]) {
            return (uint16_t)settingInteger(kSettingDebuggerPort);
        }

        if ([arg hasPrefix:@"--debug-server="]) {
            NSInteger port = [[arg substringFromIndex:15] integerValue];
            return port > 0 && port < 65536 ? (uint16_t)port : 0;
        }
    }

    return 0;
}

static void fatalError(NSString *message)
{
    NSAlert *alert = [[NSAlert alloc] init];
    [alert setAlertStyle:NSAlertStyleCritical];
    [alert setMessageText:localized(message)];
    [alert runModal];
    exit(1);
}

- (void)showAboutPanel:(id)sender
{
    NSBundle *bundle = [NSBundle mainBundle];
    id appVersion = [[bundle infoDictionary] objectForKey:@"JukuVersion"];
    NSString *versionDisplayStr = [[NSString alloc] initWithFormat:localized(@"Version %@"), appVersion];

    [NSApp orderFrontStandardAboutPanelWithOptions:@{
        NSAboutPanelOptionApplicationVersion: versionDisplayStr,
        NSAboutPanelOptionVersion: @""
    }];
}

- (void)resetMachine:(id)sender
{
    juku_reset(_machine, false);
}

- (void)powerCycleMachine:(id)sender
{
    juku_reset(_machine, true);
}

- (EmuView *)emuView
{
    return (EmuView *)[_window contentView];
}

- (void)togglePause:(id)sender
{
    [[self emuView] setPaused:![[self emuView] paused]];
}

// Cmd-W is Warp Speed in the emulator window; in any other window it keeps
// its usual meaning.
- (void)toggleWarp:(id)sender
{
    NSWindow *key = [NSApp keyWindow];

    if (key && key != _window) {
        [key performClose:sender];
        return;
    }

    [[self emuView] setWarp:![[self emuView] warp]];
}

- (void)toggleKeyboard:(id)sender
{
    if (!_keyboardWindow) {
        _keyboardWindow = [[KeyboardWindow alloc] initWithMachine:_machine];
    }

    [_keyboardWindow toggle];
}

- (void)insertDisk:(NSMenuItem *)sender
{
    [_disks chooseDiskForDrive:(int)[sender tag] window:_window];
}

- (void)newDisk:(NSMenuItem *)sender
{
    [_disks insertNewDisk:(int)[sender tag]];
}

- (void)saveDisk:(NSMenuItem *)sender
{
    [_disks saveDisk:(int)[sender tag]];
}

- (void)ejectDisk:(NSMenuItem *)sender
{
    [_disks eject:(int)[sender tag]];
}

- (void)toggleWriteProtect:(NSMenuItem *)sender
{
    [_disks toggleWriteProtect:(int)[sender tag]];
}

- (void)showSettings:(id)sender
{
    if (!_settingsWindow) {
        _settingsWindow = [[SettingsWindow alloc] init];
        [_settingsWindow setKeyClickAvailable:_keyClickAvailable];
    }

    [_settingsWindow show];
}

- (void)selectSystemRom:(NSMenuItem *)sender
{
    setSetting(kSettingSystemRom, systemRomName((int)[sender tag]));
}

- (void)selectSystemRomFile:(id)sender
{
    setSetting(kSettingSystemRom, settingString(kSettingSystemRomFile));
}

// Runs the open panel for .bin and .rom files as a sheet on the key window,
// and passes the chosen path to `done`.
- (void)chooseRomFile:(void (^)(NSString *path))done
{
    NSOpenPanel *panel = [NSOpenPanel openPanel];
    NSMutableArray<UTType *> *types = [NSMutableArray array];

    for (NSString *ext in @[@"bin", @"rom"]) {
        UTType *type = [UTType typeWithFilenameExtension:ext];

        if (type) {
            [types addObject:type];
        }
    }

    [panel setAllowedContentTypes:types];
    [panel beginSheetModalForWindow:[NSApp keyWindow] ?: _window completionHandler:^(NSModalResponse result) {
        if (result == NSModalResponseOK) {
            done([[panel URL] path]);
        }
    }];
}

// Machine → System ROM → Other…, and the same item in Settings. The file is
// checked here, so a wrong one never reaches the setting.
- (void)chooseSystemRomFile:(id)sender
{
    [self chooseRomFile:^(NSString *path) {
        if (!loadSystemRom(path)) {
            showRomAlert(@"“%@” is not a Juku ROM image (16 KB expected).", path);
            return;
        }

        setSetting(kSettingSystemRomFile, path);

        // The same file again: it may have changed, so load it anyway.
        if ([path isEqualToString:self->_romName]) {
            self->_romName = nil;
        }

        setSetting(kSettingSystemRom, path);
        [self applySettings:nil];
    }];
}

- (void)selectNoExpansionRom:(id)sender
{
    [[NSUserDefaults standardUserDefaults] removeObjectForKey:kSettingExpansionRom];
}

- (void)selectExpansionRomFile:(id)sender
{
    setSetting(kSettingExpansionRom, settingString(kSettingExpansionRomFile));
}

// Machine → Expansion ROM → Other…, and the same item in Settings. As for the
// system ROM, a file that is not an expansion ROM never reaches the setting.
- (void)chooseExpansionRomFile:(id)sender
{
    [self chooseRomFile:^(NSString *path) {
        int32_t error = checkExpansionRom(path, NULL);

        if (error) {
            showRomAlert(expansionRomError(error), path);
            return;
        }

        setSetting(kSettingExpansionRomFile, path);

        if ([path isEqualToString:self->_expansionPath]) {
            self->_expansionPath = nil;
        }

        setSetting(kSettingExpansionRom, path);
        [self applySettings:nil];
    }];
}

// ---- Snapshots (spec §6.8) --------------------------------------------------

// The machine as a snapshot file, as it is now, its disks as they are in
// memory.
- (NSData *)snapshotData
{
    NSString *version = [[NSBundle mainBundle] infoDictionary][@"JukuVersion"];
    NSMutableDictionary<NSString *, NSString *> *meta = [NSMutableDictionary dictionary];
    meta[@"app"] = [NSString stringWithFormat:@"Juku %@", version];
    meta[@"saved"] = [[[NSISO8601DateFormatter alloc] init] stringFromDate:[NSDate date]];
    meta[@"rom"] = romMetaName(_romName);
    meta[@"expansion"] = [_expansionPath lastPathComponent];

    for (int drive = 0; drive < JukuDriveCount; drive++) {
        meta[[NSString stringWithFormat:@"disk%d", drive]] = [_disks hasDisk:drive] ? [_disks nameForDrive:drive] : nil;
    }

    // Where the files are on this Mac, readable only here.
    NSMutableDictionary *links = [NSMutableDictionary dictionary];
    NSMutableArray *disks = [NSMutableArray array];

    for (int drive = 0; drive < JukuDriveCount; drive++) {
        NSString *path = [[_disks urlForDrive:drive] path];
        [disks addObject:path ? @{@"Path": path} : @{}];
    }

    links[@"Disks"] = disks;
    links[@"SystemROM"] = isSystemRomFile(_romName) ? _romName : nil;
    links[@"ExpansionROM"] = _expansionPath;

    NSData *metaData = snapshotMeta(meta);
    NSData *sealed = sealSnapshotLinks(links);
    JukuSection sections[2] = {
        {{'m', 'e', 't', 'a'}, [metaData bytes], [metaData length]},
        {{'m', 'a', 'c', ' '}, [sealed bytes], [sealed length]},
    };
    size_t len = 0;
    // The picture Finder shows as the file's thumbnail; without one the core
    // draws the frame as it is.
    NSData *thumbnail = [[self emuView] thumbnailPNG];
    uint8_t *file = juku_snapshot_save(_machine, [thumbnail bytes], [thumbnail length], sections, sealed ? 2 : 1, &len);

    if (!file && thumbnail) {
        file = juku_snapshot_save(_machine, NULL, 0, sections, sealed ? 2 : 1, &len);
    }

    NSData *data = file ? [NSData dataWithBytes:file length:len] : nil;
    juku_snapshot_free(file, len);
    return data;
}

// Restores a snapshot file. The settings are pointed at its ROMs before
// they are applied, so that applying them does not start the machine again.
// The disks are the snapshot's, with their files on this Mac if they are
// still there. NO, with an alert naming `url`, if the file could not be used.
- (BOOL)restoreSnapshot:(NSData *)file from:(NSURL *)url
{
    int32_t error = juku_snapshot_load(_machine, [file bytes], [file length]);

    if (error) {
        showSnapshotAlert(snapshotError(error), [url lastPathComponent]);
        return NO;
    }

    NSDictionary<NSString *, NSString *> *meta = parseSnapshotMeta(snapshotSection(file, "meta"));
    // Nil for a snapshot from another Mac.
    NSDictionary *links = openSnapshotLinks(snapshotSection(file, "mac "));
    NSString *romPath = [links[@"SystemROM"] isKindOfClass:[NSString class]] ? links[@"SystemROM"] : nil;
    NSString *expansionPath = [links[@"ExpansionROM"] isKindOfClass:[NSString class]] ? links[@"ExpansionROM"] : nil;
    NSArray *diskLinks = [links[@"Disks"] isKindOfClass:[NSArray class]] ? links[@"Disks"] : nil;
    // The ROMs as the snapshot put them in the machine, matched by their bytes.
    NSMutableData *rom = [NSMutableData dataWithLength:kSystemRomSize];
    juku_system_rom(_machine, [rom mutableBytes]);
    NSString *romName = systemRomSetting(rom, meta[@"rom"], romPath);

    if (romName) {
        _romName = romName;

        if (isSystemRomFile(romName)) {
            setSetting(kSettingSystemRomFile, romName);
        }

        setSetting(kSettingSystemRom, romName);
    }

    size_t windowLength = 0;
    const uint8_t *windowBytes = juku_expansion_rom(_machine, &windowLength);
    NSData *window = [NSData dataWithBytes:windowBytes length:windowLength];
    NSString *expansion = [window length] > 0 ? expansionRomSetting(window, meta[@"expansion"], expansionPath) : nil;

    if ([window length] == 0 || expansion) {
        _expansionPath = expansion;

        if (expansion) {
            setSetting(kSettingExpansionRomFile, expansion);
            setSetting(kSettingExpansionRom, expansion);
        } else {
            [[NSUserDefaults standardUserDefaults] removeObjectForKey:kSettingExpansionRom];
        }
    }

    // The key click follows the setting, not the snapshot.
    [self applySettings:nil];
    NSMutableArray *names = [NSMutableArray array];
    NSMutableArray *paths = [NSMutableArray array];

    for (int drive = 0; drive < JukuDriveCount; drive++) {
        NSDictionary *link = drive < (int)[diskLinks count] ? diskLinks[drive] : nil;
        id path = [link isKindOfClass:[NSDictionary class]] ? link[@"Path"] : nil;
        [names addObject:meta[[NSString stringWithFormat:@"disk%d", drive]] ?: [NSNull null]];
        [paths addObject:[path isKindOfClass:[NSString class]] ? path : [NSNull null]];
    }

    [_disks attachRestoredDisks:names paths:paths];
    [[self emuView] machineRestored];
    return YES;
}

// Opens a snapshot file, after asking about each changed disk, since the
// snapshot brings disks of its own. `done` runs once the snapshot is in.
- (void)openSnapshot:(NSData *)file from:(NSURL *)url done:(void (^)(void))done
{
    const uint8_t *section = NULL;
    size_t len = 0;
    // A file that could not be read has no code of the core's.
    int32_t error = file ? juku_snapshot_section([file bytes], [file length], (const uint8_t *)"meta", &section, &len) : INT32_MIN;

    if (error) {
        showSnapshotAlert(snapshotError(error), [url lastPathComponent]);
        return;
    }

    for (int drive = 0; drive < JukuDriveCount; drive++) {
        if (![_disks mayRemoveDisk:drive]) {
            return;
        }
    }

    if ([self restoreSnapshot:file from:url]) {
        done();
    }
}

- (void)openSnapshotURL:(NSURL *)url
{
    [self openSnapshot:[NSData dataWithContentsOfURL:url] from:url done:^{
        [[NSDocumentController sharedDocumentController] noteNewRecentDocumentURL:url];
    }];
}

// A snapshot dropped on the window (EmuView).
- (void)openSnapshotFromSender:(NSURL *)url
{
    [self openSnapshotURL:url];
}

- (void)openSnapshot:(id)sender
{
    NSOpenPanel *panel = [NSOpenPanel openPanel];
    [panel setAllowedContentTypes:@[snapshotContentType()]];
    [panel beginSheetModalForWindow:_window completionHandler:^(NSModalResponse result) {
        if (result == NSModalResponseOK) {
            [self openSnapshotURL:[panel URL]];
        }
    }];
}

// The snapshot is taken when the item is chosen, not when the panel closes,
// and the machine is paused while the panel is open, so that it goes on from
// the moment saved. A machine paused already stays paused.
- (void)saveSnapshot:(id)sender
{
    EmuView *view = [self emuView];
    BOOL wasPaused = [view paused];

    if (!wasPaused) {
        [view setPaused:YES];
    }

    NSData *data = [self snapshotData];

    if (!data) {
        if (!wasPaused) {
            [view setPaused:NO];
        }

        return;
    }

    [self saveSnapshot:data takenAt:[NSDate date] closed:^{
        if (!wasPaused) {
            [view setPaused:NO];
        }
    }];
}

// Machine → Save Quick Save As…: the Quick Save file copied to a snapshot
// file of its own, offered with the time it was saved in its name.
- (void)saveQuickSaveAs:(id)sender
{
    NSURL *url = quickSaveURL();
    NSData *data = [NSData dataWithContentsOfURL:url];
    NSDate *saved = nil;

    if (!data) {
        showSnapshotAlert(@"The snapshot “%@” could not be read.", [url lastPathComponent]);
        return;
    }

    [url getResourceValue:&saved forKey:NSURLContentModificationDateKey error:nil];
    [self saveSnapshot:data takenAt:saved ?: [NSDate date] closed:nil];
}

// The save panel for a snapshot file, named `Juku <date time>`. `closed`, if
// any, runs as the panel closes, before the file is written.
- (void)saveSnapshot:(NSData *)data takenAt:(NSDate *)date closed:(void (^)(void))closed
{
    NSDateFormatter *format = [[NSDateFormatter alloc] init];
    [format setDateFormat:@"yyyy-MM-dd HH.mm"];
    NSSavePanel *panel = [NSSavePanel savePanel];
    [panel setAllowedContentTypes:@[snapshotContentType()]];
    [panel setNameFieldStringValue:[NSString stringWithFormat:localized(@"Juku %@"), [format stringFromDate:date]]];
    [panel beginSheetModalForWindow:_window completionHandler:^(NSModalResponse result) {
        if (closed) {
            closed();
        }

        if (result != NSModalResponseOK) {
            return;
        }

        if ([data writeToURL:[panel URL] options:NSDataWritingAtomic error:nil]) {
            [[NSDocumentController sharedDocumentController] noteNewRecentDocumentURL:[panel URL]];
        } else {
            showSnapshotAlert(@"The snapshot could not be saved as “%@”.", [[panel URL] lastPathComponent]);
        }
    }];
}

- (void)quickSave:(id)sender
{
    NSURL *url = quickSaveURL();
    NSData *data = [self snapshotData];

    if (data && url && [data writeToURL:url options:NSDataWritingAtomic error:nil]) {
        [[self emuView] showNote:localized(@"Snapshot Saved")];
    } else {
        showSnapshotAlert(@"The snapshot could not be saved as “%@”.", [url lastPathComponent]);
    }
}

- (void)quickLoad:(id)sender
{
    NSURL *url = quickSaveURL();
    [self openSnapshot:[NSData dataWithContentsOfURL:url] from:url done:^{
        [[self emuView] showNote:localized(@"Snapshot Loaded")];
    }];
}

- (void)toggleMouseCapture:(id)sender
{
    [[self emuView] toggleMouseCapture];
}

// Every defaults change lands here. A new system ROM or expansion ROM
// cold-starts the machine, like changing the hardware with the power off.
// The key click patch is cheap and idempotent, and a new ROM needs it again.
// So are the drive sounds' settings and the T3.2 clock.
- (void)applySettings:(NSNotification *)notification
{
    if (!_machine) {
        return;
    }

    NSString *name = settingString(kSettingSystemRom);

    if (![name isEqualToString:_romName]) {
        NSData *rom = loadSystemRom(name);

        if (rom && juku_load_rom(_machine, [rom bytes], [rom length])) {
            _romName = name;
        } else {
            // The user's file may have gone. Keep the ROM in the machine.
            if (isSystemRomFile(name)) {
                dispatch_async(dispatch_get_main_queue(), ^{
                    showRomAlert(@"The ROM file “%@” could not be read.", name);
                });
            }

            setSetting(kSettingSystemRom, _romName ?: kDefaultSystemRom);
        }
    }

    [self applyExpansionRom];

    _keyClickAvailable = juku_set_key_click(_machine, settingBool(kSettingKeyClick));
    [_settingsWindow setKeyClickAvailable:_keyClickAvailable];
    juku_set_drive_sounds(_machine, settingBool(kSettingDriveSounds));
    juku_set_drive_sounds_volume(_machine, (float)settingDouble(kSettingDriveSoundsVolume));
    juku_set_drive_idle_mute(_machine, settingBool(kSettingDriveIdleMute));
    juku_set_t32_fast_clock(_machine, settingBool(kSettingT32FastClock));
    [self applyDebugger];
}

// Starts, moves or stops the debug server as the setting and the command
// line say. A port that is taken is reported once.
- (void)applyDebugger
{
    NSInteger setting = settingInteger(kSettingDebuggerPort);
    uint16_t port = _dapArgPort ?: (settingBool(kSettingDebugger) && setting > 0 && setting < 65536 ? (uint16_t)setting : 0);

    if (_dap && port == _dapPort) {
        return;
    }

    if (_dap) {
        juku_dap_free(_dap, _machine);
        _dap = NULL;
    }

    if (port) {
        _dap = juku_dap_new(port);

        if (_dap) {
            _dapFailedPort = 0;
        } else if (port != _dapFailedPort) {
            _dapFailedPort = port;
            dispatch_async(dispatch_get_main_queue(), ^{
                NSAlert *alert = [[NSAlert alloc] init];
                [alert setAlertStyle:NSAlertStyleWarning];
                [alert setMessageText:[NSString stringWithFormat:localized(@"The debugger cannot listen on port %d."), port]];
                [alert setInformativeText:localized(@"Another program may be using it.")];
                [alert runModal];
            });
        }
    } else {
        _dapFailedPort = 0;
    }

    _dapPort = _dap ? port : 0;
    [[self emuView] setDap:_dap];
}

// Plugs in, swaps or removes the expansion ROM when the setting changed. A
// file that has gone or changed into something else is reported, and the
// setting goes back to what is plugged in.
- (void)applyExpansionRom
{
    NSString *path = settingString(kSettingExpansionRom);

    if (path == _expansionPath || [path isEqualToString:_expansionPath]) {
        return;
    }

    if (!path) {
        juku_expansion_eject(_machine);
        _expansionPath = nil;
        return;
    }

    NSData *image = nil;
    int32_t error = checkExpansionRom(path, &image);

    if (!error) {
        error = juku_expansion_insert(_machine, [image bytes], [image length]);
    }

    if (!error) {
        _expansionPath = path;
        return;
    }

    dispatch_async(dispatch_get_main_queue(), ^{
        showRomAlert(expansionRomError(error), path);
    });

    if (_expansionPath) {
        setSetting(kSettingExpansionRom, _expansionPath);
    } else {
        [[NSUserDefaults standardUserDefaults] removeObjectForKey:kSettingExpansionRom];
    }
}

- (void)toggleCRTEffects:(id)sender
{
    setSetting(kSettingCRTEffects, @(!settingBool(kSettingCRTEffects)));
}

- (void)selectWindowScale:(NSMenuItem *)sender
{
    if ([_window styleMask] & NSWindowStyleMaskFullScreen) {
        return;
    }

    NSRect frame = [_window frame];
    NSRect content = [_window contentRectForFrameRect:frame];
    content.size = contentSizeForHalves((int)[sender tag], tubeMargin(NO));
    NSRect newFrame = [_window frameRectForContentRect:content];
    newFrame.origin.y = NSMaxY(frame) - newFrame.size.height;   // keep the title bar in place

    // Unless that puts the window past the bottom or the side of the screen.
    NSRect visible = [[_window screen] visibleFrame];
    newFrame.origin.y = MAX(newFrame.origin.y, NSMinY(visible));
    newFrame.origin.x = MAX(MIN(newFrame.origin.x, NSMaxX(visible) - newFrame.size.width), NSMinX(visible));
    [_window setFrame:newFrame display:YES animate:YES];
}

- (BOOL)validateMenuItem:(NSMenuItem *)item
{
    int drive = (int)[item tag];

    if ([item action] == @selector(toggleMouseCapture:)) {
        [item setState:[[self emuView] mouseCaptured] ? NSControlStateValueOn : NSControlStateValueOff];
        return YES;
    }

    if ([item action] == @selector(toggleKeyboard:)) {
        [item setState:[_keyboardWindow visible] ? NSControlStateValueOn : NSControlStateValueOff];
        return YES;
    }

    if ([item action] == @selector(togglePause:)) {
        [item setState:[[self emuView] paused] ? NSControlStateValueOn : NSControlStateValueOff];
        return YES;
    }

    if ([item action] == @selector(toggleWarp:)) {
        [item setState:[[self emuView] warp] ? NSControlStateValueOn : NSControlStateValueOff];
        return YES;
    }

    if ([item action] == @selector(selectSystemRom:)) {
        [item setState:[systemRomName((int)[item tag]) isEqualToString:_romName] ? NSControlStateValueOn : NSControlStateValueOff];
        return YES;
    }

    if ([item action] == @selector(selectNoExpansionRom:)) {
        [item setState:_expansionPath ? NSControlStateValueOff : NSControlStateValueOn];
        return YES;
    }

    if ([item action] == @selector(selectExpansionRomFile:)) {
        NSString *path = settingString(kSettingExpansionRomFile);
        [item setHidden:path == nil];
        [item setTitle:[path lastPathComponent] ?: @""];
        [item setToolTip:path];
        [item setState:[path isEqualToString:_expansionPath] ? NSControlStateValueOn : NSControlStateValueOff];
        return path != nil;
    }

    if ([item action] == @selector(selectSystemRomFile:)) {
        NSString *path = settingString(kSettingSystemRomFile);
        [item setHidden:path == nil];
        [item setTitle:[path lastPathComponent] ?: @""];
        [item setToolTip:path];
        [item setState:[path isEqualToString:_romName] ? NSControlStateValueOn : NSControlStateValueOff];
        return path != nil;
    }

    // Checked when the tube is at that size. None in full screen, and none
    // that would not fit on the screen.
    if ([item action] == @selector(selectWindowScale:)) {
        int halves = (int)[item tag];
        BOOL windowed = !([_window styleMask] & NSWindowStyleMaskFullScreen);
        BOOL current = windowed && halvesForContentSize([[_window contentView] bounds].size) == halves;
        [item setState:current ? NSControlStateValueOn : NSControlStateValueOff];
        return windowed && windowFits(_window, contentSizeForHalves(halves, tubeMargin(NO)));
    }

    if ([item action] == @selector(toggleCRTEffects:)) {
        [item setState:settingBool(kSettingCRTEffects) ? NSControlStateValueOn : NSControlStateValueOff];
        return YES;
    }

    if ([item action] == @selector(quickLoad:) || [item action] == @selector(saveQuickSaveAs:)) {
        return [quickSaveURL() checkResourceIsReachableAndReturnError:nil];
    }

    if ([item action] == @selector(saveDisk:) || [item action] == @selector(ejectDisk:)) {
        return [_disks hasDisk:drive];
    }

    if ([item action] == @selector(toggleWriteProtect:)) {
        [item setState:[_disks isWriteProtected:drive] ? NSControlStateValueOn : NSControlStateValueOff];
        return [_disks hasDisk:drive];
    }

    return YES;
}

- (void)applicationWillFinishLaunching:(NSNotification *)aNotification
{
    [Settings registerDefaults];
    // One window only: keep AppKit from adding tab items to the View menu.
    [NSWindow setAllowsAutomaticWindowTabbing:NO];
    // A ROM no longer in the app, or a file that has gone, falls back to the
    // default. A missing file is reported once the window is up.
    _romName = settingString(kSettingSystemRom);
    NSData *rom = loadSystemRom(_romName);

    if (!rom) {
        if (isSystemRomFile(_romName)) {
            NSString *missing = _romName;
            dispatch_async(dispatch_get_main_queue(), ^{
                showRomAlert(@"The ROM file “%@” could not be read.", missing);
            });
        }

        [[NSUserDefaults standardUserDefaults] removeObjectForKey:kSettingSystemRom];
        _romName = kDefaultSystemRom;
        rom = loadSystemRom(_romName);
    }

    _audio = juku_audio_new((uint32_t)kAudioSampleRate);
    _machine = rom ? juku_new([rom bytes], [rom length], _audio) : NULL;

    if (!_machine) {
        fatalError(@"ROM files are missing or damaged.");
    }

    _dapArgPort = debugServerArgument();
    [self applySettings:nil];
    [[NSNotificationCenter defaultCenter] addObserver:self
                                             selector:@selector(applySettings:)
                                                 name:NSUserDefaultsDidChangeNotification
                                               object:nil];

    // Without an output device the machine runs silently on the host clock.
    _audioOutput = [[AudioOutput alloc] initWithRing:_audio];

    _disks = [[DiskDrives alloc] initWithMachine:_machine];
    [_disks restoreDrives];
    createMenubar(_disks);
    _window = createWindow(_machine, _audioOutput ? _audio : NULL, _disks);

    if (!_window) {
        fatalError(@"Metal is not available.");
    }

    [_window setDelegate:self];
    [[self emuView] setDap:_dap];
    __weak AppDelegate *weakSelf = self;
    [[self emuView] setOpenSnapshot:^BOOL(NSString *path) {
        return [weakSelf openSnapshotForDebugger:path];
    }];
}

// A launch's snapshot (spec §12.7), opened as one from Finder is.
- (BOOL)openSnapshotForDebugger:(NSString *)path
{
    NSURL *url = [NSURL fileURLWithPath:path];
    __block BOOL opened = NO;
    [self openSnapshot:[NSData dataWithContentsOfURL:url] from:url done:^{
        opened = YES;
    }];
    return opened;
}

- (void)applicationDidFinishLaunching:(NSNotification *)aNotification
{
    [_window makeKeyAndOrderFront:self];
    [NSApp activate];
}

- (BOOL)applicationShouldTerminateAfterLastWindowClosed:(NSApplication *)sender
{
    return YES;
}

// Files opened with the app (Finder "Open With", `open -a Juku image`): a
// snapshot is restored, and a disk image goes into drive A:.
- (void)application:(NSApplication *)application openURLs:(NSArray<NSURL *> *)urls
{
    NSURL *url = [urls firstObject];

    if (isSnapshotURL(url)) {
        [self openSnapshotURL:url];
    } else {
        [_disks insertURL:url inDrive:0];
    }
}

// Each changed disk asks to be saved first; Cancel stops the quit, a log
// out included.
- (NSApplicationTerminateReply)applicationShouldTerminate:(NSApplication *)sender
{
    for (int drive = 0; drive < JukuDriveCount; drive++) {
        if (![_disks mayRemoveDisk:drive]) {
            return NSTerminateCancel;
        }
    }

    return NSTerminateNow;
}

// Closing the window quits, so the disks are asked about before it goes.
- (BOOL)windowShouldClose:(NSWindow *)window
{
    [NSApp terminate:window];
    return NO;
}

- (void)applicationWillTerminate:(NSNotification *)aNotification
{
    [_audioOutput stop];
    _audioOutput = nil;
    juku_dap_free(_dap, _machine);
    _dap = NULL;
    juku_free(_machine);
    _machine = NULL;
    juku_audio_free(_audio);
    _audio = NULL;
}

@end

int main(int argc, const char **argv)
{
    [NSApplication sharedApplication];
    [NSApp setDelegate:[[AppDelegate alloc] init]];
    [NSApp run];
    return 0;
}
