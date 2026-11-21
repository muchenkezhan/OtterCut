use eframe::egui::{self, Rect, pos2};
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromRect,
};

pub(super) fn work_area(toolbar: egui::Rect, pixels_per_point: f32) -> Option<egui::Rect> {
    if !pixels_per_point.is_finite() || pixels_per_point <= 0.0 || !toolbar.is_positive() {
        return None;
    }

    let toolbar_pixels = toolbar * pixels_per_point;
    if !toolbar_pixels.is_finite() {
        return None;
    }
    let toolbar_rect = RECT {
        left: toolbar_pixels.min.x.floor() as i32,
        top: toolbar_pixels.min.y.floor() as i32,
        right: toolbar_pixels.max.x.ceil() as i32,
        bottom: toolbar_pixels.max.y.ceil() as i32,
    };

    // SAFETY: The RECT lives through the call, which only reads it.
    let monitor = unsafe { MonitorFromRect(&toolbar_rect, MONITOR_DEFAULTTONEAREST) };
    if monitor.is_invalid() {
        return None;
    }

    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: The monitor came from MonitorFromRect and info is writable with cbSize initialized.
    if !unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
        return None;
    }

    let RECT {
        left,
        top,
        right,
        bottom,
    } = info.rcWork;
    let area = Rect::from_min_max(
        pos2(left as f32, top as f32),
        pos2(right as f32, bottom as f32),
    ) / pixels_per_point;
    (area.is_finite() && area.is_positive()).then_some(area)
}
