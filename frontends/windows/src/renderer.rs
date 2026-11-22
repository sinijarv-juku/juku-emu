//! Direct3D 12 renderer (spec §7, §11.5): the passes of
//! frontends/macos/src/Renderer.m, with the shaders in Shaders.hlsl.
//!
//! Each draw runs decode (the frame's bytes → the beam canvas), persistence
//! (the beam → the phosphor, ping-pong) and the output pass into the swap
//! chain: the CRT composite of the phosphor, or the flat pass of the beam with
//! CRT Effects off. Up to three draws are in flight, each with a command
//! allocator and a buffer of video bytes of its own, as the Mac's semaphore
//! and textures do.
//!
//! A snapshot's thumbnail is drawn by the same passes into textures of its
//! own, off the screen (`thumbnail`), so the window's phosphor is left alone.

use std::mem::ManuallyDrop;

use juku::Frame;
use juku::video::{RASTER_BYTES, RASTER_LINES, RASTER_SIZE};
use windows::Win32::Foundation::{HANDLE, HWND, RECT};
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D12::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::System::Threading::{CreateEventW, INFINITE, WaitForSingleObjectEx};
use windows::core::{Interface, Result};


// The visible part of the raster on a PAL-like monitor: 52 µs × 287 lines,
// sampled at 8 texels per µs (one texel = one Juku pixel). The visible window
// starts 10.5 µs after the start of hsync and 23 lines after the start of
// vsync. Shown on a 4:3 tube, this gives Juku pixels an aspect of about 0.92,
// whatever the video mode.
pub const CANVAS_WIDTH: u32 = 52 * 8;
pub const CANVAS_HEIGHT: u32 = 287;
const VISIBLE_START_US: f32 = 10.5;
const VISIBLE_START_LINES: f32 = 23.0;
pub const TUBE_ASPECT: f32 = 4.0 / 3.0;

// With the 3D bezel, in a window the tube is drawn smaller, so the bezel shows
// all round: a margin of this many half-heights of the tube above and below
// it. In full screen it keeps the full height, and the bezel shows at the
// sides.
const WINDOW_BEZEL: f32 = 0.095;

const IN_FLIGHT: usize = 3;
const BUFFERS: u32 = 3;
const BACKBUFFER_FORMAT: DXGI_FORMAT = DXGI_FORMAT_B8G8R8A8_UNORM;
const SWAP_CHAIN_FLAGS: DXGI_SWAP_CHAIN_FLAG = DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT;

// The Juku shipped with a black-and-white monitor: a white P4 phosphor. Its
// sRGB colour, the decay times of the main response and of the faint
// afterglow, and the afterglow strength.
const PHOSPHOR_RGB: [u8; 3] = [0xE8, 0xF0, 0xFF];
const PHOSPHOR_FAST_MS: f32 = 5.0;
const PHOSPHOR_SLOW_MS: f32 = 60.0;
const PHOSPHOR_AFTERGLOW: f32 = 0.06;

// The unlit face and the monitor frame (Renderer.m): linear colours scaled to
// unit luminance, and their levels at the top of the Background and Frame
// settings, as fractions of full white.
const BACKGROUND_COLOUR: [f32; 3] = [1.0 / 1.0164, 1.03 / 1.0164, 0.93 / 1.0164];
const BACKGROUND_MAX: f32 = 0.08;
const FRAME_COLOUR: [f32; 3] = [1.0, 0.97, 0.92];
const FRAME_MAX: f32 = 0.04;

// Beam sigma in raster lines at the two ends of the Scanlines setting.
const SCANLINE_SOFT: f32 = 0.42;
const SCANLINE_SHARP: f32 = 0.15;

static VS_FULLSCREEN: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/vs_fullscreen.cso"));
static PS_DECODE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/ps_decode.cso"));
static PS_PERSIST: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/ps_persist.cso"));
static PS_DOWNSAMPLE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/ps_downsample.cso"));
static PS_CRT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/ps_crt.cso"));
static PS_FLAT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/ps_flat.cso"));

/// The picture settings the renderer reads (Settings.m keys, spec §7.3a).
#[derive(Clone, Copy)]
pub struct CrtSettings {
    pub crt_effects: bool,
    pub curvature: f32,
    pub scanlines: f32,
    pub glow: f32,
    pub background: f32,
    pub frame: f32,
    pub bezel3d: bool,
    pub brightness: f32,
}

impl Default for CrtSettings {
    /// The Mac's registered defaults (Settings.m).
    fn default() -> CrtSettings {
        CrtSettings {
            crt_effects: true,
            curvature: 0.022,
            scanlines: 0.68,
            glow: 0.085,
            background: 0.175,
            frame: 0.1,
            bezel3d: true,
            brightness: 0.91,
        }
    }
}

impl CrtSettings {
    /// The margin `tube_rect` leaves for the bezel.
    pub fn tube_margin(&self, full_screen: bool) -> f32 {
        if self.bezel3d && self.crt_effects && !full_screen { WINDOW_BEZEL } else { 0.0 }
    }
}

/// Mirrors CRTUniforms in Shaders.hlsl; passed as root constants.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CrtUniforms {
    view: [f32; 4],
    active: [f32; 4],
    phosphor: [f32; 4],
    background: [f32; 4],
    frame: [f32; 4],
    canvas: [f32; 2],
    output: [f32; 2],
    decay_fast: f32,
    decay_slow: f32,
    afterglow: f32,
    curvature: f32,
    scanline: f32,
    bloom: f32,
    glow: f32,
    vignette: f32,
    bezel3d: u32,
    pad: [u32; 3],
}

const UNIFORM_WORDS: u32 = (size_of::<CrtUniforms>() / 4) as u32;
const _: () = assert!(size_of::<CrtUniforms>() == 144);

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

/// The largest 4:3 rectangle centred in the drawable, shrunk to leave
/// `margin` half-heights of it above and below.
pub fn tube_rect(width: f32, height: f32, margin: f32) -> [f32; 4] {
    let (mut w, mut h) = (width, height);

    if w / h > TUBE_ASPECT {
        w = h * TUBE_ASPECT;
    } else {
        h = w / TUBE_ASPECT;
    }

    w /= 1.0 + margin;
    h /= 1.0 + margin;
    [((width - w) * 0.5).floor(), ((height - h) * 0.5).floor(), w, h]
}

/// The number of mip levels of a full chain for the canvas: 9.
const MIPS: usize = (32 - (CANVAS_WIDTH | CANVAS_HEIGHT).leading_zeros()) as usize;

