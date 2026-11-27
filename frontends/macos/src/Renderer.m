#pragma once

#import <AppKit/AppKit.h>
#import <QuartzCore/QuartzCore.h>
#import <Metal/Metal.h>
#import <simd/simd.h>

#import "Core.m"
#import "Settings.m"
#import "ShaderTypes.h"

// The visible part of the raster on a PAL-like monitor: 52 µs × 287 lines,
// sampled at 8 texels per µs (one texel = one Juku pixel). The visible window
// starts 10.5 µs after the start of hsync and 23 lines after the start of
// vsync. Shown on a 4:3 tube, this gives Juku pixels an aspect of about 0.92,
// whatever the video mode.
static const NSUInteger kCanvasWidth = 52 * 8;
static const NSUInteger kCanvasHeight = 287;
static const float kVisibleStartUs = 10.5f;
static const float kVisibleStartLines = 23.0f;
static const float kTubeAspect = 4.0f / 3.0f;

// With the 3D bezel, in a window the tube is drawn smaller, so the bezel shows
// all round: a margin of this many half-heights of the tube above and below
// it. In full screen it keeps the full height, and the bezel shows at the
// sides.
static const float kWindowBezel = 0.095f;

// The room tubeRect leaves around the tube, in half-heights of it: at each
// side, above and below.
typedef struct {
    float side;
    float top;
    float bottom;
} TubeMargins;

// The same margin above and below, and at the sides as much as the tube's
// aspect makes of it, so the drawable keeps the tube's shape.
static TubeMargins evenMargins(float margin)
{
    return (TubeMargins){ margin * kTubeAspect, margin, margin };
}

// With Whole Monitor, the tube leaves room for the monitor's front around the
// glass (Shaders.metal's kFrontSide, kFrontTop and kFrontBottom) and this much
// of the room around it, in a window and in full screen.
static const float kFrontRoom = 0.08f;
static const TubeMargins kFrontMargins = { 0.30f + kFrontRoom, 0.235f + kFrontRoom, 0.44f + kFrontRoom };

#define kInFlightFrames 3

// The Juku shipped with a black-and-white monitor: a white P4 phosphor. Its
// sRGB colour, the decay times of the main response and of the faint
// afterglow, and the afterglow strength.
static const float kPhosphorRed = 0xE8 / 255.0f;
static const float kPhosphorGreen = 0xF0 / 255.0f;
static const float kPhosphorBlue = 0xFF / 255.0f;
static const float kPhosphorFastMs = 5.0f;
static const float kPhosphorSlowMs = 60.0f;
static const float kPhosphorAfterglow = 0.06f;

// The unlit face of the МС6105 is not black: room light comes back off the
// grey phosphor behind the glass. Its linear colour, a neutral grey with a
// slight green cast (from a photo of a switched-off Juku in daylight), scaled
// to unit luminance, and the level at the top of the Background setting, as a
// fraction of full white. Video frames of a working Juku in a lit room put it
// at 5–13 %, but the camera brightened them; other photos of such tubes give
// 1–4 %, and 8 % at the top of the scale looked natural in the app.
static const simd_float3 kBackgroundColour = { 1.0f / 1.0164f, 1.03f / 1.0164f, 0.93f / 1.0164f };
static const float kBackgroundMax = 0.08f;

// The room light, at the top of the Room Light setting, as a fraction of full
// white: it lights the monitor's plastic, the badge and the room around it.
// The plastic, a neutral charcoal, reflects a share of it that the Frame
// setting sets, its tone: from near black to light gray over the square of
// the setting. At the defaults the plastic is 0.4% of white, near black, as
// the МС6105's bezel is, and the room around it brighter.
static const simd_float3 kFrameColour = { 1.0f, 0.97f, 0.92f };
static const float kRoomMax = 0.4f;
static const float kPlasticDark = 0.025f;
static const float kPlasticLight = 0.6f;

// Beam sigma in raster lines at the two ends of the Scanlines setting: a soft
// beam whose lines just touch, and a thin one with wide dark gaps.
static const float kScanlineSoft = 0.42f;
static const float kScanlineSharp = 0.15f;

