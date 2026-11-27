// The passes of frontends/macos/src/Shaders.metal (spec §7.2), ported to HLSL
// with the same constants. build.rs compiles each entry point to shader model
// 5.1. Keep the two in step: a change to the picture goes into both.

// Mirrors CrtUniforms in renderer.rs (and CRTUniforms in ShaderTypes.h). It
// is passed as 36 root constants.
struct CRTUniforms {
    float4 view;        // tube area in drawable pixels: x, y, w, h
    float4 active;      // the core's raster inside the canvas: x, y, w, h
    float4 phosphor;    // linear phosphor colour, w = brightness
    float4 background;  // linear colour of the unlit face (room light on the glass)
    float4 frame;       // linear colour of the monitor's plastic: room light × its tone; w = its reflectance
    float2 canvas;      // raster canvas size: 8 texels per µs × lines
    float2 output;      // drawable size in pixels
    float decayFast;    // per-draw decay of the main phosphor component
    float decaySlow;    // per-draw decay of the faint afterglow
    float afterglow;    // afterglow strength
    float curvature;    // barrel distortion
    float scanline;     // beam sigma, in raster lines
    float bloom;        // beam widening with brightness
    float glow;         // halo strength
    float vignette;
    uint bezel3d;       // the frame's inner wall in perspective, else a flat recess
    uint monitorFront;  // the monitor's whole front around the glass, with its badge
    float room;         // the room light, linear, which lights the plastic, the badge and the room
    uint pad;
};

ConstantBuffer<CRTUniforms> u : register(b0);

// Every pass gets the same table of three views and uses the ones it needs;
// the others are null.
Texture2D<float4> tex0 : register(t0);
Texture2D<float4> tex1 : register(t1);
Buffer<uint> video : register(t2);  // the core's raster, 64 bytes × 313 lines

SamplerState linearClamp : register(s0);    // bilinear, black outside, level 0
SamplerState mipClamp : register(s1);       // trilinear, black outside

static const float kPi2 = 1.57079633;

// One triangle covering the whole target.
float4 vs_fullscreen(uint vid : SV_VertexID) : SV_Position
{
    float2 p = float2((vid << 1) & 2, vid & 2);
    return float4(p * 2.0 - 1.0, 0.0, 1.0);
}

// ---- Pass 1: decode ---------------------------------------------------------
// Renders the raster canvas (one texel per pixel clock × raster line): beam on
// (1) or off (0). Bit 7 of each byte is the leftmost pixel. Unlike the Metal
// version, which reads a texture, the bytes come straight from a buffer, at
// active.z / 8 bytes per line.
float ps_decode(float4 position : SV_Position) : SV_Target
{
    float2 c = floor(position.xy) - u.active.xy;

    if (any(c < 0.0) || any(c >= u.active.zw)) {
        return 0.0;
    }

    uint px = uint(c.x);
    uint bytesPerLine = uint(u.active.z) >> 3;
    uint b = video[uint(c.y) * bytesPerLine + (px >> 3)];
    return float((b >> (7 - (px & 7))) & 1);
}

// ---- Pass 2: phosphor persistence ------------------------------------------
// r: the main phosphor response, g: a slow faint afterglow. Each decays from
// its previous value and is re-excited by the beam. tex0 is the beam, tex1 the
// previous phosphor.
float2 ps_persist(float4 position : SV_Position) : SV_Target
{
    int3 p = int3(position.xy, 0);
    float s = tex0.Load(p).r;
    float2 old = tex1.Load(p).rg;
    return max(float2(s, s), old * float2(u.decayFast, u.decaySlow));
}

// ---- Pass 3: CRT composite -------------------------------------------------
// tex0 is the phosphor.

float phosphorAt(float2 texel, float afterglow)
{
    float2 p = tex0.SampleLevel(linearClamp, texel / u.canvas, 0).rg;
    return max(p.r, afterglow * p.g);
}

// One texel of the phosphor, read exactly; black outside the canvas.
float texelAt(int x, int y, float afterglow)
{
    // fxc takes an early return here for a use of an uninitialized value.
    float v = 0.0;

    if (x >= 0 && y >= 0 && x < int(u.canvas.x) && y < int(u.canvas.y)) {
        float2 p = tex0.Load(int3(x, y, 0)).rg;
        v = max(p.r, afterglow * p.g);
    }

    return v;
}

