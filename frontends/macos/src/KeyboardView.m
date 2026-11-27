#pragma once

#import <AppKit/AppKit.h>
#import <QuartzCore/QuartzCore.h>

#import "Core.m"
#import "Keymap.m"

// The Juku keyboard, drawn (spec §6.4). The core makes the picture
// (core/src/keyboard/drawing.rs): every key, legend and color is decided
// there, as drawing commands this file replays with Core Graphics, so that
// all front-ends draw the same keyboard. Only the Mac key labels' text is
// drawn here, with the system font. A click presses a key in the machine,
// and the keys the machine has down are drawn down, whoever pressed them.

#define kMaxKeycaps 80

// How Mac key tags are shown on the keys.
enum { kKeyTagsWhereDifferent, kKeyTagsAll, kKeyTagsOff };

static CGColorRef createColor(uint32_t rgba)
{
    return CGColorCreateSRGB((rgba >> 24) / 255.0, (rgba >> 16 & 0xFF) / 255.0, (rgba >> 8 & 0xFF) / 255.0, (rgba & 0xFF) / 255.0);
}

static NSColor *colorFrom(uint32_t rgba)
{
    return [NSColor colorWithSRGBRed:(rgba >> 24) / 255.0 green:(rgba >> 16 & 0xFF) / 255.0 blue:(rgba >> 8 & 0xFF) / 255.0 alpha:(rgba & 0xFF) / 255.0];
}

// Reads a picture's numbers, little-endian as the Mac is. Each returns
// false at the end of the picture.
typedef struct {
    const uint8_t *at;
    const uint8_t *end;
} PictureReader;

static bool readBytes(PictureReader *r, void *out, size_t n)
{
    if ((size_t)(r->end - r->at) < n) {
        return false;
    }

    memcpy(out, r->at, n);
    r->at += n;
    return true;
}

static bool readFloats(PictureReader *r, CGFloat *out, int n)
{
    for (int i = 0; i < n; i++) {
        float f;

        if (!readBytes(r, &f, 4)) {
            return false;
        }

        out[i] = f;
    }

    return true;
}

static CGGradientRef readGradient(PictureReader *r)
{
    uint8_t n;

    if (!readBytes(r, &n, 1) || n == 0) {
        return NULL;
    }

    CGFloat locations[n], components[n * 4];

    for (int i = 0; i < n; i++) {
        float offset;
        uint32_t rgba;

        if (!readBytes(r, &offset, 4) || !readBytes(r, &rgba, 4)) {
            return NULL;
        }

        locations[i] = offset;
        components[i * 4] = (rgba >> 24) / 255.0;
        components[i * 4 + 1] = (rgba >> 16 & 0xFF) / 255.0;
        components[i * 4 + 2] = (rgba >> 8 & 0xFF) / 255.0;
        components[i * 4 + 3] = (rgba & 0xFF) / 255.0;
    }

    CGColorSpaceRef space = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
    CGGradientRef gradient = CGGradientCreateWithColorComponents(space, components, locations, n);
    CGColorSpaceRelease(space);
    return gradient;
}

// A Mac key label (the LABEL command): `text` in a rounded rectangle.
static void drawLabel(NSString *text, const CGFloat *f, uint32_t fill, uint32_t ink)
{
    CGFloat centerX = f[0], top = f[1], height = f[2], maxWidth = f[3], size = f[4], radius = f[5], padding = f[6];
    NSFont *font = [NSFont systemFontOfSize:size weight:NSFontWeightSemibold];
    NSDictionary *attributes = @{NSFontAttributeName: font, NSForegroundColorAttributeName: colorFrom(ink)};
    CGFloat width = [text sizeWithAttributes:attributes].width;

    if (width + padding > maxWidth && width > 0) {
        font = [NSFont systemFontOfSize:size * (maxWidth - padding) / width weight:NSFontWeightSemibold];
        attributes = @{NSFontAttributeName: font, NSForegroundColorAttributeName: colorFrom(ink)};
        width = [text sizeWithAttributes:attributes].width;
    }

    CGFloat chipWidth = MAX(height, ceil(width) + padding);
    NSRect chip = NSMakeRect(centerX - chipWidth / 2, top, chipWidth, height);
    [colorFrom(fill) set];
    [[NSBezierPath bezierPathWithRoundedRect:chip xRadius:radius yRadius:radius] fill];
    CGFloat baseline = NSMidY(chip) + [font capHeight] / 2;
    [text drawAtPoint:NSMakePoint(NSMidX(chip) - width / 2, baseline - [font ascender]) withAttributes:attributes];
}

