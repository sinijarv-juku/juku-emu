//! Replays the core's keyboard picture (spec §6.4, `juku::keyboard::drawing`)
//! with Direct2D, as frontends/macos/src/KeyboardView.m does with Core
//! Graphics. Every key, legend and color is the core's; only the PC key
//! labels' text is set here, with DirectWrite in the system's font.

use juku::keyboard::drawing::{Label, Op, ops};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D1_BEZIER_SEGMENT, D2D1_COLOR_F, D2D1_FIGURE_BEGIN_FILLED, D2D1_FIGURE_END_CLOSED, D2D1_FIGURE_END_OPEN,
    D2D1_FILL_MODE_WINDING, D2D1_GRADIENT_STOP,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_CAP_STYLE_FLAT, D2D1_DASH_STYLE_SOLID, D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_EXTEND_MODE_CLAMP, D2D1_FACTORY_TYPE_SINGLE_THREADED,
    D2D1_GAMMA_2_2, D2D1_LINE_JOIN_MITER, D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES, D2D1_QUADRATIC_BEZIER_SEGMENT,
    D2D1_RADIAL_GRADIENT_BRUSH_PROPERTIES, D2D1_ROUNDED_RECT, D2D1_STROKE_STYLE_PROPERTIES, D2D1CreateFactory, ID2D1Factory,
    ID2D1PathGeometry, ID2D1RenderTarget, ID2D1SolidColorBrush, ID2D1StrokeStyle,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_METRICS, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT,
    DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_LINE_METRICS, DWRITE_TEXT_METRICS, DWRITE_TRIMMING, DWRITE_TRIMMING_GRANULARITY_CHARACTER,
    DWRITE_WORD_WRAPPING_NO_WRAP, DWriteCreateFactory, IDWriteFactory, IDWriteFontCollection, IDWriteTextLayout,
};
use windows::Win32::UI::HiDpi::SystemParametersInfoForDpi;
use windows::Win32::UI::WindowsAndMessaging::{NONCLIENTMETRICSW, SPI_GETNONCLIENTMETRICS};
use windows::core::{BOOL, HSTRING, Result, w};
use windows_numerics::{Matrix3x2, Vector2};

/// What drawing needs that does not depend on the window: the factories,
/// the stroke style the picture's strokes use (butt caps, miter joins), and
/// the system's interface font with its cap height, as a share of its size.
pub struct Graphics {
    pub d2d: ID2D1Factory,
    pub dwrite: IDWriteFactory,
    stroke: ID2D1StrokeStyle,
    font: HSTRING,
    cap_height: f32,
}