// The Gaussian CDF, approximated by a logistic. Like the real one it has
// F(-t) = 1 - F(t), so pixel boxes of an even field still add up to exactly 1.
float edgeCdf(float t)
{
    return 1.0 / (1.0 + exp(-1.702 * t));
}

// Beam intensity along one raster line at canvas position x. Each pixel lights
// a box one pixel wide, softened by the beam spot (a Gaussian of sigma
// texels). A bright pixel's box is also widened by `spread` texels on each
// side: the beam blooms, so white on black looks bolder and a dark stroke in
// reverse video thinner. The phosphor saturates at 1.
float lineIntensity(float x, float row, float afterglow, float sigma, float spread)
{
    int y = int(row);
    int x0 = int(floor(x));
    float sum = 0.0;

    [unroll]
    for (int i = -2; i <= 2; i++) {
        float I = texelAt(x0 + i, y, afterglow);

        if (I > 0.0) {
            float d = x - (float(x0 + i) + 0.5);
            float h = 0.5 + spread * I;
            sum += I * (edgeCdf((h - d) / sigma) + edgeCdf((h + d) / sigma) - 1.0);
        }
    }

    return min(sum, 1.0);
}

// ---- The bezel ---------------------------------------------------------------
// The МС6105's bezel, a copy of DEC's VR201's, stands about 9 mm in front of
// the edge of the glass, on a face about 18 cm tall (measured from photos,
// MD §7.4). Its inner wall leans out from the glass at `kWallSlope` from the
// line of sight, and turns into the front over a rounded lip of radius
// `kLipRadius`. Seen in perspective by a viewer in front of the middle of the
// tube, the wall shows as a band around the glass, widest at the sides.
// Lengths are in half-heights of the tube.
static const float kBezelDepth = 0.1;
static const float kWallSlope = 0.524;      // 30°
static const float kLipRadius = 0.035;
static const float kViewDistance = 4.5;     // from the glass, about 50 cm

// The unlit face fades to `kRim` of its level at the edge of the glass, in the
// shadow of the bezel, over the outer `kRimWidth`.
static const float kRim = 0.8;
static const float kRimWidth = 0.08;

// The diffuse reflectance of the wall, for the light from the face, as tuned
// for the dark plastic: a lighter one, the Frame setting's tone (u.frame.w),
// reflects more.
static const float kWallReflectance = 0.4;

// The phosphor lies behind the faceplate, about 1 cm of glass, which looks
// two thirds as deep: the wall's foot is that high above the light, and is
// not left dark next to the glass.
static const float kFaceplate = 0.07;

// The light the face sends to a point of the bezel at height h above the
// phosphor and `outside` outside the outline, over the point `foot` of the
// outline, whose outward normal is n. Both the lit phosphor and the unlit face
// (room light off the glass) count. It is for a surface square to the glass;
// the caller scales it for the surface's tilt. See Shaders.metal for the
// derivation of the five taps.
float3 faceLight(float2 foot, float2 n, float h, float outside, float2 extent)
{
    static const float taps[5] = { 0.484, 1.020, 1.732, 3.180, 9.950 };
    float2 along = float2(-n.y, n.x);
    float texels = 0.5 * u.canvas.y;    // canvas lines per half-height
    float3 sum = 0.0;

    [unroll]
    for (int i = 0; i < 5; i++) {
        float d = taps[i] * h;
        float a = max(d - outside, 0.0);    // into the glass
        float r = sqrt(d * d + h * h);
        float lod = log2(max(r * texels, 1.0));
        float I = 0.0;

        [unroll]
        for (int s = -1; s <= 1; s += 2) {
            float2 p = foot - n * a + along * (float(s) * 0.44 * r);
            float2 c = (p / extent * 0.5 + 0.5);
            float2 v = tex0.SampleLevel(mipClamp, c, lod).rg;
            I += 0.5 * max(v.r, u.afterglow * v.g);
        }

        float shade = lerp(kRim, 1.0, smoothstep(0.0, kRimWidth, a));
        sum += 0.1 * (u.phosphor.rgb * u.phosphor.w * min(I, 1.0) + u.background.rgb * shade);
    }

    return sum;
}