// Replays a picture in the current context, already scaled to drawing
// units, y down. `label` gives a keycap's Mac key label.
static void drawPicture(NSData *picture, NSString *(^label)(NSInteger index))
{
    CGContextRef context = [[NSGraphicsContext currentContext] CGContext];
    PictureReader r = { [picture bytes], (const uint8_t *)[picture bytes] + [picture length] };
    CGMutablePathRef path = CGPathCreateMutable();
    uint8_t op;

    while (readBytes(&r, &op, 1)) {
        CGFloat f[7];
        uint32_t rgba, ink;
        uint16_t index;
        CGGradientRef gradient = NULL;
        CGColorRef color = NULL;
        bool ok = true;

        switch (op) {
        case JukuDrawBackground:
            if ((ok = readBytes(&r, &rgba, 4))) {
                color = createColor(rgba);
                CGContextSetFillColorWithColor(context, color);
                CGContextFillRect(context, CGContextGetClipBoundingBox(context));
                CGColorRelease(color);
            }
            break;
        case JukuDrawMove:
            if ((ok = readFloats(&r, f, 2))) CGPathMoveToPoint(path, NULL, f[0], f[1]);
            break;
        case JukuDrawLine:
            if ((ok = readFloats(&r, f, 2))) CGPathAddLineToPoint(path, NULL, f[0], f[1]);
            break;
        case JukuDrawQuad:
            if ((ok = readFloats(&r, f, 4))) CGPathAddQuadCurveToPoint(path, NULL, f[0], f[1], f[2], f[3]);
            break;
        case JukuDrawCubic:
            if ((ok = readFloats(&r, f, 6))) CGPathAddCurveToPoint(path, NULL, f[0], f[1], f[2], f[3], f[4], f[5]);
            break;
        case JukuDrawClose:
            CGPathCloseSubpath(path);
            break;
        case JukuDrawFill:
            if ((ok = readBytes(&r, &rgba, 4))) {
                color = createColor(rgba);
                CGContextSetFillColorWithColor(context, color);
                CGContextAddPath(context, path);
                CGContextFillPath(context);     // nonzero winding
                CGColorRelease(color);
            }
            break;
        case JukuDrawLinear:
        case JukuDrawRadial:
            if (!(ok = readFloats(&r, f, op == JukuDrawLinear ? 4 : 3) && (gradient = readGradient(&r)))) {
                break;
            }

            CGContextSaveGState(context);
            CGContextAddPath(context, path);
            CGContextClip(context);

            if (op == JukuDrawLinear) {
                CGContextDrawLinearGradient(context, gradient, CGPointMake(f[0], f[1]), CGPointMake(f[2], f[3]),
                                            kCGGradientDrawsBeforeStartLocation | kCGGradientDrawsAfterEndLocation);
            } else {
                CGContextDrawRadialGradient(context, gradient, CGPointMake(f[0], f[1]), 0, CGPointMake(f[0], f[1]), f[2],
                                            kCGGradientDrawsAfterEndLocation);
            }

            CGContextRestoreGState(context);
            CGGradientRelease(gradient);
            break;
        case JukuDrawStroke:
            if ((ok = readFloats(&r, f, 1) && readBytes(&r, &rgba, 4))) {
                color = createColor(rgba);
                CGContextSetStrokeColorWithColor(context, color);
                CGContextSetLineWidth(context, f[0]);
                CGContextSetLineCap(context, kCGLineCapButt);
                CGContextSetLineJoin(context, kCGLineJoinMiter);
                CGContextAddPath(context, path);
                CGContextStrokePath(context);
                CGColorRelease(color);
            }
            break;
        case JukuDrawLabel:
            if ((ok = readBytes(&r, &index, 2) && readFloats(&r, f, 7) && readBytes(&r, &rgba, 4) && readBytes(&r, &ink, 4))) {
                NSString *text = label ? label(index) : nil;

                if ([text length]) {
                    drawLabel(text, f, rgba, ink);
                }
            }
            break;
        default:
            ok = false;
            break;
        }

        if (!ok) {
            break;
        }

        // A paint command uses up the path.
        if (op >= JukuDrawFill) {
            CGPathRelease(path);
            path = CGPathCreateMutable();
        }
    }

    CGPathRelease(path);
}

