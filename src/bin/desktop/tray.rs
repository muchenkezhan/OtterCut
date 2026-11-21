use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::ptr;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};

use anyhow::{Context as _, Result, ensure};
use eframe::egui::IconData;
use takeseam::PRODUCT_NAME;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateDIBSection, DIB_RGB_COLORS,
    DeleteObject, HBITMAP, HGDIOBJ,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_SETFOCUS, NIM_SETVERSION,
    NIN_SELECT, NOTIFYICON_VERSION_4, NOTIFYICONDATAW, NOTIFYICONDATAW_0, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIconIndirect, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyIcon,
    DestroyMenu, DestroyWindow, DispatchMessageW, EndMenu, GWLP_USERDATA, GetCursorPos,
    GetMessageW, GetWindowLongPtrW, HICON, HMENU, ICONINFO, MF_STRING, MSG, PostMessageW,
    PostQuitMessage, RegisterClassW, RegisterWindowMessageW, SetForegroundWindow,
    SetWindowLongPtrW, TPM_RIGHTBUTTON, TrackPopupMenu, TranslateMessage, UnregisterClassW,
    WINDOW_EX_STYLE, WM_APP, WM_CLOSE, WM_COMMAND, WM_CONTEXTMENU, WM_LBUTTONUP, WM_NULL,
    WM_RBUTTONUP, WNDCLASSW, WS_POPUP,
};
use windows::core::{BOOL, PCWSTR, w};

const TRAY_CLASS: PCWSTR = w!("OtterCut.Tray");
const TRAY_NOTIFICATION: u32 = WM_APP + 1;
const TRAY_ICON_ID: u32 = 1;
const NIN_KEYSELECT: u32 = NIN_SELECT | 1;
const ICON_SIDE: usize = 32;
const MASK_ROW_BYTES: usize = ICON_SIDE.div_ceil(16) * 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayAction {
    OpenEditor,
    ShowToolbar,
    Restart,
    Exit,
}

const ITEMS: [(PCWSTR, TrayAction); 4] = [
    (w!("打开剪辑台"), TrayAction::OpenEditor),
    (w!("打开工具栏"), TrayAction::ShowToolbar),
    (w!("重启软件"), TrayAction::Restart),
    (w!("退出"), TrayAction::Exit),
];

pub struct Tray {
    // Only the tray thread uses the HWND. The owner can post a close message.
    window: usize,
    thread: Option<JoinHandle<()>>,
}

impl Tray {
    pub fn new(
        icon: &IconData,
        on_action: impl FnMut(TrayAction) + Send + 'static,
    ) -> Result<Self> {
        let pixels = icon_pixels(icon)?;
        let (ready, startup) = mpsc::sync_channel(1);
        let thread = thread::Builder::new()
            .name(format!("{PRODUCT_NAME} tray"))
            .spawn(move || match TrayWindow::new(&pixels, on_action) {
                Ok(tray) => {
                    if ready.send(Ok(tray.window.0 as usize)).is_ok() {
                        tray.run();
                    }
                }
                Err(error) => {
                    let _ = ready.send(Err(error));
                }
            })
            .context("无法创建托盘线程")?;

        let startup = startup.recv().context("托盘线程未完成启动");
        match startup.and_then(|result| result) {
            Ok(window) => Ok(Self {
                window,
                thread: Some(thread),
            }),
            Err(error) => {
                let _ = thread.join();
                Err(error)
            }
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        if let Some(thread) = self.thread.take() {
            let window = HWND(self.window as *mut c_void);
            // Closing the window on its own thread releases the menu and notification icon.
            unsafe {
                let _ = PostMessageW(Some(window), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
            let _ = thread.join();
        }
    }
}

struct TrayWindow {
    window: HWND,
    state: Box<TrayState>,
    _class: WindowClass,
}

struct TrayState {
    notification: NOTIFYICONDATAW,
    taskbar_created: u32,
    menu: Menu,
    _icon: Icon,
    on_action: RefCell<Box<dyn FnMut(TrayAction) + Send>>,
    modern_events: Cell<bool>,
    closing: Cell<bool>,
}

impl TrayWindow {
    fn new(
        pixels: &IconPixels,
        on_action: impl FnMut(TrayAction) + Send + 'static,
    ) -> Result<Self> {
        let icon = Icon::new(pixels).context("无法创建托盘图标")?;
        let menu = Menu::new().context("无法创建托盘菜单")?;
        let class = WindowClass::register().context("无法注册托盘窗口")?;
        let window = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                TRAY_CLASS,
                w!("OtterCut"),
                WS_POPUP,
                0,
                0,
                0,
                0,
                None,
                None,
                Some(class.0),
                None,
            )
        }
        .context("无法创建托盘窗口")?;
        let mut tip = [0; 128];
        for (slot, unit) in tip.iter_mut().zip(PRODUCT_NAME.encode_utf16()) {
            *slot = unit;
        }
        let notification = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: window,
            uID: TRAY_ICON_ID,
            uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP | NIF_SHOWTIP,
            uCallbackMessage: TRAY_NOTIFICATION,
            hIcon: icon.0,
            szTip: tip,
            Anonymous: NOTIFYICONDATAW_0 {
                uVersion: NOTIFYICON_VERSION_4,
            },
            ..Default::default()
        };
        let tray = Self {
            window,
            state: Box::new(TrayState {
                notification,
                taskbar_created: unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) },
                menu,
                _icon: icon,
                on_action: RefCell::new(Box::new(on_action)),
                modern_events: Cell::new(false),
                closing: Cell::new(false),
            }),
            _class: class,
        };

        // State remains at this heap address until the window has been destroyed.
        unsafe {
            SetWindowLongPtrW(
                window,
                GWLP_USERDATA,
                (&*tray.state as *const TrayState) as isize,
            );
        }
        ensure!(
            tray.state.add_icon(),
            "Windows 通知区域暂时无法添加 {PRODUCT_NAME} 图标"
        );
        Ok(tray)
    }

    fn run(&self) {
        let mut message = MSG::default();
        unsafe {
            while GetMessageW(&mut message, None, 0, 0).0 > 0 {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    }
}

impl Drop for TrayWindow {
    fn drop(&mut self) {
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &self.state.notification);
            SetWindowLongPtrW(self.window, GWLP_USERDATA, 0);
            let _ = DestroyWindow(self.window);
        }
    }
}