// The monitor outside the glass at tube position p (in half-heights), where
// `edge` is the signed distance to the glass outline and n its normal.
float3 bezel(float2 p, float edge, float2 n, float2 extent)
{
    // The bezel's cross-section, with x outward from the edge of the glass
    // and z towards the viewer: the wall rises from (0, 0) at kWallSlope to
    // the lip, a quarter-ish arc about (xc, depth − R) that meets the front
    // at z = depth. A point at height z looks D / (D − z) times as far from
    // the middle, so it shows at e ≈ x + k z outside the outline, with
    // k = (p · n) / D.
    const float R = kLipRadius;
    float sa = sin(kWallSlope);
    float ca = cos(kWallSlope);
    float wall = (kBezelDepth - R * (1.0 - sa)) / ca;   // length of the straight part
    float xc = wall * sa + R * ca;
    float k = max(dot(p, n), 0.1) / kViewDistance;
    float e = max(edge, 0.0);

    // Beyond the lip only the front shows. (An early return here makes fxc
    // report uninitialized values.)
    float3 result = u.frame.rgb;

    if (e < xc + k * kBezelDepth) {
        // phi: the surface normal's angle from the line of sight, leaning in.
        float x = 0.0, z = 0.0, phi = 0.0;

        if (e < wall * (sa + k * ca)) {
            float t = e / (sa + k * ca);
            x = t * sa;
            z = t * ca;
            phi = kPi2 - kWallSlope;
        } else {
            // On the arc, x = xc − R sin phi and z = depth − R + R cos phi, so
            // −sin phi + k cos phi = (e − xc − k (depth − R)) / R.
            float m = sqrt(1.0 + k * k);
            float A = (e - xc - k * (kBezelDepth - R)) / R;
            phi = clamp(acos(clamp(A / m, -1.0, 1.0)) - atan2(1.0, k), 0.0, kPi2 - kWallSlope);
            x = xc - R * sin(phi);
            z = kBezelDepth - R + R * cos(phi);
        }

        // A surface tilted by phi sees the face over 1 − cos phi of the share a
        // square wall gets, and the room in front over (1 + cos phi) / 2. Room
        // light comes more from above: the bottom wall faces up and gets more,
        // the top one less. It is shaded towards the glass. Framebuffer y points
        // down, so the bottom wall has n.y = 1.
        float3 face = faceLight(p - n * edge, n, z + kFaceplate, x, extent);
        float room = 0.5 * (1.0 + cos(phi)) * (1.0 + 0.5 * sin(phi) * n.y) * (0.5 + 0.5 * z / kBezelDepth);
        result = max(kWallReflectance, u.frame.w) * (1.0 - cos(phi)) * face + room * u.frame.rgb;
    }

    return result;
}

// The flat bezel, with the 3D bezel off: a recess where it meets the glass,
// lit by both the glass next to it and the room light on the frame.
float3 flatBezel(float edge)
{
    float3 recess = 0.25 * (u.background.rgb * kRim + u.frame.rgb);
    return lerp(recess, u.frame.rgb, 1.0 - exp(-max(edge, 0.0) / 0.02));
}

// ---- The whole monitor -------------------------------------------------------
// With Whole Monitor on, the window shows the МС6105's front all round, in a
// dim room: the frame around the glass, its outer edge rounding back into
// the case, and the Электроника badge at the bottom left. The sizes are
// measured from photos (MD §7.4), in half-heights of the tube from the edge
// of the glass, as the bezel's are. renderer.rs's FRONT_MARGINS leave room
// for them; keep the two in step.
static const float kFrontSide = 0.30;
static const float kFrontTop = 0.235;
static const float kFrontBottom = 0.44;
static const float kFrontCorner = 0.08;     // the outline's corner radius
static const float kFrontRim = 0.035;       // the rounded outer edge, seen from the front

// What the room around the monitor, the badge's metal plate, its black paint
// and the shine on the plastic's top edge give back of the room light
// (u.room, the Room Light setting). They do not depend on the plastic's
// tone, the Frame setting: only the plastic itself, u.frame, does.
static const float kRoom = 0.35;
static const float kPlate = 0.75;
static const float kPaint = 0.03;
static const float kShine = 0.045;

