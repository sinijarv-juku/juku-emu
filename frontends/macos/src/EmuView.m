#pragma once

#import <AppKit/AppKit.h>
#import <QuartzCore/QuartzCore.h>

#import "Audio.m"
#import "Core.m"
#import "Disks.m"
#import "Keymap.m"
#import "Renderer.m"
#import "Settings.m"
#import "Snapshots.m"
#import "Strings.m"

// Longest stretch of host time emulated in one tick. Longer gaps (window
// dragged, machine asleep) are dropped instead of caught up.
static const double kMaxTickSeconds = 0.1;

// Emulation speed follows the audio clock: it is trimmed by up to ±0.5% to
// keep this many samples queued (spec §6.1).
static const double kAudioTargetFill = 2048.0;
static const double kMaxRateTrim = 0.005;

// While the display link is stopped (window hidden or minimized), a timer
// keeps the machine and its audio running.
static const double kFallbackInterval = 0.02;

// Warp speed: at most this many times real time, using at most this share of
// each tick (and never more than kWarpMaxBusy seconds), in slices of
// kWarpSlice cycles so the time limit is checked often.
static const double kWarpFactor = 20.0;
static const double kWarpBusyShare = 0.75;
static const double kWarpMaxBusy = 0.015;
static const uint64_t kWarpSlice = 20000;

// The drive letter in the title stays lit this long after a disk command, so
// short commands are visible and a run of sectors does not flicker.
static const double kDriveLightHold = 0.3;

// How often the measured warp speed in the subtitle is refreshed.
static const double kWarpReportInterval = 0.5;
// How long a note such as "Snapshot Saved" stays in the subtitle, seconds.
static const double kNoteSeconds = 2.0;

// Drawing (spec §6.1). The picture is drawn only while it can still change on
// the screen: for this long after the Juku picture, the size or a setting
// last changed, which is the time the phosphor afterglow takes to fade out.
static const double kSettleSeconds = 0.3;

// While settling, each new Juku frame is drawn, but never two draws closer
// than this (warp speed makes frames faster than anyone can see). It is
// under two ticks of a 120 Hz display and under one tick of a 60 Hz one.
static const double kMinDrawInterval = 0.012;

// With no new Juku frames (paused, or the raster stopped), the fade is drawn
// at this interval instead.
static const double kIdleDrawInterval = 0.03;

// In full screen the cursor hides when the mouse has been still this long.
static const double kCursorIdleSeconds = 3.0;

@interface EmuView: NSView<CALayerDelegate>

@property(nonatomic, readonly) BOOL mouseCaptured;
- (void)toggleMouseCapture;

// The machine stops; the picture stays on the screen. While a debugger is
// connected this is the debugger's stop (spec §12.3).
@property(nonatomic) BOOL paused;
// The debug server, or NULL when it is off.
@property(nonatomic) JukuDap *dap;
// Opens a snapshot for the debugger as the user would; YES if it did.
@property(nonatomic, copy) BOOL (^openSnapshot)(NSString *path);
// Runs as fast as the host allows, up to kWarpFactor×, with audio muted.
@property(nonatomic) BOOL warp;

- (instancetype)initWithFrame:(NSRect)frameRect
                      machine:(JukuMachine *)machine
                        audio:(const JukuAudio *)audio
                     renderer:(Renderer *)renderer
                        disks:(DiskDrives *)disks;

// A snapshot replaced the machine's state, and its time may have gone back.
- (void)machineRestored;
// Shows `note` in the subtitle for a moment.
- (void)showNote:(NSString *)note;
// The screen as a PNG for a snapshot's thumbnail, or nil.
- (NSData *)thumbnailPNG;

@end