impl TrayState {
    fn add_icon(&self) -> bool {
        let added = unsafe { Shell_NotifyIconW(NIM_ADD, &self.notification).as_bool() };
        self.modern_events.set(
            added && unsafe { Shell_NotifyIconW(NIM_SETVERSION, &self.notification).as_bool() },
        );
        added
    }

    fn emit(&self, action: TrayAction) {
        if !self.closing.get()
            && let Ok(mut callback) = self.on_action.try_borrow_mut()
        {
            callback(action);
        }
    }

    fn popup(&self, window: HWND, mut point: POINT) {
        if point.x == -1 && point.y == -1 && unsafe { GetCursorPos(&mut point) }.is_err() {
            return;
        }
        unsafe {
            // The foreground owner lets the system dismiss the menu on outside clicks.
            let _ = SetForegroundWindow(window);
            let shown = TrackPopupMenu(
                self.menu.0,
                TPM_RIGHTBUTTON,
                point.x,
                point.y,
                None,
                window,
                None,
            );
            let _ = PostMessageW(Some(window), WM_NULL, WPARAM(0), LPARAM(0));
            if !shown.as_bool() {
                let _ = Shell_NotifyIconW(NIM_SETFOCUS, &self.notification);
            }
        }
    }
}

unsafe extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let pointer = unsafe { GetWindowLongPtrW(window, GWLP_USERDATA) } as *const TrayState;
    // Windows can dispatch messages during creation and destruction before state is attached.
    let Some(state) = (unsafe { pointer.as_ref() }) else {
        return unsafe { DefWindowProcW(window, message, wparam, lparam) };
    };
    match message {
        WM_CLOSE => {
            state.closing.set(true);
            unsafe {
                let _ = EndMenu();
                PostQuitMessage(0);
            }
        }
        WM_COMMAND if lparam.0 == 0 && (wparam.0 >> 16) == 0 => {
            if let Some(index) = wparam.0.checked_sub(1)
                && let Some((_, action)) = ITEMS.get(index)
            {
                state.emit(*action);
            }
        }
        TRAY_NOTIFICATION if !state.closing.get() => {
            let modern = state.modern_events.get();
            let event = (lparam.0 as u32) & 0xFFFF;
            match event {
                WM_CONTEXTMENU if modern => {
                    let coordinate = wparam.0 as u32;
                    state.popup(
                        window,
                        POINT {
                            x: i32::from(coordinate as i16),
                            y: i32::from((coordinate >> 16) as i16),
                        },
                    );
                }
                WM_RBUTTONUP if !modern => state.popup(window, POINT { x: -1, y: -1 }),
                NIN_SELECT | NIN_KEYSELECT if modern => state.emit(TrayAction::ShowToolbar),
                WM_LBUTTONUP if !modern => state.emit(TrayAction::ShowToolbar),
                _ => {}
            }
        }
        id if id != 0 && id == state.taskbar_created && !state.closing.get() => {
            state.add_icon();
        }
        _ => return unsafe { DefWindowProcW(window, message, wparam, lparam) },
    }
    LRESULT(0)
}

struct WindowClass(HINSTANCE);

impl WindowClass {
    fn register() -> Result<Self> {
        let module = unsafe { GetModuleHandleW(PCWSTR::null()) }?;
        let instance = HINSTANCE(module.0);
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: TRAY_CLASS,
            ..Default::default()
        };
        if unsafe { RegisterClassW(&class) } == 0 {
            return Err(windows::core::Error::from_thread().into());
        }
        Ok(Self(instance))
    }
}

impl Drop for WindowClass {
    fn drop(&mut self) {
        unsafe {
            let _ = UnregisterClassW(TRAY_CLASS, Some(self.0));
        }
    }
}