// The scale and offset that fit a picture of `size` into `bounds`, centered.
static NSAffineTransform *fitTransform(NSRect bounds, NSSize size)
{
    CGFloat scale = MIN(NSWidth(bounds) / size.width, NSHeight(bounds) / size.height);
    NSAffineTransform *transform = [NSAffineTransform transform];
    [transform translateXBy:NSMinX(bounds) + (NSWidth(bounds) - size.width * scale) / 2
                        yBy:NSMinY(bounds) + (NSHeight(bounds) - size.height * scale) / 2];
    [transform scaleBy:scale];
    return transform;
}

static NSSize keyboardSize(void)
{
    float width, height;
    juku_keyboard_size(&width, &height);
    return NSMakeSize(width, height);
}

@interface KeyboardView: NSView

- (instancetype)initWithMachine:(JukuMachine *)machine;

@property(nonatomic, readonly) NSInteger keycapCount;
- (const JukuKeycap *)keycapAtIndex:(NSInteger)index;
// The Mac key tag of each keycap (NSNull for none), and which of them name
// a key other than the one the Juku legend suggests.
- (void)setTags:(NSArray *)tags differing:(NSIndexSet *)differing;
- (NSString *)tagAtIndex:(NSInteger)index;
@property(nonatomic) NSInteger tagMode;
// The key the pointer is over, or the one last pressed: its index, or −1.
@property(nonatomic, copy) void (^showKey)(NSInteger index);
@property(nonatomic, copy) void (^modeChanged)(uint8_t consw);
// Lets go of what a click pressed and stops watching the machine while the
// window is hidden.
- (void)releaseClicks;

@end

@implementation KeyboardView {
    JukuMachine *_machine;
    JukuKeycap _keycaps[kMaxKeycaps];
    NSInteger _count;
    bool _down[kMaxKeycaps];
    uint8_t _consw;
    NSArray *_tags;
    NSIndexSet *_differing;
    NSInteger _hover;
    NSInteger _held;                // pressed by the mouse, until it is let go
    NSInteger _shown;               // the key the info strip shows
    NSInteger _lingering;           // the key it keeps after the pointer left it
    NSMutableSet<NSNumber *> *_latched;   // SHIFT and CTRL, until the next key
    NSMutableData *_picture;        // reused between draws
    CADisplayLink *_displayLink;
}

- (instancetype)initWithMachine:(JukuMachine *)machine
{
    NSSize size = keyboardSize();
    self = [super initWithFrame:NSMakeRect(0, 0, size.width, size.height)];

    if (self) {
        _machine = machine;
        _count = MIN((NSInteger)juku_keycap_count(), kMaxKeycaps);

        for (NSInteger i = 0; i < _count; i++) {
            juku_keycap((size_t)i, &_keycaps[i]);
        }

        _hover = -1;
        _shown = -1;
        _lingering = -1;
        _held = -1;
        _consw = 0xFF;
        _latched = [NSMutableSet set];
        _picture = [NSMutableData dataWithLength:128 * 1024];
        [self setClipsToBounds:YES];
        [self addTrackingArea:[[NSTrackingArea alloc] initWithRect:NSZeroRect
                                                           options:NSTrackingMouseMoved | NSTrackingMouseEnteredAndExited | NSTrackingActiveAlways | NSTrackingInVisibleRect
                                                             owner:self
                                                          userInfo:nil]];
    }

    return self;
}

