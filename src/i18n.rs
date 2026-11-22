use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Language {
    ZhCn,
    EnUs,
}

impl Language {
    pub const fn code(self) -> &'static str {
        match self {
            Self::ZhCn => "zh-CN",
            Self::EnUs => "en-US",
        }
    }

    pub const fn display_name(self) -> &'static str {
        match self {
            Self::ZhCn => "简体中文",
            Self::EnUs => "English",
        }
    }

    pub fn from_tag(tag: &str) -> Option<Self> {
        let normalized = tag.trim().replace('_', "-").to_ascii_lowercase();
        match normalized.split('-').next()? {
            "zh" => Some(Self::ZhCn),
            "en" => Some(Self::EnUs),
            _ => None,
        }
    }

    pub fn detect() -> Self {
        ["OTTERCUT_LANGUAGE", "LC_ALL", "LANGUAGE", "LANG"]
            .into_iter()
            .find_map(|name| {
                std::env::var(name)
                    .ok()
                    .and_then(|value| Self::from_tag(&value))
            })
            .unwrap_or(Self::ZhCn)
    }
}

impl Default for Language {
    fn default() -> Self {
        Self::ZhCn
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MessageKey(pub &'static str);

pub mod key {
    use super::MessageKey;

    pub const APP_NAME: MessageKey = MessageKey("app.name");
    pub const WINDOW_TITLE: MessageKey = MessageKey("editor.window_title");
    pub const FILE: MessageKey = MessageKey("editor.file");
    pub const IMPORT_MEDIA: MessageKey = MessageKey("editor.import_media");
    pub const IMPORT_AUDIO: MessageKey = MessageKey("editor.import_audio");
    pub const OPEN_PROJECT: MessageKey = MessageKey("editor.open_project");
    pub const SAVE_PROJECT: MessageKey = MessageKey("editor.save_project");
    pub const EXPORT: MessageKey = MessageKey("editor.export");
    pub const MATERIALS: MessageKey = MessageKey("editor.materials");
    pub const PREVIEW: MessageKey = MessageKey("editor.preview");
    pub const INSPECTOR: MessageKey = MessageKey("editor.inspector");
    pub const TIMELINE: MessageKey = MessageKey("editor.timeline");
    pub const WORKSPACE: MessageKey = MessageKey("editor.workspace");
    pub const PLAY: MessageKey = MessageKey("editor.play");
    pub const PAUSE: MessageKey = MessageKey("editor.pause");
    pub const FIT: MessageKey = MessageKey("editor.fit");
    pub const PREVIEW_QUALITY: MessageKey = MessageKey("editor.preview_quality");
    pub const ASSETS: MessageKey = MessageKey("editor.assets");
    pub const AUDIO: MessageKey = MessageKey("editor.audio");
    pub const CAPTIONS: MessageKey = MessageKey("editor.captions");
    pub const TEXT: MessageKey = MessageKey("editor.text");
    pub const BACKGROUND: MessageKey = MessageKey("editor.background");
    pub const ADD_TO_TRACK: MessageKey = MessageKey("editor.add_to_track");
    pub const VIEW_INFO: MessageKey = MessageKey("editor.view_info");
    pub const OPEN_FOLDER: MessageKey = MessageKey("editor.open_folder");
    pub const COPY_PATH: MessageKey = MessageKey("editor.copy_path");
    pub const REMOVE_ASSET: MessageKey = MessageKey("editor.remove_asset");
    pub const APPLY: MessageKey = MessageKey("editor.apply");
    pub const CANCEL: MessageKey = MessageKey("editor.cancel");
    pub const CUSTOM_RATIO: MessageKey = MessageKey("editor.custom_ratio");
    pub const WIDTH: MessageKey = MessageKey("editor.width");
    pub const HEIGHT: MessageKey = MessageKey("editor.height");
    pub const NARRATION_SAVE_BEFORE_OPEN: MessageKey =
        MessageKey("editor.narration_save_before_open");
    pub const PREVIEW_PROGRESS: MessageKey = MessageKey("editor.preview_progress");
    pub const NARRATION_START: MessageKey = MessageKey("editor.narration_start");
    pub const NARRATION_STARTING: MessageKey = MessageKey("editor.narration_starting");
    pub const NARRATION_SAVING: MessageKey = MessageKey("editor.narration_saving");
    pub const TOOLBAR_DRAG: MessageKey = MessageKey("toolbar.drag");
    pub const RECORD_START: MessageKey = MessageKey("toolbar.record_start");
    pub const RECORD_STOP: MessageKey = MessageKey("toolbar.record_stop");
    pub const RECORD_SAVING: MessageKey = MessageKey("toolbar.record_saving");
    pub const MICROPHONE_ENABLE: MessageKey = MessageKey("toolbar.microphone_enable");
    pub const MICROPHONE_DISABLE: MessageKey = MessageKey("toolbar.microphone_disable");
    pub const CLICK_FOLLOW_ENABLE: MessageKey = MessageKey("toolbar.click_follow_enable");
    pub const CLICK_FOLLOW_DISABLE: MessageKey = MessageKey("toolbar.click_follow_disable");
    pub const EDITOR_OPEN: MessageKey = MessageKey("toolbar.editor_open");
    pub const EDITOR_OPENING: MessageKey = MessageKey("toolbar.editor_opening");
    pub const EDITOR_MINIMIZE: MessageKey = MessageKey("toolbar.editor_minimize");
    pub const EDITOR_RESTORE: MessageKey = MessageKey("toolbar.editor_restore");
    pub const SETTINGS: MessageKey = MessageKey("toolbar.settings");
    pub const HIDE_TOOLBAR: MessageKey = MessageKey("toolbar.hide");
    pub const CLOSE: MessageKey = MessageKey("common.close");
    pub const SWITCH_LIGHT: MessageKey = MessageKey("theme.switch_light");
    pub const SWITCH_DARK: MessageKey = MessageKey("theme.switch_dark");
    pub const SOURCE_TITLE: MessageKey = MessageKey("source.title");
    pub const SOURCE_DISPLAY: MessageKey = MessageKey("source.display");
    pub const SOURCE_WINDOW: MessageKey = MessageKey("source.window");
    pub const SOURCE_REFRESH: MessageKey = MessageKey("source.refresh");
    pub const SOURCE_EMPTY: MessageKey = MessageKey("source.empty");
    pub const SOURCE_REMAINING: MessageKey = MessageKey("source.remaining");
    pub const DEVICE_TITLE: MessageKey = MessageKey("device.title");
    pub const DEVICE_REFRESH: MessageKey = MessageKey("device.refresh");
    pub const DEVICE_LOCKED: MessageKey = MessageKey("device.locked");
    pub const DEVICE_READING: MessageKey = MessageKey("device.reading");
    pub const LANGUAGE: MessageKey = MessageKey("common.language");
    pub const LANGUAGE_ZH: MessageKey = MessageKey("common.language_zh");
    pub const LANGUAGE_EN: MessageKey = MessageKey("common.language_en");
    pub const LANGUAGE_SWITCHED: MessageKey = MessageKey("common.language_switched");
    pub const CLI_RECORDING_FAILED: MessageKey = MessageKey("cli.recording_failed");
    pub const CLI_RECORDING_STARTED: MessageKey = MessageKey("cli.recording_started");
    pub const DEVICE_MICROPHONE: MessageKey = MessageKey("device.microphone");
    pub const DEVICE_RECORD_MICROPHONE: MessageKey = MessageKey("device.record_microphone");
    pub const DEVICE_SYSTEM_DEFAULT: MessageKey = MessageKey("device.system_default");
    pub const DEVICE_DISCONNECTED: MessageKey = MessageKey("device.disconnected");
    pub const DEVICE_INPUT: MessageKey = MessageKey("device.input");
    pub const DEVICE_TEST_STOP: MessageKey = MessageKey("device.test_stop");
    pub const DEVICE_TEST_MICROPHONE: MessageKey = MessageKey("device.test_microphone");
    pub const DEVICE_INPUT_LEVEL: MessageKey = MessageKey("device.input_level");
    pub const DEVICE_NO_MICROPHONE: MessageKey = MessageKey("device.no_microphone");
    pub const DEVICE_CAMERA: MessageKey = MessageKey("device.camera");
    pub const DEVICE_RECORD_CAMERA: MessageKey = MessageKey("device.record_camera");
    pub const DEVICE_NO_CAMERA: MessageKey = MessageKey("device.no_camera");
    pub const DEVICE_QUALITY: MessageKey = MessageKey("device.quality");
    pub const DEVICE_DETECTING_QUALITY: MessageKey = MessageKey("device.detecting_quality");
    pub const DEVICE_NO_QUALITY: MessageKey = MessageKey("device.no_quality");
    pub const DEVICE_OPEN_CAMERA: MessageKey = MessageKey("device.open_camera");
    pub const DEVICE_CLOSE_CAMERA: MessageKey = MessageKey("device.close_camera");
    pub const DEVICE_OPENING_CAMERA: MessageKey = MessageKey("device.opening_camera");

    pub const ALL: &[MessageKey] = &[
        APP_NAME,
        WINDOW_TITLE,
        FILE,
        IMPORT_MEDIA,
        IMPORT_AUDIO,
        OPEN_PROJECT,
        SAVE_PROJECT,
        EXPORT,
        MATERIALS,
        PREVIEW,
        INSPECTOR,
        TIMELINE,
        WORKSPACE,
        PLAY,
        PAUSE,
        FIT,
        PREVIEW_QUALITY,
        ASSETS,
        AUDIO,
        CAPTIONS,
        TEXT,
        BACKGROUND,
        ADD_TO_TRACK,
        VIEW_INFO,
        OPEN_FOLDER,
        COPY_PATH,
        REMOVE_ASSET,
        APPLY,
        CANCEL,
        CUSTOM_RATIO,
        WIDTH,
        HEIGHT,
        NARRATION_SAVE_BEFORE_OPEN,
        PREVIEW_PROGRESS,
        NARRATION_START,
        NARRATION_STARTING,
        NARRATION_SAVING,
        TOOLBAR_DRAG,
        RECORD_START,
        RECORD_STOP,
        RECORD_SAVING,
        MICROPHONE_ENABLE,
        MICROPHONE_DISABLE,
        CLICK_FOLLOW_ENABLE,
        CLICK_FOLLOW_DISABLE,
        EDITOR_OPEN,
        EDITOR_OPENING,
        EDITOR_MINIMIZE,
        EDITOR_RESTORE,
        SETTINGS,
        HIDE_TOOLBAR,
        CLOSE,
        SWITCH_LIGHT,
        SWITCH_DARK,
        SOURCE_TITLE,
        SOURCE_DISPLAY,
        SOURCE_WINDOW,
        SOURCE_REFRESH,
        SOURCE_EMPTY,
        SOURCE_REMAINING,
        DEVICE_TITLE,
        DEVICE_REFRESH,
        DEVICE_LOCKED,
        DEVICE_READING,
        LANGUAGE,
        LANGUAGE_ZH,
        LANGUAGE_EN,
        LANGUAGE_SWITCHED,
        CLI_RECORDING_FAILED,
        CLI_RECORDING_STARTED,
        DEVICE_MICROPHONE,
        DEVICE_RECORD_MICROPHONE,
        DEVICE_SYSTEM_DEFAULT,
        DEVICE_DISCONNECTED,
        DEVICE_INPUT,
        DEVICE_TEST_STOP,
        DEVICE_TEST_MICROPHONE,
        DEVICE_INPUT_LEVEL,
        DEVICE_NO_MICROPHONE,
        DEVICE_CAMERA,
        DEVICE_RECORD_CAMERA,
        DEVICE_NO_CAMERA,
        DEVICE_QUALITY,
        DEVICE_DETECTING_QUALITY,
        DEVICE_NO_QUALITY,
        DEVICE_OPEN_CAMERA,
        DEVICE_CLOSE_CAMERA,
        DEVICE_OPENING_CAMERA,
    ];
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Locale {
    language: Language,
}

impl Locale {
    pub const fn new(language: Language) -> Self {
        Self { language }
    }

    pub const fn language(self) -> Language {
        self.language
    }

    pub fn text(self, key: MessageKey) -> &'static str {
        message(self.language, key)
    }

    pub fn format(self, key: MessageKey, values: &[(&str, &str)]) -> String {
        let mut text = self.text(key).to_owned();
        for &(name, value) in values {
            text = text.replace(&format!("{{{name}}}"), value);
        }
        text
    }

    pub fn from_context(ctx: &eframe::egui::Context) -> Self {
        let language = ctx
            .data(|data| data.get_temp::<Language>(language_id()))
            .unwrap_or_else(|| Self::preferred().language);
        Self::new(language)
    }

    pub fn set_context(ctx: &eframe::egui::Context, language: Language) {
        ctx.data_mut(|data| data.insert_temp(language_id(), language));
        ctx.request_repaint();
    }

    pub fn initialize_context(ctx: &eframe::egui::Context) -> Self {
        let language = preference_path()
            .and_then(|path| read_preference(&path))
            .unwrap_or_else(Language::detect);
        Self::set_context(ctx, language);
        Self::new(language)
    }

    pub fn preferred() -> Self {
        Self::new(
            preference_path()
                .and_then(|path| read_preference(&path))
                .unwrap_or_else(Language::detect),
        )
    }

    pub fn recording_failed(self, detail: &str) -> String {
        format!("{}（{detail}）", self.text(key::CLI_RECORDING_FAILED))
    }

    pub fn set_context_and_persist(
        ctx: &eframe::egui::Context,
        language: Language,
    ) -> io::Result<()> {
        if let Some(path) = preference_path() {
            write_preference(&path, language)?;
        }
        Self::set_context(ctx, language);
        Ok(())
    }
}

impl Default for Locale {
    fn default() -> Self {
        Self::new(Language::detect())
    }
}

fn language_id() -> eframe::egui::Id {
    eframe::egui::Id::new("ottercut-language")
}

pub fn preference_path() -> Option<PathBuf> {
    crate::config_directory().map(|directory| directory.join("language"))
}

pub fn read_preference(path: &Path) -> Option<Language> {
    Language::from_tag(fs::read_to_string(path).ok()?.trim())
}

pub fn write_preference(path: &Path, language: Language) -> io::Result<()> {
    let directory = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "language preference has no parent",
        )
    })?;
    fs::create_dir_all(directory)?;
    let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
    writeln!(temporary, "{}", language.code())?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}

