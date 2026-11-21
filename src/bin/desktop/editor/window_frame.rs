use std::cell::Cell;

use anyhow::{Result, anyhow, bail, ensure};
use eframe::egui;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateRoundRectRgn, DeleteObject, HGDIOBJ, ScreenToClient, SetWindowRgn,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, VIRTUAL_KEY, VK_CONTROL, VK_DELETE, VK_MENU, VK_SHIFT, VK_V,
};
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClientRect, GetWindowRect, HTBOTTOM, HTBOTTOMLEFT, HTBOTTOMRIGHT, HTCAPTION, HTCLIENT,
    HTLEFT, HTRIGHT, HTTOP, HTTOPLEFT, HTTOPRIGHT, IsIconic, IsZoomed, WM_DPICHANGED, WM_KEYDOWN,
    WM_NCDESTROY, WM_NCHITTEST, WM_SIZE,
};

const SUBCLASS_ID: usize = 1;
const RESIZE_BAND_DIP: u32 = 6;
const CORNER_RADIUS_DIP: u32 = 8;

thread_local! {
    // The editor owns one HWND on this event-loop thread.
    static CAPTION: Cell<Option<RECT>> = const { Cell::new(None) };
    static EDIT_SHORTCUT: Cell<Option<EditShortcut>> = const { Cell::new(None) };
    static ROUNDED_BOUNDS: Cell<Option<(i32, i32, i32)>> = const { Cell::new(None) };
}

#[derive(Clone, Copy)]
enum EditShortcut {
    Paste,
    RippleDelete,
}

pub(super) fn restore_clipboard_shortcut(ctx: &egui::Context, input: &mut egui::RawInput) {
    let Some(shortcut) = EDIT_SHORTCUT.take() else {
        return;
    };
    if ctx.text_edit_focused() {
        return;
    }
    match shortcut {
        EditShortcut::Paste => {
            // egui-winit drops Ctrl+V when the system clipboard has no text.
            if !input
                .events
                .iter()
                .any(|event| matches!(event, egui::Event::Paste(_)))
            {
                input.events.push(egui::Event::Paste(String::new()));
            }
        }
        EditShortcut::RippleDelete => {
            // On Windows egui-winit also emits Cut for Shift+Delete.
            if let Some(event) = input
                .events
                .iter_mut()
                .find(|event| matches!(event, egui::Event::Cut))
            {
                *event = egui::Event::Key {
                    key: egui::Key::Delete,
                    physical_key: Some(egui::Key::Delete),
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::SHIFT,
                };
            }
        }
    }
}

pub(super) fn set_caption(bounds: egui::Rect, pixels_per_point: f32) {
    CAPTION.set(Some(RECT {
        left: (bounds.left() * pixels_per_point).ceil() as i32,
        top: (bounds.top() * pixels_per_point).ceil() as i32,
        right: (bounds.right() * pixels_per_point).floor() as i32,
        bottom: (bounds.bottom() * pixels_per_point).floor() as i32,
    }));
}

pub(super) fn install(creation: &eframe::CreationContext<'_>) -> Result<()> {
    let handle = creation
        .window_handle()
        .map_err(|error| anyhow!("无法取得剪辑台窗口句柄（{error}）"))?;
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        bail!("剪辑台窗口未提供 Windows 句柄");
    };
    let window = HWND(handle.hwnd.get() as *mut _);
    // eframe creates the application on the thread that owns the window.
    let installed = unsafe { SetWindowSubclass(window, Some(window_proc), SUBCLASS_ID, 0) };
    ensure!(installed.as_bool(), "无法注册剪辑台窗口边缘缩放");
    update_corners(window);
    Ok(())
}

