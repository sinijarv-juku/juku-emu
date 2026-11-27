#pragma once

#import <AppKit/AppKit.h>

#import "KeyboardView.m"
#import "Keymap.m"
#import "Settings.m"
#import "Strings.m"

// Input → Keyboard (spec §6.4): the Juku keyboard in a floating panel that
// never takes the keyboard from the emulator window, so typing goes on while
// it is open. Below the keyboard, an info strip describes the key under the
// pointer or the one last pressed, and shows RomBios's LAT/RUS and CAPS state.
@interface KeyboardWindow: NSObject<NSWindowDelegate>

- (instancetype)initWithMachine:(JukuMachine *)machine;
// Shows the window, or closes it if it is open.
- (void)toggle;
@property(nonatomic, readonly) BOOL visible;

@end

// LAT, RUS or CAPS, lit while that mode is on.
@interface ModePill: NSView
@property(nonatomic, copy) NSString *text;
@property(nonatomic) BOOL on;
@end

@implementation ModePill

- (BOOL)isFlipped
{
    return YES;
}

- (void)setOn:(BOOL)on
{
    _on = on;
    [self setNeedsDisplay:YES];
}

- (NSDictionary *)attributes:(NSColor *)color
{
    return @{
        NSFontAttributeName: [NSFont systemFontOfSize:10.5 weight:NSFontWeightBold],
        NSForegroundColorAttributeName: color,
        NSKernAttributeName: @0.8,
    };
}

- (NSSize)intrinsicContentSize
{
    return NSMakeSize(ceil([_text sizeWithAttributes:[self attributes:[NSColor labelColor]]].width) + 18, 19);
}

- (void)drawRect:(NSRect)dirtyRect
{
    NSRect bounds = NSInsetRect([self bounds], 0.5, 0.5);
    NSBezierPath *pill = [NSBezierPath bezierPathWithRoundedRect:bounds xRadius:NSHeight(bounds) / 2 yRadius:NSHeight(bounds) / 2];

    if (_on) {
        [[NSColor labelColor] set];
        [pill fill];
    } else {
        [[NSColor tertiaryLabelColor] set];
        [pill stroke];
    }

    NSDictionary *attributes = [self attributes:_on ? [NSColor windowBackgroundColor] : [NSColor secondaryLabelColor]];
    NSSize size = [_text sizeWithAttributes:attributes];
    [_text drawAtPoint:NSMakePoint(NSMidX(bounds) - (size.width - 0.8) / 2, NSMidY(bounds) - size.height / 2) withAttributes:attributes];
}

@end

@implementation KeyboardWindow {
    JukuMachine *_machine;
    NSPanel *_panel;
    KeyboardView *_keyboard;
    KeycapPreview *_preview;
    NSTextField *_name;
    NSTextField *_line;
    NSTextField *_meta;
    ModePill *_lat;
    ModePill *_rus;
    ModePill *_caps;
    NSArray<NSArray<HostKey *> *> *_hostKeys;   // per keycap
}

- (instancetype)initWithMachine:(JukuMachine *)machine
{
    self = [super init];

    if (self) {
        _machine = machine;
    }

    return self;
}

- (BOOL)visible
{
    return [_panel isVisible];
}

- (void)toggle
{
    if ([_panel isVisible]) {
        [_panel performClose:nil];
        return;
    }

    if (!_panel) {
        [self build];

        if ([_panel setFrameUsingName:@"Keyboard"]) {
            [self fitFrame];
        } else {
            [_panel center];
        }

        [_panel setFrameAutosaveName:@"Keyboard"];
    }

    [self updateTags];
    [_panel orderFront:nil];
}

static NSTextField *stripLabel(NSFont *font, NSColor *color)
{
    NSTextField *label = [NSTextField labelWithString:@""];
    [label setFont:font];
    [label setTextColor:color];
    [label setLineBreakMode:NSLineBreakByTruncatingTail];
    [label setContentCompressionResistancePriority:NSLayoutPriorityDefaultLow forOrientation:NSLayoutConstraintOrientationHorizontal];
    return label;
}

