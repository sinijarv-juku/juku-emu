#pragma once

#import <AppKit/AppKit.h>

#import "Roms.m"
#import "Settings.m"
#import "Strings.m"

// The Settings window (spec §6.7): General (mouse, language), Display (the
// CRT), Sound (the key click and the drive sounds) and Machine (the ROMs, the
// T3.2 clock and the debugger), as panes chosen from a toolbar, each a simple
// form built in code.
// Changes apply immediately; the menu and this window stay in sync through
// NSUserDefaults notifications.
@interface SettingsWindow: NSObject<NSWindowDelegate, NSToolbarDelegate>

- (void)show;
// False grays out Key Click: the ROM in the machine has no click code the
// patch can change (spec §4.3).
- (void)setKeyClickAvailable:(BOOL)available;

@end

@implementation SettingsWindow {
    NSWindow *_window;
    NSArray<NSView *> *_panes;
    NSInteger _pane;
    NSButton *_crtEffects;
    NSButton *_bezel3d;
    NSButton *_monitorFront;
    NSButton *_keyClick;
    NSButton *_driveSounds;
    NSButton *_driveIdleMute;
    NSTextField *_driveVolumeLabel;
    NSTextField *_driveSpreadLabel;
    NSTextField *_driveSpreadNote;
    NSButton *_t32FastClock;
    NSButton *_debugger;
    NSTextField *_debuggerNote;
    NSGridRow *_keyClickNoteRow;
    BOOL _keyClickAvailable;
    NSDictionary<NSString *, NSSlider *> *_sliders;
    NSPopUpButton *_systemRom;
    NSPopUpButton *_expansionRom;
    NSPopUpButton *_language;
    NSButton *_restoreCrt;
}

// The panes in toolbar order: the toolbar item's identifier, which is also
// its label's key, and its SF Symbol.
enum { kPaneGeneral, kPaneDisplay, kPaneSound, kPaneMachine, kPaneCount };

static NSString *const kPaneNames[kPaneCount] = {@"General", @"Display", @"Sound", @"Machine"};
static NSString *const kPaneSymbols[kPaneCount] = {@"gearshape", @"display", @"speaker.wave.2", @"cpu"};

// Language codes in the Language pop-up; NSNull is "System Default". The
// languages are named in their own language, as macOS does.
static NSArray *languageCodes(void)
{
    return @[[NSNull null], @"en", @"et"];
}

static NSTextField *formLabel(NSString *key)
{
    NSTextField *label = [NSTextField labelWithString:localized(key)];
    [label setAlignment:NSTextAlignmentRight];
    return label;
}

static NSTextField *formNote(NSString *key)
{
    NSTextField *note = [NSTextField labelWithString:localized(key)];
    [note setFont:[NSFont systemFontOfSize:[NSFont smallSystemFontSize]]];
    [note setTextColor:[NSColor secondaryLabelColor]];
    return note;
}

// A note that wraps at `width`: one longer than its column's controls are
// wide, which would otherwise widen the column and the window to its length.
static NSTextField *formWrappingNote(NSString *key, CGFloat width)
{
    NSTextField *note = [NSTextField wrappingLabelWithString:localized(key)];
    [note setSelectable:NO];
    [note setFont:[NSFont systemFontOfSize:[NSFont smallSystemFontSize]]];
    [note setTextColor:[NSColor secondaryLabelColor]];
    [note setPreferredMaxLayoutWidth:width];
    return note;
}

static NSGridView *formGrid(NSArray<NSArray<NSView *> *> *rows)
{
    NSGridView *grid = [NSGridView gridViewWithViews:rows];
    [grid setRowSpacing:10.0];
    [grid setColumnSpacing:8.0];
    [grid setTranslatesAutoresizingMaskIntoConstraints:NO];
    [[grid columnAtIndex:0] setXPlacement:NSGridCellPlacementTrailing];
    [grid setRowAlignment:NSGridRowAlignmentFirstBaseline];
    return grid;
}

// A row that starts a group of its own (12) or holds the note under the row
// above it (-4).
static void padRows(NSGridView *grid, NSArray<NSNumber *> *groups, NSArray<NSNumber *> *notes)
{
    for (NSNumber *row in groups) {
        [[grid rowAtIndex:[row integerValue]] setTopPadding:12.0];
    }

    for (NSNumber *row in notes) {
        [[grid rowAtIndex:[row integerValue]] setTopPadding:-4.0];
    }
}