unsafe extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    subclass_id: usize,
    _reference: usize,
) -> LRESULT {
    if message == WM_NCDESTROY {
        CAPTION.set(None);
        EDIT_SHORTCUT.set(None);
        ROUNDED_BOUNDS.set(None);
        let _ = unsafe { RemoveWindowSubclass(window, Some(window_proc), subclass_id) };
    } else if message == WM_KEYDOWN && lparam.0 & (1 << 30) == 0 {
        let key = VIRTUAL_KEY(wparam.0 as u16);
        if matches!(key, VK_V | VK_DELETE) && !key_down(VK_MENU) {
            let control = key_down(VK_CONTROL);
            let shift = key_down(VK_SHIFT);
            match (key, control, shift) {
                (VK_V, true, false) => EDIT_SHORTCUT.set(Some(EditShortcut::Paste)),
                (VK_DELETE, false, true) => EDIT_SHORTCUT.set(Some(EditShortcut::RippleDelete)),
                _ => {}
            }
        }
    }

    let default = unsafe { DefSubclassProc(window, message, wparam, lparam) };
    if matches!(message, WM_SIZE | WM_DPICHANGED) {
        update_corners(window);
    }
    if message == WM_NCHITTEST && default.0 == HTCLIENT as isize {
        if let Some(hit) = window_client_hit(window, lparam) {
            return LRESULT(hit as isize);
        }
    }
    default
}

fn update_corners(window: HWND) {
    // Called on the window's owning thread after its native size or DPI changes.
    if unsafe { IsIconic(window) }.as_bool() {
        return;
    }
    if unsafe { IsZoomed(window) }.as_bool() {
        let old_bounds = ROUNDED_BOUNDS.take();
        if old_bounds.is_some() && unsafe { SetWindowRgn(window, None, true) } == 0 {
            ROUNDED_BOUNDS.set(old_bounds);
        }
        return;
    }

    let mut bounds = RECT::default();
    if unsafe { GetWindowRect(window, &mut bounds) }.is_err() {
        return;
    }
    let width = bounds.right - bounds.left;
    let height = bounds.bottom - bounds.top;
    if width <= 0 || height <= 0 {
        return;
    }
    let dpi = unsafe { GetDpiForWindow(window) };
    let dpi = if dpi == 0 { 96 } else { dpi };
    let diameter = (u64::from(dpi) * u64::from(CORNER_RADIUS_DIP * 2)).div_ceil(96) as i32;
    let diameter = diameter.min(width.min(height));
    let rounded = Some((width, height, diameter));
    let old_bounds = ROUNDED_BOUNDS.get();
    if rounded == old_bounds {
        return;
    }

    let region = unsafe { CreateRoundRectRgn(0, 0, width, height, diameter, diameter) };
    if region.0.is_null() {
        return;
    }
    // Cache before SetWindowRgn, which can synchronously dispatch window messages.
    ROUNDED_BOUNDS.set(rounded);
    if unsafe { SetWindowRgn(window, Some(region), true) } == 0 {
        ROUNDED_BOUNDS.set(old_bounds);
        // Windows takes ownership only after a successful call.
        let _ = unsafe { DeleteObject(HGDIOBJ(region.0)) };
    }
}

fn key_down(key: VIRTUAL_KEY) -> bool {
    unsafe { GetKeyState(i32::from(key.0)) < 0 }
}

fn window_client_hit(window: HWND, window_message_point: LPARAM) -> Option<u32> {
    // This is called only during the live window's native hit-test message.
    unsafe {
        if IsIconic(window).as_bool() {
            return None;
        }
        let mut point = screen_point(window_message_point);
        if !ScreenToClient(window, &mut point).as_bool() {
            return None;
        }
        let mut bounds = RECT::default();
        GetClientRect(window, &mut bounds).ok()?;
        let band = if IsZoomed(window).as_bool() {
            0
        } else {
            resize_band(GetDpiForWindow(window)).unwrap_or_default()
        };
        client_hit(bounds, CAPTION.get(), point, band)
    }
}

fn client_hit(bounds: RECT, caption: Option<RECT>, point: POINT, band: i32) -> Option<u32> {
    if !contains(bounds, point) {
        return None;
    }
    resize_hit(bounds, point, band).or_else(|| {
        caption
            .filter(|rect| contains(*rect, point))
            .map(|_| HTCAPTION)
    })
}

fn resize_band(dpi: u32) -> Option<i32> {
    if dpi == 0 {
        return None;
    }
    Some((u64::from(dpi) * u64::from(RESIZE_BAND_DIP)).div_ceil(96) as i32)
}