- (void)build
{
    _keyboard = [[KeyboardView alloc] initWithMachine:_machine];
    [_keyboard setTranslatesAutoresizingMaskIntoConstraints:NO];
    [_keyboard setTagMode:settingInteger(kSettingKeyboardTags)];
    __weak KeyboardWindow *weakSelf = self;
    [_keyboard setShowKey:^(NSInteger index) {
        [weakSelf showKey:index];
    }];
    [_keyboard setModeChanged:^(uint8_t consw) {
        [weakSelf showModes:consw];
    }];

    _preview = [[KeycapPreview alloc] initWithFrame:NSZeroRect];
    _name = stripLabel([NSFont systemFontOfSize:14 weight:NSFontWeightSemibold], [NSColor labelColor]);
    _line = stripLabel([NSFont systemFontOfSize:[NSFont systemFontSize]], [NSColor labelColor]);
    _meta = stripLabel([NSFont systemFontOfSize:[NSFont smallSystemFontSize]], [NSColor secondaryLabelColor]);
    NSStackView *info = [NSStackView stackViewWithViews:@[_name, _line, _meta]];
    [info setOrientation:NSUserInterfaceLayoutOrientationVertical];
    [info setAlignment:NSLayoutAttributeLeading];
    [info setSpacing:3];

    NSTextField *tagsLabel = [NSTextField labelWithString:localized(@"Mac Keys:")];
    [tagsLabel setFont:[NSFont systemFontOfSize:[NSFont smallSystemFontSize]]];
    [tagsLabel setTextColor:[NSColor secondaryLabelColor]];
    NSSegmentedControl *tags = [NSSegmentedControl segmentedControlWithLabels:@[localized(@"Where Different"), localized(@"All"), localized(@"Off")]
                                                                 trackingMode:NSSegmentSwitchTrackingSelectOne
                                                                       target:self
                                                                       action:@selector(tagModeChanged:)];
    [tags setControlSize:NSControlSizeSmall];
    [tags setFont:[NSFont systemFontOfSize:[NSFont smallSystemFontSize]]];
    [tags setSelectedSegment:[_keyboard tagMode]];
    NSStackView *tagsRow = [NSStackView stackViewWithViews:@[tagsLabel, tags]];
    [tagsRow setSpacing:8];

    _lat = [[ModePill alloc] initWithFrame:NSZeroRect];
    [_lat setText:@"LAT"];
    _rus = [[ModePill alloc] initWithFrame:NSZeroRect];
    [_rus setText:@"RUS"];
    _caps = [[ModePill alloc] initWithFrame:NSZeroRect];
    [_caps setText:@"CAPS"];
    NSStackView *modes = [NSStackView stackViewWithViews:@[_lat, _rus, _caps]];
    [modes setSpacing:6];

    NSStackView *controls = [NSStackView stackViewWithViews:@[tagsRow, modes]];
    [controls setOrientation:NSUserInterfaceLayoutOrientationVertical];
    [controls setAlignment:NSLayoutAttributeTrailing];
    [controls setSpacing:8];

    NSStackView *strip = [[NSStackView alloc] initWithFrame:NSZeroRect];
    [strip addView:_preview inGravity:NSStackViewGravityLeading];
    [strip addView:info inGravity:NSStackViewGravityLeading];
    [strip addView:controls inGravity:NSStackViewGravityTrailing];
    [strip setSpacing:16];
    [strip setAlignment:NSLayoutAttributeCenterY];
    [strip setEdgeInsets:NSEdgeInsetsMake(10, 16, 12, 16)];
    [strip setTranslatesAutoresizingMaskIntoConstraints:NO];
    [strip setHuggingPriority:NSLayoutPriorityDefaultHigh forOrientation:NSLayoutConstraintOrientationVertical];
    [info setContentHuggingPriority:NSLayoutPriorityDefaultLow forOrientation:NSLayoutConstraintOrientationHorizontal];

    NSBox *separator = [[NSBox alloc] initWithFrame:NSZeroRect];
    [separator setBoxType:NSBoxSeparator];
    [separator setTranslatesAutoresizingMaskIntoConstraints:NO];

    // Half as wide as the screen, whose physical size is not known: at most
    // 1240, where the key tags are at the system font's size, and at least 600.
    CGFloat screen = NSWidth([[NSScreen mainScreen] visibleFrame]);
    CGFloat width = MAX(600, MIN(1240, round(screen / 2))), aspect = keyboardSize().height / keyboardSize().width;
    _panel = [[NSPanel alloc] initWithContentRect:NSMakeRect(0, 0, width, width * aspect + 80)
                                        styleMask:NSWindowStyleMaskTitled | NSWindowStyleMaskClosable | NSWindowStyleMaskResizable |
                                                  NSWindowStyleMaskUtilityWindow | NSWindowStyleMaskNonactivatingPanel
                                          backing:NSBackingStoreBuffered
                                            defer:YES];
    [_panel setTitle:localized(@"Keyboard")];
    [_panel setReleasedWhenClosed:NO];
    [_panel setBecomesKeyOnlyIfNeeded:YES];
    [_panel setFloatingPanel:YES];
    [_panel setDelegate:self];
    [_panel setContentMinSize:NSMakeSize(600, 600 * aspect + 80)];

    NSView *content = [_panel contentView];
    [content addSubview:_keyboard];
    [content addSubview:separator];
    [content addSubview:strip];
    NSLayoutConstraint *proportions = [[_keyboard heightAnchor] constraintEqualToAnchor:[_keyboard widthAnchor] multiplier:aspect];
    [proportions setPriority:NSLayoutPriorityDefaultHigh];
    [NSLayoutConstraint activateConstraints:@[
        [[_keyboard topAnchor] constraintEqualToAnchor:[content topAnchor]],
        [[_keyboard leadingAnchor] constraintEqualToAnchor:[content leadingAnchor]],
        [[_keyboard trailingAnchor] constraintEqualToAnchor:[content trailingAnchor]],
        proportions,
        [[separator topAnchor] constraintEqualToAnchor:[_keyboard bottomAnchor]],
        [[separator leadingAnchor] constraintEqualToAnchor:[content leadingAnchor]],
        [[separator trailingAnchor] constraintEqualToAnchor:[content trailingAnchor]],
        [[strip topAnchor] constraintEqualToAnchor:[separator bottomAnchor]],
        [[strip leadingAnchor] constraintEqualToAnchor:[content leadingAnchor]],
        [[strip trailingAnchor] constraintEqualToAnchor:[content trailingAnchor]],
        [[strip bottomAnchor] constraintEqualToAnchor:[content bottomAnchor]],
        [[_preview widthAnchor] constraintEqualToConstant:110],
        [[_preview heightAnchor] constraintEqualToConstant:54],
    ]];

    // The tags follow the Mac's keyboard layout.
    [[NSDistributedNotificationCenter defaultCenter] addObserver:self
                                                        selector:@selector(inputSourceChanged:)
                                                            name:(__bridge NSString *)kTISNotifySelectedKeyboardInputSourceChanged
                                                          object:nil];
    [self showKey:-1];
}