// The badge, after close-up photos of it: its top left corner from the
// glass's bottom left corner, and its size. At its left end the emblem: a
// black block, and in it a light frame round a spire on a foot between a
// small Э and С; its frame's width is the emblem's unit. Then the lettering:
// where it starts, its cap height, and half its stroke, in cap heights.
static const float2 kBadgeAt = float2(0.086, 0.133);
static const float2 kBadgeSize = float2(0.69, 0.134);
static const float kBadgeEmblem = 0.108;
static const float2 kMarkAt = float2(0.026, 0.009);     // the frame's outer corner in the block
static const float kMarkUnit = 0.055;                    // the frame's width
static const float kMarkHeight = 2.04;                   // and its height, in units
static const float kTextAt = 0.148;
static const float kCapHeight = 0.056;
static const float kStroke = 0.09;

// "электроника" and the emblem's Э and С as strokes, x0, y0, x1, y1: the
// word in cap heights, x from its start and y up from the baseline, the
// letters in the emblem's units, x from the frame's left edge and y down.
// badge strokes: made by tools/badge_glyphs.py, not edited by hand
// The word, 8.96 cap heights wide.
static const int kBadgeSegments = 91;
static const float4 kBadgeText[91] = {
    float4(0.141, 0.764, 0.211, 0.866),
    float4(0.211, 0.866, 0.297, 0.909),
    float4(0.297, 0.909, 0.385, 0.885),
    float4(0.385, 0.885, 0.461, 0.798),
    float4(0.461, 0.798, 0.512, 0.662),
    float4(0.512, 0.662, 0.530, 0.500),
    float4(0.530, 0.500, 0.512, 0.338),
    float4(0.512, 0.338, 0.461, 0.202),
    float4(0.461, 0.202, 0.385, 0.115),
    float4(0.385, 0.115, 0.297, 0.091),
    float4(0.297, 0.091, 0.211, 0.134),
    float4(0.211, 0.134, 0.141, 0.236),
    float4(0.200, 0.500, 0.530, 0.500),
    float4(0.810, 0.090, 0.910, 0.120),
    float4(0.910, 0.120, 0.980, 0.250),
    float4(0.980, 0.250, 1.010, 0.450),
    float4(1.010, 0.450, 1.020, 0.910),
    float4(1.020, 0.910, 1.470, 0.910),
    float4(1.470, 0.910, 1.470, -0.020),
    float4(2.290, 0.500, 2.273, 0.658),
    float4(2.273, 0.658, 2.223, 0.792),
    float4(2.223, 0.792, 2.149, 0.881),
    float4(2.149, 0.881, 2.062, 0.910),
    float4(2.062, 0.910, 1.975, 0.876),
    float4(1.975, 0.876, 1.902, 0.783),
    float4(1.902, 0.783, 1.855, 0.647),
    float4(1.855, 0.647, 1.840, 0.488),
    float4(1.840, 0.488, 1.860, 0.330),
    float4(1.860, 0.330, 1.912, 0.199),
    float4(1.912, 0.199, 1.988, 0.115),
    float4(1.988, 0.115, 2.075, 0.090),
    float4(2.075, 0.090, 2.161, 0.129),
    float4(2.161, 0.129, 2.232, 0.226),
    float4(1.840, 0.500, 2.290, 0.500),
    float4(2.660, -0.020, 2.660, 1.020),
    float4(2.660, 0.480, 2.900, 0.480),
    float4(2.900, 0.480, 2.980, 0.500),
    float4(2.980, 0.500, 3.040, 0.560),
    float4(3.040, 0.560, 3.080, 0.650),
    float4(3.080, 0.650, 3.095, 0.770),
    float4(3.095, 0.770, 3.100, 1.020),
    float4(2.880, 0.480, 3.100, -0.020),
    float4(3.460, 0.910, 3.870, 0.910),
    float4(3.665, 0.910, 3.665, -0.020),
    float4(4.240, -0.020, 4.240, 0.910),
    float4(4.240, 0.910, 4.450, 0.910),
    float4(4.450, 0.910, 4.450, 0.920),
    float4(4.450, 0.920, 4.538, 0.902),
    float4(4.538, 0.902, 4.613, 0.853),
    float4(4.613, 0.853, 4.662, 0.778),
    float4(4.662, 0.778, 4.680, 0.690),
    float4(4.680, 0.690, 4.662, 0.602),
    float4(4.662, 0.602, 4.613, 0.527),
    float4(4.613, 0.527, 4.538, 0.478),
    float4(4.538, 0.478, 4.450, 0.460),
    float4(4.450, 0.460, 4.240, 0.460),
    float4(5.540, 0.500, 5.525, 0.640),
    float4(5.525, 0.640, 5.482, 0.764),
    float4(5.482, 0.764, 5.415, 0.855),
    float4(5.415, 0.855, 5.333, 0.904),
    float4(5.333, 0.904, 5.247, 0.904),
    float4(5.247, 0.904, 5.165, 0.855),
    float4(5.165, 0.855, 5.098, 0.764),
    float4(5.098, 0.764, 5.055, 0.640),
    float4(5.055, 0.640, 5.040, 0.500),
    float4(5.040, 0.500, 5.055, 0.360),
    float4(5.055, 0.360, 5.098, 0.236),
    float4(5.098, 0.236, 5.165, 0.145),
    float4(5.165, 0.145, 5.247, 0.096),
    float4(5.247, 0.096, 5.333, 0.096),
    float4(5.333, 0.096, 5.415, 0.145),
    float4(5.415, 0.145, 5.482, 0.236),
    float4(5.482, 0.236, 5.525, 0.360),
    float4(5.525, 0.360, 5.540, 0.500),
    float4(5.910, -0.020, 5.910, 1.020),
    float4(6.370, -0.020, 6.370, 1.020),
    float4(5.910, 0.500, 6.370, 0.500),
    float4(6.740, 1.020, 6.740, 0.090),
    float4(6.740, 0.090, 7.220, 0.910),
    float4(7.220, 0.910, 7.220, -0.020),
    float4(7.590, -0.020, 7.590, 1.020),
    float4(7.590, 0.480, 7.830, 0.480),
    float4(7.830, 0.480, 7.910, 0.500),
    float4(7.910, 0.500, 7.970, 0.560),
    float4(7.970, 0.560, 8.010, 0.650),
    float4(8.010, 0.650, 8.025, 0.770),
    float4(8.025, 0.770, 8.030, 1.020),
    float4(7.810, 0.480, 8.030, -0.020),
    float4(8.370, -0.020, 8.630, 1.020),
    float4(8.630, 1.020, 8.890, -0.020),
    float4(8.480, 0.300, 8.780, 0.300),
};
static const uint kBadgeEnds[91] = { 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 3, 1, 0, 0, 0, 0, 2, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 3, 3, 1, 0, 0, 0, 0, 2, 3, 3, 3, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 3, 3, 3, 1, 0, 2, 3, 1, 0, 0, 0, 0, 2, 3, 1, 2, 3 };
static const int kBadgeMarkSegments = 21;
static const float4 kBadgeMark[21] = {
    float4(0.126, 1.529, 0.169, 1.578),
    float4(0.169, 1.578, 0.221, 1.589),
    float4(0.221, 1.589, 0.272, 1.561),
    float4(0.272, 1.561, 0.307, 1.500),
    float4(0.307, 1.500, 0.320, 1.420),
    float4(0.320, 1.420, 0.307, 1.340),
    float4(0.307, 1.340, 0.272, 1.279),
    float4(0.272, 1.279, 0.221, 1.251),
    float4(0.221, 1.251, 0.169, 1.262),
    float4(0.169, 1.262, 0.126, 1.311),
    float4(0.150, 1.420, 0.310, 1.420),
    float4(0.794, 1.529, 0.751, 1.578),
    float4(0.751, 1.578, 0.699, 1.589),
    float4(0.699, 1.589, 0.648, 1.561),
    float4(0.648, 1.561, 0.613, 1.500),
    float4(0.613, 1.500, 0.600, 1.420),
    float4(0.600, 1.420, 0.613, 1.340),
    float4(0.613, 1.340, 0.648, 1.279),
    float4(0.648, 1.279, 0.699, 1.251),
    float4(0.699, 1.251, 0.751, 1.262),
    float4(0.751, 1.262, 0.794, 1.311),
};
// end of badge strokes

