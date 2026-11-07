use std::time::{Duration, Instant};

#[derive(Clone)]
pub struct Frame {
    pub width: i32,
    pub height: i32,
    pub pixels: Vec<u8>,
}

pub struct Transition {
    pub from: Frame,
    pub to: Frame,
    pub started: Instant,
    pub duration: Duration,
    pub from_shell: Shell,
    pub to_shell: Shell,
}

#[derive(Clone, Copy)]
pub struct Shell {
    pub color: u32,
    pub top_radius: f32,
    pub bottom_radius: f32,
    pub insets: [f32; 4],
    pub background_inset: [f32; 4],
}

impl Shell {
    pub fn interpolate(self, to: Self, progress: f32) -> Self {
        let mut color = 0;
        for shift in [0, 8, 16, 24] {
            let a = ((self.color >> shift) & 255) as f32;
            let b = ((to.color >> shift) & 255) as f32;
            color |= ((a + (b - a) * progress).round() as u32) << shift;
        }
        Self {
            color,
            top_radius: self.top_radius + (to.top_radius - self.top_radius) * progress,
            bottom_radius: self.bottom_radius + (to.bottom_radius - self.bottom_radius) * progress,
            insets: std::array::from_fn(|index| {
                self.insets[index] + (to.insets[index] - self.insets[index]) * progress
            }),
            background_inset: std::array::from_fn(|index| self.background_inset[index] + (to.background_inset[index] - self.background_inset[index]) * progress),
        }
    }
}

pub fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

impl Transition {
    pub fn progress(&self) -> f32 {
        (self.started.elapsed().as_secs_f32() / self.duration.as_secs_f32()).min(1.0)
    }
    pub fn height(&self) -> i32 {
        (self.from.height as f32
            + (self.to.height - self.from.height) as f32 * ease(self.progress()))
        .round() as i32
    }
    pub fn width(&self) -> i32 {
        (self.from.width as f32 + (self.to.width - self.from.width) as f32 * ease(self.progress()))
            .round() as i32
    }
    pub fn frame_into(&self, pixels: Vec<u8>) -> Frame {
        blend_with_buffer(&self.from, &self.to, ease(self.progress()), pixels)
    }
    pub fn shell(&self) -> Shell {
        self.from_shell
            .interpolate(self.to_shell, ease(self.progress()))
    }
}

fn pixel(frame: &Frame, x: i32, y: i32) -> [u8; 4] {
    if x < 0 || x >= frame.width || y < 0 || y >= frame.height {
        return [0, 0, 0, 0];
    }
    let at = ((y * frame.width + x) * 4) as usize;
    frame.pixels[at..at + 4].try_into().unwrap()
}

// Frames use premultiplied BGRA, just like UpdateLayeredWindow. Dimensions ease
// together while text stays on the pixel grid and crossfades between layouts.
#[allow(dead_code)]
pub fn blend(from: &Frame, to: &Frame, progress: f32, _radius: f32, _flat_top: bool) -> Frame {
    blend_with_buffer(from, to, progress, Vec::new())
}

fn blend_with_buffer(from: &Frame, to: &Frame, progress: f32, mut pixels: Vec<u8>) -> Frame {
    if progress <= 0.0 {
        return from.clone();
    }
    if progress >= 1.0 {
        return to.clone();
    }
    let height = (from.height as f32 + (to.height - from.height) as f32 * progress)
        .round()
        .max(1.0) as i32;
    let width = (from.width as f32 + (to.width - from.width) as f32 * progress)
        .round()
        .max(1.0) as i32;
    pixels.resize((width * height * 4) as usize, 0);
    // Fade through the shared shell instead of overlapping two layouts at 50%.
    let old_weight = ((1.0 - ease((progress / 0.5).min(1.0))) * 256.0).round() as u32;
    let new_weight = (ease(((progress - 0.25) / 0.75).max(0.0)) * 256.0).round() as u32;
    for y in 0..height {
        for x in 0..width {
            let a = pixel(from, x, y);
            let b = pixel(to, x, y);
            let at = ((y * width + x) * 4) as usize;
            for channel in 0..4 {
                pixels[at + channel] = ((u32::from(a[channel]) * old_weight
                    + u32::from(b[channel]) * new_weight)
                    / 256) as u8;
            }
        }
    }
    Frame {
        width,
        height,
        pixels,
    }
}