impl Graphics {
    pub fn new() -> Result<Graphics> {
        let d2d: ID2D1Factory = unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)? };
        let dwrite: IDWriteFactory = unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)? };
        let properties = D2D1_STROKE_STYLE_PROPERTIES {
            startCap: D2D1_CAP_STYLE_FLAT,
            endCap: D2D1_CAP_STYLE_FLAT,
            dashCap: D2D1_CAP_STYLE_FLAT,
            lineJoin: D2D1_LINE_JOIN_MITER,
            miterLimit: 10.0, // Core Graphics' default
            dashStyle: D2D1_DASH_STYLE_SOLID,
            dashOffset: 0.0,
        };
        let stroke = unsafe { d2d.CreateStrokeStyle(&properties, None)? };

        // The message font, Segoe UI, as the app's other windows use.
        let mut metrics = NONCLIENTMETRICSW { cbSize: size_of::<NONCLIENTMETRICSW>() as u32, ..Default::default() };
        let _ = unsafe { SystemParametersInfoForDpi(SPI_GETNONCLIENTMETRICS.0, metrics.cbSize, Some((&raw mut metrics).cast()), 0, 96) };
        let face = &metrics.lfMessageFont.lfFaceName;
        let name = String::from_utf16_lossy(&face[..face.iter().position(|&c| c == 0).unwrap_or(face.len())]);
        let font = HSTRING::from(if name.is_empty() { "Segoe UI" } else { name.as_str() });
        let cap_height = cap_height(&dwrite, &font).unwrap_or(0.7);
        Ok(Graphics { d2d, dwrite, stroke, font, cap_height })
    }

    /// `text` set on one line in the interface font, `size` high and at most
    /// `max_width` wide, cut short with an ellipsis if it is wider.
    pub fn text(&self, text: &str, size: f32, weight: DWRITE_FONT_WEIGHT, max_width: f32) -> Result<Text> {
        let wide: Vec<u16> = text.encode_utf16().collect();

        unsafe {
            let format =
                self.dwrite.CreateTextFormat(&self.font, None, weight, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, size, w!(""))?;
            format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
            let trimming = DWRITE_TRIMMING { granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER, delimiter: 0, delimiterCount: 0 };
            let sign = self.dwrite.CreateEllipsisTrimmingSign(&format)?;
            format.SetTrimming(&trimming, &sign)?;
            let layout = self.dwrite.CreateTextLayout(&wide, &format, max_width.max(1.0), size * 4.0)?;
            let mut metrics = DWRITE_TEXT_METRICS::default();
            layout.GetMetrics(&mut metrics)?;
            let mut line = [DWRITE_LINE_METRICS::default()];
            let mut count = 0;
            let _ = layout.GetLineMetrics(Some(&mut line), &mut count);
            Ok(Text { layout, width: metrics.width, height: metrics.height, baseline: line[0].baseline, cap: size * self.cap_height })
        }
    }
}

/// The cap height of `font`, semibold, per unit of size.
fn cap_height(dwrite: &IDWriteFactory, font: &HSTRING) -> Option<f32> {
    unsafe {
        let mut collection: Option<IDWriteFontCollection> = None;
        dwrite.GetSystemFontCollection(&mut collection, false).ok()?;
        let collection = collection?;
        let (mut index, mut exists) = (0, BOOL(0));
        collection.FindFamilyName(font, &mut index, &mut exists).ok()?;

        if !exists.as_bool() {
            return None;
        }

        let font = collection.GetFontFamily(index).ok()?.GetFirstMatchingFont(
            DWRITE_FONT_WEIGHT_SEMI_BOLD,
            DWRITE_FONT_STRETCH_NORMAL,
            DWRITE_FONT_STYLE_NORMAL,
        );
        let mut metrics = DWRITE_FONT_METRICS::default();
        font.ok()?.GetMetrics(&mut metrics);
        (metrics.designUnitsPerEm > 0).then_some(metrics.capHeight as f32 / metrics.designUnitsPerEm.max(1) as f32)
    }
}

/// A line of text laid out: its width and height, the baseline from its
/// top, and the height of its capitals.
pub struct Text {
    pub layout: IDWriteTextLayout,
    pub width: f32,
    pub height: f32,
    pub baseline: f32,
    pub cap: f32,
}

impl Text {
    /// Draws the text from its top left corner.
    pub fn draw(&self, target: &ID2D1RenderTarget, brush: &ID2D1SolidColorBrush, x: f32, y: f32) {
        unsafe { target.DrawTextLayout(Vector2 { X: x, Y: y }, &self.layout, brush, D2D1_DRAW_TEXT_OPTIONS_NONE) };
    }

    /// Draws the text centered on `(cx, cy)` by its capitals.
    pub fn draw_centered(&self, target: &ID2D1RenderTarget, brush: &ID2D1SolidColorBrush, cx: f32, cy: f32) {
        self.draw(target, brush, cx - self.width / 2.0, cy + self.cap / 2.0 - self.baseline);
    }
}

/// A color of the picture, 0xRRGGBBAA in sRGB, not premultiplied, as
/// Direct2D's brushes take them.
pub fn color(rgba: u32) -> D2D1_COLOR_F {
    let c = |shift: u32| ((rgba >> shift) & 0xFF) as f32 / 255.0;
    D2D1_COLOR_F { r: c(24), g: c(16), b: c(8), a: c(0) }
}

