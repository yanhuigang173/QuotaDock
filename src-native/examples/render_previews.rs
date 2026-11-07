#[path = "../src/draw.rs"]
mod draw;
#[allow(dead_code)]
#[path = "../src/motion.rs"]
mod motion;
#[path = "../src/theme.rs"]
mod theme;

use std::{path::Path, ptr::{null, null_mut}};
use windows_sys::{core::GUID, Win32::Graphics::GdiPlus::*};

unsafe fn render(path: &Path, display: &draw::Display, scale: f32, height: f32) -> motion::Frame {
    let mut display = display.clone();
    let height = if let Some((_, body, _, _, true)) = &display.notice {
        display.notice_lines = draw::task_notice_lines(body, display.notice_lines, scale);
        draw::task_notice_height(display.notice_lines)
    } else { height };
    let width = ((if display.vertical && !display.expanded && display.notice.is_none() { 28.0 } else { 500.0 }) * scale).round() as i32;
    let height = (height * scale).round() as i32;
    let ribbon_shell = if display.compact && !display.expanded && display.notice.is_none()
        && ((display.task_light_active && display.task_ribbon_color.is_some()) || display.ribbon_attached.is_some()) {
        let strip = display.strip || display.vertical;
        Some(motion::Shell {
            color: display.theme.color(if strip { 0xff080a0e } else if display.island { 0xf5020203 } else { 0xdf242a35 }),
            top_radius: (if display.ribbon_attached == Some(1) { 0.0 } else if strip { if display.fusion { 12.0 } else { 0.0 } } else { 35.0 }) * scale,
            bottom_radius: (if display.ribbon_attached == Some(3) { 0.0 } else if strip { if display.fusion { 12.0 } else { 20.0 } } else { 35.0 }) * scale,
            insets: if strip { [0.0; 4] } else { [12.0 * scale, 8.0 * scale, 12.0 * scale, 8.0 * scale] },
            background_inset: {
                let mut padding = if display.task_light_active && display.task_ribbon_color.is_some() { [display.task_ribbon_strength.body_inset(scale); 4] } else { [0.0; 4] };
                if let Some(side) = display.ribbon_attached { padding[side] = 0.0; }
                padding
            },
        })
    } else { None };
    if ribbon_shell.is_some() { display.content_only = true; }
    let mut pixels = vec![0u8; (width * height * 4) as usize];
    let mut bitmap = null_mut();
    assert_eq!(GdipCreateBitmapFromScan0(width, height, width * 4,
        0x26200a, pixels.as_mut_ptr(), &mut bitmap), 0);
    let mut graphics = null_mut();
    assert_eq!(GdipGetImageGraphicsContext(bitmap as *mut GpImage, &mut graphics), 0);
    GdipGraphicsClear(graphics, 0);
    GdipScaleWorldTransform(graphics, scale, scale, MatrixOrderAppend);
    draw::paint(graphics, &display);
    GdipDeleteGraphics(graphics);
    if let Some(shell) = ribbon_shell {
        let mut content = pixels.clone();
        for pixel in content.chunks_exact_mut(4) {
            for channel in 0..3 { pixel[channel] = (u32::from(pixel[channel]) * u32::from(pixel[3]) / 255) as u8; }
        }
        let frame = motion::Frame { width, height, pixels: content };
        motion::compose_into(&frame, shell, &mut pixels);
        // The PNG bitmap uses straight alpha; the runtime compositor uses
        // premultiplied BGRA. Convert back for a faithful static preview.
        for pixel in pixels.chunks_exact_mut(4) {
            if pixel[3] > 0 && pixel[3] < 255 {
                for channel in 0..3 { pixel[channel] = ((u32::from(pixel[channel]) * 255 + u32::from(pixel[3]) / 2) / u32::from(pixel[3])).min(255) as u8; }
            }
        }
    }
    let png = GUID { data1: 0x557cf406, data2: 0x1a04, data3: 0x11d3, data4: [0x9a, 0x73, 0, 0, 0xf8, 0x1e, 0xf3, 0x2e] };
    let filename: Vec<u16> = path.as_os_str().to_string_lossy().encode_utf16().chain(Some(0)).collect();
    assert_eq!(GdipSaveImageToFile(bitmap as *mut GpImage, filename.as_ptr(), &png, null()), 0);
    GdipDisposeImage(bitmap as *mut GpImage);
    motion::Frame { width, height, pixels }
}

