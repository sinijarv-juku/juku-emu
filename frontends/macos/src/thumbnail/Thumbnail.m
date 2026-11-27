// Quick Look thumbnail extension for snapshots (spec §6.8). A snapshot holds
// a PNG of the screen in its THMB section (§4.6), so the thumbnail is that
// picture, scaled to the size asked for. The sections are walked here
// rather than by the core, which this binary does not link; ImageIO checks
// the PNG.
//
// This is a binary of its own, built by build.sh into
// Contents/PlugIns/JukuThumbnail.appex. It is not part of the app's single
// translation unit, and nothing in the app imports it.

#import <ImageIO/ImageIO.h>
#import <QuickLookThumbnailing/QuickLookThumbnailing.h>

@interface ThumbnailProvider: QLThumbnailProvider
@end

// The PNG in a snapshot file's THMB section, or nil. A section is its tag,
// its length (u32, little-endian), its bytes and a CRC-32.
static NSData *thumbnailPNG(NSData *file)
{
    static const uint8_t signature[9] = {0x8A, 'J', 'U', 'K', 'U', 0x0D, 0x0A, 0x1A, 0x0A};
    const uint8_t *bytes = [file bytes];
    NSUInteger length = [file length];
    NSUInteger at = sizeof signature;

    if (length < at || memcmp(bytes, signature, at) != 0) {
        return nil;
    }

    while (length - at >= 12) {
        const uint8_t *section = bytes + at;
        NSUInteger size = section[4] | section[5] << 8 | section[6] << 16 | (NSUInteger)section[7] << 24;

        if (size > length - at - 12) {
            return nil;
        }

        if (memcmp(section, "THMB", 4) == 0) {
            return [file subdataWithRange:NSMakeRange(at + 8, size)];
        }

        at += 12 + size;
    }

    return nil;
}

@implementation ThumbnailProvider

- (void)provideThumbnailForFileRequest:(QLFileThumbnailRequest *)request
                     completionHandler:(void (^)(QLThumbnailReply *reply, NSError *error))handler
{
    NSData *file = [NSData dataWithContentsOfURL:[request fileURL] options:NSDataReadingMappedIfSafe error:NULL];
    NSData *png = file ? thumbnailPNG(file) : nil;
    NSDictionary *options = @{(__bridge id)kCGImageSourceTypeIdentifierHint: @"public.png"};
    CGImageSourceRef source = png ? CGImageSourceCreateWithData((__bridge CFDataRef)png, (__bridge CFDictionaryRef)options) : NULL;
    id image = source ? CFBridgingRelease(CGImageSourceCreateImageAtIndex(source, 0, NULL)) : nil;

    if (source) {
        CFRelease(source);
    }

    if (!image) {
        handler(nil, [NSError errorWithDomain:NSCocoaErrorDomain code:NSFileReadCorruptFileError userInfo:nil]);
        return;
    }

    CGFloat width = CGImageGetWidth((__bridge CGImageRef)image);
    CGFloat height = CGImageGetHeight((__bridge CGImageRef)image);
    CGSize maximum = [request maximumSize];
    CGFloat scale = fmin(maximum.width / width, maximum.height / height);
    CGSize size = CGSizeMake(fmax(1.0, round(width * scale)), fmax(1.0, round(height * scale)));

    // The context is `size` × the request's scale, in pixels and not scaled
    // to points, so the picture is drawn over the whole of it.
    handler([QLThumbnailReply replyWithContextSize:size drawingBlock:^BOOL(CGContextRef context) {
        CGContextSetInterpolationQuality(context, kCGInterpolationHigh);
        CGContextDrawImage(context, CGContextGetClipBoundingBox(context), (__bridge CGImageRef)image);
        return YES;
    }], nil);
}

@end