// The form with the window's margins around it; its fitting size is the
// window's content size while the pane is shown.
static NSView *paneView(NSGridView *grid)
{
    NSView *pane = [[NSView alloc] initWithFrame:NSZeroRect];
    [pane addSubview:grid];
    [NSLayoutConstraint activateConstraints:@[
        [[grid leadingAnchor] constraintEqualToAnchor:[pane leadingAnchor] constant:20.0],
        [[grid trailingAnchor] constraintEqualToAnchor:[pane trailingAnchor] constant:-20.0],
        [[grid topAnchor] constraintEqualToAnchor:[pane topAnchor] constant:20.0],
        [[grid bottomAnchor] constraintEqualToAnchor:[pane bottomAnchor] constant:-20.0],
    ]];
    return pane;
}

- (NSSlider *)sliderFor:(NSString *)key min:(double)min max:(double)max
{
    NSSlider *slider = [NSSlider sliderWithValue:settingDouble(key) minValue:min maxValue:max target:self action:@selector(sliderChanged:)];
    [slider setIdentifier:key];
    [slider setContinuous:YES];
    [[slider widthAnchor] constraintEqualToConstant:220.0].active = YES;
    return slider;
}

- (NSView *)generalPane
{
    _language = [[NSPopUpButton alloc] initWithFrame:NSZeroRect pullsDown:NO];
    [_language addItemsWithTitles:@[localized(@"System Default"), @"English", @"Eesti"]];
    [_language setTarget:self];
    [_language setAction:@selector(languageChanged:)];

    NSGridView *grid = formGrid(@[
        @[formLabel(@"Mouse Speed:"), _sliders[kSettingMouseSensitivity]],
        @[formLabel(@"Language:"), _language],
        @[[NSGridCell emptyContentView], formNote(@"Takes effect the next time Juku opens.")],
    ]);
    padRows(grid, @[@1], @[@2]);
    return paneView(grid);
}

- (NSView *)soundPane
{
    _keyClick = [NSButton checkboxWithTitle:localized(@"Key Click") target:self action:@selector(keyClickChanged:)];
    _driveSounds = [NSButton checkboxWithTitle:localized(@"Drive Sounds") target:self action:@selector(driveSoundsChanged:)];
    _driveIdleMute = [NSButton checkboxWithTitle:localized(@"Fade Out the Motor When Idle for 10 s") target:self action:@selector(driveIdleMuteChanged:)];
    _driveVolumeLabel = formLabel(@"Drive Volume:");
    _driveSpreadLabel = formLabel(@"Drive Separation:");
    // As wide as the widest control in the column, Fade Out's checkbox, but
    // no narrower than 290, at which the English note takes three lines.
    CGFloat noteWidth = fmax([_driveIdleMute fittingSize].width, 290.0);
    _driveSpreadNote = formWrappingNote(@"The two drives sat side by side in the disk unit under the monitor, A: on the left. The default places them as heard from in front of the monitor.", noteWidth);

    NSGridView *grid = formGrid(@[
        @[formLabel(@"Keyboard:"), _keyClick],
        @[[NSGridCell emptyContentView], formNote(@"This ROM's key click cannot be changed.")],
        @[formLabel(@"Disk Drives:"), _driveSounds],
        @[_driveVolumeLabel, _sliders[kSettingDriveSoundsVolume]],
        @[_driveSpreadLabel, _sliders[kSettingDriveSoundsSpread]],
        @[[NSGridCell emptyContentView], _driveSpreadNote],
        @[[NSGridCell emptyContentView], _driveIdleMute],
    ]);
    padRows(grid, @[@2], @[@1, @5]);
    // The note goes with the slider above it, not with the checkbox below.
    [[grid rowAtIndex:6] setTopPadding:4.0];
    _keyClickNoteRow = [grid rowAtIndex:1];
    return paneView(grid);
}

