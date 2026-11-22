#pragma once

#import <AppKit/AppKit.h>

#import "Strings.m"

// Input → Keyboard Mapping: the host keys for Juku keys that are not where
// their legends suggest. Letters, digits and the rest map by position and are
// not listed. Keep in sync with Keymap.m.
@interface KeyboardHelp: NSObject

- (void)show;

@end

// Host key (localized where it has words), Juku key legend.
static NSString *const kKeyboardHelpRows[][2] = {
    { @"[", @"Ü" },
    { @"]", @"Õ" },
    { @";", @"Ö" },
    { @"'", @"Ä" },
    { @"\\", @"* :" },
    { @"-", @"− =" },
    { @"=", @"Ю @" },
    { @"` / §", @"LAT/RUS" },
    { @"Right Option, keypad +", @"; +" },
    { @"Delete ⌫", @"←" },
    { @"Forward Delete ⌦", @"DEL" },
    { @"Home", @"ERASE" },
    { @"End", @"{ [" },
    { @"Page Down", @"} ]" },
    { @"Page Up", @"\\ ^" },
    { @"Caps Lock", @"CAPS" },
    { @"F1–F8", @"F1–F8" },
};

@implementation KeyboardHelp {
    NSWindow *_window;
}

static NSTextField *keyLabel(NSString *text, BOOL juku)
{
    NSTextField *label = [NSTextField labelWithString:text];
    CGFloat size = [NSFont systemFontSize];
    [label setFont:juku ? [NSFont monospacedSystemFontOfSize:size weight:NSFontWeightSemibold] : [NSFont systemFontOfSize:size]];
    return label;
}

- (void)build
{
    NSMutableArray<NSArray<NSView *> *> *rows = [NSMutableArray array];
    NSTextField *hostHeader = [NSTextField labelWithString:localized(@"Mac")];
    NSTextField *jukuHeader = [NSTextField labelWithString:localized(@"Juku")];
    [hostHeader setFont:[NSFont boldSystemFontOfSize:[NSFont systemFontSize]]];
    [jukuHeader setFont:[NSFont boldSystemFontOfSize:[NSFont systemFontSize]]];
    [rows addObject:@[hostHeader, jukuHeader]];

    for (size_t i = 0; i < sizeof(kKeyboardHelpRows) / sizeof(kKeyboardHelpRows[0]); i++) {
        [rows addObject:@[keyLabel(localized(kKeyboardHelpRows[i][0]), NO), keyLabel(kKeyboardHelpRows[i][1], YES)]];
    }

    NSGridView *grid = [NSGridView gridViewWithViews:rows];
    [grid setRowSpacing:4.0];
    [grid setColumnSpacing:24.0];
    [grid setTranslatesAutoresizingMaskIntoConstraints:NO];
    [[grid rowAtIndex:0] setBottomPadding:4.0];

    NSTextField *note = [NSTextField wrappingLabelWithString:localized(@"Other keys map by position: letters, digits, punctuation, the arrows, Esc, Tab, Return, Shift and Control are where they are on the Juku keyboard. ⌘ shortcuts always go to the menus.")];
    [note setTextColor:[NSColor secondaryLabelColor]];
    [note setTranslatesAutoresizingMaskIntoConstraints:NO];
    [[note widthAnchor] constraintEqualToConstant:300.0].active = YES;

    _window = [[NSWindow alloc] initWithContentRect:NSZeroRect
                                          styleMask:NSWindowStyleMaskTitled | NSWindowStyleMaskClosable
                                            backing:NSBackingStoreBuffered
                                              defer:YES];
    [_window setTitle:localized(@"Keyboard Mapping")];
    [_window setReleasedWhenClosed:NO];

    NSView *content = [_window contentView];
    [content addSubview:grid];
    [content addSubview:note];
    [NSLayoutConstraint activateConstraints:@[
        [[grid leadingAnchor] constraintEqualToAnchor:[content leadingAnchor] constant:20.0],
        [[grid trailingAnchor] constraintLessThanOrEqualToAnchor:[content trailingAnchor] constant:-20.0],
        [[grid topAnchor] constraintEqualToAnchor:[content topAnchor] constant:20.0],
        [[note topAnchor] constraintEqualToAnchor:[grid bottomAnchor] constant:16.0],
        [[note leadingAnchor] constraintEqualToAnchor:[content leadingAnchor] constant:20.0],
        [[note trailingAnchor] constraintEqualToAnchor:[content trailingAnchor] constant:-20.0],
        [[note bottomAnchor] constraintEqualToAnchor:[content bottomAnchor] constant:-20.0],
    ]];
}

- (void)show
{
    if (!_window) {
        [self build];
        [_window center];
    }

    [_window makeKeyAndOrderFront:nil];
}

@end