static float srgbToLinear(float c)
{
    return c <= 0.04045f ? c / 12.92f : powf((c + 0.055f) / 1.055f, 2.4f);
}

@interface Renderer: NSObject

@property(nonatomic, readonly) id<MTLDevice> device;
// Full screen: the tube is not shrunk to show the bezel above and below.
@property(nonatomic) BOOL fullScreen;

// Draws `frame`. `elapsed` is the time in seconds the phosphor has had to
// decay since the picture held by the previous draw.
- (void)drawFrame:(const JukuFrame *)frame toDrawable:(id<CAMetalDrawable>)drawable elapsed:(double)elapsed;
// `frame` drawn as a PNG of `width` × `height` pixels, as the window would
// show it with the current settings but with no afterglow. Nil on failure.
- (NSData *)pngForFrame:(const JukuFrame *)frame width:(NSUInteger)width height:(NSUInteger)height;

@end

@implementation Renderer
{
    id<MTLCommandQueue> _commandQueue;
    id<MTLRenderPipelineState> _decodePipeline;
    id<MTLRenderPipelineState> _persistPipeline;
    id<MTLRenderPipelineState> _crtPipeline;
    id<MTLRenderPipelineState> _flatPipeline;
    id<MTLRenderPipelineState> _downsamplePipeline;
    id<MTLTexture> _videoTextures[kInFlightFrames];
    id<MTLTexture> _beam;               // decoded canvas
    id<MTLTexture> _phosphor[2];        // persistence, ping-pong
    dispatch_semaphore_t _frameSemaphore;
    NSUInteger _frameIndex;
    MTLStorageMode _cpuStorage;         // for textures the CPU writes or reads
}

static id<MTLRenderPipelineState> makePipeline(id<MTLDevice> device, id<MTLLibrary> library, NSString *fragment, MTLPixelFormat format)
{
    MTLRenderPipelineDescriptor *desc = [[MTLRenderPipelineDescriptor alloc] init];
    [desc setLabel:fragment];
    [desc setVertexFunction:[library newFunctionWithName:@"vsh_fullscreen"]];
    [desc setFragmentFunction:[library newFunctionWithName:fragment]];
    [[desc colorAttachments][0] setPixelFormat:format];

    NSError *error;
    id<MTLRenderPipelineState> pipeline = [device newRenderPipelineStateWithDescriptor:desc error:&error];

    if (!pipeline) {
        NSLog(@"ERROR: Failed to create pipeline %@: %@", fragment, error);
    }

    return pipeline;
}

// The GPU to draw with. A Mac with two GPUs gets the integrated one, as
// NSSupportsAutomaticGraphicsSwitching in Info.plist says: the default device
// would be the discrete GPU, and using it would switch the whole Mac to it,
// costing battery for a picture this small. Apple silicon has one GPU, which
// is not a low-power one, and takes the default.
static id<MTLDevice> chooseDevice(void)
{
    for (id<MTLDevice> device in MTLCopyAllDevices()) {
        if ([device isLowPower] && ![device isRemovable] && ![device isHeadless]) {
            return device;
        }
    }

    return MTLCreateSystemDefaultDevice();
}

// The phosphor textures have mipmaps: the 3D bezel's wall is lit by wide
// averages of the face (Shaders.metal, faceLight).
static id<MTLTexture> makeCanvasTexture(id<MTLDevice> device, MTLPixelFormat format, BOOL mipmapped, NSString *label)
{
    MTLTextureDescriptor *desc = [MTLTextureDescriptor texture2DDescriptorWithPixelFormat:format
                                                                                    width:kCanvasWidth
                                                                                   height:kCanvasHeight
                                                                                mipmapped:mipmapped];
    [desc setUsage:MTLTextureUsageShaderRead | MTLTextureUsageRenderTarget];
    [desc setStorageMode:MTLStorageModePrivate];
    id<MTLTexture> texture = [device newTextureWithDescriptor:desc];
    [texture setLabel:label];
    return texture;
}