// The signed distance from p to a rectangle of half-size `size` around the
// origin with corners of radius r: negative inside.
float roundedBox(float2 p, float2 size, float r)
{
    float2 q = abs(p) - (size - r);
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
}

// The distance from p to the segment s (x0, y0, x1, y1).
float segmentDistance(float2 p, float4 s)
{
    float2 a = p - s.xy;
    float2 ba = s.zw - s.xy;
    return length(a - ba * clamp(dot(a, ba) / max(dot(ba, ba), 1e-8), 0.0, 1.0));
}

// The distance from p to a stroke along segment s, whose ends are square
// where `ends` says the path ends there (bit 0 its start, bit 1 its end)
// and round where it meets the next segment, so curves stay smooth.
float strokeDistance(float2 p, float4 s, uint ends)
{
    float2 a = p - s.xy;
    float len = length(s.zw - s.xy);
    float2 dir = (s.zw - s.xy) / len;
    float along = dot(a, dir);
    float across = abs(dir.x * a.y - dir.y * a.x);
    float beyond = max(-along, along - len);
    bool square = (ends & (along < 0.0 ? 1u : 2u)) != 0u;
    return beyond <= 0.0 ? across : square ? max(across, beyond) : length(float2(across, beyond));
}

// The emblem's light parts at m, in its units from the frame's top left
// corner, y down: the frame, the spire, its foot and the two letters. The
// distance to them, in units: negative inside.
float emblemMark(float2 m)
{
    // The frame, a stroke 0.08 wide inside the unit's width.
    float frame = abs(roundedBox(m - float2(0.5, 0.5 * kMarkHeight), float2(0.46, 0.5 * kMarkHeight - 0.04), 0.0)) - 0.04;

    // The spire narrows to a point at the top; its foot flares out over a
    // concave curve and stands on an arch.
    const float x0 = 0.46;
    float v = m.y;
    float spire = max(abs(m.x - x0) - lerp(0.015, 0.07, saturate((v - 0.14) / 1.43)), max(0.14 - v, v - 1.57));
    float flare = saturate((v - 1.57) / 0.26);
    float width = 0.07 + 0.26 * flare * flare;
    float du = m.x - x0;
    float base = 1.83 - 0.1 * saturate(1.0 - du * du / 0.1);
    float foot = max(abs(du) - width, max(1.57 - v, v - base));
    float letters = 1e9;

    [loop]
    for (int i = 0; i < kBadgeMarkSegments; i++) {
        letters = min(letters, segmentDistance(m, kBadgeMark[i]));
    }

    return min(min(frame, spire), min(foot, letters - 0.04));
}