- (BOOL)isFlipped
{
    return YES;
}

- (BOOL)acceptsFirstMouse:(NSEvent *)event
{
    return YES;
}

- (NSInteger)keycapCount
{
    return _count;
}

- (const JukuKeycap *)keycapAtIndex:(NSInteger)index
{
    return &_keycaps[index];
}

- (void)setTags:(NSArray *)tags differing:(NSIndexSet *)differing
{
    _tags = tags;
    _differing = differing;
    [self setNeedsDisplay:YES];
}

- (NSString *)tagAtIndex:(NSInteger)index
{
    id tag = index < (NSInteger)[_tags count] ? _tags[index] : nil;
    return [tag isKindOfClass:[NSString class]] ? tag : nil;
}

- (void)setTagMode:(NSInteger)tagMode
{
    _tagMode = tagMode;
    [self setNeedsDisplay:YES];
}

- (BOOL)tagShownAtIndex:(NSInteger)index
{
    switch (_tagMode) {
    case kKeyTagsAll: return [self tagAtIndex:index] != nil;
    case kKeyTagsWhereDifferent: return [_differing containsIndex:(NSUInteger)index] && [self tagAtIndex:index];
    default: return NO;
    }
}

- (void)drawRect:(NSRect)dirtyRect
{
    bool labeled[kMaxKeycaps];

    for (NSInteger i = 0; i < _count; i++) {
        labeled[i] = [self tagShownAtIndex:i];
    }

    size_t length = juku_keyboard_picture(_down, labeled, (int32_t)_hover, [_picture mutableBytes], [_picture length]);

    if (length > [_picture length]) {
        [_picture setLength:length];
        juku_keyboard_picture(_down, labeled, (int32_t)_hover, [_picture mutableBytes], length);
    }

    [NSGraphicsContext saveGraphicsState];
    [fitTransform([self bounds], keyboardSize()) concat];
    __weak KeyboardView *weakSelf = self;
    drawPicture([_picture subdataWithRange:NSMakeRange(0, length)], ^NSString *(NSInteger index) {
        return [weakSelf tagAtIndex:index];
    });
    [NSGraphicsContext restoreGraphicsState];
}

// ---- Clicks -----------------------------------------------------------------

- (NSInteger)keycapAtPoint:(NSPoint)point
{
    NSAffineTransform *toDrawing = fitTransform([self bounds], keyboardSize());
    [toDrawing invert];
    NSPoint p = [toDrawing transformPoint:point];
    return juku_keycap_at((float)p.x, (float)p.y);
}

- (void)mouseDown:(NSEvent *)event
{
    NSInteger index = [self keycapAtPoint:[self convertPoint:[event locationInWindow] fromView:nil]];

    if (index < 0 || index >= _count) {
        return;
    }

    uint8_t key = _keycaps[index].id;

    [self showInStrip:index];

    if (key == kJukuKeyCaps) {
        return;     // the guest Caps Lock follows the Mac's
    }

    if (key == JukuKeyShift || key == JukuKeyCtrl) {
        BOOL latch = ![_latched containsObject:@(key)];
        latch ? [_latched addObject:@(key)] : [_latched removeObject:@(key)];
        juku_key(_machine, key, latch);
        [self poll];
        return;
    }

    _held = index;
    juku_key(_machine, key, true);
    [self poll];
}

- (void)mouseUp:(NSEvent *)event
{
    if (_held < 0) {
        return;
    }

    juku_key(_machine, _keycaps[_held].id, false);
    _held = -1;

    for (NSNumber *key in _latched) {
        juku_key(_machine, [key unsignedCharValue], false);
    }

    [_latched removeAllObjects];
    [self poll];
}

- (void)releaseClicks
{
    if (_held >= 0) {
        [self mouseUp:[NSApp currentEvent]];
    }

    for (NSNumber *key in _latched) {
        juku_key(_machine, [key unsignedCharValue], false);
    }

    [_latched removeAllObjects];
    [self poll];
}

