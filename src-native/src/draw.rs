use windows_sys::Win32::Graphics::GdiPlus::*;

#[derive(Clone, Copy, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StatusLightStyle {
    #[default]
    Bar,
    Dot,
}

#[derive(Clone, Copy, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RibbonStrength {
    Soft,
    #[default]
    Standard,
    Strong,
    Intense,
}

impl RibbonStrength {
    pub const ALL: [Self; 4] = [Self::Soft, Self::Standard, Self::Strong, Self::Intense];
    pub fn label(self) -> &'static str {
        match self { Self::Soft => "柔和", Self::Standard => "标准", Self::Strong => "醒目", Self::Intense => "高亮" }
    }
    pub fn stroke(self, scale: f32) -> f32 {
        match self {
            Self::Soft => (1.8 * scale).clamp(1.0, 2.5),
            Self::Standard => (3.0 * scale).clamp(2.0, 3.5),
            Self::Strong => (4.2 * scale).clamp(2.8, 4.5),
            Self::Intense => (5.0 * scale).clamp(3.5, 5.5),
        }
    }
    pub fn body_inset(self, scale: f32) -> f32 {
        // Overlap the stroke's inner antialiasing fringe with the dark body.
        // Merely touching two independently rasterized edges leaves a pale seam.
        (self.stroke(scale) - 0.25).max(0.5)
    }
    fn appearance(self) -> (u32, u32, u32, f32, f32) {
        // Track, static border, glow opacity; trail length and minimum opacity.
        match self {
            Self::Soft => (24, 216, 32, 0.24, 0.03),
            Self::Standard => (80, 255, 85, 0.40, 0.18),
            Self::Strong => (112, 255, 120, 0.52, 0.32),
            Self::Intense => (144, 255, 160, 0.64, 0.45),
        }
    }
}