- (void)dealloc
{
    [[NSDistributedNotificationCenter defaultCenter] removeObserver:self];
}

// The window keeps the keyboard's proportions; the strip keeps its height.
- (NSSize)windowWillResize:(NSWindow *)window toSize:(NSSize)frameSize
{
    NSRect content = [window contentRectForFrameRect:(NSRect){ NSZeroPoint, frameSize }];
    CGFloat strip = NSHeight([[window contentView] frame]) - NSHeight([_keyboard frame]);
    content.size.height = round(NSWidth(content) * keyboardSize().height / keyboardSize().width + strip);
    return [window frameRectForContentRect:content].size;
}

// A frame saved by another version may have other proportions: keeps its
// width and top edge.
- (void)fitFrame
{
    [[_panel contentView] layoutSubtreeIfNeeded];
    NSRect frame = [_panel frame];
    NSSize size = [self windowWillResize:_panel toSize:frame.size];
    frame.origin.y += NSHeight(frame) - size.height;
    frame.size = size;
    [_panel setFrame:frame display:NO];
}

- (void)windowWillClose:(NSNotification *)notification
{
    [_keyboard releaseClicks];
}

- (void)inputSourceChanged:(NSNotification *)notification
{
    dispatch_async(dispatch_get_main_queue(), ^{
        [self updateTags];
    });
}