- (NSView *)displayPane
{
    _crtEffects = [NSButton checkboxWithTitle:localized(@"CRT Effects") target:self action:@selector(crtEffectsChanged:)];
    _bezel3d = [NSButton checkboxWithTitle:localized(@"3D Frame") target:self action:@selector(bezel3dChanged:)];
    _monitorFront = [NSButton checkboxWithTitle:localized(@"Whole Monitor") target:self action:@selector(monitorFrontChanged:)];
    _restoreCrt = [NSButton buttonWithTitle:localized(@"Restore CRT Defaults") target:self action:@selector(restoreCrtDefaults:)];

    NSGridView *grid = formGrid(@[
        @[[NSGridCell emptyContentView], _crtEffects],
        @[formLabel(@"Curvature:"), _sliders[kSettingCurvature]],
        @[formLabel(@"Scanlines:"), _sliders[kSettingScanlines]],
        @[formLabel(@"Glow:"), _sliders[kSettingGlow]],
        @[formLabel(@"Background:"), _sliders[kSettingBackground]],
        @[formLabel(@"Frame:"), _sliders[kSettingFrame]],
        @[formLabel(@"Room Light:"), _sliders[kSettingRoom]],
        @[[NSGridCell emptyContentView], _bezel3d],
        @[[NSGridCell emptyContentView], _monitorFront],
        @[formLabel(@"Brightness:"), _sliders[kSettingBrightness]],
        @[[NSGridCell emptyContentView], _restoreCrt],
    ]);
    padRows(grid, @[@10], @[]);
    return paneView(grid);
}

- (NSView *)machinePane
{
    _systemRom = [[NSPopUpButton alloc] initWithFrame:NSZeroRect pullsDown:NO];
    [_systemRom setTarget:self];
    [_systemRom setAction:@selector(systemRomChanged:)];

    _expansionRom = [[NSPopUpButton alloc] initWithFrame:NSZeroRect pullsDown:NO];
    [_expansionRom setTarget:self];
    [_expansionRom setAction:@selector(expansionRomChanged:)];

    _t32FastClock = [NSButton checkboxWithTitle:localized(@"Clock at 1.23 MHz") target:self action:@selector(t32FastClockChanged:)];
    _debugger = [NSButton checkboxWithTitle:localized(@"Accept Connections") target:self action:@selector(debuggerChanged:)];
    _debuggerNote = formNote(@"");

    NSGridView *grid = formGrid(@[
        @[formLabel(@"System ROM:"), _systemRom],
        @[[NSGridCell emptyContentView], formNote(@"Changing the ROM restarts the machine.")],
        @[formLabel(@"Expansion ROM:"), _expansionRom],
        @[[NSGridCell emptyContentView], formNote(@"Start it with A at the * prompt. Changing it restarts the machine.")],
        @[formLabel(@"Timer T3.2:"), _t32FastClock],
        @[[NSGridCell emptyContentView], formNote(@"RDEM needs it, but the RomBios stopwatch then runs too fast.")],
        @[formLabel(@"Debugger:"), _debugger],
        @[[NSGridCell emptyContentView], _debuggerNote],
    ]);
    padRows(grid, @[@2, @4, @6], @[@1, @3, @5, @7]);
    return paneView(grid);
}

- (void)build
{
    _sliders = @{
        kSettingCurvature: [self sliderFor:kSettingCurvature min:0.0 max:0.12],
        kSettingScanlines: [self sliderFor:kSettingScanlines min:0.0 max:1.0],
        kSettingGlow: [self sliderFor:kSettingGlow min:0.0 max:0.4],
        kSettingBackground: [self sliderFor:kSettingBackground min:0.0 max:1.0],
        kSettingFrame: [self sliderFor:kSettingFrame min:0.0 max:1.0],
        kSettingRoom: [self sliderFor:kSettingRoom min:0.0 max:1.0],
        kSettingBrightness: [self sliderFor:kSettingBrightness min:0.5 max:1.5],
        kSettingMouseSensitivity: [self sliderFor:kSettingMouseSensitivity min:0.1 max:2.0],
        kSettingDriveSoundsVolume: [self sliderFor:kSettingDriveSoundsVolume min:0.0 max:2.0],
        kSettingDriveSoundsSpread: [self sliderFor:kSettingDriveSoundsSpread min:0.0 max:1.0],
    };

    _panes = @[[self generalPane], [self displayPane], [self soundPane], [self machinePane]];

    _window = [[NSWindow alloc] initWithContentRect:NSZeroRect
                                          styleMask:NSWindowStyleMaskTitled | NSWindowStyleMaskClosable
                                            backing:NSBackingStoreBuffered
                                              defer:YES];
    [_window setReleasedWhenClosed:NO];
    [_window setDelegate:self];

    NSToolbar *toolbar = [[NSToolbar alloc] initWithIdentifier:@"Settings"];
    [toolbar setDelegate:self];
    [_window setToolbar:toolbar];
    [_window setToolbarStyle:NSWindowToolbarStylePreference];

    [[NSNotificationCenter defaultCenter] addObserver:self
                                             selector:@selector(defaultsChanged:)
                                                 name:NSUserDefaultsDidChangeNotification
                                               object:nil];
}