struct Menu(HMENU);

impl Menu {
    fn new() -> Result<Self> {
        let menu = Self(unsafe { CreatePopupMenu() }?);
        for (index, (label, _)) in ITEMS.iter().enumerate() {
            unsafe { AppendMenuW(menu.0, MF_STRING, index + 1, *label) }?;
        }
        Ok(menu)
    }
}

impl Drop for Menu {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyMenu(self.0);
        }
    }
}

struct Icon(HICON);
struct Bitmap(HBITMAP);

impl Icon {
    fn new(pixels: &IconPixels) -> Result<Self> {
        let side = ICON_SIDE as i32;
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: side,
                biHeight: -side,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = ptr::null_mut::<c_void>();
        let color =
            Bitmap(unsafe { CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0) }?);
        ensure!(!bits.is_null(), "托盘图标像素缓冲区不可用");
        let mask =
            Bitmap(unsafe { CreateBitmap(side, side, 1, 1, Some(pixels.mask.as_ptr().cast())) });
        ensure!(!mask.0.is_invalid(), "无法创建托盘图标遮罩");
        // CreateDIBSection allocated a 32-bit row for every row of BGRA data.
        unsafe {
            ptr::copy_nonoverlapping(pixels.bgra.as_ptr(), bits.cast(), pixels.bgra.len());
        }
        let spec = ICONINFO {
            fIcon: BOOL(1),
            hbmMask: mask.0,
            hbmColor: color.0,
            ..Default::default()
        };
        Ok(Self(unsafe { CreateIconIndirect(&spec) }?))
    }
}

impl Drop for Icon {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyIcon(self.0);
        }
    }
}

impl Drop for Bitmap {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteObject(HGDIOBJ(self.0.0));
        }
    }
}

struct IconPixels {
    bgra: [u8; ICON_SIDE * ICON_SIDE * 4],
    mask: [u8; MASK_ROW_BYTES * ICON_SIDE],
}

fn icon_pixels(icon: &IconData) -> Result<IconPixels> {
    let width = icon.width as usize;
    let height = icon.height as usize;
    ensure!(
        width > 0
            && height > 0
            && width
                .checked_mul(height)
                .and_then(|area| area.checked_mul(4))
                == Some(icon.rgba.len()),
        "托盘图标尺寸无效"
    );

    let mut pixels = IconPixels {
        bgra: [0; ICON_SIDE * ICON_SIDE * 4],
        mask: [u8::MAX; MASK_ROW_BYTES * ICON_SIDE],
    };
    let longest = width.max(height);
    let scaled_width = (width * ICON_SIDE / longest).max(1);
    let scaled_height = (height * ICON_SIDE / longest).max(1);
    let offset_x = (ICON_SIDE - scaled_width) / 2;
    let offset_y = (ICON_SIDE - scaled_height) / 2;

    for y in 0..scaled_height {
        let y0 = y * height / scaled_height;
        let y1 = ((y + 1) * height / scaled_height).max(y0 + 1);
        for x in 0..scaled_width {
            let x0 = x * width / scaled_width;
            let x1 = ((x + 1) * width / scaled_width).max(x0 + 1);
            let mut total = [0u64; 4];

            for sy in y0..y1 {
                for sx in x0..x1 {
                    let i = (sy * width + sx) * 4;
                    let alpha = u64::from(icon.rgba[i + 3]);
                    for (channel, byte) in [2, 1, 0].into_iter().enumerate() {
                        total[channel] += u64::from(icon.rgba[i + byte]) * alpha;
                    }
                    total[3] += alpha;
                }
            }
            let samples = ((x1 - x0) * (y1 - y0)) as u64;
            let x = x + offset_x;
            let y = y + offset_y;
            let out = (y * ICON_SIDE + x) * 4;
            let color_samples = samples * 255;
            for (channel, sum) in total[..3].iter().enumerate() {
                pixels.bgra[out + channel] = ((sum + color_samples / 2) / color_samples) as u8;
            }
            let alpha = ((total[3] + samples / 2) / samples) as u8;
            pixels.bgra[out + 3] = alpha;
            if alpha > 0 {
                // CreateBitmap's monochrome data stores rows bottom-up.
                let mask = (ICON_SIDE - y - 1) * MASK_ROW_BYTES + x / 8;
                pixels.mask[mask] &= !(0x80 >> (x % 8));
            }
        }
    }
    Ok(pixels)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brand_logo_keeps_its_primary_color_and_transparent_corners_at_tray_size() {
        let icon = eframe::icon_data::from_png_bytes(crate::desktop::LOGO).unwrap();
        let pixels = icon_pixels(&icon).unwrap();
        assert_eq!(pixels.bgra[3], 0);
        assert_eq!(pixels.bgra[(ICON_SIDE - 1) * 4 + 3], 0);
        assert!(
            pixels
                .bgra
                .chunks_exact(4)
                .any(|pixel| pixel == [0x49, 0x63, 0x3A, 0xFF])
        );
    }
}