// The badge over `front` at b, from the badge's top left corner, y down.
// `pixel` is the size of a drawable pixel in half-heights.
float3 badge(float2 b, float3 front, float pixel)
{
    float2 hs = kBadgeSize * 0.5;
    float edge = roundedBox(b - hs, hs, 0.004);
    float plate = 1.0 - smoothstep(-0.5 * pixel, 0.5 * pixel, edge);
    float3 result = front;

    if (plate > 0.0) {
        // Brushed metal, a little brighter towards the top, where the room
        // light comes from, and at its rim; black paint on it.
        float3 paint = u.room * kPaint;
        float rim = smoothstep(-0.004, 0.0, edge);
        float3 metal = u.room * kPlate * (1.08 - 0.16 * b.y / kBadgeSize.y + 0.15 * rim);
        float3 colour = metal;

        // The emblem.
        float block = roundedBox(b - float2(0.5 * kBadgeEmblem + 0.002, hs.y), float2(0.5 * kBadgeEmblem, hs.y - 0.003), 0.002);
        colour = lerp(colour, paint, 1.0 - smoothstep(-0.5 * pixel, 0.5 * pixel, block));
        float mark = emblemMark((b - kMarkAt) / kMarkUnit) * kMarkUnit;
        colour = lerp(colour, metal * 0.9, 1.0 - smoothstep(-0.5 * pixel, 0.5 * pixel, mark));

        // The lettering, in cap heights, y up.
        float2 t = float2(b.x - kTextAt, hs.y + 0.5 * kCapHeight - b.y) / kCapHeight;

        if (t.x > -0.2 && t.x < 9.3 && t.y > -0.2 && t.y < 1.2) {
            float d = 1e9;

            [loop]
            for (int i = 0; i < kBadgeSegments; i++) {
                d = min(d, strokeDistance(t, kBadgeText[i], kBadgeEnds[i]));
            }

            // Cut off at the baseline and the cap height, so diagonals end flat.
            float shape = max(d - kStroke, max(-t.y, t.y - 1.0));
            float soft = 0.5 * pixel / kCapHeight;
            colour = lerp(colour, paint, 1.0 - smoothstep(-soft, soft, shape));
        }

        result = lerp(front, colour, plate);
    }

    return result;
}