// Descriptor tables of three views (t0, t1, t2), one per way a pass is fed.
const TABLE_DECODE: usize = 0; // + in-flight slot: [null, null, video]
const TABLE_PERSIST: usize = TABLE_DECODE + IN_FLIGHT; // + previous phosphor: [beam, phosphor, null]
const TABLE_FLAT: usize = TABLE_PERSIST + 2; // [beam, null, null]
const TABLE_CRT: usize = TABLE_FLAT + 1; // + phosphor: [phosphor, null, null]
const TABLE_MIP: usize = TABLE_CRT + 2; // + phosphor × (MIPS − 1) + level − 1: [the level above, null, null]
const TABLES: usize = TABLE_MIP + 2 * (MIPS - 1);

// The thumbnail's tables, after the window's.
const TABLE_THUMB_DECODE: usize = TABLES; // [null, null, video]
const TABLE_THUMB_PERSIST: usize = TABLE_THUMB_DECODE + 1; // [beam, null, null]: no afterglow
const TABLE_THUMB_FLAT: usize = TABLE_THUMB_PERSIST + 1; // [beam, null, null]
const TABLE_THUMB_CRT: usize = TABLE_THUMB_FLAT + 1; // [phosphor, null, null]
const TABLE_THUMB_MIP: usize = TABLE_THUMB_CRT + 1; // + level − 1: [the level above, null, null]
const ALL_TABLES: usize = TABLE_THUMB_MIP + MIPS - 1;

// Render target views.
const RTV_BEAM: usize = 0;
const RTV_PHOSPHOR: usize = 1; // + phosphor × MIPS + level
const RTV_BACKBUFFER: usize = RTV_PHOSPHOR + 2 * MIPS; // + index
const RTV_THUMB_BEAM: usize = RTV_BACKBUFFER + BUFFERS as usize;
const RTV_THUMB_PHOSPHOR: usize = RTV_THUMB_BEAM + 1; // + level
const RTV_THUMB_TARGET: usize = RTV_THUMB_PHOSPHOR + MIPS;
const RTVS: usize = RTV_THUMB_TARGET + 1;

/// A snapshot's thumbnail: big enough for Explorer's largest icons (§6.8).
pub const THUMBNAIL_WIDTH: u32 = 640;
pub const THUMBNAIL_HEIGHT: u32 = 480;

fn rtv_phosphor(index: usize, level: usize) -> usize {
    RTV_PHOSPHOR + index * MIPS + level
}

fn table_mip(index: usize, level: usize) -> usize {
    TABLE_MIP + index * (MIPS - 1) + level - 1
}

/// What one draw in flight owns until the GPU is done with it.
struct InFlight {
    allocator: ID3D12CommandAllocator,
    fence_value: u64,
    video: ID3D12Resource, // upload heap, mapped
    video_bytes: *mut u8,
}

/// The thumbnail's textures, made the first time one is drawn. They rest in
/// the render target state between thumbnails.
struct Thumbnail {
    beam: ID3D12Resource,
    phosphor: ID3D12Resource,
    target: ID3D12Resource,
    readback: ID3D12Resource,
    #[allow(dead_code)] // owns the memory video_bytes points into
    video: ID3D12Resource, // upload heap, mapped
    video_bytes: *mut u8,
}

pub struct Renderer {
    device: ID3D12Device,
    queue: ID3D12CommandQueue,
    list: ID3D12GraphicsCommandList,
    root_signature: ID3D12RootSignature,
    decode: ID3D12PipelineState,
    persist: ID3D12PipelineState,
    downsample: ID3D12PipelineState,
    crt: ID3D12PipelineState,
    flat: ID3D12PipelineState,
    swap_chain: IDXGISwapChain3,
    waitable: HANDLE,
    backbuffers: Vec<ID3D12Resource>,
    rtv_heap: ID3D12DescriptorHeap,
    rtv_size: usize,
    srv_heap: ID3D12DescriptorHeap,
    srv_size: usize,
    beam: ID3D12Resource,
    phosphor: [ID3D12Resource; 2],
    in_flight: Vec<InFlight>,
    draws: u64,
    fence: ID3D12Fence,
    fence_value: u64,
    fence_event: HANDLE,
    width: u32,
    height: u32,
    thumbnail: Option<Thumbnail>,
}

fn transition(resource: &ID3D12Resource, before: D3D12_RESOURCE_STATES, after: D3D12_RESOURCE_STATES) -> D3D12_RESOURCE_BARRIER {
    transition_subresource(resource, D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES, before, after)
}

/// A transition of one subresource (here a mip level).
fn transition_subresource(
    resource: &ID3D12Resource,
    subresource: u32,
    before: D3D12_RESOURCE_STATES,
    after: D3D12_RESOURCE_STATES,
) -> D3D12_RESOURCE_BARRIER {
    D3D12_RESOURCE_BARRIER {
        Type: D3D12_RESOURCE_BARRIER_TYPE_TRANSITION,
        Flags: D3D12_RESOURCE_BARRIER_FLAG_NONE,
        Anonymous: D3D12_RESOURCE_BARRIER_0 {
            Transition: ManuallyDrop::new(D3D12_RESOURCE_TRANSITION_BARRIER {
                // Borrowed for the call: no reference is taken or released.
                pResource: unsafe { std::mem::transmute_copy(resource) },
                Subresource: subresource,
                StateBefore: before,
                StateAfter: after,
            }),
        },
    }
}

fn bytecode(code: &[u8]) -> D3D12_SHADER_BYTECODE {
    D3D12_SHADER_BYTECODE { pShaderBytecode: code.as_ptr().cast(), BytecodeLength: code.len() }
}

fn heap(kind: D3D12_HEAP_TYPE) -> D3D12_HEAP_PROPERTIES {
    D3D12_HEAP_PROPERTIES { Type: kind, ..Default::default() }
}