pub fn rect(x: f32, y: f32, w: f32, h: f32) -> D2D_RECT_F {
    D2D_RECT_F { left: x, top: y, right: x + w, bottom: y + h }
}

/// The scale and offset that fit a picture `size` into `bounds`, centered.
pub fn fit(bounds: D2D_RECT_F, size: (f32, f32)) -> Matrix3x2 {
    let (w, h) = (bounds.right - bounds.left, bounds.bottom - bounds.top);
    let scale = (w / size.0).min(h / size.1);
    let mut m = Matrix3x2::scale(scale, scale);
    m.M31 = bounds.left + (w - size.0 * scale) / 2.0;
    m.M32 = bounds.top + (h - size.1 * scale) / 2.0;
    m
}

/// A point of the view in the picture `m` places.
pub fn to_picture(m: &Matrix3x2, x: f32, y: f32) -> (f32, f32) {
    ((x - m.M31) / m.M11, (y - m.M32) / m.M22)
}

/// The path being built, until a paint command uses it up.
enum Segment {
    Move(Vector2),
    Line(Vector2),
    Quad(Vector2, Vector2),
    Cubic(Vector2, Vector2, Vector2),
    Close,
}

fn point(x: f32, y: f32) -> Vector2 {
    Vector2 { X: x, Y: y }
}

/// The path as a geometry, filled by the nonzero rule, which the legends'
/// outlines need.
fn geometry(g: &Graphics, path: &[Segment]) -> Result<ID2D1PathGeometry> {
    unsafe {
        let geometry = g.d2d.CreatePathGeometry()?;
        let sink = geometry.Open()?;
        sink.SetFillMode(D2D1_FILL_MODE_WINDING);
        let (mut open, mut start) = (false, point(0.0, 0.0));

        for segment in path {
            // A segment after a close continues from where the subpath began.
            if !open && !matches!(segment, Segment::Move(_) | Segment::Close) {
                sink.BeginFigure(start, D2D1_FIGURE_BEGIN_FILLED);
                open = true;
            }

            match *segment {
                Segment::Move(p) => {
                    if open {
                        sink.EndFigure(D2D1_FIGURE_END_OPEN);
                    }

                    sink.BeginFigure(p, D2D1_FIGURE_BEGIN_FILLED);
                    (open, start) = (true, p);
                }
                Segment::Line(p) => sink.AddLine(p),
                Segment::Quad(c, p) => sink.AddQuadraticBezier(&D2D1_QUADRATIC_BEZIER_SEGMENT { point1: c, point2: p }),
                Segment::Cubic(c1, c2, p) => sink.AddBezier(&D2D1_BEZIER_SEGMENT { point1: c1, point2: c2, point3: p }),
                Segment::Close => {
                    if open {
                        sink.EndFigure(D2D1_FIGURE_END_CLOSED);
                        open = false;
                    }
                }
            }
        }

        if open {
            sink.EndFigure(D2D1_FIGURE_END_OPEN);
        }

        sink.Close()?;
        Ok(geometry)
    }
}

fn stops(stops: &[(f32, u32)]) -> Vec<D2D1_GRADIENT_STOP> {
    stops.iter().map(|&(position, rgba)| D2D1_GRADIENT_STOP { position, color: color(rgba) }).collect()
}

