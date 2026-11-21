use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Graphics::Dwm::{DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, HMONITOR, MONITORINFO};
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetThreadDpiAwarenessContext,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON, VK_RBUTTON};
use windows::Win32::UI::WindowsAndMessaging::{
    GA_ROOT, GetAncestor, GetPhysicalCursorPos, GetSystemMetrics, GetWindowThreadProcessId,
    SM_SWAPBUTTON, WindowFromPhysicalPoint,
};

use crate::windows_recorder::CaptureSource;

#[derive(Clone, Copy, Debug)]
pub struct Click {
    pub at: Instant,
    pub position: [f32; 2],
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PointerSample {
    pub position: Option<[f32; 2]>,
    pub click: Option<Click>,
}

pub struct ClickTracker {
    source: CaptureSource,
    primary_button: i32,
    was_pressed: bool,
    sample: Arc<Mutex<PointerSample>>,
}

impl ClickTracker {
    pub fn new(source: CaptureSource) -> Self {
        let primary = if unsafe { GetSystemMetrics(SM_SWAPBUTTON) } == 0 {
            VK_LBUTTON
        } else {
            VK_RBUTTON
        };
        let primary_button = i32::from(primary.0);
        Self {
            source,
            primary_button,
            was_pressed: unsafe { GetAsyncKeyState(primary_button) } < 0,
            sample: Arc::new(Mutex::new(PointerSample::default())),
        }
    }

    pub fn sample_handle(&self) -> Arc<Mutex<PointerSample>> {
        Arc::clone(&self.sample)
    }

    pub fn poll(&mut self) {
        let pressed = unsafe { GetAsyncKeyState(self.primary_button) } < 0;
        let clicked = pressed && !self.was_pressed;
        self.was_pressed = pressed;

        let position = pointer_in_source(self.source);
        let mut sample = self.sample.lock().unwrap_or_else(PoisonError::into_inner);
        sample.position = position;
        if clicked && let Some(position) = position {
            sample.click = Some(Click {
                at: Instant::now(),
                position,
            });
        }
    }
}

fn pointer_in_source(source: CaptureSource) -> Option<[f32; 2]> {
    let mut point = POINT::default();
    unsafe { GetPhysicalCursorPos(&mut point) }.ok()?;
    let bounds = source_bounds(source)?;
    if point.x < bounds.left
        || point.x >= bounds.right
        || point.y < bounds.top
        || point.y >= bounds.bottom
    {
        return None;
    }

    let hit = unsafe { WindowFromPhysicalPoint(point) };
    let root = unsafe { GetAncestor(hit, GA_ROOT) };
    if matches!(source, CaptureSource::Window(window) if root != HWND(window.as_raw_hwnd())) {
        return None;
    }

    let mut process_id = 0;
    if !root.0.is_null() {
        unsafe { GetWindowThreadProcessId(root, Some(&mut process_id)) };
    }
    if process_id == std::process::id() {
        return None;
    }

    Some([
        (point.x - bounds.left) as f32 / (bounds.right - bounds.left) as f32,
        (point.y - bounds.top) as f32 / (bounds.bottom - bounds.top) as f32,
    ])
}

fn source_bounds(source: CaptureSource) -> Option<RECT> {
    match source {
        CaptureSource::Monitor(monitor) => {
            let mut info = MONITORINFO {
                cbSize: size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            // Match the physical cursor coordinates even on a DPI-unaware recorder thread.
            let previous =
                unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
            if previous.0.is_null() {
                return None;
            }
            let found = unsafe { GetMonitorInfoW(HMONITOR(monitor.as_raw_hmonitor()), &mut info) };
            unsafe { SetThreadDpiAwarenessContext(previous) };
            found.as_bool().then_some(info.rcMonitor)
        }
        CaptureSource::Window(window) => {
            let mut bounds = RECT::default();
            unsafe {
                DwmGetWindowAttribute(
                    HWND(window.as_raw_hwnd()),
                    DWMWA_EXTENDED_FRAME_BOUNDS,
                    std::ptr::from_mut(&mut bounds).cast(),
                    size_of::<RECT>() as u32,
                )
            }
            .ok()?;
            Some(bounds)
        }
    }
}