fn create_root_signature(device: &ID3D12Device) -> Result<ID3D12RootSignature> {
    let range = D3D12_DESCRIPTOR_RANGE {
        RangeType: D3D12_DESCRIPTOR_RANGE_TYPE_SRV,
        NumDescriptors: 3,
        BaseShaderRegister: 0,
        RegisterSpace: 0,
        OffsetInDescriptorsFromTableStart: 0,
    };
    let parameters = [
        D3D12_ROOT_PARAMETER {
            ParameterType: D3D12_ROOT_PARAMETER_TYPE_32BIT_CONSTANTS,
            Anonymous: D3D12_ROOT_PARAMETER_0 {
                Constants: D3D12_ROOT_CONSTANTS { ShaderRegister: 0, RegisterSpace: 0, Num32BitValues: UNIFORM_WORDS },
            },
            ShaderVisibility: D3D12_SHADER_VISIBILITY_PIXEL,
        },
        D3D12_ROOT_PARAMETER {
            ParameterType: D3D12_ROOT_PARAMETER_TYPE_DESCRIPTOR_TABLE,
            Anonymous: D3D12_ROOT_PARAMETER_0 {
                DescriptorTable: D3D12_ROOT_DESCRIPTOR_TABLE { NumDescriptorRanges: 1, pDescriptorRanges: &range },
            },
            ShaderVisibility: D3D12_SHADER_VISIBILITY_PIXEL,
        },
    ];
    // s0: bilinear, level 0 only (Metal's linearClamp has no mip filter); s1:
    // trilinear. Both read black outside the texture (clamp_to_zero).
    let sampler = |register, filter, max_lod| D3D12_STATIC_SAMPLER_DESC {
        Filter: filter,
        AddressU: D3D12_TEXTURE_ADDRESS_MODE_BORDER,
        AddressV: D3D12_TEXTURE_ADDRESS_MODE_BORDER,
        AddressW: D3D12_TEXTURE_ADDRESS_MODE_BORDER,
        MipLODBias: 0.0,
        MaxAnisotropy: 0,
        ComparisonFunc: D3D12_COMPARISON_FUNC_NEVER,
        BorderColor: D3D12_STATIC_BORDER_COLOR_TRANSPARENT_BLACK,
        MinLOD: 0.0,
        MaxLOD: max_lod,
        ShaderRegister: register,
        RegisterSpace: 0,
        ShaderVisibility: D3D12_SHADER_VISIBILITY_PIXEL,
    };
    let samplers = [
        sampler(0, D3D12_FILTER_MIN_MAG_LINEAR_MIP_POINT, 0.0),
        sampler(1, D3D12_FILTER_MIN_MAG_MIP_LINEAR, D3D12_FLOAT32_MAX),
    ];
    let desc = D3D12_ROOT_SIGNATURE_DESC {
        NumParameters: parameters.len() as u32,
        pParameters: parameters.as_ptr(),
        NumStaticSamplers: samplers.len() as u32,
        pStaticSamplers: samplers.as_ptr(),
        Flags: D3D12_ROOT_SIGNATURE_FLAG_NONE,
    };

    let mut blob = None;
    let mut errors = None;
    let serialized = unsafe { D3D12SerializeRootSignature(&desc, D3D_ROOT_SIGNATURE_VERSION_1, &mut blob, Some(&mut errors)) };

    if let Err(e) = serialized {
        if let Some(errors) = errors {
            let text = unsafe { std::slice::from_raw_parts(errors.GetBufferPointer() as *const u8, errors.GetBufferSize()) };
            log!("root signature: {}", String::from_utf8_lossy(text));
        }

        return Err(e);
    }

    let blob = blob.unwrap();
    let bytes = unsafe { std::slice::from_raw_parts(blob.GetBufferPointer() as *const u8, blob.GetBufferSize()) };
    unsafe { device.CreateRootSignature(0, bytes) }
}

fn create_pipeline(device: &ID3D12Device, root: &ID3D12RootSignature, ps: &[u8], format: DXGI_FORMAT) -> Result<ID3D12PipelineState> {
    let mut blend = D3D12_BLEND_DESC::default();
    blend.RenderTarget[0].RenderTargetWriteMask = D3D12_COLOR_WRITE_ENABLE_ALL.0 as u8;
    let mut formats = [DXGI_FORMAT_UNKNOWN; 8];
    formats[0] = format;

    let desc = D3D12_GRAPHICS_PIPELINE_STATE_DESC {
        pRootSignature: unsafe { std::mem::transmute_copy(root) },
        VS: bytecode(VS_FULLSCREEN),
        PS: bytecode(ps),
        BlendState: blend,
        SampleMask: u32::MAX,
        RasterizerState: D3D12_RASTERIZER_DESC {
            FillMode: D3D12_FILL_MODE_SOLID,
            CullMode: D3D12_CULL_MODE_NONE,
            DepthClipEnable: true.into(),
            ..Default::default()
        },
        PrimitiveTopologyType: D3D12_PRIMITIVE_TOPOLOGY_TYPE_TRIANGLE,
        NumRenderTargets: 1,
        RTVFormats: formats,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        ..Default::default()
    };
    unsafe { device.CreateGraphicsPipelineState(&desc) }
}

fn create_canvas(device: &ID3D12Device, format: DXGI_FORMAT, mips: u16) -> Result<ID3D12Resource> {
    create_texture(device, format, CANVAS_WIDTH, CANVAS_HEIGHT, mips)
}

fn create_texture(device: &ID3D12Device, format: DXGI_FORMAT, width: u32, height: u32, mips: u16) -> Result<ID3D12Resource> {
    let desc = D3D12_RESOURCE_DESC {
        Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
        Width: width as u64,
        Height: height,
        DepthOrArraySize: 1,
        MipLevels: mips,
        Format: format,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Flags: D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET,
        ..Default::default()
    };
    let clear = D3D12_CLEAR_VALUE { Format: format, Anonymous: D3D12_CLEAR_VALUE_0 { Color: [0.0; 4] } };
    let mut resource = None;
    unsafe {
        device.CreateCommittedResource(
            &heap(D3D12_HEAP_TYPE_DEFAULT),
            D3D12_HEAP_FLAG_NONE,
            &desc,
            D3D12_RESOURCE_STATE_RENDER_TARGET,
            Some(&clear),
            &mut resource,
        )?
    };
    Ok(resource.unwrap())
}

fn create_upload_buffer(device: &ID3D12Device, size: u64) -> Result<ID3D12Resource> {
    create_buffer(device, D3D12_HEAP_TYPE_UPLOAD, D3D12_RESOURCE_STATE_GENERIC_READ, size)
}

fn create_buffer(device: &ID3D12Device, kind: D3D12_HEAP_TYPE, state: D3D12_RESOURCE_STATES, size: u64) -> Result<ID3D12Resource> {
    let desc = D3D12_RESOURCE_DESC {
        Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
        Width: size,
        Height: 1,
        DepthOrArraySize: 1,
        MipLevels: 1,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
        ..Default::default()
    };
    let mut resource = None;
    unsafe {
        device.CreateCommittedResource(&heap(kind), D3D12_HEAP_FLAG_NONE, &desc, state, None, &mut resource)?
    };
    Ok(resource.unwrap())
}