// Draw exactly one shell at the current animated height, then composite the
// transparent content over it. No old rounded edge can remain across the middle.
#[allow(dead_code)]
pub fn compose_into(frame: &Frame, shell: Shell, output: &mut [u8]) {
    compose_strided(frame, shell, output, frame.width as usize * 4);
}

fn background_coverage(x: f32, y: f32, width: f32, height: f32, top: f32, bottom: f32) -> u32 {
    if x < 0.0 || x >= width || y < 0.0 || y >= height { return 0; }
    let radius = if y < top { top } else { bottom };
    let dx = if x < radius { radius - x - 0.5 } else if x > width - radius { x + 0.5 - (width - radius) } else { 0.0 };
    let dy = if y < top { top - y - 0.5 } else if y > height - bottom { y + 0.5 - (height - bottom) } else { 0.0 };
    if dx > 0.0 && dy > 0.0 { ((radius - dx.hypot(dy) + 0.5).clamp(0.0, 1.0) * 256.0) as u32 } else { 256 }
}

pub fn compose_strided(frame: &Frame, shell: Shell, output: &mut [u8], stride: usize) {
    let alpha = (shell.color >> 24) & 255;
    let background = [
        ((shell.color & 255) * alpha / 255),
        (((shell.color >> 8) & 255) * alpha / 255),
        (((shell.color >> 16) & 255) * alpha / 255),
        alpha,
    ];
    let width = frame.width as f32 - shell.insets[0] - shell.insets[2];
    let height = frame.height as f32 - shell.insets[1] - shell.insets[3];
    let half = width.min(height).max(0.0) / 2.0;
    let top = shell.top_radius.min(half);
    let bottom = shell.bottom_radius.min(half);
    let padding = shell.background_inset.map(|value| value.clamp(0.0, half));
    let has_padding = padding.iter().any(|value| *value > 0.0);
    let inner_width = width - padding[0] - padding[2];
    let inner_height = height - padding[1] - padding[3];
    let inner_half = inner_width.min(inner_height).max(0.0) / 2.0;
    let inner_top = (top - padding[1]).clamp(0.0, inner_half);
    let inner_bottom = (bottom - padding[3]).clamp(0.0, inner_half);
    for y in 0..frame.height {
        let local_y = y as f32 - shell.insets[1];
        let radius = if local_y < top { top } else { bottom };
        let dy = if local_y < top {
            top - local_y - 0.5
        } else if local_y > height - bottom {
            local_y + 0.5 - (height - bottom)
        } else {
            0.0
        };
        for x in 0..frame.width {
            let local_x = x as f32 - shell.insets[0];
            let dx = if local_x < radius {
                radius - local_x - 0.5
            } else if local_x > width - radius {
                local_x + 0.5 - (width - radius)
            } else {
                0.0
            };
            let mask = if local_x < 0.0 || local_x >= width || local_y < 0.0 || local_y >= height {
                0
            } else if dx > 0.0 && dy > 0.0 {
                ((radius - (dx * dx + dy * dy).sqrt() + 0.5).clamp(0.0, 1.0) * 256.0) as u32
            } else {
                256
            };
            let at = ((y * frame.width + x) * 4) as usize;
            let target = y as usize * stride + x as usize * 4;
            let inverse = 255 - u32::from(frame.pixels[at + 3]);
            // Keep the ribbon outside the dark body while retaining the same
            // outer window bounds and rounded clipping throughout transitions.
            let body_mask = if has_padding {
                background_coverage(local_x - padding[0], local_y - padding[1], inner_width, inner_height, inner_top, inner_bottom)
            } else { 256 };
            for channel in 0..4 {
                let value =
                    u32::from(frame.pixels[at + channel]) + background[channel] * body_mask / 256 * inverse / 255;
                output[target + channel] = (value.min(255) * mask / 256) as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn frame(height: i32, color: u8) -> Frame {
        Frame {
            width: 32,
            height,
            pixels: vec![color; 32 * height as usize * 4],
        }
    }
    #[test]
    fn width_stays_fixed_and_endpoints_match() {
        let a = frame(8, 60);
        let b = frame(28, 180);
        for step in 0..=60 {
            let image = blend(&a, &b, ease(step as f32 / 60.0), 4.0, true);
            assert_eq!(image.width, 32);
            assert!((8..=28).contains(&image.height));
        }
        assert_eq!(blend(&a, &b, 0.0, 4.0, true).pixels, a.pixels);
        assert_eq!(blend(&a, &b, 1.0, 4.0, true).pixels, b.pixels);
    }
    #[test]
    fn reusable_surface_stride_keeps_rows_aligned() {
        let image = Frame {
            width: 8,
            height: 6,
            pixels: vec![0; 8 * 6 * 4],
        };
        let shell = Shell {
            color: 0xff123456,
            top_radius: 0.0,
            bottom_radius: 0.0,
            insets: [0.0; 4],
            background_inset: [0.0; 4],
        };
        let mut tight = vec![0; image.pixels.len()];
        let mut padded = vec![77; 20 * 6 * 4];
        compose_into(&image, shell, &mut tight);
        compose_strided(&image, shell, &mut padded, 20 * 4);
        for row in 0..6 {
            assert_eq!(
                &tight[row * 32..(row + 1) * 32],
                &padded[row * 80..row * 80 + 32]
            );
            assert!(padded[row * 80 + 32..(row + 1) * 80]
                .iter()
                .all(|byte| *byte == 77));
        }
    }
    #[test]
    fn vertical_strip_morphs_in_both_dimensions() {
        let a = Frame {
            width: 20,
            height: 345,
            pixels: vec![60; 20 * 345 * 4],
        };
        let b = Frame {
            width: 345,
            height: 121,
            pixels: vec![180; 345 * 121 * 4],
        };
        let mut last_width = a.width;
        let mut last_height = a.height;
        for step in 0..=60 {
            let image = blend(&a, &b, ease(step as f32 / 60.0), 0.0, false);
            assert!(image.width >= last_width && image.height <= last_height);
            assert_eq!(
                image.pixels.len(),
                (image.width * image.height * 4) as usize
            );
            last_width = image.width;
            last_height = image.height;
        }
        assert_eq!((last_width, last_height), (b.width, b.height));
    }
    #[test]
    fn interrupted_animation_starts_from_the_visible_frame() {
        let a = frame(8, 60);
        let b = frame(28, 180);
        let middle = blend(&a, &b, 0.4, 4.0, true);
        let reverse_start = blend(&middle, &a, 0.0, 4.0, true);
        assert_eq!(reverse_start.height, middle.height);
        assert_eq!(reverse_start.pixels, middle.pixels);
        assert!(ease(0.01) < 0.0001 && 1.0 - ease(0.99) < 0.0001);
    }
    #[test]
    fn growing_shell_has_no_old_bottom_edge_or_seam() {
        let from = Frame {
            width: 64,
            height: 12,
            pixels: vec![0; 64 * 12 * 4],
        };
        let to = Frame {
            width: 64,
            height: 48,
            pixels: vec![0; 64 * 48 * 4],
        };
        let content = blend(&from, &to, 0.5, 8.0, true);
        let mut output = vec![0; content.pixels.len()];
        compose_into(
            &content,
            Shell {
                color: 0xff080a0e,
                top_radius: 0.0,
                bottom_radius: 8.0,
                insets: [0.0; 4],
            background_inset: [0.0; 4],
            },
            &mut output,
        );
        for y in 8..20 {
            let at = ((y * 64 + 32) * 4) as usize;
            assert_eq!(&output[at..at + 4], &[14, 10, 8, 255]);
        }
    }
}
