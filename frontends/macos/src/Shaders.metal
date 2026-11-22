#include <metal_stdlib>

#include "ShaderTypes.h"

using namespace metal;

struct VertexOut {
    float4 position [[ position ]];
};

// One triangle covering the whole target.
vertex VertexOut vsh_fullscreen(uint vid [[ vertex_id ]])
{
    float2 p = float2((vid << 1) & 2, vid & 2);
    VertexOut out;
    out.position = float4(p * 2.0 - 1.0, 0.0, 1.0);
    return out;
}

// ---- Pass 1: decode ---------------------------------------------------------
// Renders the raster canvas (one texel per pixel clock × raster line): beam on
// (1) or off (0). Bit 7 of each byte is the leftmost pixel.
fragment float fsh_decode(VertexOut in [[ stage_in ]],
                          constant CRTUniforms &u [[ buffer(0) ]],
                          texture2d<uint, access::read> video [[ texture(0) ]])
{
    float2 c = floor(in.position.xy) - u.active.xy;

    if (any(c < 0.0) || any(c >= u.active.zw)) {
        return 0.0;
    }

    uint px = uint(c.x);
    uint byte = video.read(uint2(px >> 3, uint(c.y))).r;
    return float((byte >> (7 - (px & 7))) & 1);
}

// ---- Pass 2: phosphor persistence ------------------------------------------
// r: the main phosphor response, g: a slow faint afterglow. Each decays from
// its previous value and is re-excited by the beam.
fragment float2 fsh_persist(VertexOut in [[ stage_in ]],
                            constant CRTUniforms &u [[ buffer(0) ]],
                            texture2d<float, access::read> beam [[ texture(0) ]],
                            texture2d<float, access::read> prev [[ texture(1) ]])
{
    uint2 p = uint2(in.position.xy);
    float s = beam.read(p).r;
    float2 old = prev.read(p).rg;
    return max(float2(s), old * float2(u.decayFast, u.decaySlow));
}

// ---- Pass 3: CRT composite -------------------------------------------------

constexpr sampler linearClamp(filter::linear, address::clamp_to_zero);

static float phosphorAt(texture2d<float> tex, float2 texel, float afterglow)
{
    float2 p = tex.sample(linearClamp, texel / float2(tex.get_width(), tex.get_height())).rg;
    return max(p.r, afterglow * p.g);
}

// One texel of the phosphor, read exactly; black outside the canvas.
static float texelAt(texture2d<float> tex, int x, int y, float afterglow)
{
    if (x < 0 || y < 0 || x >= int(tex.get_width()) || y >= int(tex.get_height())) {
        return 0.0;
    }

    float2 p = tex.read(uint2(x, y)).rg;
    return max(p.r, afterglow * p.g);
}

// The Gaussian CDF, approximated by a logistic. Like the real one it has
// F(-t) = 1 - F(t), so pixel boxes of an even field still add up to exactly 1.
static float edgeCdf(float t)
{
    return 1.0 / (1.0 + exp(-1.702 * t));
}

// Beam intensity along one raster line at canvas position x. Each pixel lights
// a box one pixel wide, softened by the beam spot (a Gaussian of sigma
// texels). A bright pixel's box is also widened by `spread` texels on each
// side: the beam blooms, so white on black looks bolder and a dark stroke in
// reverse video thinner. The phosphor saturates at 1.
static float lineIntensity(texture2d<float> tex, float x, float line, float afterglow, float sigma, float spread)
{
    int y = int(line);
    int x0 = int(floor(x));
    float sum = 0.0;

    for (int i = -2; i <= 2; i++) {
        float I = texelAt(tex, x0 + i, y, afterglow);

        if (I > 0.0) {
            float d = x - (float(x0 + i) + 0.5);
            float h = 0.5 + spread * I;
            sum += I * (edgeCdf((h - d) / sigma) + edgeCdf((h + d) / sigma) - 1.0);
        }
    }

    return min(sum, 1.0);
}

// ---- The bezel ---------------------------------------------------------------
// The МС6105's bezel stands about 1 cm in front of the edge of the glass, on a
// face about 18 cm tall. Its inner wall leans out from the glass at
// `kWallSlope` from the line of sight, and turns into the front over a
// rounded lip of radius `kLipRadius`. Seen in perspective by a viewer in front
// of the middle of the tube, the wall shows as a band around the glass, widest
// at the sides. Lengths are in half-heights of the tube.
constant float kBezelDepth = 0.11;
constant float kWallSlope = 0.524;      // 30°
constant float kLipRadius = 0.04;
constant float kViewDistance = 4.5;     // from the glass, about 50 cm

// The unlit face fades to `kRim` of its level at the edge of the glass, in the
// shadow of the bezel, over the outer `kRimWidth`.
constant float kRim = 0.8;
constant float kRimWidth = 0.08;