fn message(language: Language, key: MessageKey) -> &'static str {
    match language {
        Language::ZhCn => message_zh(key),
        Language::EnUs => message_en(key),
    }
}

fn message_zh(key: MessageKey) -> &'static str {
    match key.0 {
        "app.name" => "OtterCut",
        "editor.window_title" => "OtterCut 编辑控制台",
        "editor.file" => "文件",
        "editor.import_media" => "导入素材",
        "editor.import_audio" => "导入音频",
        "editor.open_project" => "打开工程",
        "editor.save_project" => "保存工程",
        "editor.export" => "导出",
        "editor.materials" => "素材",
        "editor.preview" => "预览",
        "editor.inspector" => "属性",
        "editor.timeline" => "时间线",
        "editor.workspace" => "工作区",
        "editor.play" => "播放",
        "editor.pause" => "暂停",
        "editor.fit" => "适应",
        "editor.preview_quality" => "预览质量",
        "editor.assets" => "素材",
        "editor.audio" => "音频",
        "editor.captions" => "字幕",
        "editor.text" => "文本",
        "editor.background" => "背景",
        "editor.add_to_track" => "添加到时间线",
        "editor.view_info" => "查看素材信息",
        "editor.open_folder" => "打开所在文件夹",
        "editor.copy_path" => "复制文件路径",
        "editor.remove_asset" => "移除素材",
        "editor.apply" => "应用",
        "editor.cancel" => "取消",
        "editor.custom_ratio" => "自定义比例",
        "editor.width" => "宽度",
        "editor.height" => "高度",
        "editor.narration_save_before_open" => "旁白保存后可打开工程",
        "editor.preview_progress" => "预览进度",
        "editor.narration_start" => "录制旁白",
        "editor.narration_starting" => "启动录音中…",
        "editor.narration_saving" => "保存旁白中…",
        "toolbar.drag" => "拖动工具条",
        "toolbar.record_start" => "开始录制",
        "toolbar.record_stop" => "停止录制",
        "toolbar.record_saving" => "正在保存",
        "toolbar.microphone_enable" => "开启麦克风",
        "toolbar.microphone_disable" => "关闭麦克风",
        "toolbar.click_follow_enable" => "开启点击跟随",
        "toolbar.click_follow_disable" => "关闭点击跟随",
        "toolbar.editor_open" => "打开剪辑台",
        "toolbar.editor_opening" => "正在打开剪辑台",
        "toolbar.editor_minimize" => "收起剪辑台",
        "toolbar.editor_restore" => "恢复剪辑台",
        "toolbar.settings" => "设备设置",
        "toolbar.hide" => "隐藏工具栏",
        "common.close" => "关闭",
        "theme.switch_light" => "切换到浅色模式",
        "theme.switch_dark" => "切换到深色模式",
        "source.title" => "录制来源",
        "source.display" => "录制屏幕",
        "source.window" => "录制窗口",
        "source.refresh" => "刷新来源",
        "source.empty" => "当前没有可录制的来源。打开目标窗口后可刷新列表。",
        "source.remaining" => "右侧还有 {count} 张",
        "device.title" => "设备设置",
        "device.refresh" => "刷新设备",
        "device.locked" => "录制中 · 设备已锁定",
        "device.reading" => "正在读取设备…",
        "common.language" => "语言",
        "common.language_zh" => "简体中文",
        "common.language_en" => "English",
        "common.language_switched" => "语言已切换",
        "cli.recording_failed" => "录制失败",
        "cli.recording_started" => "正在录制，按 Ctrl+C 停止",
        "device.microphone" => "麦克风",
        "device.record_microphone" => "录制麦克风",
        "device.system_default" => "系统默认",
        "device.disconnected" => "已断开",
        "device.input" => "输入设备",
        "device.test_stop" => "停止测试",
        "device.test_microphone" => "测试麦克风",
        "device.input_level" => "输入电平 {value}%",
        "device.no_microphone" => "未找到麦克风输入",
        "device.camera" => "摄像头",
        "device.record_camera" => "录制摄像头",
        "device.no_camera" => "未检测到摄像头",
        "device.quality" => "画质",
        "device.detecting_quality" => "正在检测画质…",
        "device.no_quality" => "无可用画质",
        "device.open_camera" => "打开摄像头预览",
        "device.close_camera" => "关闭摄像头预览",
        "device.opening_camera" => "正在打开摄像头…",
        _ => key.0,
    }
}