static void encodePass(id<MTLCommandBuffer> commandBuffer, id<MTLTexture> target, MTLLoadAction load,
                       id<MTLRenderPipelineState> pipeline, const CRTUniforms *uniforms,
                       id<MTLTexture> texture0, id<MTLTexture> texture1)
{
    MTLRenderPassDescriptor *pass = [MTLRenderPassDescriptor renderPassDescriptor];
    MTLRenderPassColorAttachmentDescriptor *color = [pass colorAttachments][0];
    [color setTexture:target];
    [color setLoadAction:load];
    [color setStoreAction:MTLStoreActionStore];
    [color setClearColor:MTLClearColorMake(0.0, 0.0, 0.0, 1.0)];

    id<MTLRenderCommandEncoder> encoder = [commandBuffer renderCommandEncoderWithDescriptor:pass];

    if (pipeline) {     // without one the pass only clears
        [encoder setLabel:[pipeline label]];
        [encoder setRenderPipelineState:pipeline];
        [encoder setFragmentBytes:uniforms length:sizeof(*uniforms) atIndex:0];
        [encoder setFragmentTexture:texture0 atIndex:0];
        [encoder setFragmentTexture:texture1 atIndex:1];
        [encoder drawPrimitives:MTLPrimitiveTypeTriangle vertexStart:0 vertexCount:3];
    }

    [encoder endEncoding];
}

// Each mip level of `texture` drawn from the one above it (Shaders.metal,
// fsh_downsample), which it reads through a view of that level alone.
static void encodeMipmaps(id<MTLCommandBuffer> commandBuffer, id<MTLRenderPipelineState> pipeline, id<MTLTexture> texture)
{
    for (NSUInteger level = 1; level < [texture mipmapLevelCount]; level++) {
        id<MTLTexture> above = [texture newTextureViewWithPixelFormat:[texture pixelFormat]
                                                          textureType:MTLTextureType2D
                                                               levels:NSMakeRange(level - 1, 1)
                                                               slices:NSMakeRange(0, 1)];
        MTLRenderPassDescriptor *pass = [MTLRenderPassDescriptor renderPassDescriptor];
        MTLRenderPassColorAttachmentDescriptor *color = [pass colorAttachments][0];
        [color setTexture:texture];
        [color setLevel:level];
        [color setLoadAction:MTLLoadActionDontCare];
        [color setStoreAction:MTLStoreActionStore];

        id<MTLRenderCommandEncoder> encoder = [commandBuffer renderCommandEncoderWithDescriptor:pass];
        [encoder setLabel:[pipeline label]];
        [encoder setRenderPipelineState:pipeline];
        [encoder setFragmentTexture:above atIndex:0];
        [encoder drawPrimitives:MTLPrimitiveTypeTriangle vertexStart:0 vertexCount:3];
        [encoder endEncoding];
    }
}