impl Renderer {
    /// A renderer drawing into `hwnd`'s client area of `width` × `height`
    /// pixels.
    pub fn new(hwnd: HWND, width: u32, height: u32) -> Result<Renderer> {
        if cfg!(debug_assertions) {
            // The debug layer, if the Graphics Tools are installed.
            let mut debug: Option<ID3D12Debug> = None;

            if unsafe { D3D12GetDebugInterface(&mut debug) }.is_ok()
                && let Some(debug) = debug
            {
                unsafe { debug.EnableDebugLayer() };
                log!("Direct3D debug layer on");
            }
        }

        let factory: IDXGIFactory4 = unsafe { CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0))? };
        let mut device: Option<ID3D12Device> = None;
        unsafe { D3D12CreateDevice(None, D3D_FEATURE_LEVEL_11_0, &mut device)? };
        let device = device.unwrap();

        let queue: ID3D12CommandQueue = unsafe {
            device.CreateCommandQueue(&D3D12_COMMAND_QUEUE_DESC { Type: D3D12_COMMAND_LIST_TYPE_DIRECT, ..Default::default() })?
        };

        let swap_chain_desc = DXGI_SWAP_CHAIN_DESC1 {
            Width: width.max(1),
            Height: height.max(1),
            Format: BACKBUFFER_FORMAT,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: BUFFERS,
            Scaling: DXGI_SCALING_STRETCH,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
            AlphaMode: DXGI_ALPHA_MODE_IGNORE,
            Flags: SWAP_CHAIN_FLAGS.0 as u32,
            ..Default::default()
        };
        let swap_chain: IDXGISwapChain3 = unsafe { factory.CreateSwapChainForHwnd(&queue, hwnd, &swap_chain_desc, None, None)? }.cast()?;
        // Alt+Enter is the app's own full screen (§11.3).
        unsafe { factory.MakeWindowAssociation(hwnd, DXGI_MWA_NO_ALT_ENTER)? };
        unsafe { swap_chain.SetMaximumFrameLatency(1)? };
        let waitable = unsafe { swap_chain.GetFrameLatencyWaitableObject() };

        let rtv_heap: ID3D12DescriptorHeap = unsafe {
            device.CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
                Type: D3D12_DESCRIPTOR_HEAP_TYPE_RTV,
                NumDescriptors: RTVS as u32,
                ..Default::default()
            })?
        };
        let srv_heap: ID3D12DescriptorHeap = unsafe {
            device.CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
                Type: D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
                NumDescriptors: (ALL_TABLES * 3) as u32,
                Flags: D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE,
                ..Default::default()
            })?
        };
        let rtv_size = unsafe { device.GetDescriptorHandleIncrementSize(D3D12_DESCRIPTOR_HEAP_TYPE_RTV) } as usize;
        let srv_size = unsafe { device.GetDescriptorHandleIncrementSize(D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV) } as usize;

        let root_signature = create_root_signature(&device)?;
        let decode = create_pipeline(&device, &root_signature, PS_DECODE, DXGI_FORMAT_R16_FLOAT)?;
        let persist = create_pipeline(&device, &root_signature, PS_PERSIST, DXGI_FORMAT_R16G16_FLOAT)?;
        let downsample = create_pipeline(&device, &root_signature, PS_DOWNSAMPLE, DXGI_FORMAT_R16G16_FLOAT)?;
        let crt = create_pipeline(&device, &root_signature, PS_CRT, DXGI_FORMAT_B8G8R8A8_UNORM_SRGB)?;
        let flat = create_pipeline(&device, &root_signature, PS_FLAT, DXGI_FORMAT_B8G8R8A8_UNORM_SRGB)?;

        // The phosphor textures have mipmaps: the 3D bezel's wall is lit by
        // wide averages of the face (Shaders.hlsl, faceLight).
        let beam = create_canvas(&device, DXGI_FORMAT_R16_FLOAT, 1)?;
        let phosphor = [
            create_canvas(&device, DXGI_FORMAT_R16G16_FLOAT, MIPS as u16)?,
            create_canvas(&device, DXGI_FORMAT_R16G16_FLOAT, MIPS as u16)?,
        ];

        let mut in_flight = Vec::new();

        for _ in 0..IN_FLIGHT {
            let allocator: ID3D12CommandAllocator = unsafe { device.CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT)? };
            let video = create_upload_buffer(&device, RASTER_SIZE as u64)?;
            let mut bytes = std::ptr::null_mut();
            unsafe { video.Map(0, Some(&D3D12_RANGE { Begin: 0, End: 0 }), Some(&mut bytes))? };
            in_flight.push(InFlight { allocator, fence_value: 0, video, video_bytes: bytes.cast() });
        }

        let list: ID3D12GraphicsCommandList =
            unsafe { device.CreateCommandList(0, D3D12_COMMAND_LIST_TYPE_DIRECT, &in_flight[0].allocator, None)? };
        let fence: ID3D12Fence = unsafe { device.CreateFence(0, D3D12_FENCE_FLAG_NONE)? };
        let fence_event = unsafe { CreateEventW(None, false, false, None)? };

        let mut renderer = Renderer {
            device,
            queue,
            list,
            root_signature,
            decode,
            persist,
            downsample,
            crt,
            flat,
            swap_chain,
            waitable,
            backbuffers: Vec::new(),
            rtv_heap,
            rtv_size,
            srv_heap,
            srv_size,
            beam,
            phosphor,
            in_flight,
            draws: 0,
            fence,
            fence_value: 0,
            fence_event,
            width: width.max(1),
            height: height.max(1),
            thumbnail: None,
        };

        renderer.create_views();
        renderer.create_backbuffer_views()?;

        // Start the phosphor dark; the canvases go to their resting state.
        unsafe {
            let list = &renderer.list;
            let black = [0.0f32; 4];
            for index in 0..2 {
                for level in 0..MIPS {
                    list.ClearRenderTargetView(renderer.rtv(rtv_phosphor(index, level)), &black, None);
                }
            }

            list.ResourceBarrier(&[
                transition(&renderer.beam, D3D12_RESOURCE_STATE_RENDER_TARGET, D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE),
                transition(&renderer.phosphor[0], D3D12_RESOURCE_STATE_RENDER_TARGET, D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE),
                transition(&renderer.phosphor[1], D3D12_RESOURCE_STATE_RENDER_TARGET, D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE),
            ]);
            list.Close()?;
            renderer.queue.ExecuteCommandLists(&[Some(list.cast()?)]);
        }

        renderer.wait_idle();
        renderer.log_debug_messages();
        Ok(renderer)
    }

    fn rtv(&self, index: usize) -> D3D12_CPU_DESCRIPTOR_HANDLE {
        let start = unsafe { self.rtv_heap.GetCPUDescriptorHandleForHeapStart() };
        D3D12_CPU_DESCRIPTOR_HANDLE { ptr: start.ptr + index * self.rtv_size }
    }

    fn srv_cpu(&self, index: usize) -> D3D12_CPU_DESCRIPTOR_HANDLE {
        let start = unsafe { self.srv_heap.GetCPUDescriptorHandleForHeapStart() };
        D3D12_CPU_DESCRIPTOR_HANDLE { ptr: start.ptr + index * self.srv_size }
    }

    fn table(&self, table: usize) -> D3D12_GPU_DESCRIPTOR_HANDLE {
        let start = unsafe { self.srv_heap.GetGPUDescriptorHandleForHeapStart() };
        D3D12_GPU_DESCRIPTOR_HANDLE { ptr: start.ptr + (table * 3 * self.srv_size) as u64 }
    }

    /// The canvases' render target views and every descriptor table.
    fn create_views(&self) {
        let levels = |format, first: u32, mips: u32| D3D12_SHADER_RESOURCE_VIEW_DESC {
            Format: format,
            ViewDimension: D3D12_SRV_DIMENSION_TEXTURE2D,
            Shader4ComponentMapping: D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
            Anonymous: D3D12_SHADER_RESOURCE_VIEW_DESC_0 {
                Texture2D: D3D12_TEX2D_SRV { MostDetailedMip: first, MipLevels: mips, ..Default::default() },
            },
        };
        let texture = |format, mips: u32| levels(format, 0, mips);
        let buffer = D3D12_SHADER_RESOURCE_VIEW_DESC {
            Format: DXGI_FORMAT_R8_UINT,
            ViewDimension: D3D12_SRV_DIMENSION_BUFFER,
            Shader4ComponentMapping: D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
            Anonymous: D3D12_SHADER_RESOURCE_VIEW_DESC_0 {
                Buffer: D3D12_BUFFER_SRV { FirstElement: 0, NumElements: RASTER_SIZE as u32, ..Default::default() },
            },
        };
        let null_texture = texture(DXGI_FORMAT_R16_FLOAT, 1);
        let beam = texture(DXGI_FORMAT_R16_FLOAT, 1);
        let phosphor = texture(DXGI_FORMAT_R16G16_FLOAT, u32::MAX);

        // [t0, t1, t2] of table `t`.
        let fill = |t: usize, views: [(Option<&ID3D12Resource>, &D3D12_SHADER_RESOURCE_VIEW_DESC); 3]| {
            for (i, (resource, desc)) in views.into_iter().enumerate() {
                unsafe { self.device.CreateShaderResourceView(resource, Some(desc), self.srv_cpu(t * 3 + i)) };
            }
        };

        for (slot, frame) in self.in_flight.iter().enumerate() {
            fill(TABLE_DECODE + slot, [(None, &null_texture), (None, &null_texture), (Some(&frame.video), &buffer)]);
        }

        for (i, previous) in self.phosphor.iter().enumerate() {
            fill(TABLE_PERSIST + i, [(Some(&self.beam), &beam), (Some(previous), &phosphor), (None, &buffer)]);
            fill(TABLE_CRT + i, [(Some(previous), &phosphor), (None, &null_texture), (None, &buffer)]);
        }

        fill(TABLE_FLAT, [(Some(&self.beam), &beam), (None, &null_texture), (None, &buffer)]);

        // Each mip level is drawn from the one above it.
        for (index, phosphor) in self.phosphor.iter().enumerate() {
            for level in 1..MIPS {
                let above = levels(DXGI_FORMAT_R16G16_FLOAT, level as u32 - 1, 1);
                fill(table_mip(index, level), [(Some(phosphor), &above), (None, &null_texture), (None, &buffer)]);
            }
        }

        unsafe {
            self.device.CreateRenderTargetView(&self.beam, None, self.rtv(RTV_BEAM));

            for (index, phosphor) in self.phosphor.iter().enumerate() {
                for level in 0..MIPS {
                    let desc = D3D12_RENDER_TARGET_VIEW_DESC {
                        Format: DXGI_FORMAT_R16G16_FLOAT,
                        ViewDimension: D3D12_RTV_DIMENSION_TEXTURE2D,
                        Anonymous: D3D12_RENDER_TARGET_VIEW_DESC_0 { Texture2D: D3D12_TEX2D_RTV { MipSlice: level as u32, PlaneSlice: 0 } },
                    };
                    self.device.CreateRenderTargetView(phosphor, Some(&desc), self.rtv(rtv_phosphor(index, level)));
                }
            }
        }
    }

    /// The thumbnail's textures and their views.
    fn create_thumbnail(&self) -> Result<Thumbnail> {
        let device = &self.device;
        let beam = create_canvas(device, DXGI_FORMAT_R16_FLOAT, 1)?;
        let phosphor = create_canvas(device, DXGI_FORMAT_R16G16_FLOAT, MIPS as u16)?;
        let target = create_texture(device, DXGI_FORMAT_B8G8R8A8_UNORM_SRGB, THUMBNAIL_WIDTH, THUMBNAIL_HEIGHT, 1)?;
        let size = (THUMBNAIL_WIDTH * 4 * THUMBNAIL_HEIGHT) as u64;
        let readback = create_buffer(device, D3D12_HEAP_TYPE_READBACK, D3D12_RESOURCE_STATE_COPY_DEST, size)?;
        let video = create_upload_buffer(device, RASTER_SIZE as u64)?;
        let mut bytes = std::ptr::null_mut();
        unsafe { video.Map(0, Some(&D3D12_RANGE { Begin: 0, End: 0 }), Some(&mut bytes))? };

        let null_texture = shader_view(DXGI_FORMAT_R16_FLOAT, 0, 1);
        let buffer = D3D12_SHADER_RESOURCE_VIEW_DESC {
            Format: DXGI_FORMAT_R8_UINT,
            ViewDimension: D3D12_SRV_DIMENSION_BUFFER,
            Shader4ComponentMapping: D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
            Anonymous: D3D12_SHADER_RESOURCE_VIEW_DESC_0 {
                Buffer: D3D12_BUFFER_SRV { FirstElement: 0, NumElements: RASTER_SIZE as u32, ..Default::default() },
            },
        };
        let beam_view = shader_view(DXGI_FORMAT_R16_FLOAT, 0, 1);
        let fill = |t: usize, views: [(Option<&ID3D12Resource>, &D3D12_SHADER_RESOURCE_VIEW_DESC); 3]| {
            for (i, (resource, desc)) in views.into_iter().enumerate() {
                unsafe { device.CreateShaderResourceView(resource, Some(desc), self.srv_cpu(t * 3 + i)) };
            }
        };

        fill(TABLE_THUMB_DECODE, [(None, &null_texture), (None, &null_texture), (Some(&video), &buffer)]);
        // With no picture before it, the phosphor holds only the beam.
        let no_phosphor = shader_view(DXGI_FORMAT_R16G16_FLOAT, 0, 1);
        fill(TABLE_THUMB_PERSIST, [(Some(&beam), &beam_view), (None, &no_phosphor), (None, &buffer)]);
        fill(TABLE_THUMB_FLAT, [(Some(&beam), &beam_view), (None, &null_texture), (None, &buffer)]);
        fill(TABLE_THUMB_CRT, [(Some(&phosphor), &shader_view(DXGI_FORMAT_R16G16_FLOAT, 0, u32::MAX)), (None, &null_texture), (None, &buffer)]);

        for level in 1..MIPS {
            let above = shader_view(DXGI_FORMAT_R16G16_FLOAT, level as u32 - 1, 1);
            fill(TABLE_THUMB_MIP + level - 1, [(Some(&phosphor), &above), (None, &null_texture), (None, &buffer)]);
        }

        unsafe {
            device.CreateRenderTargetView(&beam, None, self.rtv(RTV_THUMB_BEAM));

            for level in 0..MIPS {
                let desc = D3D12_RENDER_TARGET_VIEW_DESC {
                    Format: DXGI_FORMAT_R16G16_FLOAT,
                    ViewDimension: D3D12_RTV_DIMENSION_TEXTURE2D,
                    Anonymous: D3D12_RENDER_TARGET_VIEW_DESC_0 { Texture2D: D3D12_TEX2D_RTV { MipSlice: level as u32, PlaneSlice: 0 } },
                };
                device.CreateRenderTargetView(&phosphor, Some(&desc), self.rtv(RTV_THUMB_PHOSPHOR + level));
            }

            device.CreateRenderTargetView(&target, None, self.rtv(RTV_THUMB_TARGET));
        }

        Ok(Thumbnail { beam, phosphor, target, readback, video, video_bytes: bytes.cast() })
    }

    /// The screen as the window shows it, at THUMBNAIL_WIDTH ×
    /// THUMBNAIL_HEIGHT with no afterglow, as BGRA rows of sRGB pixels
    /// (Renderer.m, pngForFrame). None if it could not be drawn.
    pub fn thumbnail(&mut self, frame: &Frame, settings: &CrtSettings) -> Option<Vec<u8>> {
        let pixels = self.draw_thumbnail(frame, settings);
        self.log_debug_messages();

        match pixels {
            Ok(pixels) => Some(pixels),
            Err(e) => {
                log!("drawing the thumbnail: {e}");
                None
            }
        }
    }

    fn draw_thumbnail(&mut self, frame: &Frame, settings: &CrtSettings) -> Result<Vec<u8>> {
        if self.thumbnail.is_none() {
            self.thumbnail = Some(self.create_thumbnail()?);
        }

        // Every draw in flight is done, so any allocator can be used.
        self.wait_idle();
        let t = self.thumbnail.as_ref().unwrap();
        unsafe { std::ptr::copy_nonoverlapping(frame.data.as_ptr(), t.video_bytes, RASTER_SIZE) };

        // A long time since the last picture: no afterglow from an old one.
        let (width, height) = (THUMBNAIL_WIDTH, THUMBNAIL_HEIGHT);
        let uniforms = Self::uniforms(width, height, 10.0, settings, false);
        let list = &self.list;
        let allocator = &self.in_flight[0].allocator;
        let row_pitch = width * 4;

        unsafe {
            allocator.Reset()?;
            list.Reset(allocator, None)?;
            list.SetGraphicsRootSignature(&self.root_signature);
            list.SetDescriptorHeaps(&[Some(self.srv_heap.clone())]);
            list.SetGraphicsRoot32BitConstants(0, UNIFORM_WORDS, (&raw const uniforms).cast(), 0);
            list.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);

            self.pass(RTV_THUMB_BEAM, CANVAS_WIDTH, CANVAS_HEIGHT, &self.decode, TABLE_THUMB_DECODE);
            list.ResourceBarrier(&[transition(&t.beam, D3D12_RESOURCE_STATE_RENDER_TARGET, D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE)]);
            self.pass(RTV_THUMB_PHOSPHOR, CANVAS_WIDTH, CANVAS_HEIGHT, &self.persist, TABLE_THUMB_PERSIST);

            if settings.crt_effects && settings.bezel3d {
                for level in 1..MIPS {
                    list.ResourceBarrier(&[transition_subresource(
                        &t.phosphor,
                        (level - 1) as u32,
                        D3D12_RESOURCE_STATE_RENDER_TARGET,
                        D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE,
                    )]);
                    let (w, h) = ((CANVAS_WIDTH >> level).max(1), (CANVAS_HEIGHT >> level).max(1));
                    self.pass(RTV_THUMB_PHOSPHOR + level, w, h, &self.downsample, TABLE_THUMB_MIP + level - 1);
                }

                list.ResourceBarrier(&[transition_subresource(
                    &t.phosphor,
                    (MIPS - 1) as u32,
                    D3D12_RESOURCE_STATE_RENDER_TARGET,
                    D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE,
                )]);
            } else {
                list.ResourceBarrier(&[transition(&t.phosphor, D3D12_RESOURCE_STATE_RENDER_TARGET, D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE)]);
            }

            if settings.crt_effects {
                self.pass(RTV_THUMB_TARGET, width, height, &self.crt, TABLE_THUMB_CRT);
            } else {
                self.pass(RTV_THUMB_TARGET, width, height, &self.flat, TABLE_THUMB_FLAT);
            }

            list.ResourceBarrier(&[transition(&t.target, D3D12_RESOURCE_STATE_RENDER_TARGET, D3D12_RESOURCE_STATE_COPY_SOURCE)]);
            let destination = D3D12_TEXTURE_COPY_LOCATION {
                pResource: std::mem::transmute_copy(&t.readback),
                Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
                Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
                    PlacedFootprint: D3D12_PLACED_SUBRESOURCE_FOOTPRINT {
                        Offset: 0,
                        Footprint: D3D12_SUBRESOURCE_FOOTPRINT {
                            Format: DXGI_FORMAT_B8G8R8A8_UNORM_SRGB,
                            Width: width,
                            Height: height,
                            Depth: 1,
                            RowPitch: row_pitch,
                        },
                    },
                },
            };
            let source = D3D12_TEXTURE_COPY_LOCATION {
                pResource: std::mem::transmute_copy(&t.target),
                Type: D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
                Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 { SubresourceIndex: 0 },
            };
            list.CopyTextureRegion(&destination, 0, 0, 0, &source, None);

            // Back to rest.
            list.ResourceBarrier(&[
                transition(&t.target, D3D12_RESOURCE_STATE_COPY_SOURCE, D3D12_RESOURCE_STATE_RENDER_TARGET),
                transition(&t.beam, D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE, D3D12_RESOURCE_STATE_RENDER_TARGET),
                transition(&t.phosphor, D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE, D3D12_RESOURCE_STATE_RENDER_TARGET),
            ]);
            list.Close()?;
            self.queue.ExecuteCommandLists(&[Some(list.cast()?)]);
        }

        self.wait_idle();
        let t = self.thumbnail.as_ref().unwrap();
        let size = (row_pitch * height) as usize;
        let mut mapped = std::ptr::null_mut();

        unsafe {
            t.readback.Map(0, Some(&D3D12_RANGE { Begin: 0, End: size }), Some(&mut mapped))?;
            let pixels = std::slice::from_raw_parts(mapped as *const u8, size).to_vec();
            t.readback.Unmap(0, Some(&D3D12_RANGE { Begin: 0, End: 0 }));
            Ok(pixels)
        }
    }

    fn create_backbuffer_views(&mut self) -> Result<()> {
        let desc = D3D12_RENDER_TARGET_VIEW_DESC {
            Format: DXGI_FORMAT_B8G8R8A8_UNORM_SRGB,
            ViewDimension: D3D12_RTV_DIMENSION_TEXTURE2D,
            ..Default::default()
        };
        self.backbuffers.clear();

        for i in 0..BUFFERS {
            let buffer: ID3D12Resource = unsafe { self.swap_chain.GetBuffer(i)? };
            unsafe { self.device.CreateRenderTargetView(&buffer, Some(&desc), self.rtv(RTV_BACKBUFFER + i as usize)) };
            self.backbuffers.push(buffer);
        }

        Ok(())
    }

    /// Waits until the GPU has finished all work given to it.
    fn wait_idle(&mut self) {
        self.fence_value += 1;

        unsafe {
            if self.queue.Signal(&self.fence, self.fence_value).is_ok()
                && self.fence.GetCompletedValue() < self.fence_value
                && self.fence.SetEventOnCompletion(self.fence_value, self.fence_event).is_ok()
            {
                WaitForSingleObjectEx(self.fence_event, INFINITE, false);
            }
        }
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Debug builds: logs what the Direct3D debug layer has to say, if it is
    /// installed (Windows' optional Graphics Tools).
    fn log_debug_messages(&self) {
        if !cfg!(debug_assertions) {
            return;
        }

        let Ok(queue) = self.device.cast::<ID3D12InfoQueue>() else {
            return;
        };

        unsafe {
            for i in 0..queue.GetNumStoredMessages() {
                let mut len = 0;
                let _ = queue.GetMessage(i, None, &mut len);
                let mut buffer = vec![0u64; len.div_ceil(8)];
                let message = buffer.as_mut_ptr().cast::<D3D12_MESSAGE>();

                if queue.GetMessage(i, Some(message), &mut len).is_ok() {
                    let text = std::slice::from_raw_parts((*message).pDescription, (*message).DescriptionByteLength);
                    log!("Direct3D: {}", String::from_utf8_lossy(text).trim_end_matches('\0'));
                }
            }

            queue.ClearStoredMessages();
        }
    }

    /// Resizes the swap chain to a client area of `width` × `height` pixels.
    pub fn resize(&mut self, width: u32, height: u32) {
        let (width, height) = (width.max(1), height.max(1));

        if (width, height) == (self.width, self.height) {
            return;
        }

        self.wait_idle();
        self.backbuffers.clear();
        let resized = unsafe { self.swap_chain.ResizeBuffers(BUFFERS, width, height, DXGI_FORMAT_UNKNOWN, SWAP_CHAIN_FLAGS) };

        if let Err(e) = resized.and_then(|()| self.create_backbuffer_views()) {
            log!("resizing the swap chain: {e}");
            return;
        }

        self.width = width;
        self.height = height;
    }

    fn uniforms(width: u32, height: u32, elapsed: f64, settings: &CrtSettings, full_screen: bool) -> CrtUniforms {
        // Long enough and both decays are zero: nothing is left of the old picture.
        let dt = elapsed.clamp(0.0, 10.0) as f32;
        let unit = |v: f32| v.clamp(0.0, 1.0);
        let background = BACKGROUND_MAX * unit(settings.background);
        let frame_level = FRAME_MAX * unit(settings.frame);
        let [r, g, b] = PHOSPHOR_RGB.map(|c| srgb_to_linear(c as f32 / 255.0));

        CrtUniforms {
            view: tube_rect(width as f32, height as f32, settings.tube_margin(full_screen)),
            // The core's raster starts at the start of the sync pulses.
            active: [
                -VISIBLE_START_US * 8.0,
                -VISIBLE_START_LINES,
                RASTER_BYTES as f32 * 8.0,
                RASTER_LINES as f32,
            ],
            phosphor: [r, g, b, settings.brightness],
            background: [BACKGROUND_COLOUR[0] * background, BACKGROUND_COLOUR[1] * background, BACKGROUND_COLOUR[2] * background, 0.0],
            frame: [FRAME_COLOUR[0] * frame_level, FRAME_COLOUR[1] * frame_level, FRAME_COLOUR[2] * frame_level, 0.0],
            canvas: [CANVAS_WIDTH as f32, CANVAS_HEIGHT as f32],
            output: [width as f32, height as f32],
            decay_fast: (-dt * 1000.0 / PHOSPHOR_FAST_MS).exp(),
            decay_slow: (-dt * 1000.0 / PHOSPHOR_SLOW_MS).exp(),
            afterglow: PHOSPHOR_AFTERGLOW,
            curvature: settings.curvature,
            scanline: SCANLINE_SOFT + (SCANLINE_SHARP - SCANLINE_SOFT) * unit(settings.scanlines),
            bloom: 0.15,
            glow: settings.glow,
            vignette: 0.25,
            bezel3d: settings.bezel3d as u32,
            pad: [0; 3],
        }
    }

    /// One full-screen pass into `rtv` of `width` × `height`, fed by `table`.
    fn pass(&self, rtv: usize, width: u32, height: u32, pipeline: &ID3D12PipelineState, table: usize) {
        let viewport = D3D12_VIEWPORT { TopLeftX: 0.0, TopLeftY: 0.0, Width: width as f32, Height: height as f32, MinDepth: 0.0, MaxDepth: 1.0 };
        let scissor = RECT { left: 0, top: 0, right: width as i32, bottom: height as i32 };

        unsafe {
            self.list.SetPipelineState(pipeline);
            self.list.SetGraphicsRootDescriptorTable(1, self.table(table));
            self.list.RSSetViewports(&[viewport]);
            self.list.RSSetScissorRects(&[scissor]);
            self.list.OMSetRenderTargets(1, Some(&self.rtv(rtv)), false, None);
            self.list.DrawInstanced(3, 1, 0, 0);
        }
    }

    /// Draws `frame`. `elapsed` is the time in seconds the phosphor has had
    /// to decay since the picture held by the previous draw. Returns false,
    /// drawing nothing, if the swap chain cannot take a picture yet.
    pub fn draw(&mut self, frame: &Frame, elapsed: f64, settings: &CrtSettings, full_screen: bool) -> bool {
        // Never block the emulation: a draw that cannot be shown yet waits
        // for the next tick.
        if unsafe { WaitForSingleObjectEx(self.waitable, 0, false) } != windows::Win32::Foundation::WAIT_OBJECT_0 {
            return false;
        }

        let drawn = self.record(frame, elapsed, settings, full_screen);
        self.log_debug_messages();

        match drawn {
            Ok(()) => true,
            Err(e) => {
                log!("drawing: {e}");
                false
            }
        }
    }

    fn record(&mut self, frame: &Frame, elapsed: f64, settings: &CrtSettings, full_screen: bool) -> Result<()> {
        let slot = (self.draws % IN_FLIGHT as u64) as usize;
        let previous = (self.draws & 1) as usize;
        let current = previous ^ 1;

        // The slot's last draw must be done with its allocator and bytes.
        let done = self.in_flight[slot].fence_value;

        unsafe {
            if self.fence.GetCompletedValue() < done {
                self.fence.SetEventOnCompletion(done, self.fence_event)?;
                WaitForSingleObjectEx(self.fence_event, INFINITE, false);
            }
        }

        unsafe { std::ptr::copy_nonoverlapping(frame.data.as_ptr(), self.in_flight[slot].video_bytes, RASTER_SIZE) };

        let uniforms = Self::uniforms(self.width, self.height, elapsed, settings, full_screen);
        let backbuffer_index = unsafe { self.swap_chain.GetCurrentBackBufferIndex() } as usize;
        let backbuffer = &self.backbuffers[backbuffer_index];
        let list = &self.list;
        let to_target = |r| transition(r, D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE, D3D12_RESOURCE_STATE_RENDER_TARGET);
        let to_shader = |r| transition(r, D3D12_RESOURCE_STATE_RENDER_TARGET, D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE);

        unsafe {
            self.in_flight[slot].allocator.Reset()?;
            list.Reset(&self.in_flight[slot].allocator, None)?;
            list.SetGraphicsRootSignature(&self.root_signature);
            list.SetDescriptorHeaps(&[Some(self.srv_heap.clone())]);
            list.SetGraphicsRoot32BitConstants(0, UNIFORM_WORDS, (&raw const uniforms).cast(), 0);
            list.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);

            list.ResourceBarrier(&[to_target(&self.beam)]);
            self.pass(RTV_BEAM, CANVAS_WIDTH, CANVAS_HEIGHT, &self.decode, TABLE_DECODE + slot);
            list.ResourceBarrier(&[to_shader(&self.beam), to_target(&self.phosphor[current])]);
            self.pass(rtv_phosphor(current, 0), CANVAS_WIDTH, CANVAS_HEIGHT, &self.persist, TABLE_PERSIST + previous);

            if settings.crt_effects && settings.bezel3d {
                // The mipmaps, for the light on the bezel's wall: each level,
                // once drawn, becomes the source of the next.
                for level in 1..MIPS {
                    let above = (level - 1) as u32;
                    list.ResourceBarrier(&[transition_subresource(
                        &self.phosphor[current],
                        above,
                        D3D12_RESOURCE_STATE_RENDER_TARGET,
                        D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE,
                    )]);
                    let (w, h) = ((CANVAS_WIDTH >> level).max(1), (CANVAS_HEIGHT >> level).max(1));
                    self.pass(rtv_phosphor(current, level), w, h, &self.downsample, table_mip(current, level));
                }

                list.ResourceBarrier(&[transition_subresource(
                    &self.phosphor[current],
                    (MIPS - 1) as u32,
                    D3D12_RESOURCE_STATE_RENDER_TARGET,
                    D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE,
                )]);
            } else {
                list.ResourceBarrier(&[to_shader(&self.phosphor[current])]);
            }

            list.ResourceBarrier(&[transition(backbuffer, D3D12_RESOURCE_STATE_PRESENT, D3D12_RESOURCE_STATE_RENDER_TARGET)]);

            if settings.crt_effects {
                self.pass(RTV_BACKBUFFER + backbuffer_index, self.width, self.height, &self.crt, TABLE_CRT + current);
            } else {
                self.pass(RTV_BACKBUFFER + backbuffer_index, self.width, self.height, &self.flat, TABLE_FLAT);
            }

            list.ResourceBarrier(&[transition(backbuffer, D3D12_RESOURCE_STATE_RENDER_TARGET, D3D12_RESOURCE_STATE_PRESENT)]);
            list.Close()?;
            self.queue.ExecuteCommandLists(&[Some(list.cast()?)]);
            self.swap_chain.Present(1, DXGI_PRESENT(0)).ok()?;

            self.fence_value += 1;
            self.queue.Signal(&self.fence, self.fence_value)?;
        }

        self.in_flight[slot].fence_value = self.fence_value;
        self.draws += 1;
        Ok(())
    }
}

fn shader_view(format: DXGI_FORMAT, first: u32, mips: u32) -> D3D12_SHADER_RESOURCE_VIEW_DESC {
    D3D12_SHADER_RESOURCE_VIEW_DESC {
        Format: format,
        ViewDimension: D3D12_SRV_DIMENSION_TEXTURE2D,
        Shader4ComponentMapping: D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
        Anonymous: D3D12_SHADER_RESOURCE_VIEW_DESC_0 {
            Texture2D: D3D12_TEX2D_SRV { MostDetailedMip: first, MipLevels: mips, ..Default::default() },
        },
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        self.wait_idle();
    }
}