- (NSArray<NSToolbarItemIdentifier> *)paneIdentifiers
{
    return [NSArray arrayWithObjects:kPaneNames count:kPaneCount];
}

- (NSArray<NSToolbarItemIdentifier> *)toolbarAllowedItemIdentifiers:(NSToolbar *)toolbar
{
    return [self paneIdentifiers];
}

- (NSArray<NSToolbarItemIdentifier> *)toolbarDefaultItemIdentifiers:(NSToolbar *)toolbar
{
    return [self paneIdentifiers];
}

- (NSArray<NSToolbarItemIdentifier> *)toolbarSelectableItemIdentifiers:(NSToolbar *)toolbar
{
    return [self paneIdentifiers];
}

- (NSToolbarItem *)toolbar:(NSToolbar *)toolbar itemForItemIdentifier:(NSToolbarItemIdentifier)identifier willBeInsertedIntoToolbar:(BOOL)flag
{
    NSInteger pane = [[self paneIdentifiers] indexOfObject:identifier];
    NSToolbarItem *item = [[NSToolbarItem alloc] initWithItemIdentifier:identifier];
    [item setLabel:localized(identifier)];
    [item setImage:[NSImage imageWithSystemSymbolName:kPaneSymbols[pane] accessibilityDescription:nil]];
    [item setTag:pane];
    [item setTarget:self];
    [item setAction:@selector(paneChosen:)];
    return item;
}

- (void)paneChosen:(NSToolbarItem *)sender
{
    [self showPane:[sender tag] animate:YES];
}

// Shows the pane with its name as the window's title, as System Settings
// does, and fits the window to it, keeping the title bar in place. The new
// pane goes in once the window has its size, so its constraints never
// fight the animation.
- (void)showPane:(NSInteger)pane animate:(BOOL)animate
{
    _pane = pane;
    [[_window toolbar] setSelectedItemIdentifier:kPaneNames[pane]];
    [_window setTitle:localized(kPaneNames[pane])];
    [_window setContentView:[[NSView alloc] initWithFrame:NSZeroRect]];
    [self fitWindowTo:_panes[pane] animate:animate];
    [_window setContentView:_panes[pane]];
}

- (void)fitWindowTo:(NSView *)pane animate:(BOOL)animate
{
    NSRect frame = [_window frame];
    NSRect content = [_window contentRectForFrameRect:frame];
    content.size = [pane fittingSize];
    NSRect newFrame = [_window frameRectForContentRect:content];
    newFrame.origin.y = NSMaxY(frame) - newFrame.size.height;
    [_window setFrame:newFrame display:YES animate:animate && [_window isVisible]];
}

- (instancetype)init
{
    self = [super init];
    _keyClickAvailable = YES;
    return self;
}

- (void)setKeyClickAvailable:(BOOL)available
{
    _keyClickAvailable = available;
    [_keyClick setEnabled:available];

    if (!_keyClickNoteRow || [_keyClickNoteRow isHidden] == available) {
        return;
    }

    // Another pane fits the window when it is shown. Later, because this
    // may be called from a defaults change posted while a fitting size is
    // being measured, and measuring again inside it aborts in the text
    // layout.
    [_keyClickNoteRow setHidden:available];
    dispatch_async(dispatch_get_main_queue(), ^{
        if (self->_pane == kPaneSound && [self->_window contentView] == self->_panes[kPaneSound]) {
            [self fitWindowTo:self->_panes[kPaneSound] animate:NO];
        }
    });
}

