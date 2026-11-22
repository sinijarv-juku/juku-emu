// The disk image's artwork, made by package.sh (spec §3.4): the Finder
// window's background and the volume's icon. A program of its own, outside
// the app's translation unit, built and run only when packaging.
//
//   DiskImageArt <out folder> <app icon.icns> [<drive icon.icns>]
//
// writes into <out folder>:
//   background.png, background@2x.png   660×400 points: a light face and a
//       chevron from the app's place to the Applications folder's
//   volume.iconset/                     the drive icon with the app's icon on
//       it, in every size iconutil packs; without a drive icon, the app's
//       icon alone

#import <AppKit/AppKit.h>

// The window's layout, in points from its top left corner; package.sh puts
// the icons at the same places.
static const CGFloat kWidth = 660, kHeight = 400;
static const CGFloat kAppX = 180, kApplicationsX = 480, kIconsY = 170;

static void fail(NSString *message)
{
    fprintf(stderr, "DiskImageArt: %s\n", message.UTF8String);
    exit(1);
}

// A bitmap `width` × `height` pixels, drawn by `draw` in points, `scale`
// pixels to the point, with AppKit's y going up.
static NSBitmapImageRep *render(NSInteger width, NSInteger height, CGFloat scale, void (^draw)(void))
{
    NSBitmapImageRep *rep = [[NSBitmapImageRep alloc] initWithBitmapDataPlanes:NULL
                                                                    pixelsWide:width
                                                                    pixelsHigh:height
                                                                 bitsPerSample:8
                                                               samplesPerPixel:4
                                                                      hasAlpha:YES
                                                                      isPlanar:NO
                                                                colorSpaceName:NSCalibratedRGBColorSpace
                                                                   bytesPerRow:0
                                                                  bitsPerPixel:0];
    rep.size = NSMakeSize(width / scale, height / scale);
    NSGraphicsContext *context = [NSGraphicsContext graphicsContextWithBitmapImageRep:rep];
    [NSGraphicsContext saveGraphicsState];
    NSGraphicsContext.currentContext = context;
    context.imageInterpolation = NSImageInterpolationHigh;
    draw();
    [NSGraphicsContext restoreGraphicsState];
    return rep;
}

static void writePNG(NSBitmapImageRep *rep, NSString *path)
{
    NSData *png = [rep representationUsingType:NSBitmapImageFileTypePNG properties:@{}];
    if (![png writeToFile:path atomically:YES]) {
        fail([NSString stringWithFormat:@"cannot write %@", path]);
    }
}

// The background: a light grey face, as Finder's own windows have, and a
// dark chevron halfway between the two icons, at their height.
static void writeBackground(NSString *folder)
{
    for (NSInteger scale = 1; scale <= 2; scale++) {
        NSBitmapImageRep *rep = render(kWidth * scale, kHeight * scale, scale, ^{
            [[NSColor colorWithSRGBRed:0.94 green:0.94 blue:0.96 alpha:1] setFill];
            NSRectFill(NSMakeRect(0, 0, kWidth, kHeight));

            // AppKit's y goes up; the layout's goes down.
            CGFloat x = (kAppX + kApplicationsX) / 2, y = kHeight - kIconsY - 10;
            NSBezierPath *chevron = [NSBezierPath bezierPath];
            [chevron moveToPoint:NSMakePoint(x - 12, y + 22)];
            [chevron lineToPoint:NSMakePoint(x + 12, y)];
            [chevron lineToPoint:NSMakePoint(x - 12, y - 22)];
            chevron.lineWidth = 5;
            chevron.lineCapStyle = NSLineCapStyleRound;
            chevron.lineJoinStyle = NSLineJoinStyleRound;
            [[NSColor colorWithSRGBRed:0.17 green:0.17 blue:0.18 alpha:1] setStroke];
            [chevron stroke];
        });
        writePNG(rep, [folder stringByAppendingPathComponent:scale == 1 ? @"background.png" : @"background@2x.png"]);
    }
}

// The volume's icon in each size of an iconset: the drive, and the app's icon
// on its face, where the drive's own symbol is, which it covers.
static void writeVolumeIcon(NSString *folder, NSImage *app, NSImage *drive)
{
    NSString *iconset = [folder stringByAppendingPathComponent:@"volume.iconset"];
    [NSFileManager.defaultManager createDirectoryAtPath:iconset withIntermediateDirectories:YES attributes:nil error:NULL];

    for (NSNumber *size in @[ @16, @32, @128, @256, @512 ]) {
        NSInteger points = size.integerValue;
        for (NSInteger scale = 1; scale <= 2; scale++) {
            NSInteger pixels = points * scale;
            NSBitmapImageRep *rep = render(pixels, pixels, 1, ^{
                NSRect all = NSMakeRect(0, 0, pixels, pixels);
                if (drive == nil) {
                    [app drawInRect:all];
                    return;
                }
                [drive drawInRect:all];
                // The app's icon has a margin around its square: half the
                // canvas wide, it covers the middle 46% of the drive's face.
                CGFloat side = pixels * 0.5;
                [app drawInRect:NSMakeRect((pixels - side) / 2, pixels * 0.555 - side / 2, side, side)];
            });
            NSString *name = scale == 1 ? [NSString stringWithFormat:@"icon_%ldx%ld.png", points, points]
                                        : [NSString stringWithFormat:@"icon_%ldx%ld@2x.png", points, points];
            writePNG(rep, [iconset stringByAppendingPathComponent:name]);
        }
    }
}

int main(int argc, const char *argv[])
{
    @autoreleasepool {
        if (argc < 3) {
            fail(@"usage: DiskImageArt <out folder> <app icon.icns> [<drive icon.icns>]");
        }
        NSString *folder = @(argv[1]);
        NSImage *app = [[NSImage alloc] initWithContentsOfFile:@(argv[2])];
        if (app == nil) {
            fail([NSString stringWithFormat:@"cannot read %s", argv[2]]);
        }
        NSImage *drive = argc > 3 ? [[NSImage alloc] initWithContentsOfFile:@(argv[3])] : nil;

        [NSFileManager.defaultManager createDirectoryAtPath:folder withIntermediateDirectories:YES attributes:nil error:NULL];
        writeBackground(folder);
        writeVolumeIcon(folder, app, drive);
    }
    return 0;
}