unsafe fn save_frame(path: &Path, frame: &motion::Frame) {
    let mut bitmap = null_mut();
    assert_eq!(GdipCreateBitmapFromScan0(frame.width, frame.height, frame.width * 4, 0x26200a, frame.pixels.as_ptr(), &mut bitmap), 0);
    let png = GUID { data1: 0x557cf406, data2: 0x1a04, data3: 0x11d3, data4: [0x9a, 0x73, 0, 0, 0xf8, 0x1e, 0xf3, 0x2e] };
    let filename: Vec<u16> = path.as_os_str().to_string_lossy().encode_utf16().chain(Some(0)).collect();
    assert_eq!(GdipSaveImageToFile(bitmap as *mut GpImage, filename.as_ptr(), &png, null()), 0);
    GdipDisposeImage(bitmap as *mut GpImage);
}

fn main() {
    let output = std::env::args().nth(1).unwrap_or_else(|| "assets/ui-previews".into());
    std::fs::create_dir_all(&output).unwrap();
    let mut display = draw::Display {
        theme: theme::Theme::Default,
        compact: true, fusion: true, strip: true, vertical: false, expanded: true, island: true, pinned: true,
        short: Some(93.0), long: Some(84.0), short_ideal: Some(96.0), long_ideal: Some(97.0),
        short_velocity: None, long_velocity: None,
        short_reset: "10/20 23:33".into(), long_reset: "10/27 14:33".into(),
        short_left: "3h 30m".into(), long_left: "162h 28m".into(),
        plan: "PLUS".into(), status: "近期偏快".into(), notice: None, notice_lines: 2, task_state_color: Some(0xff4f9dff), task_light_style: draw::StatusLightStyle::Bar, task_light_rotation: None, task_ribbon_color: None, task_ribbon_rotation: None, task_ribbon_strength: draw::RibbonStrength::Standard, ribbon_attached: None, task_light_active: true, content_only: false,
    };
    unsafe {
        let mut token = 0;
        let input = GdiplusStartupInput { GdiplusVersion: 1, DebugEventCallback: 0, SuppressBackgroundThread: 0, SuppressExternalCodecs: 0 };
        assert_eq!(GdiplusStartup(&mut token, &input, null_mut()), 0);
        let scale = 0.46 * 1.5;
        render(&Path::new(&output).join("hover-topbar.png"), &display, scale, 176.0);
        display.expanded = false;
        render(&Path::new(&output).join("collapsed-topbar.png"), &display, scale, 28.0);
        for (state, color) in [("idle", 0xff626d7a), ("running", 0xff4f9dff), ("complete", 0xff9bea58), ("interrupted", 0xffffab62)] {
            display.task_ribbon_color = None;
            display.task_ribbon_rotation = None;
            display.task_light_style = draw::StatusLightStyle::Bar;
            display.task_light_rotation = None;
            display.task_state_color = Some(color);
            render(&Path::new(&output).join(format!("task-status-{state}.png")), &display, scale, 28.0);
            display.task_light_style = draw::StatusLightStyle::Dot;
            display.task_light_rotation = (state == "running").then_some(30.0);
            render(&Path::new(&output).join(format!("task-dot-{state}.png")), &display, scale, 28.0);
            display.task_ribbon_color = Some(color);
            display.task_ribbon_rotation = (state == "running").then_some(30.0);
            display.task_state_color = None;
            display.task_light_active = state != "idle";
            render(&Path::new(&output).join(format!("task-ribbon-{state}.png")), &display, scale, 28.0);
        }
        display.task_state_color = Some(0xff4f9dff);
        display.task_ribbon_color = None;
        display.task_ribbon_rotation = None;
        display.task_light_active = true;
        display.task_light_style = draw::StatusLightStyle::Dot;
        for angle in [0, 90, 180, 270] {
            display.task_light_rotation = Some(angle as f32);
            render(&Path::new(&output).join(format!("task-dot-spin-{angle}.png")), &display, scale, 28.0);
        }
        display.task_ribbon_color = Some(0xff4f9dff);
        display.task_state_color = None;
        display.task_light_rotation = None;
        for angle in [0, 90, 180, 270] {
            display.task_ribbon_rotation = Some(angle as f32);
            render(&Path::new(&output).join(format!("task-ribbon-flow-{angle}.png")), &display, scale, 28.0);
        }
        display.task_state_color = Some(0xff4f9dff);
        display.task_light_rotation = Some(35.0);
        render(&Path::new(&output).join("task-ribbon-dot-together.png"), &display, scale, 28.0);
        display.task_light_style = draw::StatusLightStyle::Bar;
        display.task_light_rotation = None;
        render(&Path::new(&output).join("task-ribbon-bar-together.png"), &display, scale, 28.0);
        display.task_state_color = Some(0xff9bea58);
        display.task_ribbon_color = Some(0xff9bea58);
        display.task_ribbon_rotation = None;
        for (name, side) in [("top", 1), ("bottom", 3)] {
            display.ribbon_attached = Some(side);
            render(&Path::new(&output).join(format!("task-ribbon-attached-{name}.png")), &display, scale, 28.0);
        }
        display.task_state_color = Some(0xff4f9dff);
        display.task_ribbon_color = Some(0xff4f9dff);
        display.task_ribbon_rotation = Some(120.0);
        display.ribbon_attached = Some(1);
        render(&Path::new(&output).join("task-ribbon-attached-flow.png"), &display, scale, 28.0);
        for (name, strength) in [("soft", draw::RibbonStrength::Soft), ("standard", draw::RibbonStrength::Standard), ("strong", draw::RibbonStrength::Strong), ("intense", draw::RibbonStrength::Intense)] {
            display.task_ribbon_strength = strength;
            render(&Path::new(&output).join(format!("task-ribbon-strength-{name}.png")), &display, scale, 28.0);
        }
        display.task_ribbon_strength = draw::RibbonStrength::Standard;
        display.ribbon_attached = None;
        display.vertical = true;
        render(&Path::new(&output).join("task-ribbon-vertical.png"), &display, scale, 500.0);
        display.vertical = false;
        display.strip = false;
        render(&Path::new(&output).join("task-ribbon-hud.png"), &display, scale, 86.0);
        display.strip = true;
        display.task_light_style = draw::StatusLightStyle::Bar;
        display.task_ribbon_color = None;
        display.task_ribbon_rotation = None;
        display.task_light_rotation = None;
        display.task_state_color = Some(0xff4f9dff);
        display.notice = Some(("QuotaDock · 7 天额度已重置".into(), "当前剩余 100%，下次重置 10/27 14:33。".into(), true, 0.75, false));
        render(&Path::new(&output).join("island-notice.png"), &display, scale, 146.0);
        display.notice = None;
        display.vertical = true;
        render(&Path::new(&output).join("fusion-vertical.png"), &display, scale, 500.0);
        display.vertical = false;
        display.strip = false;
        render(&Path::new(&output).join("fusion-hud.png"), &display, scale, 86.0);
        display.strip = true;
        display.notice = Some(("QuotaDock · 7 天额度已重置".into(), "当前剩余 100%，下次重置 10/27 14:33。".into(), true, 0.75, false));
        let quota_notice = display.notice.clone();
        display.notice = Some(("QuotaDock · QuotaDock · 优化任务通知 · 本轮结束".into(), "已加入 Codex 本轮结束通知。灵动岛显示项目名与回复摘要，结束后恢复原来的窗口形态。".into(), false, 0.75, true));
        render(&Path::new(&output).join("codex-task-notice.png"), &display, scale, draw::task_notice_height(2));
        display.notice = Some(("QuotaDock · QuotaDock · 优化任务通知 · 本轮结束".into(), "已加入接收方式设置，默认使用本地记录。选择会保存，重启后保留。".into(), false, 0.65, true));
        render(&Path::new(&output).join("codex-task-long.png"), &display, scale, draw::task_notice_height(2));
        display.notice = Some(("QuotaDock · QuotaDock · 调整界面 · 本轮结束".into(), "1".into(), false, 0.85, true));
        render(&Path::new(&output).join("codex-task-short.png"), &display, scale, draw::task_notice_height(2));
        display.notice = Some(("QuotaDock · codexWT · 用 Rust 重构打包方案 · 本轮结束".into(), "已加入通知摘要设置。你可以在托盘菜单中分别设置摘要字数和显示行数，超出的内容自动省略。设置会保存，重启后仍然有效。项目名与任务名保持显示，通知结束后恢复原来的窗口形态。".into(), false, 0.65, true));
        for lines in [1, 3, 6] {
            display.notice_lines = lines;
            render(&Path::new(&output).join(format!("codex-task-{lines}-lines.png")), &display, scale, draw::task_notice_height(lines));
        }
        display.notice_lines = 6;
        display.notice = Some(("QuotaDock · codexWT · 优化通知布局 · 本轮结束".into(), "已完成本轮修改。这里会显示 Codex 最后回复的摘要，通知结束后恢复原来的窗口形态。".into(), false, 0.65, true));
        render(&Path::new(&output).join("codex-task-cap6-auto.png"), &display, scale, draw::task_notice_height(6));
        display.notice = Some(("QuotaDock · codexWT · 优化通知布局 · 本轮结束".into(), "已完成。".into(), false, 0.65, true));
        render(&Path::new(&output).join("codex-task-cap6-short.png"), &display, scale, draw::task_notice_height(6));
        display.notice_lines = 2;
        display.notice = quota_notice;
        let frames = Path::new(&output).join("motion");
        std::fs::create_dir_all(&frames).unwrap();
        display.content_only = true;
        let notice = render(&frames.join("notice-content.png"), &display, scale, 146.0);
        display.notice = None;
        let collapsed = render(&frames.join("collapsed-content.png"), &display, scale, 28.0);
        display.expanded = true;
        let expanded = render(&frames.join("expanded-content.png"), &display, scale, 176.0);
        let small = motion::Shell { color: 0xff080a0e, top_radius: 0.0, bottom_radius: 18.0 * scale, insets: [0.0; 4], background_inset: [0.0; 4] };
        let large = motion::Shell { bottom_radius: 20.0 * scale, ..small };
        for (stage, from, to, from_shell, to_shell) in [("hover", &collapsed, &expanded, small, large), ("notice", &expanded, &notice, large, large), ("close", &notice, &collapsed, large, small)] {
            for index in 0..=21 {
                let progress = motion::ease(index as f32 / 21.0);
                let frame = motion::blend(from, to, progress, 0.0, true);
                let mut pixels = vec![0; frame.pixels.len()];
                motion::compose_into(&frame, from_shell.interpolate(to_shell, progress), &mut pixels);
                save_frame(&frames.join(format!("{stage}-{index:02}.png")), &motion::Frame { pixels, ..frame });
            }
        }
        for (name, theme) in [("default", theme::Theme::Default), ("ocean", theme::Theme::Ocean), ("forest", theme::Theme::Forest), ("violet", theme::Theme::Violet), ("amber", theme::Theme::Amber)] {
            display.theme = theme;
            display.content_only = false;
            display.expanded = false;
            display.notice = None;
            display.ribbon_attached = Some(1);
            display.task_ribbon_color = Some(0xff9bea58);
            display.task_light_active = true;
            render(&Path::new(&output).join(format!("theme-{name}-strip.png")), &display, scale, 28.0);
            display.expanded = true;
            render(&Path::new(&output).join(format!("theme-{name}-expanded.png")), &display, scale, 176.0);
            display.notice_lines = 3;
            display.notice = Some(("QuotaDock · codexWT · 优化通知布局 · 本轮结束".into(), "已完成本轮修改。这里会显示任务回复摘要，通知结束后恢复原来的窗口形态。".into(), false, 0.75, true));
            render(&Path::new(&output).join(format!("theme-{name}-notice.png")), &display, scale, draw::task_notice_height(3));
        }
        GdiplusShutdown(token);
    }
    println!("{output}");
}