// The monitor's front at p, in half-heights from the middle of the glass, y
// down and not curved with the glass, over `inner`, the bezel drawn there.
float3 monitorFront(float2 p, float2 extent, float3 inner, float pixel)
{
    float2 size = float2(extent.x + kFrontSide, 1.0 + 0.5 * (kFrontTop + kFrontBottom));
    float2 d = p - float2(0.0, 0.5 * (kFrontBottom - kFrontTop));
    float e = roundedBox(d, size, kFrontCorner);
    float3 room = u.room * kRoom;
    float3 result = room;

    if (e <= pixel) {
        // The outline's outward normal; y points down.
        float2 q = abs(d) - (size - kFrontCorner);
        float2 n;

        if (length(max(q, 0.0)) > 0.0) {
            n = normalize(max(q, 0.0));
        } else {
            n = q.x > q.y ? float2(1.0, 0.0) : float2(0.0, 1.0);
        }

        n *= float2(d.x < 0.0 ? -1.0 : 1.0, d.y < 0.0 ? -1.0 : 1.0);

        // The outer edge turns from facing the viewer to facing out, over a
        // quarter circle. Room light comes more from above, as on the
        // bezel's wall, and the top edge catches a soft highlight off the
        // plastic.
        float phi = asin(saturate(1.0 + e / kFrontRim));
        float lit = 0.5 * (1.0 + cos(phi)) * (1.0 - 0.5 * sin(phi) * n.y);
        float shine = (phi - 0.9) / 0.3;
        float3 front = inner * lit + u.room * kShine * max(-n.y, 0.0) * exp(-shine * shine);

        front = badge(p - float2(-extent.x, 1.0) - kBadgeAt, front, pixel);
        result = lerp(front, room, smoothstep(-0.5 * pixel, 0.5 * pixel, e));
    }

    return result;
}

float4 ps_crt(float4 position : SV_Position) : SV_Target
{
    // Tube coordinates: -1…1 across the 4:3 face, then barrel distortion.
    float2 uv = (position.xy - u.view.xy) / u.view.zw;
    float2 cc = uv * 2.0 - 1.0;
    float2 straight = cc;
    cc *= 1.0 + u.curvature * dot(cc, cc);

    // The glass: a rounded rectangle, measured in half-heights of the tube.
    // `edge` is the signed distance to its outline (negative inside), and n
    // the outline's outward normal.
    const float corner = 0.12;
    float2 extent = float2(u.view.z / u.view.w, 1.0);
    float2 p = cc * extent;
    float2 q = abs(p) - (extent - corner);
    float2 outer = max(q, 0.0);
    float edge = length(outer) + min(max(q.x, q.y), 0.0) - corner;
    float2 n;

    if (length(outer) > 0.0) {
        n = normalize(outer);
    } else {
        n = q.x > q.y ? float2(1.0, 0.0) : float2(0.0, 1.0);
    }

    n *= float2(p.x < 0.0 ? -1.0 : 1.0, p.y < 0.0 ? -1.0 : 1.0);
    float pixel = 2.0 / u.view.w;
    float mask = 1.0 - smoothstep(-pixel, pixel, edge);
    float3 frame = 0.0;

    if (mask < 1.0) {
        frame = u.bezel3d != 0 ? bezel(p, edge, n, extent) : flatBezel(edge);

        if (u.monitorFront != 0) {
            frame = monitorFront(straight * extent, extent, frame, pixel);
        }
    }

    if (mask <= 0.0) {
        return float4(frame, 1.0);
    }

    float2 c = (cc * 0.5 + 0.5) * u.canvas;     // raster canvas position

    // Vertical beam profile: Gaussian per raster line, widened by brightness.
    // Below ~3 drawable pixels per line the profile is blended towards a flat
    // field, so small windows do not alias.
    float linesPx = u.view.w / u.canvas.y;
    float base = floor(c.y - 0.5);
    float lit = 0.0;
    float flatField = 0.0;

    [unroll]
    for (int i = 0; i < 2; i++) {
        float row = base + i;
        float I = lineIntensity(c.x, row, u.afterglow, 0.3, 0.5 * u.bloom);
        float d = c.y - (row + 0.5);
        float sigma = u.scanline * (1.0 + u.bloom * I);
        lit += I * exp(-d * d / (2.0 * sigma * sigma));
        flatField += I * max(0.0, 1.0 - abs(d));
    }

    float scan = smoothstep(2.0, 4.0, linesPx);
    float norm = 1.0 / (u.scanline * 2.5066);   // flat field ≈ 1 after the profile
    float beam = lerp(flatField, min(lit * norm, 1.5), scan);

    // Halo: a soft blur of the phosphor (binomial weights, bilinear taps).
    float halo = 0.0;
    static const float hw[3] = { 1.0, 2.0, 1.0 };

    [unroll]
    for (int y = -1; y <= 1; y++) {
        [unroll]
        for (int x = -1; x <= 1; x++) {
            float w = hw[x + 1] * hw[y + 1];
            halo += w * phosphorAt(c + float2(x * 2.5, y * 1.5), u.afterglow);
        }
    }

    beam += u.glow * halo / 16.0;
    beam = min(beam, 1.0);     // overdriving one channel would desaturate the phosphor colour

    // The unlit face: room light off the glass, fading towards the bezel.
    float shade = lerp(kRim, 1.0, smoothstep(0.0, kRimWidth, -edge));

    float vignette = 1.0 - u.vignette * dot(cc, cc) * 0.5;
    float3 rgb = u.phosphor.rgb * beam * u.phosphor.w * vignette + u.background.rgb * shade;
    return float4(lerp(frame, rgb, mask), 1.0);
}

