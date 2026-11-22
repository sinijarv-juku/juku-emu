#pragma once

#import <AudioToolbox/AudioToolbox.h>
#import <Foundation/Foundation.h>

#import "Core.m"

static const double kAudioSampleRate = 48000.0;

// Plays the core's audio ring through the default output device. The render
// callback runs on the audio thread and only calls juku_audio_pull.
@interface AudioOutput: NSObject

- (instancetype)initWithRing:(const JukuAudio *)ring;
- (void)stop;

@end

static OSStatus renderAudio(void *refCon, AudioUnitRenderActionFlags *flags, const AudioTimeStamp *timeStamp,
                            UInt32 bus, UInt32 frames, AudioBufferList *data)
{
    // The unit is set up for two non-interleaved channels: left, right.
    float *left = data->mBuffers[0].mData;
    float *right = data->mBuffers[1].mData;

    if (juku_audio_pull((const JukuAudio *)refCon, left, right, frames) == 0) {
        *flags |= kAudioUnitRenderAction_OutputIsSilence;
    }

    return noErr;
}

@implementation AudioOutput {
    AudioComponentInstance _unit;
}

- (instancetype)initWithRing:(const JukuAudio *)ring
{
    self = [super init];

    if (!self) {
        return nil;
    }

    AudioComponentDescription desc = {
        .componentType = kAudioUnitType_Output,
        .componentSubType = kAudioUnitSubType_DefaultOutput,
        .componentManufacturer = kAudioUnitManufacturer_Apple,
    };
    AudioComponent component = AudioComponentFindNext(NULL, &desc);

    if (!component || AudioComponentInstanceNew(component, &_unit) != noErr) {
        return nil;
    }

    AudioStreamBasicDescription format = {
        .mSampleRate = kAudioSampleRate,
        .mFormatID = kAudioFormatLinearPCM,
        .mFormatFlags = kAudioFormatFlagIsFloat | kAudioFormatFlagIsPacked | kAudioFormatFlagIsNonInterleaved,
        .mBytesPerPacket = sizeof(float),
        .mFramesPerPacket = 1,
        .mBytesPerFrame = sizeof(float),
        .mChannelsPerFrame = 2,
        .mBitsPerChannel = 32,
    };
    AURenderCallbackStruct callback = {
        .inputProc = renderAudio,
        .inputProcRefCon = (void *)ring,
    };

    if (AudioUnitSetProperty(_unit, kAudioUnitProperty_StreamFormat, kAudioUnitScope_Input, 0, &format, sizeof(format)) != noErr ||
        AudioUnitSetProperty(_unit, kAudioUnitProperty_SetRenderCallback, kAudioUnitScope_Input, 0, &callback, sizeof(callback)) != noErr ||
        AudioUnitInitialize(_unit) != noErr) {
        AudioComponentInstanceDispose(_unit);
        return nil;
    }

    if (AudioOutputUnitStart(_unit) != noErr) {
        AudioUnitUninitialize(_unit);
        AudioComponentInstanceDispose(_unit);
        return nil;
    }

    return self;
}

- (void)stop
{
    if (_unit) {
        AudioOutputUnitStop(_unit);
        AudioUnitUninitialize(_unit);
        AudioComponentInstanceDispose(_unit);
        _unit = NULL;
    }
}

- (void)dealloc
{
    [self stop];
}

@end