- (instancetype)init
{
    self = [super init];

    if (!self) {
        return nil;
    }

    _device = chooseDevice();
    // Apple's GPUs share the Mac's memory with the CPU, so a texture the CPU
    // writes or reads can be shared. Intel's and AMD's GPUs take no shared
    // textures: there it is managed, a copy on each side that Metal keeps in
    // step, and `pngForFrame:` brings its target back with a blit.
    _cpuStorage = [_device supportsFamily:MTLGPUFamilyApple1] ? MTLStorageModeShared : MTLStorageModeManaged;
    _commandQueue = [_device newCommandQueue];
    id<MTLLibrary> library = [_device newDefaultLibrary];

    if (!library) {
        NSLog(@"ERROR: Cannot load the default shader library");
        return nil;
    }

    _decodePipeline = makePipeline(_device, library, @"fsh_decode", MTLPixelFormatR16Float);
    _persistPipeline = makePipeline(_device, library, @"fsh_persist", MTLPixelFormatRG16Float);
    _crtPipeline = makePipeline(_device, library, @"fsh_crt", MTLPixelFormatBGRA8Unorm_sRGB);
    _flatPipeline = makePipeline(_device, library, @"fsh_flat", MTLPixelFormatBGRA8Unorm_sRGB);
    _downsamplePipeline = makePipeline(_device, library, @"fsh_downsample", MTLPixelFormatRG16Float);

    if (!_decodePipeline || !_persistPipeline || !_crtPipeline || !_flatPipeline || !_downsamplePipeline) {
        return nil;
    }

    MTLTextureDescriptor *videoDesc = [MTLTextureDescriptor texture2DDescriptorWithPixelFormat:MTLPixelFormatR8Uint
                                                                                         width:JukuRasterBytes
                                                                                        height:JukuRasterLines
                                                                                     mipmapped:NO];
    [videoDesc setUsage:MTLTextureUsageShaderRead];
    [videoDesc setStorageMode:_cpuStorage];

    for (int i = 0; i < kInFlightFrames; i++) {
        _videoTextures[i] = [_device newTextureWithDescriptor:videoDesc];
        [_videoTextures[i] setLabel:[NSString stringWithFormat:@"Video RAM %d", i]];
    }

    _beam = makeCanvasTexture(_device, MTLPixelFormatR16Float, NO, @"Beam");
    _phosphor[0] = makeCanvasTexture(_device, MTLPixelFormatRG16Float, YES, @"Phosphor 0");
    _phosphor[1] = makeCanvasTexture(_device, MTLPixelFormatRG16Float, YES, @"Phosphor 1");
    _frameSemaphore = dispatch_semaphore_create(kInFlightFrames);

    // Start the phosphor dark.
    id<MTLCommandBuffer> clear = [_commandQueue commandBuffer];
    for (int i = 0; i < 2; i++) {
        encodePass(clear, _phosphor[i], MTLLoadActionClear, nil, NULL, nil, nil);
    }

    [clear commit];
    return self;
}

// The largest 4:3 tube that fits in the drawable with `margins` around it,
// centred across and placed between its top and bottom margins.
static simd_float4 tubeRect(float width, float height, TubeMargins m)
{
    float scale = fminf(width / (2.0f * (kTubeAspect + m.side)), height / (2.0f + m.top + m.bottom));
    float w = 2.0f * kTubeAspect * scale;
    float h = 2.0f * scale;
    float y = (height - scale * (2.0f + m.top + m.bottom)) * 0.5f + scale * m.top;

    return (simd_float4){ floorf((width - w) * 0.5f), floorf(y), w, h };
}

// Whole Monitor is drawn: it is on, with CRT Effects.
static BOOL monitorFrontShown(void)
{
    return settingBool(kSettingMonitorFront) && settingBool(kSettingCRTEffects);
}

// The room tubeRect leaves for the bezel, or for the whole monitor, with the
// current settings.
static TubeMargins tubeMargins(BOOL fullScreen)
{
    if (monitorFrontShown()) {
        return kFrontMargins;
    }

    BOOL windowBezel = settingBool(kSettingBezel3D) && !fullScreen && settingBool(kSettingCRTEffects);
    return evenMargins(windowBezel ? kWindowBezel : 0.0f);
}