@implementation EmuView {
    JukuMachine *_machine;
    const JukuAudio *_audio;   // NULL when there is no audio output to follow
    Renderer *_renderer;
    DiskDrives *_disks;
    CAMetalLayer *_metalLayer;
    CADisplayLink *_displayLink;
    NSTimer *_fallbackTimer;
    CFTimeInterval _lastTick;
    double _cycleRemainder;
    // Mouse capture: movement not yet sent (fractions of a count), buttons.
    double _mouseX;
    double _mouseY;
    uint8_t _mouseButtons;
    // Full screen: the cursor hides after kCursorIdleSeconds without movement.
    NSTimer *_cursorTimer;
    CFTimeInterval _cursorMovedAt;
    // Title bar: drive lights and the measured warp speed.
    CFTimeInterval _driveLitUntil[2];
    CFTimeInterval _warpReportStart;
    uint64_t _warpReportCycles;
    double _warpSpeed;
    NSString *_note;
    CFTimeInterval _noteUntil;
    // Drawing: see -displayLinkFired:.
    JukuFrame _seen;                // the last Juku frame looked at
    uint64_t _drawnSeq;             // the last Juku frame drawn
    CFTimeInterval _settleUntil;    // drawing goes on until then
    CFTimeInterval _lastDraw;
    CFTimeInterval _phosphorTime;   // when the phosphor textures were last right
    uint32_t _debugState;           // JukuDap* bits from the last poll
}

// Both of its accessors are written out below.
@synthesize paused = _paused;

- (instancetype)initWithFrame:(NSRect)frameRect
                      machine:(JukuMachine *)machine
                        audio:(const JukuAudio *)audio
                     renderer:(Renderer *)renderer
                        disks:(DiskDrives *)disks
{
    self = [super initWithFrame:frameRect];

    if (self) {
        _machine = machine;
        _audio = audio;
        _renderer = renderer;
        _disks = disks;
        [self registerForDraggedTypes:@[NSPasteboardTypeFileURL]];

        [self setWantsLayer:YES];
        _metalLayer = (CAMetalLayer *)[self layer];
        [_metalLayer setDelegate:self];
    }

    return self;
}

- (CALayer *)makeBackingLayer
{
    CAMetalLayer *layer = [CAMetalLayer layer];

    [layer setDevice:[_renderer device]];
    [layer setPixelFormat:MTLPixelFormatBGRA8Unorm_sRGB];
    [layer setFramebufferOnly:YES];

    CGColorSpaceRef colorSpace = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
    [layer setColorspace:colorSpace];
    CGColorSpaceRelease(colorSpace);

    return layer;
}

- (BOOL)acceptsFirstResponder
{
    return YES;
}

// Keys go to the Juku by position. Cmd combinations belong to the menus, and
// host auto-repeat is ignored because the BIOS repeats keys itself.
- (void)keyDown:(NSEvent *)event
{
    if ([event modifierFlags] & NSEventModifierFlagCommand) {
        [super keyDown:event];
        return;
    }

    int key = jukuKeyForKeyCode([event keyCode]);

    if (key >= 0 && ![event isARepeat]) {
        juku_key(_machine, (uint8_t)key, true);
    }
}

- (void)keyUp:(NSEvent *)event
{
    int key = jukuKeyForKeyCode([event keyCode]);

    if (key >= 0) {
        juku_key(_machine, (uint8_t)key, false);
    }
}

- (void)flagsChanged:(NSEvent *)event
{
    NSEventModifierFlags flags = [event modifierFlags];

    switch ([event keyCode]) {
    case kVK_Shift:
    case kVK_RightShift:
        juku_key(_machine, JukuKeyShift, (flags & NSEventModifierFlagShift) != 0);
        break;
    case kVK_Control:
    case kVK_RightControl:
        juku_key(_machine, JukuKeyCtrl, (flags & NSEventModifierFlagControl) != 0);
        break;
    case kVK_RightOption:
        juku_key(_machine, kJukuKeySemicolonPlus, (flags & NX_DEVICERALTKEYMASK) != 0);
        break;
    }
}

// Dropping a disk image inserts it in A:, or in B: with Option held.
// A snapshot is opened by the app delegate (-openSnapshotFromSender:).
- (NSURL *)draggedURL:(id<NSDraggingInfo>)sender
{
    NSArray<NSURL *> *urls = [[sender draggingPasteboard] readObjectsForClasses:@[[NSURL class]]
                                                                        options:@{NSPasteboardURLReadingFileURLsOnlyKey: @YES}];
    return [urls firstObject];
}