// The diffuse reflectance of the wall, for the light from the face. It is the
// same matte plastic as the front, which the room light gives the Frame level.
constant float kWallReflectance = 0.4;

// The phosphor lies behind the faceplate, about 1 cm of glass, which looks
// two thirds as deep: the wall's foot is that high above the light, and is
// not left dark next to the glass.
constant float kFaceplate = 0.07;

constexpr sampler mipClamp(filter::linear, mip_filter::linear, address::clamp_to_zero);

// The light the face sends to a point of the bezel at height h above the
// phosphor and `out` outside the outline, over the point `foot` of the
// outline, whose outward normal is n. Both the lit phosphor and the unlit face
// (room light off the glass) count. It is for a surface square to the glass;
// the caller scales it for the surface's tilt.
//
// A point of the face at distance d from the wall and b along it lights the
// wall by d·h / (π (d² + b² + h²)²). Over b this is
// d·h / (2 (d² + h²)^{3/2}): half of the hemisphere, most of it from within a
// few h of the wall. Five taps sit at the middles of five equal shares of it,
// each spread along the wall over ±0.44 r, its quartiles there, and read from
// a phosphor mip level as wide as r, so a wall point sees a smooth average.
static float3 faceLight(constant CRTUniforms &u, texture2d<float> phosphor,
                        float2 foot, float2 n, float h, float out, float2 extent)
{
    const float taps[5] = { 0.484, 1.020, 1.732, 3.180, 9.950 };
    float2 along = float2(-n.y, n.x);
    float texels = 0.5 * u.canvas.y;    // canvas lines per half-height
    float3 sum = 0.0;

    for (int i = 0; i < 5; i++) {
        float d = taps[i] * h;
        float a = max(d - out, 0.0);    // into the glass
        float r = sqrt(d * d + h * h);
        float lod = log2(max(r * texels, 1.0));
        float I = 0.0;

        for (int s = -1; s <= 1; s += 2) {
            float2 p = foot - n * a + along * (float(s) * 0.44 * r);
            float2 c = (p / extent * 0.5 + 0.5);
            float2 v = phosphor.sample(mipClamp, c, level(lod)).rg;
            I += 0.5 * max(v.r, u.afterglow * v.g);
        }

        float shade = mix(kRim, 1.0, smoothstep(0.0, kRimWidth, a));
        sum += 0.1 * (u.phosphor.rgb * u.phosphor.w * min(I, 1.0) + u.background.rgb * shade);
    }

    return sum;
}

// The monitor outside the glass at tube position p (in half-heights), where
// `edge` is the signed distance to the glass outline and n its normal.
static float3 bezel(constant CRTUniforms &u, texture2d<float> phosphor,
                    float2 p, float edge, float2 n, float2 extent)
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

    if (e >= xc + k * kBezelDepth) {
        return u.frame.rgb;
    }

    // phi: the surface normal's angle from the line of sight, leaning in.
    float x, z, phi;

    if (e < wall * (sa + k * ca)) {
        float t = e / (sa + k * ca);
        x = t * sa;
        z = t * ca;
        phi = M_PI_2_F - kWallSlope;
    } else {
        // On the arc, x = xc − R sin phi and z = depth − R + R cos phi, so
        // −sin phi + k cos phi = (e − xc − k (depth − R)) / R.
        float m = sqrt(1.0 + k * k);
        float A = (e - xc - k * (kBezelDepth - R)) / R;
        phi = clamp(acos(clamp(A / m, -1.0, 1.0)) - atan2(1.0, k), 0.0, M_PI_2_F - kWallSlope);
        x = xc - R * sin(phi);
        z = kBezelDepth - R + R * cos(phi);
    }

    // A surface tilted by phi sees the face over 1 − cos phi of the share a
    // square wall gets, and the room in front over (1 + cos phi) / 2. Room
    // light comes more from above: the bottom wall faces up and gets more,
    // the top one less. It is shaded towards the glass. Framebuffer y points
    // down, so the bottom wall has n.y = 1.
    float3 face = faceLight(u, phosphor, p - n * edge, n, z + kFaceplate, x, extent);
    float room = 0.5 * (1.0 + cos(phi)) * (1.0 + 0.5 * sin(phi) * n.y) * (0.5 + 0.5 * z / kBezelDepth);
    return kWallReflectance * (1.0 - cos(phi)) * face + room * u.frame.rgb;
}

// The flat bezel, with the 3D bezel off: a recess where it meets the glass.
// The recess is lit by both the glass next to it and the room light on the
// frame: at the glass it is a quarter of the two together, and it eases out
// to the frame's own level. A bright face lights it, a bright frame still
// shows a soft dip, so the outline of the glass stays visible without a hard
// black line.
static float3 flatBezel(constant CRTUniforms &u, float edge)
{
    float3 recess = 0.25 * (u.background.rgb * kRim + u.frame.rgb);
    return mix(recess, u.frame.rgb, 1.0 - exp(-max(edge, 0.0) / 0.02));
}