// `thumbnail`: the screen with its bezel as in a window, never the whole
// monitor, so that a file's thumbnail shows the picture large.
- (CRTUniforms)uniformsForFrame:(const JukuFrame *)frame target:(id<MTLTexture>)target elapsed:(double)elapsed
                       fullScreen:(BOOL)fullScreen thumbnail:(BOOL)thumbnail
{
    // Long enough and both decays are zero: nothing is left of the old picture.
    float dt = (float)fmin(fmax(elapsed, 0.0), 10.0);
    float scanlines = (float)settingDouble(kSettingScanlines);
    float background = kBackgroundMax * fminf(fmaxf((float)settingDouble(kSettingBackground), 0.0f), 1.0f);
    float tone = fminf(fmaxf((float)settingDouble(kSettingFrame), 0.0f), 1.0f);
    float plastic = kPlasticDark + (kPlasticLight - kPlasticDark) * tone * tone;
    float room = kRoomMax * fminf(fmaxf((float)settingDouble(kSettingRoom), 0.0f), 1.0f);
    BOOL bezel3d = settingBool(kSettingBezel3D);
    BOOL front = monitorFrontShown() && !thumbnail;
    TubeMargins margins = front ? kFrontMargins
                        : thumbnail ? evenMargins(bezel3d && settingBool(kSettingCRTEffects) ? kWindowBezel : 0.0f)
                        : tubeMargins(fullScreen);

    return (CRTUniforms){
        .view = tubeRect((float)[target width], (float)[target height], margins),
        // The core's raster starts at the start of the sync pulses.
        .active = { -kVisibleStartUs * 8.0f, -kVisibleStartLines, JukuRasterBytes * 8.0f, JukuRasterLines },
        .phosphor = {
            srgbToLinear(kPhosphorRed), srgbToLinear(kPhosphorGreen), srgbToLinear(kPhosphorBlue),
            (float)settingDouble(kSettingBrightness),
        },
        .background = simd_make_float4(kBackgroundColour * background, 0.0f),
        .frame = simd_make_float4(kFrameColour * plastic * room, plastic),
        .canvas = { (float)kCanvasWidth, (float)kCanvasHeight },
        .output = { (float)[target width], (float)[target height] },
        .decayFast = expf(-dt * 1000.0f / kPhosphorFastMs),
        .decaySlow = expf(-dt * 1000.0f / kPhosphorSlowMs),
        .afterglow = kPhosphorAfterglow,
        .curvature = (float)settingDouble(kSettingCurvature),
        .scanline = kScanlineSoft + (kScanlineSharp - kScanlineSoft) * fminf(fmaxf(scanlines, 0.0f), 1.0f),
        .bloom = 0.15f,
        .glow = (float)settingDouble(kSettingGlow),
        .vignette = 0.25f,
        .bezel3d = bezel3d,
        .monitorFront = front,
        .room = room,
    };
}

- (void)drawFrame:(const JukuFrame *)frame toDrawable:(id<CAMetalDrawable>)drawable elapsed:(double)elapsed
{
    if (!drawable) {
        return;
    }

    dispatch_semaphore_wait(_frameSemaphore, DISPATCH_TIME_FOREVER);

    id<MTLTexture> video = _videoTextures[_frameIndex % kInFlightFrames];
    id<MTLTexture> previous = _phosphor[_frameIndex & 1];
    id<MTLTexture> phosphor = _phosphor[(_frameIndex + 1) & 1];
    _frameIndex++;

    [video replaceRegion:MTLRegionMake2D(0, 0, JukuRasterBytes, JukuRasterLines)
             mipmapLevel:0
               withBytes:frame->data
             bytesPerRow:JukuRasterBytes];

    id<MTLTexture> target = [drawable texture];
    CRTUniforms uniforms = [self uniformsForFrame:frame target:target elapsed:elapsed fullScreen:_fullScreen thumbnail:NO];

    id<MTLCommandBuffer> commandBuffer = [_commandQueue commandBuffer];
    dispatch_semaphore_t semaphore = _frameSemaphore;
    [commandBuffer addCompletedHandler:^(id<MTLCommandBuffer> _) {
        dispatch_semaphore_signal(semaphore);
    }];

    encodePass(commandBuffer, _beam, MTLLoadActionDontCare, _decodePipeline, &uniforms, video, nil);
    encodePass(commandBuffer, phosphor, MTLLoadActionDontCare, _persistPipeline, &uniforms, _beam, previous);

    if (settingBool(kSettingCRTEffects)) {
        if (uniforms.bezel3d) {
            encodeMipmaps(commandBuffer, _downsamplePipeline, phosphor);
        }

        encodePass(commandBuffer, target, MTLLoadActionDontCare, _crtPipeline, &uniforms, phosphor, nil);
    } else {
        encodePass(commandBuffer, target, MTLLoadActionDontCare, _flatPipeline, &uniforms, _beam, nil);
    }

    [commandBuffer presentDrawable:drawable];
    [commandBuffer commit];
}