- (NSDragOperation)draggingEntered:(id<NSDraggingInfo>)sender
{
    return [self draggedURL:sender] ? NSDragOperationCopy : NSDragOperationNone;
}

- (BOOL)performDragOperation:(id<NSDraggingInfo>)sender
{
    NSURL *url = [self draggedURL:sender];

    if (!url) {
        return NO;
    }

    if (isSnapshotURL(url)) {
        return [NSApp sendAction:@selector(openSnapshotFromSender:) to:nil from:url];
    }

    int drive = ([NSEvent modifierFlags] & NSEventModifierFlagOption) ? 1 : 0;
    [_disks insertURL:url inDrive:drive];
    return YES;
}

- (void)windowDidResignKey:(NSNotification *)notification
{
    juku_keys_release_all(_machine);
    [self releaseMouse];
}

// ---- Mouse (E4701) ----------------------------------------------------------
// While captured the host cursor is hidden and detached, and raw movement and
// buttons go to the Juku. A click captures the mouse only when the running
// program uses it; Cmd-M toggles capture at any time.

- (void)toggleMouseCapture
{
    if (_mouseCaptured) {
        [self releaseMouse];
    } else {
        [self captureMouse];
    }
}

- (void)captureMouse
{
    if (_mouseCaptured || ![self window]) {
        return;
    }

    // Park the hidden cursor in the middle of the view, where it reappears.
    NSRect inWindow = [self convertRect:[self bounds] toView:nil];
    NSRect onScreen = [[self window] convertRectToScreen:inWindow];
    CGFloat screenHeight = NSMaxY([[[NSScreen screens] firstObject] frame]);
    CGWarpMouseCursorPosition(CGPointMake(NSMidX(onScreen), screenHeight - NSMidY(onScreen)));

    CGAssociateMouseAndMouseCursorPosition(false);
    [NSCursor hide];
    [[self window] setAcceptsMouseMovedEvents:YES];
    _mouseCaptured = YES;
    [self updateTitle];
    _mouseX = 0;
    _mouseY = 0;
}

- (void)releaseMouse
{
    if (!_mouseCaptured) {
        return;
    }

    CGAssociateMouseAndMouseCursorPosition(true);
    [NSCursor unhide];
    _mouseCaptured = NO;
    [self cursorMoved];
    [self updateTitle];
    _mouseButtons = 0;
    juku_mouse(_machine, 0, 0, 0);
}

- (void)mouseMovedBy:(NSEvent *)event
{
    if (!_mouseCaptured) {
        return;
    }

    double scale = settingDouble(kSettingMouseSensitivity);
    _mouseX += [event deltaX] * scale;
    _mouseY += [event deltaY] * scale;     // AppKit deltaY is positive downwards, as on the Juku
    int32_t dx = (int32_t)_mouseX;
    int32_t dy = (int32_t)_mouseY;
    _mouseX -= dx;
    _mouseY -= dy;

    if (dx != 0 || dy != 0) {
        juku_mouse(_machine, dx, dy, _mouseButtons);
    }
}

- (void)setMouseButton:(uint8_t)button down:(BOOL)down
{
    _mouseButtons = down ? (_mouseButtons | button) : (_mouseButtons & ~button);
    juku_mouse(_machine, 0, 0, _mouseButtons);
}

// ---- Cursor in full screen --------------------------------------------------
// The cursor hides on entering full screen, shows again when the mouse moves
// (AppKit's setHiddenUntilMouseMoves) and hides once it has been still for
// kCursorIdleSeconds. A captured mouse hides it anyway (above).

- (BOOL)fullScreen
{
    return ([[self window] styleMask] & NSWindowStyleMaskFullScreen) != 0;
}

- (void)updateTrackingAreas
{
    [super updateTrackingAreas];

    if ([[self trackingAreas] count] == 0) {
        NSTrackingAreaOptions options = NSTrackingMouseMoved | NSTrackingActiveInKeyWindow | NSTrackingInVisibleRect;
        [self addTrackingArea:[[NSTrackingArea alloc] initWithRect:NSZeroRect options:options owner:self userInfo:nil]];
    }
}