- (void)tagModeChanged:(NSSegmentedControl *)sender
{
    setSetting(kSettingKeyboardTags, @([sender selectedSegment]));
    [_keyboard setTagMode:[sender selectedSegment]];
}

// Each keycap's Mac keys in the current input source. Its tag names the
// first key not where the Juku legend says, if there is one, else the key
// that is.
- (void)updateTags
{
    NSMutableArray *hostKeys = [NSMutableArray array];
    NSMutableArray *tags = [NSMutableArray array];
    NSMutableIndexSet *differing = [NSMutableIndexSet indexSet];

    for (NSInteger i = 0; i < [_keyboard keycapCount]; i++) {
        NSArray<HostKey *> *keys = hostKeysForKeycap([_keyboard keycapAtIndex:i]);
        HostKey *tag = nil;

        for (HostKey *key in keys) {
            if (![key keypad] && (!tag || ([tag natural] && ![key natural]))) {
                tag = key;
            }
        }

        if (tag && ![tag natural]) {
            [differing addIndex:(NSUInteger)i];
        }

        [hostKeys addObject:keys];
        [tags addObject:[tag tag] ?: [NSNull null]];
    }

    _hostKeys = hostKeys;
    [_keyboard setTags:tags differing:differing];
}

- (void)showKey:(NSInteger)index
{
    if (index < 0) {
        [_preview showKeycapAtIndex:-1];
        [_name setStringValue:localized(@"Juku E5101 keyboard")];
        [_line setStringValue:localized(@"Click a key to press it. SHIFT and CTRL stay down until the next key.")];
        [_meta setStringValue:localized(@"Keys go down as you type, too.")];
        return;
    }

    const JukuKeycap *keycap = [_keyboard keycapAtIndex:index];
    NSString *top = [NSString stringWithUTF8String:keycap->top];
    NSString *bottom = [NSString stringWithUTF8String:keycap->bottom];
    NSString *name = top;

    if (keycap->legend == JukuLegendPair || keycap->legend == JukuLegendTwoLines) {
        name = [NSString stringWithFormat:@"%@ %@", top, bottom];
    } else if (keycap->legend == JukuLegendBlank) {
        name = localized(@"Space bar");
    }

    NSMutableArray<NSString *> *names = [NSMutableArray array];

    for (HostKey *key in index < (NSInteger)[_hostKeys count] ? _hostKeys[index] : @[]) {
        [names addObject:[key name]];
    }

    NSString *meta;

    if (keycap->id == kJukuKeyCaps) {
        meta = localized(@"It follows your Mac’s Caps Lock, so it cannot be clicked.");
    } else if (keycap->id == JukuKeyShift || keycap->id == JukuKeyCtrl) {
        meta = localized(@"A click holds it down until the next key.");
    } else {
        meta = [NSString stringWithFormat:localized(@"Column %d, row %d of the key matrix"), keycap->id >> 3, keycap->id & 7];
    }

    [_preview showKeycapAtIndex:index];
    [_name setStringValue:name];
    [_line setStringValue:[NSString stringWithFormat:localized(@"On your Mac: %@"), [names count] ? [names componentsJoinedByString:@", "] : @"—"]];
    [_meta setStringValue:meta];
}

- (void)showModes:(uint8_t)consw
{
    [_lat setOn:!(consw & 2)];
    [_rus setOn:(consw & 2) != 0];
    [_caps setOn:(consw & 1) != 0];
}

@end
