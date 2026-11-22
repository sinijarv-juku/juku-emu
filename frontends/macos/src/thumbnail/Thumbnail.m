// Quick Look thumbnail extension for snapshots (spec §6.8). A snapshot is a
// PNG whose picture is the screen, so the thumbnail is that picture, scaled
// to the size asked for. ImageIO is told the file is a PNG, since it does
// not know the snapshot type, and it skips the state in the private chunks.
//
// The type does not conform to public.png: if it did, Quick Look would hand
// snapshots to the system's image thumbnailer instead, which passes the
// snapshot type to ImageIO and fails.
//
// This is a binary of its own, built by build.sh into
// Contents/PlugIns/JukuThumbnail.appex. It is not part of the app's single
// translation unit, and nothing in the app imports it.

#import <ImageIO/ImageIO.h>
#import <QuickLookThumbnailing/QuickLookThumbnailing.h>

@interface ThumbnailProvider: QLThumbnailProvider
@end

@implementation ThumbnailProvider

- (void)provideThumbnailForFileRequest:(QLFileThumbnailRequest *)request
                     completionHandler:(void (^)(QLThumbnailReply *reply, NSError *error))handler
{
    NSDictionary *options = @{(__bridge id)kCGImageSourceTypeIdentifierHint: @"public.png"};
    CGImageSourceRef source = CGImageSourceCreateWithURL((__bridge CFURLRef)[request fileURL], (__bridge CFDictionaryRef)options);
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