- (void)windowDidEnterFullScreen:(NSNotification *)notification
{
    [self hideIdleCursor];
}

- (void)windowWillExitFullScreen:(NSNotification *)notification
{
    [_cursorTimer invalidate];
    _cursorTimer = nil;
    [NSCursor setHiddenUntilMouseMoves:NO];
}

// The mouse moved: the cursor shows (AppKit does that) and the countdown
// starts again. One timer at a time: when it fires early, it waits on.
- (void)cursorMoved
{
    if (![self fullScreen]) {
        return;
    }

    _cursorMovedAt = CACurrentMediaTime();

    if (!_cursorTimer) {
        [self startCursorTimer:kCursorIdleSeconds];
    }
}

// The default run loop mode only: the timer waits while a menu is open.
- (void)startCursorTimer:(double)seconds
{
    _cursorTimer = [NSTimer timerWithTimeInterval:seconds
                                           target:self
                                         selector:@selector(cursorTimerFired:)
                                         userInfo:nil
                                          repeats:NO];
    [[NSRunLoop mainRunLoop] addTimer:_cursorTimer forMode:NSDefaultRunLoopMode];
}

- (void)cursorTimerFired:(NSTimer *)timer
{
    _cursorTimer = nil;
    double left = _cursorMovedAt + kCursorIdleSeconds - CACurrentMediaTime();

    if (left > 0.01) {
        [self startCursorTimer:left];
    } else {
        [self hideIdleCursor];
    }
}

// Hides the cursor until the mouse moves, unless it is on the menu bar, on
// another screen, or holding a button.
- (void)hideIdleCursor
{
    NSWindow *window = [self window];

    if (![self fullScreen] || _mouseCaptured || ![window isKeyWindow] || [NSMenu menuBarVisible] || [NSEvent pressedMouseButtons] != 0
        || !NSPointInRect([NSEvent mouseLocation], [window frame])) {
        return;
    }

    [NSCursor setHiddenUntilMouseMoves:YES];
}

- (void)mouseMoved:(NSEvent *)event
{
    [self cursorMoved];
    [self mouseMovedBy:event];
}

- (void)mouseDragged:(NSEvent *)event
{
    [self cursorMoved];
    [self mouseMovedBy:event];
}

- (void)rightMouseDragged:(NSEvent *)event
{
    [self cursorMoved];
    [self mouseMovedBy:event];
}

- (void)mouseDown:(NSEvent *)event
{
    if (_mouseCaptured) {
        [self setMouseButton:JukuMouseLeft down:YES];
    } else if (juku_mouse_enabled(_machine)) {
        [self captureMouse];
    }
}

- (void)mouseUp:(NSEvent *)event
{
    if (_mouseCaptured) {
        [self setMouseButton:JukuMouseLeft down:NO];
    }
}

- (void)rightMouseDown:(NSEvent *)event
{
    if (_mouseCaptured) {
        [self setMouseButton:JukuMouseRight down:YES];
    } else {
        [super rightMouseDown:event];
    }
}

- (void)rightMouseUp:(NSEvent *)event
{
    if (_mouseCaptured) {
        [self setMouseButton:JukuMouseRight down:NO];
    }
}

- (void)viewDidMoveToWindow
{
    [super viewDidMoveToWindow];

    if ([self window]) {
        _displayLink = [self displayLinkWithTarget:self selector:@selector(displayLinkFired:)];
        [_displayLink addToRunLoop:[NSRunLoop mainRunLoop] forMode:NSRunLoopCommonModes];

        _fallbackTimer = [NSTimer timerWithTimeInterval:kFallbackInterval
                                                 target:self
                                               selector:@selector(fallbackTimerFired:)
                                               userInfo:nil
                                                repeats:YES];
        [[NSRunLoop mainRunLoop] addTimer:_fallbackTimer forMode:NSRunLoopCommonModes];

        NSNotificationCenter *center = [NSNotificationCenter defaultCenter];
        [center addObserver:self selector:@selector(windowWillClose:) name:NSWindowWillCloseNotification object:[self window]];
        [center addObserver:self selector:@selector(windowDidResignKey:) name:NSWindowDidResignKeyNotification object:[self window]];
        [center addObserver:self selector:@selector(setNeedsRedraw) name:NSUserDefaultsDidChangeNotification object:nil];
        [center addObserver:self selector:@selector(setNeedsRedraw) name:NSWindowDidChangeOcclusionStateNotification object:[self window]];
        [center addObserver:self selector:@selector(windowDidEnterFullScreen:) name:NSWindowDidEnterFullScreenNotification object:[self window]];
        [center addObserver:self selector:@selector(windowWillExitFullScreen:) name:NSWindowWillExitFullScreenNotification object:[self window]];
        [self resizeLayer];
    }
}

