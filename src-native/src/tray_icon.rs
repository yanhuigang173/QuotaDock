pub fn quantize(percent: Option<f64>, step: u8) -> Option<u8> {
    percent.filter(|value| value.is_finite()).map(|value| {
        if value >= 100.0 {
            100
        } else {
            ((value.clamp(0.0, 100.0) / f64::from(step)).round() * f64::from(step))
                .clamp(0.0, 100.0) as u8
        }
    })
}

fn rounded(x: f64, y: f64, left: f64, top: f64, right: f64, bottom: f64, radius: f64) -> bool {
    let radius = radius.min((right - left) / 2.0).min((bottom - top) / 2.0);
    let nearest_x = x.clamp(left + radius, right - radius);
    let nearest_y = y.clamp(top + radius, bottom - radius);
    (x - nearest_x).powi(2) + (y - nearest_y).powi(2) <= radius.powi(2)
}

fn sample(x: f64, y: f64, short: Option<u8>, long: Option<u8>, stale: bool) -> [u8; 4] {
    if !rounded(x, y, 0.15, 0.15, 15.85, 15.85, 4.15) {
        return [0, 0, 0, 0];
    }
    let mut color = [0x17, 0x1a, 0x1f, 255];
    if rounded(x, y, 0.65, 0.65, 15.35, 15.35, 3.7) {
        color = [0x45, 0x4b, 0x55, 255];
    }
    if rounded(x, y, 1.05, 1.05, 14.95, 14.95, 3.45) {
        color = [0x25, 0x29, 0x30, 255];
    }

    for (top, percent, active) in [
        (5.0, short, [0xbc, 0x70, 0xf4, 255]),
        (9.05, long, [0xff, 0x66, 0x61, 255]),
    ] {
        let track = [0x58, 0x5e, 0x68, 255];
        // Cached quota is still useful: retain the quota colors and indicate
        // refresh trouble on the status dot instead of graying out both bars.
        let fill = active;
        if rounded(x, y, 3.0, top, 11.1, top + 2.15, 1.07) {
            color = track;
        }
        if let Some(percent) = percent.filter(|value| *value > 0) {
            let end = 3.0 + 8.1 * f64::from(percent) / 100.0;
            if rounded(x, y, 3.0, top, end, top + 2.15, 1.07) {
                color = fill;
            }
        }
        if rounded(x, y, 12.45, top + 0.02, 14.55, top + 2.12, 1.05) {
            color = if percent.is_some() {
                if stale { [0xf2, 0xb9, 0x55, 255] } else { fill }
            } else {
                [0x67, 0x6d, 0x76, 255]
            };
        }
    }
    color
}

pub fn render(size: u32, short: Option<u8>, long: Option<u8>, stale: bool) -> Vec<u8> {
    let mut rgba = vec![0; (size * size * 4) as usize];
    let supersample = 4;
    for py in 0..size {
        for px in 0..size {
            let mut total = [0_u32; 4];
            for sy in 0..supersample {
                for sx in 0..supersample {
                    let x = (f64::from(px) + (f64::from(sx) + 0.5) / f64::from(supersample)) * 16.0
                        / f64::from(size);
                    let y = (f64::from(py) + (f64::from(sy) + 0.5) / f64::from(supersample)) * 16.0
                        / f64::from(size);
                    let color = sample(x, y, short, long, stale);
                    for channel in 0..4 {
                        total[channel] += u32::from(color[channel]);
                    }
                }
            }
            let offset = ((py * size + px) * 4) as usize;
            for channel in 0..4 {
                rgba[offset + channel] = (total[channel] / (supersample * supersample)) as u8;
            }
        }
    }
    rgba
}
