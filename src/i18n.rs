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

    pub const CURSOR_EFFECT: MessageKey = MessageKey("cursor.effect");
    pub const CURSOR_APPEARANCE: MessageKey = MessageKey("cursor.appearance");
    pub const CURSOR_ANIMATION: MessageKey = MessageKey("cursor.animation");
    pub const CURSOR_LOOK: MessageKey = MessageKey("cursor.look");
    pub const CURSOR_RANGE: MessageKey = MessageKey("cursor.range");
    pub const CURSOR_CLICK_POINT: MessageKey = MessageKey("cursor.click_point");
    pub const CURSOR_CLICK_DELETE: MessageKey = MessageKey("cursor.click_delete");
    pub const CURSOR_CLICK_DELETE_HINT: MessageKey = MessageKey("cursor.click_delete_hint");
    pub const CURSOR_START: MessageKey = MessageKey("cursor.start");
    pub const CURSOR_END: MessageKey = MessageKey("cursor.end");
    pub const CURSOR_SIZE: MessageKey = MessageKey("cursor.size");
    pub const CURSOR_STYLE: MessageKey = MessageKey("cursor.style");
    pub const CURSOR_FEEDBACK: MessageKey = MessageKey("cursor.feedback");
    pub const CURSOR_RING: MessageKey = MessageKey("cursor.ring");
    pub const CURSOR_DOT: MessageKey = MessageKey("cursor.dot");
    pub const CURSOR_NONE: MessageKey = MessageKey("cursor.none");
    pub const CURSOR_ZOOM: MessageKey = MessageKey("cursor.zoom");
    pub const CURSOR_SCALE: MessageKey = MessageKey("cursor.scale");
    pub const CURSOR_FOLLOW: MessageKey = MessageKey("cursor.follow");
    pub const CURSOR_FOLLOW_POINTER: MessageKey = MessageKey("cursor.follow_pointer");
    pub const CURSOR_FOLLOW_CLICKS: MessageKey = MessageKey("cursor.follow_clicks");
    pub const CURSOR_SMOOTHING: MessageKey = MessageKey("cursor.smoothing");
    pub const CURSOR_PAN: MessageKey = MessageKey("cursor.pan");
    pub const CURSOR_HOLD: MessageKey = MessageKey("cursor.hold");
    pub const CURSOR_TRANSITION: MessageKey = MessageKey("cursor.transition");
    pub const CURSOR_AUTO: MessageKey = MessageKey("cursor.auto");
    pub const CURSOR_CUSTOM: MessageKey = MessageKey("cursor.custom");
    pub const CURSOR_ZOOM_IN: MessageKey = MessageKey("cursor.zoom_in");
    pub const CURSOR_ZOOM_OUT: MessageKey = MessageKey("cursor.zoom_out");
    pub const CURSOR_TRANSITION_TIME: MessageKey = MessageKey("cursor.transition_time");
    pub const CURSOR_SECONDS: MessageKey = MessageKey("cursor.seconds");
    pub const CURSOR_NUMBER_HELP: MessageKey = MessageKey("cursor.number_help");
    pub const CURSOR_STYLE_DEFAULT: MessageKey = MessageKey("cursor.style_default");
    pub const CURSOR_STYLE_STUDIO_INK: MessageKey = MessageKey("cursor.style_studio_ink");
    pub const CURSOR_STYLE_PRISM: MessageKey = MessageKey("cursor.style_prism");
    pub const CURSOR_STYLE_CORAL: MessageKey = MessageKey("cursor.style_coral");
    pub const CURSOR_STYLE_PIXEL: MessageKey = MessageKey("cursor.style_pixel");
    pub const CURSOR_STYLE_STAR_SPROUT: MessageKey = MessageKey("cursor.style_star_sprout");
    pub const CURSOR_STYLE_GLASS_LENS: MessageKey = MessageKey("cursor.style_glass_lens");

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
    pub const TRANSITIONS: MessageKey = MessageKey("editor.transitions");
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
    pub const CAMERA_ENABLE: MessageKey = MessageKey("toolbar.camera_enable");
    pub const CAMERA_DISABLE: MessageKey = MessageKey("toolbar.camera_disable");
    pub const CAMERA_PREVIEW_SHOW: MessageKey = MessageKey("toolbar.camera_preview_show");
    pub const CAMERA_PREVIEW_HIDE: MessageKey = MessageKey("toolbar.camera_preview_hide");
    pub const CAMERA_PREVIEW_SHAPE: MessageKey = MessageKey("device.camera_preview_shape");
    pub const CAMERA_PREVIEW_CIRCLE: MessageKey = MessageKey("device.camera_preview_circle");
    pub const CAMERA_PREVIEW_SQUARE: MessageKey = MessageKey("device.camera_preview_square");
    pub const CAMERA_PREVIEW_UNAVAILABLE: MessageKey =
        MessageKey("toolbar.camera_preview_unavailable");
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
    pub const SOURCE_PICKER: MessageKey = MessageKey("source.picker");
    pub const SOURCE_NOT_SELECTED: MessageKey = MessageKey("source.not_selected");
    pub const SOURCE_SELECTED_DISPLAY: MessageKey = MessageKey("source.selected_display");
    pub const SOURCE_SELECTED_WINDOW: MessageKey = MessageKey("source.selected_window");
    pub const SOURCE_SAVE_TO: MessageKey = MessageKey("source.save_to");
    pub const SOURCE_CHANGE_FOLDER: MessageKey = MessageKey("source.change_folder");
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
    pub const DEVICE_PREVIEW_EXTERNAL: MessageKey = MessageKey("device.preview_external");
    pub const EXPORT_SETTINGS: MessageKey = MessageKey("export.settings");
    pub const EXPORT_OUTPUT: MessageKey = MessageKey("export.output");
    pub const EXPORT_TITLE: MessageKey = MessageKey("export.title");
    pub const EXPORT_FORMAT: MessageKey = MessageKey("export.format");
    pub const EXPORT_MP4: MessageKey = MessageKey("export.mp4");
    pub const EXPORT_QUALITY: MessageKey = MessageKey("export.quality");
    pub const EXPORT_QUALITY_HIGH: MessageKey = MessageKey("export.quality_high");
    pub const EXPORT_QUALITY_STANDARD: MessageKey = MessageKey("export.quality_standard");
    pub const EXPORT_QUALITY_SMALL: MessageKey = MessageKey("export.quality_small");
    pub const EXPORT_AUDIO: MessageKey = MessageKey("export.audio");
    pub const EXPORT_AUDIO_BITRATE: MessageKey = MessageKey("export.audio_bitrate");
    pub const EXPORT_BACKGROUND: MessageKey = MessageKey("export.background");
    pub const EXPORT_START: MessageKey = MessageKey("export.start");
    pub const EXPORT_PATH_HINT: MessageKey = MessageKey("export.path_hint");
    pub const EXPORT_TIMELINE: MessageKey = MessageKey("export.timeline");
    pub const EXPORT_MAIN_TIMELINE: MessageKey = MessageKey("export.main_timeline");
    pub const EXPORT_SUBTITLES: MessageKey = MessageKey("export.subtitles");
    pub const EXPORT_ENCODER: MessageKey = MessageKey("export.encoder");
    pub const EXPORT_AUTO: MessageKey = MessageKey("export.auto");
    pub const EXPORT_ESTIMATE: MessageKey = MessageKey("export.estimate");
    pub const EXPORT_PREVIEW: MessageKey = MessageKey("export.preview");
    pub const EXPORT_RESOLUTION: MessageKey = MessageKey("export.resolution");
    pub const EXPORT_RESOLUTION_CANVAS: MessageKey = MessageKey("export.resolution_canvas");
    pub const EXPORT_RESOLUTION_720: MessageKey = MessageKey("export.resolution_720");
    pub const EXPORT_RESOLUTION_1080: MessageKey = MessageKey("export.resolution_1080");
    pub const EXPORT_RESOLUTION_4K: MessageKey = MessageKey("export.resolution_4k");

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
        CURSOR_EFFECT,
        CURSOR_APPEARANCE,
        CURSOR_ANIMATION,
        CURSOR_LOOK,
        CURSOR_RANGE,
        CURSOR_CLICK_POINT,
        CURSOR_CLICK_DELETE,
        CURSOR_CLICK_DELETE_HINT,
        CURSOR_START,
        CURSOR_END,
        CURSOR_SIZE,
        CURSOR_STYLE,
        CURSOR_FEEDBACK,
        CURSOR_RING,
        CURSOR_DOT,
        CURSOR_NONE,
        CURSOR_ZOOM,
        CURSOR_SCALE,
        CURSOR_FOLLOW,
        CURSOR_FOLLOW_POINTER,
        CURSOR_FOLLOW_CLICKS,
        CURSOR_SMOOTHING,
        CURSOR_PAN,
        CURSOR_HOLD,
        CURSOR_TRANSITION,
        CURSOR_AUTO,
        CURSOR_CUSTOM,
        CURSOR_ZOOM_IN,
        CURSOR_ZOOM_OUT,
        CURSOR_TRANSITION_TIME,
        CURSOR_SECONDS,
        CURSOR_NUMBER_HELP,
        CURSOR_STYLE_DEFAULT,
        CURSOR_STYLE_STUDIO_INK,
        CURSOR_STYLE_PRISM,
        CURSOR_STYLE_CORAL,
        CURSOR_STYLE_PIXEL,
        CURSOR_STYLE_STAR_SPROUT,
        CURSOR_STYLE_GLASS_LENS,
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
        TRANSITIONS,
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
        CAMERA_ENABLE,
        CAMERA_DISABLE,
        CAMERA_PREVIEW_SHOW,
        CAMERA_PREVIEW_HIDE,
        CAMERA_PREVIEW_SHAPE,
        CAMERA_PREVIEW_CIRCLE,
        CAMERA_PREVIEW_SQUARE,
        CAMERA_PREVIEW_UNAVAILABLE,
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
        SOURCE_PICKER,
        SOURCE_NOT_SELECTED,
        SOURCE_SELECTED_DISPLAY,
        SOURCE_SELECTED_WINDOW,
        SOURCE_SAVE_TO,
        SOURCE_CHANGE_FOLDER,
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
        DEVICE_PREVIEW_EXTERNAL,
        EXPORT_SETTINGS,
        EXPORT_OUTPUT,
        EXPORT_TITLE,
        EXPORT_FORMAT,
        EXPORT_MP4,
        EXPORT_QUALITY,
        EXPORT_QUALITY_HIGH,
        EXPORT_QUALITY_STANDARD,
        EXPORT_QUALITY_SMALL,
        EXPORT_AUDIO,
        EXPORT_AUDIO_BITRATE,
        EXPORT_BACKGROUND,
        EXPORT_START,
        EXPORT_PATH_HINT,
        EXPORT_TIMELINE,
        EXPORT_MAIN_TIMELINE,
        EXPORT_SUBTITLES,
        EXPORT_ENCODER,
        EXPORT_AUTO,
        EXPORT_ESTIMATE,
        EXPORT_PREVIEW,
        EXPORT_RESOLUTION,
        EXPORT_RESOLUTION_CANVAS,
        EXPORT_RESOLUTION_720,
        EXPORT_RESOLUTION_1080,
        EXPORT_RESOLUTION_4K,
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
        "cursor.effect" => "鼠标效果",
        "cursor.appearance" => "光标",
        "cursor.animation" => "动画",
        "cursor.look" => "外观",
        "cursor.range" => "生效范围",
        "cursor.click_point" => "鼠标点击",
        "cursor.click_delete" => "删除点击点",
        "cursor.click_delete_hint" => "选中后按 Delete 删除",
        "cursor.start" => "开始时间",
        "cursor.end" => "结束时间",
        "cursor.size" => "大小",
        "cursor.style" => "光标样式",
        "cursor.feedback" => "点击反馈",
        "cursor.ring" => "波纹",
        "cursor.dot" => "圆点",
        "cursor.none" => "无",
        "cursor.zoom" => "点击放大",
        "cursor.scale" => "放大倍率",
        "cursor.follow" => "镜头模式",
        "cursor.follow_pointer" => "跟随鼠标",
        "cursor.follow_clicks" => "点击定位",
        "cursor.smoothing" => "跟随平滑",
        "cursor.pan" => "平移时长",
        "cursor.hold" => "停留时长",
        "cursor.transition" => "过渡",
        "cursor.auto" => "自动",
        "cursor.custom" => "自定义",
        "cursor.zoom_in" => "放大时长",
        "cursor.zoom_out" => "缩回时长",
        "cursor.transition_time" => "过渡时长",
        "cursor.seconds" => "秒",
        "cursor.number_help" => "单击输入数值，横向拖动微调",
        "cursor.style_default" => "默认",
        "cursor.style_studio_ink" => "墨黑",
        "cursor.style_prism" => "棱镜",
        "cursor.style_coral" => "珊瑚",
        "cursor.style_pixel" => "像素",
        "cursor.style_star_sprout" => "星芽",
        "cursor.style_glass_lens" => "玻璃",
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
        "editor.transitions" => "转场",
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
        "toolbar.camera_enable" => "开启摄像头录制",
        "toolbar.camera_disable" => "关闭摄像头录制",
        "toolbar.camera_preview_show" => "显示摄像头预览",
        "toolbar.camera_preview_hide" => "隐藏摄像头预览",
        "device.camera_preview_shape" => "预览形状",
        "device.camera_preview_circle" => "圆形",
        "device.camera_preview_square" => "正方形",
        "toolbar.camera_preview_unavailable" => "当前录制来源无法显示悬浮预览",
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
        "source.picker" => "系统选择器…",
        "source.not_selected" => "尚未选择来源",
        "source.selected_display" => "已选择屏幕",
        "source.selected_window" => "已选择窗口",
        "source.save_to" => "保存至",
        "source.change_folder" => "更改…",
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
        "device.preview_external" => "预览在独立窗口显示",
        "export.settings" => "导出设置",
        "export.output" => "输出文件",
        "export.title" => "标题",
        "export.format" => "格式",
        "export.mp4" => "MP4 视频",
        "export.quality" => "视频质量",
        "export.quality_high" => "高质量",
        "export.quality_standard" => "标准",
        "export.quality_small" => "小文件",
        "export.audio" => "导出音频",
        "export.audio_bitrate" => "音频码率",
        "export.background" => "包含动态背景",
        "export.start" => "开始导出",
        "export.path_hint" => "可输入绝对路径或相对工程目录的文件名",
        "export.timeline" => "导出时间线",
        "export.main_timeline" => "主时间线",
        "export.subtitles" => "字幕与画面文字",
        "export.encoder" => "视频编码",
        "export.auto" => "自动 · H.264",
        "export.estimate" => "时长 {duration} · 预计大小 {size}",
        "export.preview" => "成片预览",
        "export.resolution" => "分辨率",
        "export.resolution_canvas" => "项目画幅",
        "export.resolution_720" => "720p",
        "export.resolution_1080" => "1080p",
        "export.resolution_4k" => "4K",
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
        "cursor.effect" => "Mouse effects",
        "cursor.appearance" => "Cursor",
        "cursor.animation" => "Animation",
        "cursor.look" => "Appearance",
        "cursor.range" => "Active range",
        "cursor.click_point" => "Cursor click",
        "cursor.click_delete" => "Delete click point",
        "cursor.click_delete_hint" => "Select and press Delete to remove",
        "cursor.start" => "Start time",
        "cursor.end" => "End time",
        "cursor.size" => "Size",
        "cursor.style" => "Cursor style",
        "cursor.feedback" => "Click feedback",
        "cursor.ring" => "Ring",
        "cursor.dot" => "Dot",
        "cursor.none" => "None",
        "cursor.zoom" => "Click zoom",
        "cursor.scale" => "Zoom scale",
        "cursor.follow" => "Camera mode",
        "cursor.follow_pointer" => "Follow cursor",
        "cursor.follow_clicks" => "Click points",
        "cursor.smoothing" => "Smoothing",
        "cursor.pan" => "Pan time",
        "cursor.hold" => "Hold time",
        "cursor.transition" => "Transition",
        "cursor.auto" => "Auto",
        "cursor.custom" => "Custom",
        "cursor.zoom_in" => "Zoom in",
        "cursor.zoom_out" => "Zoom out",
        "cursor.transition_time" => "Duration",
        "cursor.seconds" => "s",
        "cursor.number_help" => "Click to enter a value or drag sideways to adjust it",
        "cursor.style_default" => "Default",
        "cursor.style_studio_ink" => "Studio Ink",
        "cursor.style_prism" => "Prism",
        "cursor.style_coral" => "Coral",
        "cursor.style_pixel" => "Pixel",
        "cursor.style_star_sprout" => "Star Sprout",
        "cursor.style_glass_lens" => "Glass Lens",
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
        "editor.transitions" => "Transitions",
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
        "toolbar.camera_enable" => "Enable camera recording",
        "toolbar.camera_disable" => "Disable camera recording",
        "toolbar.camera_preview_show" => "Show camera preview",
        "toolbar.camera_preview_hide" => "Hide camera preview",
        "device.camera_preview_shape" => "Preview shape",
        "device.camera_preview_circle" => "Circle",
        "device.camera_preview_square" => "Square",
        "toolbar.camera_preview_unavailable" => {
            "Floating camera preview is unavailable for this recording source"
        }
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
        "source.picker" => "System picker…",
        "source.not_selected" => "No source selected",
        "source.selected_display" => "Display selected",
        "source.selected_window" => "Window selected",
        "source.save_to" => "Save to",
        "source.change_folder" => "Change…",
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
        "device.preview_external" => "Preview opens in a separate window",
        "export.settings" => "Export settings",
        "export.output" => "Output file",
        "export.title" => "Title",
        "export.format" => "Format",
        "export.mp4" => "MP4 video",
        "export.quality" => "Video quality",
        "export.quality_high" => "High quality",
        "export.quality_standard" => "Standard",
        "export.quality_small" => "Small file",
        "export.audio" => "Export audio",
        "export.audio_bitrate" => "Audio bitrate",
        "export.background" => "Include dynamic background",
        "export.start" => "Start export",
        "export.path_hint" => "Enter an absolute path or a filename relative to the project folder",
        "export.timeline" => "Export timeline",
        "export.main_timeline" => "Main timeline",
        "export.subtitles" => "Subtitles and on-screen text",
        "export.encoder" => "Video encoder",
        "export.auto" => "Automatic · H.264",
        "export.estimate" => "Duration {duration} · Estimated size {size}",
        "export.preview" => "Export preview",
        "export.resolution" => "Resolution",
        "export.resolution_canvas" => "Project canvas",
        "export.resolution_720" => "720p",
        "export.resolution_1080" => "1080p",
        "export.resolution_4k" => "4K",
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