- (void)windowWillClose:(NSNotification *)notification
{
    [self releaseMouse];
    [_displayLink invalidate];
    _displayLink = nil;
    [_fallbackTimer invalidate];
    _fallbackTimer = nil;
    [_cursorTimer invalidate];
    _cursorTimer = nil;
}

- (void)setFrameSize:(NSSize)size
{
    [super setFrameSize:size];
    [self resizeLayer];
}

- (void)viewDidChangeBackingProperties
{
    [super viewDidChangeBackingProperties];
    [self resizeLayer];
}

- (void)resizeLayer
{
    NSSize size = [self convertSizeToBacking:[self bounds].size];

    if (size.width <= 0 || size.height <= 0) {
        return;
    }

    CGSize current = [_metalLayer drawableSize];

    if (current.width != size.width || current.height != size.height) {
        [_metalLayer setDrawableSize:size];
        [self setNeedsRedraw];
        _lastDraw = 0;      // on the next tick, so a resize follows the mouse
    }
}

- (BOOL)paused
{
    return _paused || (_debugState & JukuDapStopped);
}

// With a client connected, Pause stops the machine in the debugger, and the
// client is told.
- (void)setPaused:(BOOL)paused
{
    if (_debugState & JukuDapConnected) {
        _debugState = juku_dap_set_paused(_dap, _machine, paused);
    } else {
        _paused = paused;
    }

    [self setNeedsRedraw];
    [self updateTitle];
}

- (void)setDap:(JukuDap *)dap
{
    _dap = dap;
    _debugState = 0;
    [self updateTitle];
}

// Runs the machine. With the debug server on, the server runs it, so that a
// breakpoint it goes on from (a log point, a condition that does not hold)
// costs no time, and a stop is reported at once.
- (uint64_t)run:(uint64_t)cycles
{
    if (!_dap) {
        return juku_run_cycles(_machine, cycles);
    }

    uint64_t ran = 0;
    uint32_t state = juku_dap_run(_dap, _machine, cycles, &ran);
    [self serveDebugger];

    if (state != _debugState) {
        _debugState = state;
        [self setNeedsRedraw];
        [self updateTitle];
    }

    return ran;
}

// Opens the snapshot a launch asks for (spec §12.7), after this tick: the
// app asks about changed disks first, as for any snapshot it opens.
- (void)serveDebugger
{
    uint8_t path[4096];
    size_t length = juku_dap_snapshot_request(_dap, path, sizeof path);

    if (length == 0) {
        return;
    }

    NSString *file = [[NSString alloc] initWithBytes:path length:length encoding:NSUTF8StringEncoding];
    dispatch_async(dispatch_get_main_queue(), ^{
        BOOL opened = file && self.openSnapshot && self.openSnapshot(file);

        if (self->_dap) {
            self->_debugState = juku_dap_snapshot_opened(self->_dap, self->_machine, opened);
            [self updateTitle];
        }
    });
}

// Lets the debug server take a client and handle its messages, and takes in
// the state. A client that connects to a paused machine finds it stopped in
// the debugger. Called on every tick, so a request waits at most one.
- (void)pollDebugger
{
    if (!_dap) {
        return;
    }

    uint32_t state = juku_dap_poll(_dap, _machine);
    [self serveDebugger];

    if ((state & JukuDapConnected) && !(_debugState & JukuDapConnected) && _paused) {
        _paused = NO;
        state = juku_dap_set_paused(_dap, _machine, YES);
    }

    if (state != _debugState) {
        _debugState = state;
        [self setNeedsRedraw];
        [self updateTitle];
    }
}