// ---- Mipmaps -------------------------------------------------------------------
// The phosphor's mipmaps, for the light on the bezel's wall: each level is
// drawn from the one above it, tex0. A texel is the average of the box it
// stands for in the level above, in the texture's coordinates: 2 × 2 texels
// where that level's size is even, a little over 2 where it is odd, each
// texel weighted by how much of it the box covers. So every level keeps the
// light of the one above, in the same place; a plain 2 × 2 average drops the
// last row or column of an odd level, and puts the deeper levels' light out
// of place.
float2 ps_downsample(float4 position : SV_Position) : SV_Target
{
    uint w, h;
    tex0.GetDimensions(w, h);
    float2 size = float2(w, h);
    float2 ratio = size / max(floor(size * 0.5), 1.0);
    float2 lo = floor(position.xy) * ratio;
    float2 hi = lo + ratio;
    float2 sum = 0.0;

    // The box is at most 3 texels long, so it touches at most 4 of them.
    for (int j = 0; j < 4; j++) {
        float y = floor(lo.y) + float(j);
        float wy = max(min(hi.y, y + 1.0) - max(lo.y, y), 0.0);

        for (int i = 0; i < 4; i++) {
            float x = floor(lo.x) + float(i);
            float wx = max(min(hi.x, x + 1.0) - max(lo.x, x), 0.0);

            if (wx * wy > 0.0) {
                sum += wx * wy * tex0.Load(int3(min(int2(x, y), int2(w, h) - 1), 0)).rg;
            }
        }
    }

    return sum / (ratio.x * ratio.y);
}

// ---- CRT effects off ----------------------------------------------------------
// Sharp-bilinear scaling of the beam canvas (tex0): crisp pixels without the
// uneven widths of nearest-neighbour at non-integer scales.
float4 ps_flat(float4 position : SV_Position) : SV_Target
{
    float2 uv = (position.xy - u.view.xy) / u.view.zw;

    if (any(uv < 0.0) || any(uv >= 1.0)) {
        return float4(0.0, 0.0, 0.0, 1.0);
    }

    float2 texel = uv * u.canvas;
    float2 scale = max(floor(u.view.zw / u.canvas), 1.0);
    float2 centre = frac(texel) - 0.5;
    float2 range = 0.5 - 0.5 / scale;
    float2 f = (centre - clamp(centre, -range, range)) * scale + 0.5;
    float I = tex0.SampleLevel(linearClamp, (floor(texel) + f) / u.canvas, 0).r;
    return float4(u.phosphor.rgb * I * u.phosphor.w, 1.0);
}