/// A PC key label (the LABEL command): `text` in a rounded rectangle.
fn label(g: &Graphics, target: &ID2D1RenderTarget, brush: &ID2D1SolidColorBrush, l: &Label, text: &str) -> Result<()> {
    let mut set = g.text(text, l.font_size, DWRITE_FONT_WEIGHT_SEMI_BOLD, 10_000.0)?;

    if set.width + l.padding > l.max_width && set.width > 0.0 {
        set = g.text(text, l.font_size * (l.max_width - l.padding) / set.width, DWRITE_FONT_WEIGHT_SEMI_BOLD, 10_000.0)?;
    }

    let width = l.height.max(set.width.ceil() + l.padding);
    let chip = rect(l.center_x - width / 2.0, l.top, width, l.height);

    unsafe {
        brush.SetColor(&color(l.fill));
        target.FillRoundedRectangle(&D2D1_ROUNDED_RECT { rect: chip, radiusX: l.radius, radiusY: l.radius }, brush);
        brush.SetColor(&color(l.text));
    }

    set.draw_centered(target, brush, l.center_x, l.top + l.height / 2.0);
    Ok(())
}

/// Replays `picture` into `target` with `transform`, from drawing units to
/// the view's. BACKGROUND fills `view`, around the picture too. `labels`
/// gives a keycap's PC key label.
pub fn replay(g: &Graphics, target: &ID2D1RenderTarget, picture: &[u8], view: D2D_RECT_F, transform: &Matrix3x2, labels: &dyn Fn(u16) -> Option<String>) -> Result<()> {
    let brush = unsafe { target.CreateSolidColorBrush(&color(0), None)? };
    let mut path = Vec::new();
    unsafe { target.SetTransform(transform) };

    for op in ops(picture) {
        // A paint command uses up the path.
        let paints = matches!(op, Op::Fill(_) | Op::Linear { .. } | Op::Radial { .. } | Op::Stroke { .. });

        match op {
            Op::Background(rgba) => unsafe {
                brush.SetColor(&color(rgba));
                target.SetTransform(&Matrix3x2::identity());
                target.FillRectangle(&view, &brush);
                target.SetTransform(transform);
            },
            Op::Move(x, y) => path.push(Segment::Move(point(x, y))),
            Op::Line(x, y) => path.push(Segment::Line(point(x, y))),
            Op::Quad(cx, cy, x, y) => path.push(Segment::Quad(point(cx, cy), point(x, y))),
            Op::Cubic(c1x, c1y, c2x, c2y, x, y) => path.push(Segment::Cubic(point(c1x, c1y), point(c2x, c2y), point(x, y))),
            Op::Close => path.push(Segment::Close),
            Op::Fill(rgba) => unsafe {
                brush.SetColor(&color(rgba));
                target.FillGeometry(&geometry(g, &path)?, &brush, None);
            },
            Op::Linear { from, to, stops: s } => unsafe {
                let collection = target.CreateGradientStopCollection(&stops(&s), D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP)?;
                let line = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES { startPoint: point(from.0, from.1), endPoint: point(to.0, to.1) };
                let gradient = target.CreateLinearGradientBrush(&line, None, &collection)?;
                target.FillGeometry(&geometry(g, &path)?, &gradient, None);
            },
            Op::Radial { center, radius, stops: s } => unsafe {
                let collection = target.CreateGradientStopCollection(&stops(&s), D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP)?;
                let circle = D2D1_RADIAL_GRADIENT_BRUSH_PROPERTIES {
                    center: point(center.0, center.1),
                    gradientOriginOffset: point(0.0, 0.0),
                    radiusX: radius,
                    radiusY: radius,
                };
                let gradient = target.CreateRadialGradientBrush(&circle, None, &collection)?;
                target.FillGeometry(&geometry(g, &path)?, &gradient, None);
            },
            Op::Stroke { width, color: rgba } => unsafe {
                brush.SetColor(&color(rgba));
                target.DrawGeometry(&geometry(g, &path)?, &brush, width, &g.stroke);
            },
            Op::Label(l) => {
                if let Some(text) = labels(l.index).filter(|t| !t.is_empty()) {
                    label(g, target, &brush, &l, &text)?;
                }
            }
        }

        if paints {
            path.clear();
        }
    }

    unsafe { target.SetTransform(&Matrix3x2::identity()) };
    Ok(())
}