- (void)setWarp:(BOOL)warp
{
    _warp = warp;
    // Muted, the audio ring drains; afterwards it re-primes at the normal
    // latency instead of playing a backlog.
    juku_audio_mute(_machine, warp);
    _warpReportStart = CACurrentMediaTime();
    _warpReportCycles = juku_cycles(_machine);
    _warpSpeed = 0.0;
    [self updateTitle];
}

- (void)machineRestored
{
    juku_keys_release_all(_machine);
    _cycleRemainder = 0.0;
    _warpReportStart = CACurrentMediaTime();
    _warpReportCycles = juku_cycles(_machine);
    [self setNeedsRedraw];
}

// 640 × 480: big enough for Finder's largest icons and Quick Look, and
// about 50 KB.
- (NSData *)thumbnailPNG
{
    return [_renderer pngForFrame:juku_frame(_machine) width:640 height:480];
}

- (void)showNote:(NSString *)note
{
    _note = note;
    _noteUntil = CACurrentMediaTime() + kNoteSeconds;
    [self updateTitle];
}

// Title: "Juku", plus a lit letter per busy drive ("Juku — A●"). Subtitle:
// how to release the mouse, a note, or the debugger and the paused or warp
// state ("Debugging · Paused").
- (void)updateTitle
{
    NSWindow *window = [self window];
    CFTimeInterval now = CACurrentMediaTime();
    uint8_t activity = juku_drive_activity(_machine);
    NSMutableArray<NSString *> *lights = [NSMutableArray array];

    for (int drive = 0; drive < 2; drive++) {
        if (activity & (1 << drive)) {
            _driveLitUntil[drive] = now + kDriveLightHold;
        }

        if (now < _driveLitUntil[drive]) {
            [lights addObject:drive == 0 ? @"A●" : @"B●"];
        }
    }

    NSString *title = localized(@"Window");

    if ([lights count] > 0) {
        title = [NSString stringWithFormat:@"%@ — %@", title, [lights componentsJoinedByString:@" "]];
    }

    NSString *subtitle = @"";

    if (_mouseCaptured) {
        subtitle = localized(@"Press ⌘M to release the mouse");
    } else if (_note && now < _noteUntil) {
        subtitle = _note;
    } else {
        NSMutableArray<NSString *> *states = [NSMutableArray array];

        if (_debugState & JukuDapConnected) {
            [states addObject:localized(@"Debugging")];
        }

        if ([self paused]) {
            [states addObject:localized(@"Paused")];
        } else if (_warp && _warpSpeed > 0.0) {
            [states addObject:[NSString stringWithFormat:localized(@"Warp Speed (%.0f×)"), _warpSpeed]];
        } else if (_warp) {
            [states addObject:localized(@"Warp Speed")];
        }

        subtitle = [states componentsJoinedByString:@" · "];
    }

    if (![[window title] isEqualToString:title]) {
        [window setTitle:title];
    }

    if (![[window subtitle] isEqualToString:subtitle]) {
        [window setSubtitle:subtitle];
    }
}

// Warp: runs slices until kWarpFactor × the elapsed time is emulated or the
// tick's share of host time is used up.
- (void)runWarp:(double)elapsed
{
    CFTimeInterval start = CACurrentMediaTime();
    CFTimeInterval deadline = start + fmin(elapsed * kWarpBusyShare, kWarpMaxBusy);
    uint64_t target = (uint64_t)(elapsed * JukuCpuHz * kWarpFactor);
    uint64_t done = 0;

    while (done < target) {
        done += [self run:MIN(kWarpSlice, target - done)];

        if (CACurrentMediaTime() >= deadline) {
            break;
        }
    }

    CFTimeInterval span = start - _warpReportStart;

    if (span >= kWarpReportInterval) {
        uint64_t cycles = juku_cycles(_machine);
        _warpSpeed = (double)(cycles - _warpReportCycles) / (span * JukuCpuHz);
        _warpReportStart = start;
        _warpReportCycles = cycles;
    }
}