// For snapshot thumbnails (spec §6.8). The passes are the window's, into
// textures of their own, so the window's phosphor is left alone.
- (NSData *)pngForFrame:(const JukuFrame *)frame width:(NSUInteger)width height:(NSUInteger)height
{
    MTLTextureDescriptor *desc = [MTLTextureDescriptor texture2DDescriptorWithPixelFormat:MTLPixelFormatR8Uint
                                                                                    width:JukuRasterBytes
                                                                                   height:JukuRasterLines
                                                                                mipmapped:NO];
    [desc setUsage:MTLTextureUsageShaderRead];
    [desc setStorageMode:_cpuStorage];
    id<MTLTexture> video = [_device newTextureWithDescriptor:desc];
    id<MTLTexture> beam = makeCanvasTexture(_device, MTLPixelFormatR16Float, NO, @"Thumbnail beam");
    id<MTLTexture> phosphor = makeCanvasTexture(_device, MTLPixelFormatRG16Float, YES, @"Thumbnail phosphor");
    desc = [MTLTextureDescriptor texture2DDescriptorWithPixelFormat:MTLPixelFormatBGRA8Unorm_sRGB width:width height:height mipmapped:NO];
    [desc setUsage:MTLTextureUsageRenderTarget];
    [desc setStorageMode:_cpuStorage];
    id<MTLTexture> target = [_device newTextureWithDescriptor:desc];

    if (!video || !beam || !phosphor || !target) {
        return nil;
    }

    [video replaceRegion:MTLRegionMake2D(0, 0, JukuRasterBytes, JukuRasterLines)
             mipmapLevel:0
               withBytes:frame->data
             bytesPerRow:JukuRasterBytes];

    // A long time since the last picture: no afterglow from an old one.
    CRTUniforms uniforms = [self uniformsForFrame:frame target:target elapsed:10.0 fullScreen:NO thumbnail:YES];
    id<MTLCommandBuffer> commandBuffer = [_commandQueue commandBuffer];
    encodePass(commandBuffer, beam, MTLLoadActionDontCare, _decodePipeline, &uniforms, video, nil);
    encodePass(commandBuffer, phosphor, MTLLoadActionDontCare, _persistPipeline, &uniforms, beam, beam);
    BOOL crt = settingBool(kSettingCRTEffects);

    if (crt && uniforms.bezel3d) {
        encodeMipmaps(commandBuffer, _downsamplePipeline, phosphor);
    }

    encodePass(commandBuffer, target, MTLLoadActionDontCare, crt ? _crtPipeline : _flatPipeline, &uniforms, crt ? phosphor : beam, nil);

    if (_cpuStorage == MTLStorageModeManaged) {
        id<MTLBlitCommandEncoder> blit = [commandBuffer blitCommandEncoder];
        [blit synchronizeResource:target];
        [blit endEncoding];
    }

    [commandBuffer commit];
    [commandBuffer waitUntilCompleted];

    NSBitmapImageRep *image = [[NSBitmapImageRep alloc] initWithBitmapDataPlanes:NULL
                                                                      pixelsWide:(NSInteger)width
                                                                      pixelsHigh:(NSInteger)height
                                                                   bitsPerSample:8
                                                                 samplesPerPixel:3
                                                                        hasAlpha:NO
                                                                        isPlanar:NO
                                                                  colorSpaceName:NSCalibratedRGBColorSpace
                                                                     bytesPerRow:width * 3
                                                                    bitsPerPixel:24];
    NSMutableData *bgra = [NSMutableData dataWithLength:width * height * 4];
    [target getBytes:[bgra mutableBytes] bytesPerRow:width * 4 fromRegion:MTLRegionMake2D(0, 0, width, height) mipmapLevel:0];
    const uint8_t *from = [bgra bytes];
    uint8_t *to = [image bitmapData];

    for (NSUInteger i = 0; i < width * height; i++) {
        to[3 * i] = from[4 * i + 2];
        to[3 * i + 1] = from[4 * i + 1];
        to[3 * i + 2] = from[4 * i];
    }

    image = [image bitmapImageRepByRetaggingWithColorSpace:[NSColorSpace sRGBColorSpace]];
    return [image representationUsingType:NSBitmapImageFileTypePNG properties:@{}];
}

@end