fragment float4 fsh_crt(VertexOut in [[ stage_in ]],
                        constant CRTUniforms &u [[ buffer(0) ]],
                        texture2d<float> phosphor [[ texture(0) ]])
{
    // Tube coordinates: -1…1 across the 4:3 face, then barrel distortion.
    float2 uv = (in.position.xy - u.view.xy) / u.view.zw;
    float2 cc = uv * 2.0 - 1.0;
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
    float2 n = length(outer) > 0.0 ? normalize(outer) : (q.x > q.y ? float2(1.0, 0.0) : float2(0.0, 1.0));
    n *= select(float2(1.0), float2(-1.0), p < 0.0);
    float pixel = 2.0 / u.view.w;
    float mask = 1.0 - smoothstep(-pixel, pixel, edge);
    float3 frame = mask >= 1.0 ? 0.0
                 : u.bezel3d != 0 ? bezel(u, phosphor, p, edge, n, extent)
                 : flatBezel(u, edge);

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
    float flat = 0.0;

    for (int i = 0; i < 2; i++) {
        float line = base + i;
        float I = lineIntensity(phosphor, c.x, line, u.afterglow, 0.3, 0.5 * u.bloom);
        float d = c.y - (line + 0.5);
        float sigma = u.scanline * (1.0 + u.bloom * I);
        lit += I * exp(-d * d / (2.0 * sigma * sigma));
        flat += I * max(0.0, 1.0 - abs(d));
    }

    float scan = smoothstep(2.0, 4.0, linesPx);
    float norm = 1.0 / (u.scanline * 2.5066);   // flat field ≈ 1 after the profile
    float beam = mix(flat, min(lit * norm, 1.5), scan);

    // Halo: a soft blur of the phosphor (binomial weights, bilinear taps).
    float halo = 0.0;
    const float hw[3] = { 1.0, 2.0, 1.0 };

    for (int y = -1; y <= 1; y++) {
        for (int x = -1; x <= 1; x++) {
            float w = hw[x + 1] * hw[y + 1];
            halo += w * phosphorAt(phosphor, c + float2(x * 2.5, y * 1.5), u.afterglow);
        }
    }

    beam += u.glow * halo / 16.0;
    beam = min(beam, 1.0);     // overdriving one channel would desaturate the phosphor colour

    // The unlit face: room light off the glass, fading towards the bezel.
    float shade = mix(kRim, 1.0, smoothstep(0.0, kRimWidth, -edge));

    float vignette = 1.0 - u.vignette * dot(cc, cc) * 0.5;
    float3 rgb = u.phosphor.rgb * beam * u.phosphor.w * vignette + u.background.rgb * shade;
    return float4(mix(frame, rgb, mask), 1.0);
}

// ---- Mipmaps -------------------------------------------------------------------
// The phosphor's mipmaps, for the light on the bezel's wall: each level is
// drawn from the one above it. A texel is the average of the box it stands
// for in the level above, in the texture's coordinates: 2 × 2 texels where
// that level's size is even, a little over 2 where it is odd, each texel
// weighted by how much of it the box covers. So every level keeps the light
// of the one above, in the same place. The canvas's sizes are odd at most
// levels, and generateMipmaps does neither there: a bar moving down the face
// lit the wall in jumps.
fragment float2 fsh_downsample(VertexOut in [[ stage_in ]],
                               texture2d<float, access::read> above [[ texture(0) ]])
{
    float2 size = float2(above.get_width(), above.get_height());
    float2 ratio = size / max(floor(size * 0.5), 1.0);
    float2 lo = floor(in.position.xy) * ratio;
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
                sum += wx * wy * above.read(min(uint2(x, y), uint2(size) - 1)).rg;
            }
        }
    }

    return sum / (ratio.x * ratio.y);
}

// ---- CRT effects off ----------------------------------------------------------
// Sharp-bilinear scaling of the beam canvas: crisp pixels without the uneven
// widths of nearest-neighbour at non-integer scales.
fragment float4 fsh_flat(VertexOut in [[ stage_in ]],
                         constant CRTUniforms &u [[ buffer(0) ]],
                         texture2d<float> beam [[ texture(0) ]])
{
    float2 uv = (in.position.xy - u.view.xy) / u.view.zw;

    if (any(uv < 0.0) || any(uv >= 1.0)) {
        return float4(0.0, 0.0, 0.0, 1.0);
    }

    float2 texel = uv * u.canvas;
    float2 scale = max(floor(u.view.zw / u.canvas), 1.0);
    float2 centre = fract(texel) - 0.5;
    float2 range = 0.5 - 0.5 / scale;
    float2 f = (centre - clamp(centre, -range, range)) * scale + 0.5;
    float I = beam.sample(linearClamp, (floor(texel) + f) / u.canvas).r;
    return float4(u.phosphor.rgb * I * u.phosphor.w, 1.0);
}