// Runs the machine for the host time elapsed since the last tick.
- (void)runUntil:(CFTimeInterval)now
{
    double elapsed = (_lastTick > 0) ? now - _lastTick : 0.0;
    _lastTick = now;
    elapsed = fmin(elapsed, kMaxTickSeconds);
    [self pollDebugger];

    if ([self paused]) {
        _cycleRemainder = 0.0;
        [self updateTitle];
        return;
    }

    // The guest Caps Lock follows the host's, focused or not (spec §6.4).
    juku_caps_lock(_machine, ([NSEvent modifierFlags] & NSEventModifierFlagCapsLock) != 0);

    if (_warp) {
        [self runWarp:elapsed];
        [self updateTitle];
        return;
    }

    double trim = 1.0;

    if (_audio) {
        double error = (kAudioTargetFill - (double)juku_audio_fill(_audio)) / kAudioTargetFill;
        trim += kMaxRateTrim * fmax(-1.0, fmin(1.0, error));
    }

    double cycles = elapsed * JukuCpuHz * trim + _cycleRemainder;
    uint64_t whole = (uint64_t)cycles;
    _cycleRemainder = cycles - (double)whole;

    [self run:whole];
    [self updateTitle];
}

// The picture has to be drawn again: the size, a setting or the pause state
// changed, or the window came back into view.
- (void)setNeedsRedraw
{
    _settleUntil = CACurrentMediaTime() + kSettleSeconds;
}

// True if `frame` shows something else than the last frame looked at.
- (BOOL)pictureChanged:(const JukuFrame *)frame
{
    size_t header = offsetof(JukuFrame, data) - offsetof(JukuFrame, active_x);
    BOOL changed = memcmp(&frame->active_x, &_seen.active_x, header) != 0
                || memcmp(frame->data, _seen.data, sizeof frame->data) != 0;

    if (changed) {
        memcpy(&_seen.active_x, &frame->active_x, header);
        memcpy(_seen.data, frame->data, sizeof frame->data);
    }

    return changed;
}

// Runs the machine and draws the picture if there is anything new to show.
//
// The display link ticks at the display's rate, up to 120 Hz, and the Juku
// makes 50 frames a second, most of them the same as the one before. Drawing
// and presenting every tick is what costs: about 60% of one core between this
// process and WindowServer, and at 120 Hz in full screen a GPU at full clock.
// So a tick draws only when
//   - the window is in view,
//   - the picture is still settling (kSettleSeconds after the last change), and
//   - there is a Juku frame not drawn yet, or none has come for a while.
- (void)displayLinkFired:(CADisplayLink *)displayLink
{
    [self runUntil:[displayLink timestamp]];

    if (!([[self window] occlusionState] & NSWindowOcclusionStateVisible)) {
        return;     // what is on the screen when it comes back is drawn then
    }

    CFTimeInterval now = CACurrentMediaTime();
    const JukuFrame *frame = juku_frame(_machine);

    if (frame->seq != _seen.seq) {
        _seen.seq = frame->seq;

        if ([self pictureChanged:frame]) {
            _settleUntil = now + kSettleSeconds;
        }
    }

    if (now >= _settleUntil) {
        // Settled: the textures hold the picture as the tube shows it now.
        _phosphorTime = now;
        return;
    }

    BOOL fresh = frame->seq != _drawnSeq;

    if (now - _lastDraw < (fresh ? kMinDrawInterval : kIdleDrawInterval)) {
        return;
    }

    @autoreleasepool {
        [_renderer setFullScreen:[self fullScreen]];
        [_renderer drawFrame:frame toDrawable:[_metalLayer nextDrawable] elapsed:now - _phosphorTime];
    }

    _drawnSeq = frame->seq;
    _lastDraw = now;
    _phosphorTime = now;
}

- (void)fallbackTimerFired:(NSTimer *)timer
{
    CFTimeInterval now = CACurrentMediaTime();

    if (now - _lastTick > 2.0 * kFallbackInterval) {
        [self runUntil:now];
    }
}

@end
