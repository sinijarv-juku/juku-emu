// Types shared by Renderer.m and Shaders.metal, which are compiled separately.

#pragma once

#include <simd/simd.h>

typedef struct {
    simd_float4 view;       // tube area in drawable pixels: x, y, w, h
    simd_float4 active;     // the core's raster inside the canvas: x, y, w, h
    simd_float4 phosphor;   // linear phosphor colour, w = brightness
    simd_float4 background; // linear colour of the unlit face (room light on the glass)
    simd_float4 frame;      // linear colour of the monitor frame around the glass
    simd_float2 canvas;     // raster canvas size: 8 texels per µs × lines
    simd_float2 output;     // drawable size in pixels
    float decayFast;        // per-draw decay of the main phosphor component
    float decaySlow;        // per-draw decay of the faint afterglow
    float afterglow;        // afterglow strength
    float curvature;        // barrel distortion
    float scanline;         // beam sigma, in raster lines
    float bloom;            // beam widening with brightness
    float glow;             // halo strength
    float vignette;
    unsigned int bezel3d;   // the frame's inner wall in perspective, else a flat recess
    unsigned int pad[3];
} CRTUniforms;