- (void)show
{
    if (!_window) {
        [self build];
        [self setKeyClickAvailable:_keyClickAvailable];
        [self defaultsChanged:nil];
        [self showPane:_pane animate:NO];
        [_window center];
    }

    [_window makeKeyAndOrderFront:nil];
}

- (void)defaultsChanged:(NSNotification *)notification
{
    [_crtEffects setState:settingBool(kSettingCRTEffects) ? NSControlStateValueOn : NSControlStateValueOff];
    [_bezel3d setState:settingBool(kSettingBezel3D) ? NSControlStateValueOn : NSControlStateValueOff];
    [_monitorFront setState:settingBool(kSettingMonitorFront) ? NSControlStateValueOn : NSControlStateValueOff];
    [_keyClick setState:settingBool(kSettingKeyClick) ? NSControlStateValueOn : NSControlStateValueOff];
    [_driveSounds setState:settingBool(kSettingDriveSounds) ? NSControlStateValueOn : NSControlStateValueOff];
    [_driveIdleMute setState:settingBool(kSettingDriveIdleMute) ? NSControlStateValueOn : NSControlStateValueOff];
    [_t32FastClock setState:settingBool(kSettingT32FastClock) ? NSControlStateValueOn : NSControlStateValueOff];
    [_debugger setState:settingBool(kSettingDebugger) ? NSControlStateValueOn : NSControlStateValueOff];
    [_debuggerNote setStringValue:[NSString stringWithFormat:localized(@"VS Code and other DAP clients connect on port %ld, from this Mac only."),
                                                             (long)settingInteger(kSettingDebuggerPort)]];
    // The drive sounds' settings apply only while they are on, and the
    // sliders' labels are grayed with them.
    BOOL driveSounds = settingBool(kSettingDriveSounds);
    [_sliders[kSettingDriveSoundsVolume] setEnabled:driveSounds];
    [_sliders[kSettingDriveSoundsSpread] setEnabled:driveSounds];
    [_driveVolumeLabel setTextColor:driveSounds ? [NSColor labelColor] : [NSColor disabledControlTextColor]];
    [_driveSpreadLabel setTextColor:driveSounds ? [NSColor labelColor] : [NSColor disabledControlTextColor]];
    [_driveSpreadNote setTextColor:driveSounds ? [NSColor secondaryLabelColor] : [NSColor disabledControlTextColor]];
    [_driveIdleMute setEnabled:driveSounds];

    for (NSString *key in _sliders) {
        [_sliders[key] setDoubleValue:settingDouble(key)];
    }

    // Only the app's own domain: the registered defaults are not changes.
    NSDictionary *saved = [[NSUserDefaults standardUserDefaults] persistentDomainForName:[[NSBundle mainBundle] bundleIdentifier]];
    BOOL changed = NO;

    for (NSString *key in crtSettingKeys()) {
        changed = changed || saved[key] != nil;
    }

    [_restoreCrt setEnabled:changed];

    [self updateSystemRom];
    [self updateExpansionRom];

    // System Settings may store a region too, as in "et-EE".
    NSString *language = settingLanguage();
    NSInteger index = 0;

    for (NSInteger i = 1; language && i < (NSInteger)[languageCodes() count]; i++) {
        NSString *code = languageCodes()[i];

        if ([language isEqualToString:code] || [language hasPrefix:[code stringByAppendingString:@"-"]]) {
            index = i;
        }
    }

    [_language selectItemAtIndex:index];
}

- (void)crtEffectsChanged:(NSButton *)sender
{
    setSetting(kSettingCRTEffects, @([sender state] == NSControlStateValueOn));
}

- (void)bezel3dChanged:(NSButton *)sender
{
    setSetting(kSettingBezel3D, @([sender state] == NSControlStateValueOn));
}

- (void)monitorFrontChanged:(NSButton *)sender
{
    setSetting(kSettingMonitorFront, @([sender state] == NSControlStateValueOn));
}

- (void)keyClickChanged:(NSButton *)sender
{
    setSetting(kSettingKeyClick, @([sender state] == NSControlStateValueOn));
}

- (void)driveSoundsChanged:(NSButton *)sender
{
    setSetting(kSettingDriveSounds, @([sender state] == NSControlStateValueOn));
}