fn message_en(key: MessageKey) -> &'static str {
    match key.0 {
        "app.name" => "OtterCut",
        "editor.window_title" => "OtterCut Editor",
        "editor.file" => "File",
        "editor.import_media" => "Import media",
        "editor.import_audio" => "Import audio",
        "editor.open_project" => "Open project",
        "editor.save_project" => "Save project",
        "editor.export" => "Export",
        "editor.materials" => "Materials",
        "editor.preview" => "Preview",
        "editor.inspector" => "Inspector",
        "editor.timeline" => "Timeline",
        "editor.workspace" => "Workspace",
        "editor.play" => "Play",
        "editor.pause" => "Pause",
        "editor.fit" => "Fit",
        "editor.preview_quality" => "Preview quality",
        "editor.assets" => "Assets",
        "editor.audio" => "Audio",
        "editor.captions" => "Captions",
        "editor.text" => "Text",
        "editor.background" => "Background",
        "editor.add_to_track" => "Add to timeline",
        "editor.view_info" => "View media info",
        "editor.open_folder" => "Open containing folder",
        "editor.copy_path" => "Copy file path",
        "editor.remove_asset" => "Remove media",
        "editor.apply" => "Apply",
        "editor.cancel" => "Cancel",
        "editor.custom_ratio" => "Custom ratio",
        "editor.width" => "Width",
        "editor.height" => "Height",
        "editor.narration_save_before_open" => "Save narration before opening a project",
        "editor.preview_progress" => "Preview progress",
        "editor.narration_start" => "Record narration",
        "editor.narration_starting" => "Starting narration…",
        "editor.narration_saving" => "Saving narration…",
        "toolbar.drag" => "Drag toolbar",
        "toolbar.record_start" => "Start recording",
        "toolbar.record_stop" => "Stop recording",
        "toolbar.record_saving" => "Saving",
        "toolbar.microphone_enable" => "Enable microphone",
        "toolbar.microphone_disable" => "Disable microphone",
        "toolbar.click_follow_enable" => "Enable click follow",
        "toolbar.click_follow_disable" => "Disable click follow",
        "toolbar.editor_open" => "Open editor",
        "toolbar.editor_opening" => "Opening editor",
        "toolbar.editor_minimize" => "Minimize editor",
        "toolbar.editor_restore" => "Restore editor",
        "toolbar.settings" => "Device settings",
        "toolbar.hide" => "Hide toolbar",
        "common.close" => "Close",
        "theme.switch_light" => "Switch to light mode",
        "theme.switch_dark" => "Switch to dark mode",
        "source.title" => "Recording source",
        "source.display" => "Record screen",
        "source.window" => "Record window",
        "source.refresh" => "Refresh sources",
        "source.empty" => "No recordable sources. Open a target window and refresh the list.",
        "source.remaining" => "{count} more on the right",
        "device.title" => "Device settings",
        "device.refresh" => "Refresh devices",
        "device.locked" => "Recording · devices locked",
        "device.reading" => "Reading devices…",
        "common.language" => "Language",
        "common.language_zh" => "简体中文",
        "common.language_en" => "English",
        "common.language_switched" => "Language changed",
        "cli.recording_failed" => "Recording failed",
        "cli.recording_started" => "Recording. Press Ctrl+C to stop",
        "device.microphone" => "Microphone",
        "device.record_microphone" => "Record microphone",
        "device.system_default" => "System default",
        "device.disconnected" => "Disconnected",
        "device.input" => "Input device",
        "device.test_stop" => "Stop test",
        "device.test_microphone" => "Test microphone",
        "device.input_level" => "Input level {value}%",
        "device.no_microphone" => "No microphone input found",
        "device.camera" => "Camera",
        "device.record_camera" => "Record camera",
        "device.no_camera" => "No camera detected",
        "device.quality" => "Quality",
        "device.detecting_quality" => "Detecting quality…",
        "device.no_quality" => "No available quality",
        "device.open_camera" => "Open camera preview",
        "device.close_camera" => "Close camera preview",
        "device.opening_camera" => "Opening camera…",
        _ => message_zh(key),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_use_language_prefixes() {
        assert_eq!(Language::from_tag("en-US"), Some(Language::EnUs));
        assert_eq!(Language::from_tag("zh_CN"), Some(Language::ZhCn));
        assert_eq!(Language::from_tag("fr-FR"), None);
    }

    #[test]
    fn format_replaces_named_values() {
        let locale = Locale::new(Language::EnUs);
        assert_eq!(
            locale.format(key::SOURCE_REMAINING, &[("count", "3")]),
            "3 more on the right"
        );
    }

    #[test]
    fn preference_round_trips() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("language");
        write_preference(&path, Language::EnUs).unwrap();
        assert_eq!(read_preference(&path), Some(Language::EnUs));
    }

    #[test]
    fn all_declared_messages_are_translated() {
        for key in key::ALL {
            assert_ne!(Locale::new(Language::ZhCn).text(*key), key.0);
            assert_ne!(Locale::new(Language::EnUs).text(*key), key.0);
        }
    }
}
