#[cfg(windows)]
use std::path::Path;
use std::path::PathBuf;
#[cfg(windows)]
use std::process::{Child, Command};
#[cfg(windows)]
use std::time::Duration;

#[cfg(windows)]
use anyhow::{Context as _, Result};
use eframe::egui;
#[cfg(windows)]
use takeseam::PRODUCT_NAME;
use takeseam::i18n::{Language, Locale, key};
#[cfg(windows)]
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindowVisible, PostMessageW,
    SW_MINIMIZE, SW_RESTORE, SetForegroundWindow, ShowWindowAsync, WM_CLOSE,
};
#[cfg(windows)]
use windows::core::BOOL;

use super::ui_theme;
use crate::{APP_LOGO, load_logo};

#[path = "editor/asset_cards.rs"]
mod asset_cards;
#[path = "editor/asset_info.rs"]
mod asset_info;
#[path = "editor/background.rs"]
mod background;
#[path = "editor/background_panel.rs"]
mod background_panel;
#[path = "editor/canvas_format.rs"]
mod canvas_format;
#[path = "editor/cursor_effect.rs"]
mod cursor_effect;
#[path = "editor/dock_model.rs"]
mod dock_model;
#[path = "editor/dock_presets.rs"]
mod dock_presets;
#[path = "editor/dock_view.rs"]
mod dock_view;
#[path = "editor/dynamic_recipe.rs"]
mod dynamic_recipe;
#[path = "editor/export.rs"]
mod export;
#[path = "editor/feature_panel.rs"]
mod feature_panel;
#[path = "editor/file_menu.rs"]
mod file_menu;
#[path = "editor/inspector_details.rs"]
mod inspector_details;
#[path = "editor/layout.rs"]
mod layout;
#[path = "editor/library.rs"]
mod library;
#[path = "editor/media.rs"]
mod media;
#[path = "editor/narration_capture.rs"]
mod narration_capture;
#[path = "editor/player.rs"]
mod player;
#[path = "editor/proxy_cache.rs"]
mod proxy_cache;
#[path = "editor/preview_viewport.rs"]
mod preview_viewport;
#[path = "editor/project.rs"]
mod project;
#[path = "editor/subrip.rs"]
mod subrip;
#[path = "editor/speech.rs"]
mod speech;
#[path = "editor/subtitle_canvas.rs"]
mod subtitle_canvas;
#[path = "editor/text_panel.rs"]
mod text_panel;
#[path = "editor/transition_panel.rs"]
mod transition_panel;
#[path = "editor/thumbnails.rs"]
mod thumbnails;
#[path = "editor/timeline_view.rs"]
mod timeline_view;
#[path = "editor/waveforms.rs"]
mod waveforms;
#[cfg(windows)]
#[path = "editor/window_chrome.rs"]
mod window_chrome;
#[cfg(windows)]
#[path = "editor/window_frame.rs"]
mod window_frame;
#[path = "editor/workspace.rs"]
mod workspace;

#[cfg(windows)]
const WINDOW_TITLE: &str = "OtterCut 编辑控制台";

#[cfg(windows)]
pub(super) fn recording_ffmpeg_path() -> anyhow::Result<PathBuf> {
    media::MediaTools::ffmpeg_path()
}
#[cfg(windows)]
const WINDOW_POLL_INTERVAL: Duration = Duration::from_millis(125);

#[cfg(windows)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum EditorWindowState {
    #[default]
    Closed,
    Opening,
    Visible,
    Minimized,
}

#[cfg(windows)]
#[derive(Default)]
pub struct EditorWindow {
    child: Option<Child>,
    state: EditorWindowState,
}

#[cfg(windows)]
impl EditorWindow {
    pub fn open(&mut self, ctx: &egui::Context, directory: &Path) -> Result<()> {
        let window = self.refresh()?;
        self.show_or_launch(ctx, directory, window)
    }

    pub fn toggle(&mut self, ctx: &egui::Context, directory: &Path) -> Result<()> {
        let window = self.refresh()?;
        if let Some(handle) = window
            && self.state == EditorWindowState::Visible
        {
            // SAFETY: This handle was found in the launched child process.
            let _ = unsafe { ShowWindowAsync(handle, SW_MINIMIZE) };
            ctx.request_repaint_after(WINDOW_POLL_INTERVAL);
            return Ok(());
        }
        self.show_or_launch(ctx, directory, window)
    }

    fn show_or_launch(
        &mut self,
        ctx: &egui::Context,
        directory: &Path,
        window: Option<HWND>,
    ) -> Result<()> {
        if self.child.is_none() {
            let executable =
                std::env::current_exe().with_context(|| format!("找不到 {PRODUCT_NAME} 程序"))?;
            self.child = Some(
                Command::new(executable)
                    .arg("--editor")
                    .arg(directory)
                    .spawn()
                    .context("无法启动编辑控制台")?,
            );
            self.state = EditorWindowState::Opening;
            ctx.request_repaint();
        } else if let Some(handle) = window
            && self.state != EditorWindowState::Opening
        {
            // SAFETY: This handle was found in the launched child process.
            unsafe {
                if self.state == EditorWindowState::Minimized {
                    let _ = ShowWindowAsync(handle, SW_RESTORE);
                }
                let _ = SetForegroundWindow(handle);
            }
        }
        ctx.request_repaint_after(WINDOW_POLL_INTERVAL);
        Ok(())
    }