- (void)driveIdleMuteChanged:(NSButton *)sender
{
    setSetting(kSettingDriveIdleMute, @([sender state] == NSControlStateValueOn));
}

- (void)t32FastClockChanged:(NSButton *)sender
{
    setSetting(kSettingT32FastClock, @([sender state] == NSControlStateValueOn));
}

- (void)debuggerChanged:(NSButton *)sender
{
    setSetting(kSettingDebugger, @([sender state] == NSControlStateValueOn));
}

// The bundled ROMs, the user's last ROM file if any, and Other…, as in
// Machine → System ROM. Each item but Other… holds its ROM's name.
- (void)updateSystemRom
{
    [_systemRom removeAllItems];

    for (int i = 0; i < kSystemRomCount; i++) {
        [_systemRom addItemWithTitle:systemRomTitle(i)];
        [[_systemRom lastItem] setRepresentedObject:systemRomName(i)];
    }

    [[_systemRom menu] addItem:[NSMenuItem separatorItem]];
    NSString *file = settingString(kSettingSystemRomFile);

    if (file) {
        [_systemRom addItemWithTitle:[file lastPathComponent]];
        [[_systemRom lastItem] setRepresentedObject:file];
        [[_systemRom lastItem] setToolTip:file];
    }

    [_systemRom addItemWithTitle:localized(@"Other…")];
    NSInteger index = [_systemRom indexOfItemWithRepresentedObject:settingString(kSettingSystemRom)];
    [_systemRom selectItemAtIndex:index >= 0 ? index : systemRomIndex(kDefaultSystemRom)];
}

- (void)systemRomChanged:(NSPopUpButton *)sender
{
    NSString *rom = [[sender selectedItem] representedObject];

    if (rom) {
        setSetting(kSettingSystemRom, rom);
        return;
    }

    // Other…: show the ROM in use until a file is chosen. The app delegate
    // runs the panel, as for the menu item.
    [self updateSystemRom];
    [NSApp sendAction:@selector(chooseSystemRomFile:) to:nil from:sender];
}

// None, the user's last expansion ROM file if any, and Other…, as in
// Machine → Expansion ROM. None holds NSNull and the file its path.
- (void)updateExpansionRom
{
    [_expansionRom removeAllItems];
    [_expansionRom addItemWithTitle:localized(@"None")];
    [[_expansionRom lastItem] setRepresentedObject:[NSNull null]];
    [[_expansionRom menu] addItem:[NSMenuItem separatorItem]];
    NSString *file = settingString(kSettingExpansionRomFile);

    if (file) {
        [_expansionRom addItemWithTitle:[file lastPathComponent]];
        [[_expansionRom lastItem] setRepresentedObject:file];
        [[_expansionRom lastItem] setToolTip:file];
    }

    [_expansionRom addItemWithTitle:localized(@"Other…")];
    NSString *current = settingString(kSettingExpansionRom);
    NSInteger index = current ? [_expansionRom indexOfItemWithRepresentedObject:current] : 0;
    [_expansionRom selectItemAtIndex:MAX(index, 0)];
}

- (void)expansionRomChanged:(NSPopUpButton *)sender
{
    id rom = [[sender selectedItem] representedObject];

    if (rom == [NSNull null]) {
        [[NSUserDefaults standardUserDefaults] removeObjectForKey:kSettingExpansionRom];
        return;
    }

    if (rom) {
        setSetting(kSettingExpansionRom, rom);
        return;
    }

    // Other…: as for the system ROM.
    [self updateExpansionRom];
    [NSApp sendAction:@selector(chooseExpansionRomFile:) to:nil from:sender];
}

- (void)languageChanged:(NSPopUpButton *)sender
{
    id code = languageCodes()[[sender indexOfSelectedItem]];
    setSettingLanguage(code == [NSNull null] ? nil : code);
}

// Back to the registered defaults; CRT Effects and 3D Frame on or off are
// left alone.
- (void)restoreCrtDefaults:(NSButton *)sender
{
    for (NSString *key in crtSettingKeys()) {
        [[NSUserDefaults standardUserDefaults] removeObjectForKey:key];
    }
}

- (void)sliderChanged:(NSSlider *)sender
{
    setSetting([sender identifier], @([sender doubleValue]));
}

@end