- (void)setHover:(NSInteger)index
{
    if (index == _hover) {
        return;
    }

    _hover = index;
    [self setNeedsDisplay:YES];
    [NSObject cancelPreviousPerformRequestsWithTarget:self selector:@selector(lingerOver) object:nil];

    // Off a key, the strip keeps it a moment, so that crossing the gap
    // between two keys does not flash its idle text.
    if (index < 0 && _shown >= 0) {
        _lingering = _shown;
        [self performSelector:@selector(lingerOver) withObject:nil afterDelay:0.15 inModes:@[NSRunLoopCommonModes]];
        return;
    }

    [self showInStrip:index];
}

// The moment off a key is over: the strip shows no key, unless the pointer is
// on one again or a key was typed meanwhile.
- (void)lingerOver
{
    if (_hover < 0 && _shown == _lingering) {
        [self showInStrip:-1];
    }

    _lingering = -1;
}

- (void)showInStrip:(NSInteger)index
{
    _shown = index;

    if (_showKey) {
        _showKey(index);
    }
}

- (void)mouseMoved:(NSEvent *)event
{
    [self setHover:[self keycapAtPoint:[self convertPoint:[event locationInWindow] fromView:nil]]];
}

- (void)mouseExited:(NSEvent *)event
{
    [self setHover:-1];
}

// ---- Following the machine -----------------------------------------------------

- (void)viewDidMoveToWindow
{
    [_displayLink invalidate];
    _displayLink = nil;

    if ([self window]) {
        _displayLink = [self displayLinkWithTarget:self selector:@selector(displayLinkFired:)];
        [_displayLink addToRunLoop:[NSRunLoop mainRunLoop] forMode:NSRunLoopCommonModes];
    }
}

- (void)displayLinkFired:(CADisplayLink *)link
{
    if ([[self window] occlusionState] & NSWindowOcclusionStateVisible) {
        [self poll];
    }
}

// Redraws the keys whose state changed. A key the user typed shows in the
// info strip, unless the pointer is over another one.
- (void)poll
{
    NSInteger typed = -1;

    for (NSInteger i = 0; i < _count; i++) {
        bool down = juku_key_down(_machine, _keycaps[i].id);

        if (down == _down[i]) {
            continue;
        }

        _down[i] = down;
        [self setNeedsDisplay:YES];

        // The Caps Lock follower presses CAPS by itself.
        if (down && typed < 0 && i != _held && _keycaps[i].id != kJukuKeyCaps) {
            typed = i;
        }
    }

    if (typed >= 0 && _hover < 0 && ![_latched containsObject:@(_keycaps[typed].id)]) {
        [self showInStrip:typed];
    }

    uint8_t consw = juku_consw(_machine) & 3;

    if (consw != _consw) {
        _consw = consw;

        if (_modeChanged) {
            _modeChanged(consw);
        }
    }
}

@end

// One keycap, larger, on a piece of the opening: the info strip's picture of
// the key it describes.
@interface KeycapPreview: NSView
- (void)showKeycapAtIndex:(NSInteger)index;   // −1 for none
@end

@implementation KeycapPreview {
    NSData *_picture;
    NSSize _size;
}

- (BOOL)isFlipped
{
    return YES;
}

- (void)showKeycapAtIndex:(NSInteger)index
{
    float width = 0, height = 0;
    size_t length = index >= 0 ? juku_keycap_picture((size_t)index, NULL, 0, &width, &height) : 0;
    NSMutableData *picture = [NSMutableData dataWithLength:length];

    if (length) {
        juku_keycap_picture((size_t)index, [picture mutableBytes], length, &width, &height);
    }

    _picture = length ? picture : nil;
    _size = NSMakeSize(width, height);
    [self setNeedsDisplay:YES];
}

- (void)drawRect:(NSRect)dirtyRect
{
    if (!_picture) {
        return;
    }

    [fitTransform([self bounds], _size) concat];
    drawPicture(_picture, nil);
}

@end