#[derive(Clone)]
pub struct Display {
    pub theme: crate::theme::Theme,
    pub compact: bool,
    pub fusion: bool,
    pub strip: bool,
    pub vertical: bool,
    pub expanded: bool,
    pub island: bool,
    pub pinned: bool,
    pub short: Option<f64>,
    pub long: Option<f64>,
    pub short_ideal: Option<f64>,
    pub long_ideal: Option<f64>,
    pub short_velocity: Option<f64>,
    pub long_velocity: Option<f64>,
    pub short_reset: String,
    pub long_reset: String,
    pub short_left: String,
    pub long_left: String,
    pub plan: String,
    pub status: String,
    pub notice: Option<(String, String, bool, f32, bool)>,
    pub notice_lines: u8,
    pub task_state_color: Option<u32>,
    pub task_light_style: StatusLightStyle,
    pub task_light_rotation: Option<f32>,
    pub task_ribbon_color: Option<u32>,
    pub task_ribbon_rotation: Option<f32>,
    pub task_ribbon_strength: RibbonStrength,
    // Attached side in left/top/right/bottom order; that border remains flush.
    pub ribbon_attached: Option<usize>,
    pub task_light_active: bool,
    pub content_only: bool,
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod regression_tests {
    use super::*;
    #[test]
    fn ribbon_strength_round_trips_and_increases_at_supported_scales() {
        for scale in [0.46, 0.69, 0.92, 1.25, 1.875, 2.5] {
            let mut previous = 0.0;
            for strength in RibbonStrength::ALL {
                let loaded: RibbonStrength = serde_json::from_value(serde_json::to_value(strength).unwrap()).unwrap();
                assert!(loaded == strength);
                let stroke = strength.stroke(scale);
                assert!(stroke > previous);
                assert!(strength.body_inset(scale) < stroke);
                previous = stroke;
            }
        }
    }
    #[test]
    fn attached_ribbon_paths_are_open_and_have_no_screen_facing_segment() {
        for side in 0..4 {
            let points = ribbon_visible_outline(ribbon_outline(1.5, 1.5, 340.0, 16.0, 5.0, 5.0), Some(side));
            assert!(points.len() > 2);
            let a = points.first().unwrap();
            let b = points.last().unwrap();
            assert!((a.X - b.X).hypot(a.Y - b.Y) > 1.0);
            for pair in points.windows(2) {
                let hidden = match side {
                    0 => pair.iter().all(|p| p.X <= 1.55),
                    1 => pair.iter().all(|p| p.Y <= 1.55),
                    2 => pair.iter().all(|p| p.X >= 341.45),
                    _ => pair.iter().all(|p| p.Y >= 17.45),
                };
                assert!(!hidden);
            }
        }
    }
}

pub unsafe fn rounded(g: *mut GpGraphics, x: f32, y: f32, w: f32, h: f32, r: f32, color: u32) {
    let mut path = std::ptr::null_mut();
    let mut brush = std::ptr::null_mut();
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    GdipCreatePath(FillModeAlternate, &mut path);
    GdipAddPathArc(path, x, y, r * 2.0, r * 2.0, 180.0, 90.0);
    GdipAddPathArc(path, x + w - r * 2.0, y, r * 2.0, r * 2.0, 270.0, 90.0);
    GdipAddPathArc(
        path,
        x + w - r * 2.0,
        y + h - r * 2.0,
        r * 2.0,
        r * 2.0,
        0.0,
        90.0,
    );
    GdipAddPathArc(path, x, y + h - r * 2.0, r * 2.0, r * 2.0, 90.0, 90.0);
    GdipClosePathFigure(path);
    GdipCreateSolidFill(crate::theme::color(color), &mut brush);
    GdipFillPath(g, brush as *mut GpBrush, path);
    GdipDeleteBrush(brush as *mut GpBrush);
    GdipDeletePath(path);
}

unsafe fn line(g: *mut GpGraphics, x1: f32, y1: f32, x2: f32, y2: f32, width: f32, color: u32) {
    let mut pen = std::ptr::null_mut();
    GdipCreatePen1(crate::theme::color(color), width, UnitPixel, &mut pen);
    GdipDrawLine(g, pen, x1, y1, x2, y2);
    GdipDeletePen(pen);
}

unsafe fn text(
    g: *mut GpGraphics,
    content: &str,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    size: f32,
    bold: bool,
    color: u32,
) {
    text_with_align(g, content, x, y, w, h, size, bold, color, false);
}

unsafe fn text_center(
    g: *mut GpGraphics,
    content: &str,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    size: f32,
    bold: bool,
    color: u32,
) {
    text_with_align(g, content, x, y, w, h, size, bold, color, true);
}

unsafe fn text_with_align(
    g: *mut GpGraphics,
    content: &str,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    size: f32,
    bold: bool,
    color: u32,
    centered: bool,
) {
    let mut family = std::ptr::null_mut();
    let mut font = std::ptr::null_mut();
    let mut brush = std::ptr::null_mut();
    let mut format = std::ptr::null_mut();
    GdipCreateFontFamilyFromName(wide("Segoe UI").as_ptr(), std::ptr::null_mut(), &mut family);
    if family.is_null() {
        GdipCreateFontFamilyFromName(wide("Arial").as_ptr(), std::ptr::null_mut(), &mut family);
    }
    if family.is_null() {
        return;
    }
    GdipCreateFont(family, size, if bold { FontStyleBold } else { FontStyleRegular }, UnitPixel, &mut font);
    GdipCreateSolidFill(crate::theme::color(color), &mut brush);
    GdipCreateStringFormat(0, 0, &mut format);
    GdipSetStringFormatTrimming(format, StringTrimmingEllipsisCharacter);
    if centered {
        GdipSetStringFormatAlign(format, StringAlignmentCenter);
    }
    let utf16: Vec<u16> = content.encode_utf16().collect();
    let rect = RectF {
        X: x,
        Y: y,
        Width: w,
        Height: h,
    };
    GdipDrawString(
        g,
        utf16.as_ptr(),
        utf16.len() as i32,
        font,
        &rect,
        format,
        brush as *const GpBrush,
    );
    GdipDeleteStringFormat(format);
    GdipDeleteBrush(brush as *mut GpBrush);
    GdipDeleteFont(font);
    GdipDeleteFontFamily(family);
}

fn percent(v: Option<f64>) -> String {
    v.map_or("--".into(), |p| format!("{}%", p.round() as i32))
}
fn delta(v: Option<f64>, ideal: Option<f64>) -> String {
    match (v, ideal) {
        (Some(v), Some(i)) => format!("{:+.0}", v - i),
        _ => "--".into(),
    }
}

unsafe fn ellipse(g: *mut GpGraphics, x: f32, y: f32, w: f32, h: f32, color: u32) {
    let mut brush = std::ptr::null_mut();
    GdipCreateSolidFill(crate::theme::color(color), &mut brush);
    GdipFillEllipse(g, brush as *mut GpBrush, x, y, w, h);
    GdipDeleteBrush(brush as *mut GpBrush);
}

unsafe fn task_status_light(g: *mut GpGraphics, x: f32, y: f32, vertical: bool, color: Option<u32>, style: StatusLightStyle, rotation: Option<f32>) {
    let Some(color) = color else { return; };
    let (w, h) = if vertical { (16.0, 7.0) } else { (7.0, 16.0) };
    let mut points = [PointF { X: x, Y: y }, PointF { X: x + w, Y: y + h }];
    GdipTransformPoints(g, CoordinateSpaceDevice, CoordinateSpaceWorld, points.as_mut_ptr(), 2);
    let width = (points[1].X - points[0].X).round().max(1.0);
    let height = (points[1].Y - points[0].Y).round().max(1.0);
    let left = ((points[0].X + points[1].X - width) / 2.0).round();
    let top = ((points[0].Y + points[1].Y - height) / 2.0).round();
    let radius = width.min(height) / 2.0;
    let mut state = 0;
    GdipSaveGraphics(g, &mut state);
    GdipResetWorldTransform(g);
    if style == StatusLightStyle::Dot {
        let scale = ((points[1].X - points[0].X) / w).abs();
        let spinning = rotation.is_some();
        let diameter = ((if spinning { 14.0 } else { 12.0 }) * scale).round().max(if spinning { 8.0 } else { 6.0 });
        let cx = (points[0].X + points[1].X) / 2.0;
        let cy = (points[0].Y + points[1].Y) / 2.0;
        let left = (cx - diameter / 2.0).round();
        let top = (cy - diameter / 2.0).round();
        if let Some(angle) = rotation {
            // A quiet circular track and one rounded moving arc, without
            // arrowheads that become cluttered at tray-sized dimensions.
            let stroke = (3.5 * scale).max(2.0);
            let inset = stroke / 2.0;
            let side = diameter - stroke;
            let mut track = std::ptr::null_mut();
            let mut arc = std::ptr::null_mut();
            GdipCreatePen1((color & 0x00ffffff) | 0x35000000, stroke, UnitPixel, &mut track);
            GdipDrawEllipse(g, track, left + inset, top + inset, side, side);
            GdipDeletePen(track);
            GdipCreatePen1(color, stroke, UnitPixel, &mut arc);
            GdipSetPenLineCap197819(arc, LineCapRound, LineCapRound, DashCapRound);
            GdipDrawArc(g, arc, left + inset, top + inset, side, side, angle - 90.0, 115.0);
            GdipDeletePen(arc);
        } else {
            ellipse(g, left - 1.0, top - 1.0, diameter + 2.0, diameter + 2.0, (color & 0x00ffffff) | 0x20000000);
            ellipse(g, left, top, diameter, diameter, color);
        }
        GdipRestoreGraphics(g, state);
        return;
    }
    let glow_alpha = 0x24 * (color >> 24) / 255;
    rounded(g, left - 1.0, top - 1.0, width + 2.0, height + 2.0, radius + 1.0, (color & 0x00ffffff) | (glow_alpha << 24));
    rounded(g, left, top, width, height, radius, color);
    GdipRestoreGraphics(g, state);
}

fn ribbon_outline(x: f32, y: f32, w: f32, h: f32, top: f32, bottom: f32) -> Vec<PointF> {
    let mut points: Vec<PointF> = Vec::new();
    let mut segment = |from: PointF, to: PointF, arc: Option<(PointF, f32, f32)>| {
        let length = arc.map_or_else(|| (to.X - from.X).hypot(to.Y - from.Y), |(_, r, _)| r * std::f32::consts::FRAC_PI_2);
        if length < 0.001 { return; }
        let steps = (length / if arc.is_some() { 0.75 } else { 3.0 }).ceil().max(1.0) as usize;
        for index in 0..=steps {
            if index == 0 && !points.is_empty() { continue; }
            let t = index as f32 / steps as f32;
            points.push(if let Some((center, r, start)) = arc {
                let a = start + t * std::f32::consts::FRAC_PI_2;
                PointF { X: center.X + r * a.cos(), Y: center.Y + r * a.sin() }
            } else { PointF { X: from.X + (to.X - from.X) * t, Y: from.Y + (to.Y - from.Y) * t } });
        }
    };
    let p = |x, y| PointF { X: x, Y: y };
    segment(p(x + top, y), p(x + w - top, y), None);
    segment(p(x + w - top, y), p(x + w, y + top), Some((p(x + w - top, y + top), top, -std::f32::consts::FRAC_PI_2)));
    segment(p(x + w, y + top), p(x + w, y + h - bottom), None);
    segment(p(x + w, y + h - bottom), p(x + w - bottom, y + h), Some((p(x + w - bottom, y + h - bottom), bottom, 0.0)));
    segment(p(x + w - bottom, y + h), p(x + bottom, y + h), None);
    segment(p(x + bottom, y + h), p(x, y + h - bottom), Some((p(x + bottom, y + h - bottom), bottom, std::f32::consts::FRAC_PI_2)));
    segment(p(x, y + h - bottom), p(x, y + top), None);
    segment(p(x, y + top), p(x + top, y), Some((p(x + top, y + top), top, std::f32::consts::PI)));
    if let Some(first) = points.first().copied() { points.push(first); }
    points
}

fn ribbon_point(points: &[PointF], distances: &[f32], at: f32) -> PointF {
    let next = distances.partition_point(|distance| *distance < at).clamp(1, points.len() - 1);
    let t = ((at - distances[next - 1]) / (distances[next] - distances[next - 1]).max(0.001)).clamp(0.0, 1.0);
    PointF { X: points[next - 1].X + (points[next].X - points[next - 1].X) * t, Y: points[next - 1].Y + (points[next].Y - points[next - 1].Y) * t }
}

fn ribbon_visible_outline(points: Vec<PointF>, attached: Option<usize>) -> Vec<PointF> {
    let Some(side) = attached else { return points; };
    let count = points.len().saturating_sub(1);
    if count < 2 { return points; }
    let min_x = points.iter().map(|p| p.X).fold(f32::INFINITY, f32::min);
    let max_x = points.iter().map(|p| p.X).fold(f32::NEG_INFINITY, f32::max);
    let min_y = points.iter().map(|p| p.Y).fold(f32::INFINITY, f32::min);
    let max_y = points.iter().map(|p| p.Y).fold(f32::NEG_INFINITY, f32::max);
    let hidden: Vec<bool> = points.windows(2).map(|pair| match side {
        0 => pair.iter().all(|p| p.X <= min_x + 0.05),
        1 => pair.iter().all(|p| p.Y <= min_y + 0.05),
        2 => pair.iter().all(|p| p.X >= max_x - 0.05),
        3 => pair.iter().all(|p| p.Y >= max_y - 0.05),
        _ => false,
    }).collect();
    let Some(start) = (0..count).find(|i| !hidden[*i] && hidden[(*i + count - 1) % count]) else { return points; };
    let mut visible = vec![points[start]];
    for step in 0..count {
        let index = (start + step) % count;
        if hidden[index] { break; }
        visible.push(points[(index + 1) % count]);
    }
    visible
}

unsafe fn ribbon_range(g: *mut GpGraphics, pen: *mut GpPen, points: &[PointF], distances: &[f32], start: f32, length: f32) {
    let total = *distances.last().unwrap();
    let start = start.rem_euclid(total);
    let draw_part = |from: f32, to: f32| {
        let mut section = vec![ribbon_point(points, distances, from)];
        section.extend(points.iter().zip(distances).filter(|(_, d)| **d > from && **d < to).map(|(p, _)| *p));
        section.push(ribbon_point(points, distances, to));
        GdipDrawLines(g, pen, section.as_ptr(), section.len() as i32);
    };
    draw_part(start, (start + length).min(total));
    if start + length > total { draw_part(0.0, start + length - total); }
}

unsafe fn task_edge_ribbon(g: *mut GpGraphics, d: &Display) {
    if !d.task_light_active { return; }
    let Some(color) = d.task_ribbon_color else { return; };
    let (x, y, w, h, mut top, mut bottom) = if d.vertical {
        (0.0, 0.0, 28.0, 500.0, 12.0, 12.0)
    } else if d.strip {
        (0.0, 0.0, 500.0, 28.0, if d.fusion { 12.0 } else { 0.0 }, if d.fusion { 12.0 } else { 14.0 })
    } else { (12.0, 8.0, 476.0, 70.0, 35.0, 35.0) };
    if d.ribbon_attached == Some(1) { top = 0.0; }
    if d.ribbon_attached == Some(3) { bottom = 0.0; }
    let mut corners = [PointF { X: x, Y: y }, PointF { X: x + w, Y: y + h }];
    GdipTransformPoints(g, CoordinateSpaceDevice, CoordinateSpaceWorld, corners.as_mut_ptr(), 2);
    let scale = ((corners[1].X - corners[0].X) / w).abs();
    let stroke = d.task_ribbon_strength.stroke(scale);
    let (track_alpha, border_alpha, glow_alpha, trail_length, trail_alpha) = d.task_ribbon_strength.appearance();
    let inset = stroke / 2.0 + 0.55;
    let width = (corners[1].X - corners[0].X).round() - 2.0 * inset;
    let height = (corners[1].Y - corners[0].Y).round() - 2.0 * inset;
    if width <= 0.0 || height <= 0.0 { return; }
    let cap = width.min(height) / 2.0;
    let points = ribbon_outline(corners[0].X.round() + inset, corners[0].Y.round() + inset, width, height, (top * scale - inset).clamp(0.0, cap), (bottom * scale - inset).clamp(0.0, cap));
    // Animate only along the visible sides, avoiding a long pause while the
    // moving head would otherwise travel along the hidden screen-facing edge.
    let mut points = ribbon_visible_outline(points, d.ribbon_attached);
    if let Some(side) = d.ribbon_attached {
        // End the open outline at the window edge, rather than at the inset
        // used for the hidden segment. This keeps both side rails flush.
        let last = points.len() - 1;
        for index in [0, last] {
            match side {
                0 => points[index].X = corners[0].X.floor() - stroke,
                1 => points[index].Y = corners[0].Y.floor() - stroke,
                2 => points[index].X = corners[1].X.ceil() + stroke,
                3 => points[index].Y = corners[1].Y.ceil() + stroke,
                _ => {},
            }
        }
    }
    let mut distances = vec![0.0];
    for pair in points.windows(2) {
        distances.push(distances.last().unwrap() + (pair[1].X - pair[0].X).hypot(pair[1].Y - pair[0].Y));
    }
    let total = *distances.last().unwrap();
    if total <= 0.0 { return; }
    let mut state = 0;
    GdipSaveGraphics(g, &mut state);
    GdipResetWorldTransform(g);
    let rgb = color & 0x00ffffff;
    let mut pen = std::ptr::null_mut();
    GdipCreatePen1(rgb | (if d.task_ribbon_rotation.is_some() { track_alpha } else { border_alpha } << 24), stroke, UnitPixel, &mut pen);
    GdipSetPenLineJoin(pen, LineJoinRound);
    GdipSetPenLineCap197819(pen, LineCapRound, LineCapRound, DashCapRound);
    GdipDrawLines(g, pen, points.as_ptr(), points.len() as i32);
    if let Some(angle) = d.task_ribbon_rotation {
        let head = angle / 360.0 * total;
        let trail = total * trail_length;
        let mut glow = std::ptr::null_mut();
        GdipCreatePen1(rgb | (glow_alpha << 24), stroke + 1.8, UnitPixel, &mut glow);
        GdipSetPenLineCap197819(glow, LineCapRound, LineCapRound, DashCapRound);
        ribbon_range(g, glow, &points, &distances, head - trail, trail);
        GdipDeletePen(glow);
        GdipSetPenLineCap197819(pen, LineCapFlat, LineCapFlat, DashCapFlat);
        for index in 0..32 {
            let t = (index + 1) as f32 / 32.0;
            let ramp = if d.task_ribbon_strength == RibbonStrength::Soft { t * t } else { t };
            let alpha = ((trail_alpha + (1.0 - trail_alpha) * ramp) * 255.0).round() as u32;
            GdipSetPenColor(pen, rgb | (alpha << 24));
            ribbon_range(g, pen, &points, &distances, head - trail + index as f32 * trail / 32.0, trail / 32.0);
        }
        let tip = ribbon_point(&points, &distances, head.rem_euclid(total));
        ellipse(g, tip.X - stroke / 2.0, tip.Y - stroke / 2.0, stroke, stroke, color);
    }
    GdipDeletePen(pen);
    GdipRestoreGraphics(g, state);
}

unsafe fn orb(g: *mut GpGraphics, value: Option<f64>) {
    let (x, y, diameter) = (21.0, 76.0, 123.0);
    ellipse(g, x, y, diameter, diameter, 0xff344a51);
    ellipse(
        g,
        x + 4.0,
        y + 4.0,
        diameter - 8.0,
        diameter - 8.0,
        0xff40565e,
    );
    if let Some(value) = value {
        let mut clip = std::ptr::null_mut();
        GdipCreatePath(FillModeAlternate, &mut clip);
        GdipAddPathEllipse(clip, x + 4.0, y + 4.0, diameter - 8.0, diameter - 8.0);
        GdipSetClipPath(g, clip, CombineModeReplace);
        let fill_y = y + diameter * (1.0 - (value / 100.0).clamp(0.0, 1.0)) as f32;
        let start = PointF {
            X: x + 15.0,
            Y: y + 14.0,
        };
        let end = PointF {
            X: x + diameter - 9.0,
            Y: y + diameter,
        };
        let mut gradient = std::ptr::null_mut();
        GdipCreateLineBrush(
            &start,
            &end,
            0xff78edb1,
            0xff38adf5,
            WrapModeTile,
            &mut gradient,
        );
        GdipFillRectangle(
            g,
            gradient as *mut GpBrush,
            x,
            fill_y,
            diameter,
            (y + diameter - fill_y).max(0.0),
        );
        GdipDeleteBrush(gradient as *mut GpBrush);
        GdipResetClip(g);
        GdipDeletePath(clip);
    }
    let mut outline = std::ptr::null_mut();
    GdipCreatePen1(0x997fd0d7, 1.5, UnitPixel, &mut outline);
    GdipDrawEllipse(g, outline, x + 3.0, y + 3.0, diameter - 6.0, diameter - 6.0);
    GdipDeletePen(outline);
    text_center(
        g,
        &percent(value),
        x + 9.0,
        y + 38.0,
        diameter - 18.0,
        49.0,
        34.0,
        true,
        0xffffffff,
    );
    text_center(
        g,
        "剩余",
        x + 9.0,
        y + 81.0,
        diameter - 18.0,
        25.0,
        14.0,
        true,
        0xffecf7fb,
    );
}

unsafe fn quota_card(
    g: *mut GpGraphics,
    y: f32,
    label: &str,
    value: Option<f64>,
    time_left: &str,
    fill: u32,
    accent: u32,
) {
    rounded(g, 153.0, y, 180.0, 53.0, 13.0, fill);
    text(
        g,
        label,
        165.0,
        y + 6.0,
        150.0,
        20.0,
        12.0,
        true,
        0xffaebfc9,
    );
    text(
        g,
        &format!("{} · {}", percent(value), time_left),
        165.0,
        y + 22.0,
        160.0,
        28.0,
        16.0,
        true,
        0xfff5fbff,
    );
    rounded(g, 165.0, y + 47.0, 155.0, 2.0, 1.0, 0x666d8793);
    if let Some(v) = value {
        rounded(
            g,
            165.0,
            y + 47.0,
            (155.0 * (v / 100.0).clamp(0.0, 1.0)) as f32,
            2.0,
            1.0,
            accent,
        );
    }
}

unsafe fn toolbar_button(g: *mut GpGraphics, x: f32, glyph: &str, active: bool) {
    rounded(
        g,
        x,
        17.0,
        24.0,
        24.0,
        8.0,
        if active { 0xff42636a } else { 0xff3a4b55 },
    );
    text_center(g, glyph, x, 18.0, 24.0, 22.0, 15.0, true, 0xffe5f4f8);
}

unsafe fn metric(
    g: *mut GpGraphics,
    x: f32,
    y: f32,
    label: &str,
    value: Option<f64>,
    ideal: Option<f64>,
    velocity: Option<f64>,
    accent: u32,
    compact: bool,
) {
    if compact {
        text(g, label, x, y + 17.0, 46.0, 28.0, 25.0, true, 0xffe9edf4);
        text(
            g,
            &percent(value),
            x + 58.0,
            y + 16.0,
            94.0,
            34.0,
            30.0,
            true,
            accent,
        );
        text(
            g,
            &delta(value, ideal),
            x + 154.0,
            y + 22.0,
            55.0,
            26.0,
            20.0,
            true,
            accent,
        );
        rounded(g, x, y + 55.0, 188.0, 6.0, 3.0, 0xff515761);
        if let Some(v) = value {
            rounded(
                g,
                x,
                y + 55.0,
                (188.0 * (v / 100.0).clamp(0.0, 1.0)) as f32,
                6.0,
                3.0,
                accent,
            );
        }
        if let Some(i) = ideal {
            let at = x + 188.0 * (i / 100.0).clamp(0.0, 1.0) as f32;
            line(g, at, y + 49.0, at, y + 67.0, 3.0, 0xfff7faff);
        }
        if let Some(v) = velocity {
            let at = x + 188.0 * (v / 100.0).clamp(0.0, 1.0) as f32;
            line(g, at, y + 49.0, at, y + 67.0, 2.0, 0xff37d9ff);
        }
    } else {
        text(g, label, x, y, 78.0, 30.0, 18.0, true, 0xffb7c1d0);
        text(
            g,
            &percent(value),
            x + 98.0,
            y - 4.0,
            110.0,
            44.0,
            31.0,
            true,
            accent,
        );
        rounded(g, x, y + 42.0, 316.0, 13.0, 7.0, 0xff444b58);
        if let Some(v) = value {
            rounded(
                g,
                x,
                y + 42.0,
                (316.0 * (v / 100.0).clamp(0.0, 1.0)) as f32,
                13.0,
                7.0,
                accent,
            );
        }
        if let Some(i) = ideal {
            let at = x + 316.0 * (i / 100.0).clamp(0.0, 1.0) as f32;
            line(g, at, y + 37.0, at, y + 60.0, 2.0, 0xffffffff);
        }
        if let Some(v) = velocity {
            let at = x + 316.0 * (v / 100.0).clamp(0.0, 1.0) as f32;
            line(g, at, y + 37.0, at, y + 60.0, 2.0, 0xff37d9ff);
        }
    }
}

unsafe fn mini_metric(
    g: *mut GpGraphics,
    y: f32,
    label: &str,
    value: Option<f64>,
    ideal: Option<f64>,
    velocity: Option<f64>,
    accent: u32,
) {
    notification_text(g, label, 24.0, y + 6.0, 42.0, 27.0, 22.0, true, 0xffe7ebf2, true);
    notification_text(
        g,
        &percent(value),
        70.0,
        y + 2.0,
        94.0,
        34.0,
        28.0,
        true,
        accent,
        true,
    );
    notification_text(
        g,
        &delta(value, ideal),
        163.0,
        y + 10.0,
        58.0,
        25.0,
        18.0,
        true,
        accent,
        true,
    );
    let (x, width) = (238.0, 232.0);
    rounded(g, x, y + 24.0, width, 7.0, 3.5, 0xff333944);
    if let Some(value) = value {
        rounded(
            g,
            x,
            y + 24.0,
            width * (value / 100.0).clamp(0.0, 1.0) as f32,
            7.0,
            3.5,
            accent,
        );
    }
    if let Some(ideal) = ideal {
        let at = x + width * (ideal / 100.0).clamp(0.0, 1.0) as f32;
        line(g, at, y + 19.0, at, y + 36.0, 2.0, 0xfff5f8ff);
        let label_x = (at - 22.0).clamp(x, 448.0);
        percentage_tag(g, label_x, y, &format!("{ideal:.0}%"));
    }
    if let Some(velocity) = velocity {
        let at = x + width * (velocity / 100.0).clamp(0.0, 1.0) as f32;
        line(g, at, y + 19.0, at, y + 36.0, 1.5, 0xff39d9f3);
    }
}

unsafe fn percentage_tag(g: *mut GpGraphics, x: f32, y: f32, content: &str) {
    // A small 44 x 18 badge; draw on device pixels so fractional HUD scaling
    // does not soften the text and rim together.
    let mut corners = [
        PointF { X: x, Y: y },
        PointF {
            X: x + 44.0,
            Y: y + 18.0,
        },
    ];
    GdipTransformPoints(
        g,
        CoordinateSpaceDevice,
        CoordinateSpaceWorld,
        corners.as_mut_ptr(),
        2,
    );
    let left = corners[0].X.round();
    let top = corners[0].Y.round();
    let width = (corners[1].X - corners[0].X).round().max(1.0);
    let height = (corners[1].Y - corners[0].Y).round().max(1.0);
    let mut state = 0;
    GdipSaveGraphics(g, &mut state);
    GdipResetWorldTransform(g);
    rounded(g, left, top, width, height, height / 2.0, 0xff576271);
    rounded(
        g,
        left + 0.65,
        top + 0.65,
        width - 1.3,
        height - 1.3,
        (height - 1.3) / 2.0,
        0xff343b45,
    );
    GdipSetTextRenderingHint(g, TextRenderingHintAntiAliasGridFit);
    let mut family = std::ptr::null_mut();
    let mut font = std::ptr::null_mut();
    let mut suffix_font = std::ptr::null_mut();
    let mut brush = std::ptr::null_mut();
    let mut format = std::ptr::null_mut();
    GdipCreateFontFamilyFromName(
        wide("Segoe UI Black").as_ptr(),
        std::ptr::null_mut(),
        &mut family,
    );
    let mut style = 0;
    if family.is_null() {
        GdipCreateFontFamilyFromName(
            wide("Arial Black").as_ptr(),
            std::ptr::null_mut(),
            &mut family,
        );
    }
    if family.is_null() {
        GdipCreateFontFamilyFromName(wide("Arial").as_ptr(), std::ptr::null_mut(), &mut family);
        style = 1;
    }
    if !family.is_null() {
        GdipCreateFont(family, height * 0.94, style, UnitPixel, &mut font);
        GdipCreateFont(family, height * 0.82, style, UnitPixel, &mut suffix_font);
        GdipCreateSolidFill(0xffffffff, &mut brush);
        GdipStringFormatGetGenericTypographic(&mut format);
        GdipSetStringFormatAlign(format, StringAlignmentNear);
        GdipSetStringFormatLineAlign(format, StringAlignmentCenter);
        GdipSetStringFormatFlags(format, StringFormatFlagsNoWrap);
        let digits: Vec<u16> = content.trim_end_matches('%').encode_utf16().collect();
        let suffix = ['%' as u16];
        let measure = RectF {
            X: 0.0,
            Y: 0.0,
            Width: 200.0,
            Height: 100.0,
        };
        let mut digits_bounds: RectF = std::mem::zeroed();
        let mut suffix_bounds: RectF = std::mem::zeroed();
        GdipMeasureString(
            g,
            digits.as_ptr(),
            digits.len() as i32,
            font,
            &measure,
            format,
            &mut digits_bounds,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        );
        GdipMeasureString(
            g,
            suffix.as_ptr(),
            1,
            suffix_font,
            &measure,
            format,
            &mut suffix_bounds,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        );
        let available = width - 3.0;
        let measured = digits_bounds.Width + suffix_bounds.Width + 0.5;
        if measured > available {
            let fit = available / measured;
            GdipDeleteFont(font);
            GdipDeleteFont(suffix_font);
            GdipCreateFont(family, height * 0.94 * fit, style, UnitPixel, &mut font);
            GdipCreateFont(
                family,
                height * 0.82 * fit,
                style,
                UnitPixel,
                &mut suffix_font,
            );
            GdipMeasureString(
                g,
                digits.as_ptr(),
                digits.len() as i32,
                font,
                &measure,
                format,
                &mut digits_bounds,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            );
            GdipMeasureString(
                g,
                suffix.as_ptr(),
                1,
                suffix_font,
                &measure,
                format,
                &mut suffix_bounds,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            );
        }
        let gap = 0.5;
        let start_x =
            left + ((width - digits_bounds.Width - gap - suffix_bounds.Width) / 2.0).round();
        let bounds = RectF {
            X: start_x,
            Y: top,
            Width: digits_bounds.Width + 1.0,
            Height: height,
        };
        GdipDrawString(
            g,
            digits.as_ptr(),
            digits.len() as i32,
            font,
            &bounds,
            format,
            brush as *const GpBrush,
        );
        let bounds = RectF {
            X: (start_x + digits_bounds.Width + gap).round(),
            Y: top,
            Width: suffix_bounds.Width + 1.0,
            Height: height,
        };
        GdipDrawString(
            g,
            suffix.as_ptr(),
            1,
            suffix_font,
            &bounds,
            format,
            brush as *const GpBrush,
        );
        GdipDeleteStringFormat(format);
        GdipDeleteBrush(brush as *mut GpBrush);
        GdipDeleteFont(font);
        GdipDeleteFont(suffix_font);
        GdipDeleteFontFamily(family);
    }
    GdipRestoreGraphics(g, state);
}

// Snap notification text to physical pixels so small Chinese glyphs stay crisp
// at fractional HUD scaling. Ellipsis is applied to complete lines.
unsafe fn notification_text(g: *mut GpGraphics, content: &str, x: f32, y: f32, w: f32, h: f32, size: f32, bold: bool, color: u32, single_line: bool) {
    notification_text_render(g, content, x, y, w, h, size, bold, color, single_line, TextRenderingHintAntiAliasGridFit);
}

unsafe fn notification_font(size: f32, bold: bool) -> (*mut GpFontFamily, *mut GpFont) {
    for name in ["Noto Sans SC", "Microsoft YaHei UI", "Segoe UI"] {
        // GDI+ cannot reliably draw synthesized bold from variable CJK fonts.
        if bold && name == "Noto Sans SC" { continue; }
        let mut family = std::ptr::null_mut();
        let mut font = std::ptr::null_mut();
        if GdipCreateFontFamilyFromName(wide(name).as_ptr(), std::ptr::null_mut(), &mut family) == 0 && !family.is_null() {
            if GdipCreateFont(family, size, if bold { FontStyleBold } else { FontStyleRegular }, UnitPixel, &mut font) == 0 && !font.is_null() {
                return (family, font);
            }
            if !font.is_null() { GdipDeleteFont(font); }
            GdipDeleteFontFamily(family);
        }
    }
    (std::ptr::null_mut(), std::ptr::null_mut())
}

// Rasterize at the final physical size with grayscale antialiasing, then copy
// 1:1 into the layered window to keep CJK strokes clean without color fringes.
unsafe fn opaque_notification_text(g: *mut GpGraphics, content: &str, x: f32, y: f32, w: f32, h: f32, size: f32, bold: bool, color: u32, single_line: bool, background: u32) {
    let mut points = [PointF { X: x, Y: y }, PointF { X: x + w, Y: y + h }];
    GdipTransformPoints(g, CoordinateSpaceDevice, CoordinateSpaceWorld, points.as_mut_ptr(), 2);
    let width = (points[1].X - points[0].X).round().max(1.0) as i32;
    let height = (points[1].Y - points[0].Y).round().max(1.0) as i32;
    let scale = (points[1].Y - points[0].Y) / h;
    let mut bitmap = std::ptr::null_mut();
    // PixelFormat32bppRGB: fully opaque, allowing ClearType's background blend.
    if GdipCreateBitmapFromScan0(width, height, 0, 0x22009, std::ptr::null(), &mut bitmap) != 0 {
        notification_text(g, content, x, y, w, h, size, bold, color, single_line);
        return;
    }
    let mut tile = std::ptr::null_mut();
    if GdipGetImageGraphicsContext(bitmap as *mut GpImage, &mut tile) == 0 {
        GdipGraphicsClear(tile, crate::theme::color(background));
        notification_text_render(tile, content, 0.0, 0.0, width as f32, height as f32, size * scale, bold, color, single_line, TextRenderingHintAntiAliasGridFit);
        GdipDeleteGraphics(tile);
        let mut state = 0;
        GdipSaveGraphics(g, &mut state);
        GdipResetWorldTransform(g);
        GdipSetInterpolationMode(g, InterpolationModeNearestNeighbor);
        GdipDrawImageRectRectI(g, bitmap as *mut GpImage, points[0].X.round() as i32, points[0].Y.round() as i32, width, height, 0, 0, width, height, UnitPixel, std::ptr::null(), 0, std::ptr::null_mut());
        GdipRestoreGraphics(g, state);
    } else {
        notification_text(g, content, x, y, w, h, size, bold, color, single_line);
    }
    GdipDisposeImage(bitmap as *mut GpImage);
}

unsafe fn notification_text_render(g: *mut GpGraphics, content: &str, x: f32, y: f32, w: f32, h: f32, size: f32, bold: bool, color: u32, single_line: bool, hint: TextRenderingHint) {
    let mut points = [PointF { X: x, Y: y }, PointF { X: x + w, Y: y + h }];
    GdipTransformPoints(g, CoordinateSpaceDevice, CoordinateSpaceWorld, points.as_mut_ptr(), 2);
    let scale = (points[1].Y - points[0].Y) / h;
    let mut state = 0;
    GdipSaveGraphics(g, &mut state);
    GdipResetWorldTransform(g);
    GdipSetTextRenderingHint(g, hint);
    let (family, font) = notification_font((size * scale).round().max(if single_line { 10.0 } else { 12.0 }), bold);
    let mut brush = std::ptr::null_mut();
    let mut format = std::ptr::null_mut();
    if !font.is_null() {
        GdipCreateSolidFill(crate::theme::color(color), &mut brush);
        GdipCreateStringFormat(0, 0, &mut format);
        GdipSetStringFormatFlags(format, StringFormatFlagsLineLimit | if single_line { StringFormatFlagsNoWrap } else { 0 });
        GdipSetStringFormatTrimming(format, StringTrimmingEllipsisCharacter);
        let rect = RectF { X: points[0].X.round(), Y: points[0].Y.round(), Width: (points[1].X - points[0].X).round(), Height: (points[1].Y - points[0].Y).round() };
        let utf16: Vec<u16> = content.encode_utf16().collect();
        GdipDrawString(g, utf16.as_ptr(), utf16.len() as i32, font, &rect, format, brush as *const GpBrush);
        GdipDeleteStringFormat(format);
        GdipDeleteBrush(brush as *mut GpBrush);
        GdipDeleteFont(font);
        GdipDeleteFontFamily(family);
    }
    GdipRestoreGraphics(g, state);
}

const TASK_BODY_SIZE: f32 = 24.0;
const TASK_BODY_LINE: f32 = 36.0;
const TASK_BODY_WIDTH: f32 = 440.0;
pub fn task_notice_height(lines: u8) -> f32 { 132.0 + TASK_BODY_LINE * f32::from(lines.clamp(1, 6)) }

// Match the body renderer's physical pixel size, font, wrap width and line-limit
// flags. Find the smallest number of rows that fits; the setting is only a cap.
pub unsafe fn task_notice_lines(body: &str, max_lines: u8, scale: f32) -> u8 {
    let limit = max_lines.clamp(1, 6);
    if body.is_empty() || limit == 1 { return 1; }
    if !scale.is_finite() || scale <= 0.0 { return limit; }
    let mut bitmap = std::ptr::null_mut();
    if GdipCreateBitmapFromScan0(1, 1, 0, 0x22009, std::ptr::null(), &mut bitmap) != 0 { return limit; }
    let mut graphics = std::ptr::null_mut();
    let mut family = std::ptr::null_mut();
    let mut font = std::ptr::null_mut();
    let mut format = std::ptr::null_mut();
    let mut rows = limit;
    if GdipGetImageGraphicsContext(bitmap as *mut GpImage, &mut graphics) == 0 {
        GdipSetTextRenderingHint(graphics, TextRenderingHintAntiAliasGridFit);
        (family, font) = notification_font((TASK_BODY_SIZE * scale).round().max(12.0), false);
        if !font.is_null() {
            GdipCreateStringFormat(0, 0, &mut format);
            if !font.is_null() && !format.is_null() {
                GdipSetStringFormatFlags(format, StringFormatFlagsLineLimit);
                GdipSetStringFormatTrimming(format, StringTrimmingNone);
                let text: Vec<u16> = body.encode_utf16().collect();
                for candidate in 1..=limit {
                    let rect = RectF { X: 0.0, Y: 0.0, Width: (TASK_BODY_WIDTH * scale).round().max(1.0), Height: (TASK_BODY_LINE * f32::from(candidate) * scale).round().max(1.0) };
                    let mut bounds: RectF = std::mem::zeroed();
                    let mut fitted = 0;
                    let mut lines = 0;
                    if GdipMeasureString(graphics, text.as_ptr(), text.len() as i32, font, &rect, format, &mut bounds, &mut fitted, &mut lines) == 0
                        && fitted >= text.len() as i32 {
                        rows = candidate;
                        break;
                    }
                }
            }
        }
    }
    if !format.is_null() { GdipDeleteStringFormat(format); }
    if !font.is_null() { GdipDeleteFont(font); }
    if !family.is_null() { GdipDeleteFontFamily(family); }
    if !graphics.is_null() { GdipDeleteGraphics(graphics); }
    GdipDisposeImage(bitmap as *mut GpImage);
    rows
}

unsafe fn task_notification(g: *mut GpGraphics, context: &str, body: &str, progress: f32, lines: u8) {
    let body_height = TASK_BODY_LINE * f32::from(lines.clamp(1, 6));
    let progress_y = 124.0 + body_height;
    ellipse(g, 20.0, 16.0, 52.0, 52.0, 0xff103c38);
    ellipse(g, 25.0, 21.0, 42.0, 42.0, 0xff17614d);
    ellipse(g, 28.0, 24.0, 36.0, 36.0, 0xff124b3e);
    line(g, 35.0, 42.0, 43.0, 50.0, 4.0, 0xff74f2b8);
    line(g, 43.0, 50.0, 58.0, 33.0, 4.0, 0xff74f2b8);
    rounded(g, 83.0, 23.0, 264.0, 37.0, 18.5, 0xff103b3b);
    opaque_notification_text(g, "Codex · 本轮结束", 96.0, 25.0, 243.0, 34.0, 24.0, true, 0xff82f5cd, true, 0xff103b3b);
    rounded(g, 449.0, 25.0, 30.0, 30.0, 15.0, 0xff243447);
    line(g, 459.0, 35.0, 469.0, 45.0, 1.8, 0xffd2dbe8);
    line(g, 469.0, 35.0, 459.0, 45.0, 1.8, 0xffd2dbe8);
    opaque_notification_text(g, context, 28.0, 73.0, 444.0, 29.0, 21.0, true, 0xffdce5f1, true, 0xff101d2e);
    rounded(g, 16.0, 107.0, 468.0, body_height + 11.0, 12.0, 0xff233247);
    rounded(g, 17.0, 108.0, 466.0, body_height + 9.0, 11.0, 0xff142234);
    opaque_notification_text(g, body, 30.0, 112.0, TASK_BODY_WIDTH, body_height, TASK_BODY_SIZE, false, 0xffeaf0f7, false, 0xff142234);
    rounded(g, 25.0, progress_y, 450.0, 2.0, 1.0, 0xff293c4c);
    rounded(g, 25.0, progress_y, (450.0 * progress.clamp(0.0, 1.0)).max(1.0), 2.0, 1.0, 0xff58dcb1);
}

pub unsafe fn paint(g: *mut GpGraphics, d: &Display) {
    let _theme = crate::theme::enter(d.theme);
    GdipSetSmoothingMode(g, SmoothingModeAntiAlias);
    GdipSetTextRenderingHint(g, TextRenderingHintAntiAlias);
    if let Some((title, body, important, progress, task)) = &d.notice {
        let accent = if *important { 0xffff7773 } else { 0xffbd80f8 };
        if !d.content_only && *task {
            rounded(g, 0.0, 0.0, 500.0, task_notice_height(d.notice_lines), 20.0, 0xff101d2e);
        } else if !d.content_only {
            rounded(g, 0.0, -12.0, 500.0, 156.0, 22.0, 0xff080a0e);
        }
        if *task {
            let context = title.strip_prefix("QuotaDock · ").unwrap_or(title);
            let context = context.strip_suffix(" · 本轮结束").unwrap_or(context);
            task_notification(g, context, body, *progress, d.notice_lines);
            return;
        } else {
            rounded(g, 22.0, 18.0, 5.0, 22.0, 2.5, accent);
            text(
                g,
                title.strip_prefix("QuotaDock · ").unwrap_or(title),
                39.0,
                13.0,
                393.0,
                31.0,
                23.0,
                true,
                0xfff5f8ff,
            );
            rounded(g, 451.0, 14.0, 27.0, 27.0, 13.5, 0xff282f3a);
            text_center(g, "×", 451.0, 14.0, 27.0, 27.0, 21.0, false, 0xffc4cedb);
            text(g, body, 23.0, 50.0, 454.0, 43.0, 16.0, false, 0xffbdc7d6);
            text(
                g,
                &format!("7d  {}", percent(d.long)),
                24.0,
                104.0,
                111.0,
                26.0,
                18.0,
                true,
                0xffff7773,
            );
            rounded(g, 135.0, 115.0, 88.0, 4.0, 2.0, 0xff343842);
            if let Some(value) = d.long {
                rounded(
                    g,
                    135.0,
                    115.0,
                    88.0 * (value / 100.0).clamp(0.0, 1.0) as f32,
                    4.0,
                    2.0,
                    0xffff7773,
                );
            }
            text(
                g,
                &format!("5h  {}", percent(d.short)),
                270.0,
                104.0,
                111.0,
                26.0,
                18.0,
                true,
                0xffbd80f8,
            );
            rounded(g, 381.0, 115.0, 94.0, 4.0, 2.0, 0xff343842);
            if let Some(value) = d.short {
                rounded(
                    g,
                    381.0,
                    115.0,
                    94.0 * (value / 100.0).clamp(0.0, 1.0) as f32,
                    4.0,
                    2.0,
                    0xffbd80f8,
                );
            }
        }
        rounded(g, 23.0, 138.0, 454.0, 2.0, 1.0, 0xff272e39);
        rounded(
            g,
            23.0,
            138.0,
            (454.0 * progress.clamp(0.0, 1.0)).max(1.0),
            2.0,
            1.0,
            accent,
        );
        return;
    }
    if d.compact {
        if d.vertical && !d.expanded {
            if !d.content_only {
                rounded(g, 0.0, 0.0, 28.0, 500.0, 12.0, 0xff080a0e);
            }
            for (x, value, ideal, velocity, accent) in [
                (6.0, d.long, d.long_ideal, d.long_velocity, 0xffff6661),
                (18.0, d.short, d.short_ideal, d.short_velocity, 0xffbc70f4),
            ] {
                ellipse(g, x - 0.5, 10.0, 5.0, 5.0, accent);
                rounded(g, x, 28.0, 4.0, 444.0, 2.0, 0xff333944);
                if let Some(value) = value {
                    let fill = 444.0 * (value / 100.0).clamp(0.0, 1.0) as f32;
                    rounded(g, x, 472.0 - fill, 4.0, fill, 2.0, accent);
                }
                if let Some(ideal) = ideal {
                    let y = 472.0 - 444.0 * (ideal / 100.0).clamp(0.0, 1.0) as f32;
                    line(g, x - 2.0, y, x + 6.0, y, 1.5, 0xfff6f9ff);
                }
                if let Some(velocity) = velocity {
                    let y = 472.0 - 444.0 * (velocity / 100.0).clamp(0.0, 1.0) as f32;
                    line(g, x - 2.0, y, x + 6.0, y, 1.0, 0xff39d9f3);
                }
            }
            task_status_light(g, 6.0, 483.0, true, d.task_state_color, d.task_light_style, d.task_light_rotation);
            task_edge_ribbon(g, d);
            return;
        }
        if d.expanded {
            let top = if d.strip { -10.0 } else { 0.0 };
            if !d.content_only {
                rounded(g, 0.0, top, 500.0, 172.0 - top, 20.0, 0xff080a0e);
            }
            mini_metric(
                g,
                9.0,
                "7d",
                d.long,
                d.long_ideal,
                d.long_velocity,
                0xffff6661,
            );
            mini_metric(
                g,
                55.0,
                "5h",
                d.short,
                d.short_ideal,
                d.short_velocity,
                0xffbc70f4,
            );
            line(g, 24.0, 101.0, 476.0, 101.0, 1.0, 0xff252e39);
            rounded(g, 16.0, 111.0, 230.0, 59.0, 9.0, 0xff121820);
            rounded(g, 254.0, 111.0, 230.0, 59.0, 9.0, 0xff121820);
            for (x, label, reset, accent) in [
                (28.0, "7 天重置", &d.long_reset, 0xffff8c87),
                (266.0, "5 小时重置", &d.short_reset, 0xffc99bff),
            ] {
                notification_text(g, label, x, 115.0, 206.0, 23.0, 16.0, true, accent, true);
                notification_text(g, reset, x, 137.0, 206.0, 30.0, 22.0, true, 0xfff4f7fc, true);
            }
            return;
        }
        if d.strip {
            if !d.content_only {
                if d.fusion { rounded(g, 0.0, 0.0, 500.0, 28.0, 12.0, 0xff080a0e); }
                else { rounded(g, 0.0, -10.0, 500.0, 38.0, 18.0, 0xf6020203); }
            }
            let text_inset = if d.task_ribbon_color.is_some() { 4.0 } else { 0.0 };
            // Balance the visible glyphs against the lower rounded edge.
            // Keep the shell and ribbon anchored while lifting only the content.
            let mut content_state = 0;
            GdipSaveGraphics(g, &mut content_state);
            GdipTranslateWorldTransform(g, 0.0, -1.5, MatrixOrderPrepend);
            text(g, "7d", 16.0 + text_inset, 1.0, 40.0, 24.0, 20.0, true, 0xffe9edf4);
            text(
                g,
                &percent(d.long),
                57.0 + text_inset,
                0.0,
                68.0,
                25.0,
                21.0,
                true,
                0xffff6661,
            );
            rounded(g, 132.0, 11.0, 94.0, 6.0, 3.0, 0xff515761);
            if let Some(v) = d.long {
                rounded(
                    g,
                    132.0,
                    11.0,
                    (94.0 * (v / 100.0).clamp(0.0, 1.0)) as f32,
                    6.0,
                    3.0,
                    0xffff6661,
                );
            }
            text(g, "5h", 264.0 + text_inset, 1.0, 40.0, 24.0, 20.0, true, 0xffe9edf4);
            text(
                g,
                &percent(d.short),
                305.0 + text_inset,
                0.0,
                68.0,
                25.0,
                21.0,
                true,
                0xffbc70f4,
            );
            rounded(g, 380.0, 11.0, 94.0, 6.0, 3.0, 0xff515761);
            if let Some(v) = d.short {
                rounded(
                    g,
                    380.0,
                    11.0,
                    (94.0 * (v / 100.0).clamp(0.0, 1.0)) as f32,
                    6.0,
                    3.0,
                    0xffbc70f4,
                );
            }
            task_status_light(g, 483.0, 6.0, false, d.task_state_color, d.task_light_style, d.task_light_rotation);
            GdipRestoreGraphics(g, content_state);
            task_edge_ribbon(g, d);
            return;
        } else {
            let bg = if d.island { 0xf5020203 } else { 0xdf242a35 };
            if !d.content_only {
                rounded(g, 12.0, 8.0, 476.0, 70.0, 35.0, bg);
                rounded(
                    g,
                    15.0,
                    11.0,
                    470.0,
                    23.0,
                    12.0,
                    if d.island { 0x12000000 } else { 0x21ffffff },
                );
            }
            metric(
                g,
                34.0,
                0.0,
                "7d",
                d.long,
                d.long_ideal,
                d.long_velocity,
                0xffff6661,
                true,
            );
            metric(
                g,
                270.0,
                0.0,
                "5h",
                d.short,
                d.short_ideal,
                d.short_velocity,
                0xffbc70f4,
                true,
            );
            task_status_light(g, 477.0, 35.0, false, d.task_state_color, d.task_light_style, d.task_light_rotation);
            task_edge_ribbon(g, d);
        }
    } else {
        if !d.content_only {
            rounded(g, 0.0, 0.0, 350.0, 218.0, 22.0, 0xff25313a);
        }
        rounded(g, 1.0, 1.0, 348.0, 52.0, 21.0, 0xff31434d);
        let status_color = if d.status.contains("耗尽") || d.status.contains("减速") {
            0xffff6868
        } else if d.status.contains("偏快") {
            0xffffc766
        } else {
            0xff65e0a1
        };
        ellipse(g, 14.0, 23.0, 16.0, 16.0, 0x4478edbb);
        ellipse(g, 18.0, 27.0, 8.0, 8.0, status_color);
        text(
            g,
            "QuotaDock",
            35.0,
            9.0,
            160.0,
            27.0,
            18.0,
            true,
            0xfff7fbff,
        );
        text(
            g, &d.status, 35.0, 32.0, 173.0, 20.0, 11.0, true, 0xffbcd0d8,
        );
        toolbar_button(g, 249.0, "置", d.pinned);
        toolbar_button(g, 278.0, "刷", false);
        toolbar_button(g, 307.0, "×", false);
        orb(g, d.short.or(d.long));
        quota_card(
            g,
            62.0,
            "5 小时窗口",
            d.short,
            &d.short_left,
            0xff28514d,
            0xff61e0bc,
        );
        quota_card(
            g,
            121.0,
            "7 天窗口",
            d.long,
            &d.long_left,
            0xff36474f,
            0xff7cb8ef,
        );
        rounded(g, 153.0, 181.0, 180.0, 27.0, 10.0, 0xff34434d);
        text(g, "套餐", 165.0, 184.0, 52.0, 21.0, 12.0, true, 0xffb5c5ce);
        text(g, &d.plan, 221.0, 182.0, 97.0, 24.0, 15.0, true, 0xffeef8fb);
    }
}