fn screen_point(packed: LPARAM) -> POINT {
    // Each signed coordinate occupies one word, including on negative-origin displays.
    POINT {
        x: i32::from(packed.0 as i16),
        y: i32::from((packed.0 >> 16) as i16),
    }
}

fn resize_hit(bounds: RECT, point: POINT, band: i32) -> Option<u32> {
    let north = point.y - bounds.top < band;
    let south = bounds.bottom - point.y <= band;
    let west = point.x - bounds.left < band;
    let east = bounds.right - point.x <= band;
    match (north, south, west, east) {
        (true, _, true, _) => Some(HTTOPLEFT),
        (true, _, _, true) => Some(HTTOPRIGHT),
        (_, true, true, _) => Some(HTBOTTOMLEFT),
        (_, true, _, true) => Some(HTBOTTOMRIGHT),
        (true, _, _, _) => Some(HTTOP),
        (_, true, _, _) => Some(HTBOTTOM),
        (_, _, true, _) => Some(HTLEFT),
        (_, _, _, true) => Some(HTRIGHT),
        _ => None,
    }
}

fn contains(bounds: RECT, point: POINT) -> bool {
    (bounds.left..bounds.right).contains(&point.x) && (bounds.top..bounds.bottom).contains(&point.y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restores_empty_system_clipboard_paste_once_and_keeps_existing_paste_events() {
        let ctx = egui::Context::default();
        let mut input = egui::RawInput::default();
        EDIT_SHORTCUT.set(Some(EditShortcut::Paste));
        restore_clipboard_shortcut(&ctx, &mut input);
        assert!(matches!(input.events.as_slice(), [egui::Event::Paste(value)] if value.is_empty()));

        input.events = vec![egui::Event::Paste("文字".into())];
        EDIT_SHORTCUT.set(Some(EditShortcut::Paste));
        restore_clipboard_shortcut(&ctx, &mut input);
        assert!(matches!(input.events.as_slice(), [egui::Event::Paste(value)] if value == "文字"));
    }

    #[test]
    fn client_edges_expose_all_eight_native_resize_directions() {
        let bounds = RECT {
            left: 0,
            top: 0,
            right: 800,
            bottom: 500,
        };
        let band = resize_band(144).unwrap();
        assert_eq!(band, 9);
        for (x, y, hit) in [
            (8, 8, HTTOPLEFT),
            (400, 8, HTTOP),
            (791, 8, HTTOPRIGHT),
            (791, 250, HTRIGHT),
            (791, 491, HTBOTTOMRIGHT),
            (400, 491, HTBOTTOM),
            (8, 491, HTBOTTOMLEFT),
            (8, 250, HTLEFT),
        ] {
            assert_eq!(client_hit(bounds, None, POINT { x, y }, band), Some(hit));
        }
        assert_eq!(client_hit(bounds, None, POINT { x: 400, y: 9 }, band), None);
        assert_eq!(
            client_hit(bounds, None, POINT { x: 800, y: 499 }, band),
            None
        );
    }

    #[test]
    fn hit_test_coordinates_preserve_negative_monitor_origins() {
        let packed = (u32::from(-15_i16 as u16) << 16) | u32::from(-1800_i16 as u16);
        let point = screen_point(LPARAM(packed as isize));
        assert_eq!((point.x, point.y), (-1800, -15));
    }

    #[test]
    fn caption_uses_native_drag_only_outside_the_action_group() {
        let bounds = RECT {
            left: 0,
            top: 0,
            right: 800,
            bottom: 500,
        };
        let caption = Some(RECT {
            left: 12,
            top: 6,
            right: 486,
            bottom: 38,
        });
        assert_eq!(
            client_hit(bounds, caption, POINT { x: 400, y: 22 }, 6),
            Some(HTCAPTION)
        );
        assert_eq!(
            client_hit(bounds, caption, POINT { x: 400, y: 22 }, 0),
            Some(HTCAPTION)
        );
        assert_eq!(
            client_hit(bounds, caption, POINT { x: 486, y: 22 }, 6),
            None
        );
        assert_eq!(
            client_hit(bounds, caption, POINT { x: 400, y: 2 }, 6),
            Some(HTTOP)
        );
    }
}