    pub fn poll(&mut self, ctx: &egui::Context) {
        let _ = self.refresh();
        if self.child.is_some() {
            ctx.request_repaint_after(WINDOW_POLL_INTERVAL);
        }
    }

    pub fn state(&self) -> EditorWindowState {
        self.state
    }

    fn running_process(&mut self) -> Result<Option<u32>> {
        let Some(child) = &mut self.child else {
            return Ok(None);
        };
        if child
            .try_wait()
            .context("无法查询编辑控制台状态")?
            .is_none()
        {
            return Ok(Some(child.id()));
        }
        *self = Self::default();
        Ok(None)
    }

    fn refresh(&mut self) -> Result<Option<HWND>> {
        let Some(process) = self.running_process()? else {
            return Ok(None);
        };
        let handle = process_windows(process).find(|handle| is_editor_window(*handle));
        // SAFETY: The handles came from EnumWindows and the queries take no pointers.
        self.state = unsafe {
            match handle {
                Some(handle) if IsIconic(handle).as_bool() => EditorWindowState::Minimized,
                Some(handle) if IsWindowVisible(handle).as_bool() => EditorWindowState::Visible,
                _ => EditorWindowState::Opening,
            }
        };
        Ok(handle)
    }

    pub fn close_for_shutdown(&mut self, ctx: &egui::Context) -> bool {
        match self.running_process() {
            Ok(None) => return true,
            Ok(Some(process)) => {
                for handle in process_windows(process) {
                    // SAFETY: Notify only the launched editor and its visible dialogs.
                    unsafe {
                        if IsWindowVisible(handle).as_bool() || is_editor_window(handle) {
                            let _ = PostMessageW(Some(handle), WM_CLOSE, WPARAM(0), LPARAM(0));
                        }
                    }
                }
            }
            Err(_) => {}
        }
        ctx.request_repaint_after(Duration::from_millis(200));
        false
    }
}

#[cfg(windows)]
fn process_windows(process: u32) -> impl Iterator<Item = HWND> {
    struct Search {
        process: u32,
        windows: Vec<HWND>,
    }

    unsafe extern "system" fn collect(handle: HWND, state: LPARAM) -> BOOL {
        // SAFETY: EnumWindows calls this synchronously with the pointer to Search below.
        let search = unsafe { &mut *(state.0 as *mut Search) };
        let mut process = 0;
        // SAFETY: Windows supplied the handle and process points to writable storage.
        unsafe { GetWindowThreadProcessId(handle, Some(&mut process)) };
        if process == search.process {
            search.windows.push(handle);
        }
        BOOL(1)
    }

    let mut search = Search {
        process,
        windows: Vec::new(),
    };
    // SAFETY: Search stays alive through enumeration and collect uses the same pointer type.
    let _ = unsafe { EnumWindows(Some(collect), LPARAM(&mut search as *mut Search as isize)) };
    search.windows.into_iter()
}

#[cfg(windows)]
fn is_editor_window(handle: HWND) -> bool {
    let mut title = [0; 64];
    // SAFETY: The handle came from EnumWindows and title is a writable buffer.
    let length = unsafe { GetWindowTextW(handle, &mut title) } as usize;
    title[..length]
        .iter()
        .copied()
        .eq(WINDOW_TITLE.encode_utf16())
}

pub fn launch(directory: PathBuf) -> eframe::Result {
    let icon = eframe::icon_data::from_png_bytes(APP_LOGO)
        .map_err(|error| eframe::Error::AppCreation(Box::new(error)))?;
    let language = takeseam::i18n::preference_path()
        .and_then(|path| takeseam::i18n::read_preference(&path))
        .unwrap_or_else(Language::detect);
    let title = Locale::new(language).text(key::WINDOW_TITLE).to_owned();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(&title)
            .with_decorations(!cfg!(windows))
            .with_has_shadow(true)
            .with_inner_size([1080.0, 680.0])
            .with_min_inner_size([760.0, 490.0])
            .with_icon(icon.clone()),
        centered: true,
        ..Default::default()
    };
    eframe::run_native(
        &title,
        options,
        Box::new(|creation| {
            #[cfg(windows)]
            window_frame::install(creation).map_err(anyhow::Error::into_boxed_dyn_error)?;
            Locale::set_context(&creation.egui_ctx, language);
            ui_theme::initialize_editor(&creation.egui_ctx);
            Ok(Box::new(workspace::Workspace::new(
                &creation.egui_ctx,
                load_logo(&creation.egui_ctx, &icon),
                directory,
            )))
        }),
    )
}
