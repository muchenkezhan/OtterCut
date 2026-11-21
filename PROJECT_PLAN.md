# OtterCut 项目规划

OtterCut 是一款面向教程作者、开发者和产品演示人员的开源桌面应用。用 Rust 实现录制和剪辑核心，面向 Windows、macOS 和 Linux。录制结束后，素材直接进入本地工程，用户可以手动剪辑、用自然语言让内置 AI 助手修改时间线，也可以通过 MCP 让外部助手参与剪辑。

## 命名与 Logo

![OtterCut Logo](assets/takeseam-brand.png)

**OtterCut** 是软件的显示名称。对外描述可用「开源跨平台录屏与 AI 剪辑工具」。

### 早期候选名称

| 候选名 | 含义 |
| --- | --- |
| **TakeSeam（录序）** | Take 代表一次录制，Seam 代表剪辑接缝。「录序」对应录下内容，再整理顺序。建议仓库名 `takeseam`。 |
| TakeNotch | Notch 是切口，名字强调剪辑的切分动作。 |
| ReelLatch | Reel 指视频片段，Latch 指扣合，名字强调片段拼接。 |

窗口和托盘使用森林绿底的品牌图案，录制工具条使用奶油色主体的透明图标。两种标志均保留 SVG 源文件，运行时使用由 SVG 导出的 PNG。

## 首版体验

以剪映的基础剪辑流程为体验参照，优先做录屏后的快速成片。用户录完或导入素材后，能在同一个窗口裁掉多余片段、加字幕、预览并导出。 [剪映专业版](https://www.jianying.com/web/professional)

| 环节 | v0.1 范围 |
| --- | --- |
| 录制 | 选择屏幕或窗口，录制画面和可选麦克风。提供开始、停止、录制状态、剩余磁盘空间及错误反馈。以 1080p / 30 fps 为验证目标。 |
| 剪辑 | 停止录制后直接创建工程。支持导入已验证编码的本地 MP4 素材，在单视频轨上预览、裁头尾、切分、删除、重排和撤销。录制的麦克风保留为独立音轨，可调音量或静音。 |
| 字幕 | 手动输入短字幕，调整起止时间。字幕在视频预览中可见，导出时烧录到成片。 |
| AI 助手 | 支持中英文自然语言指令。首版能按时间或片段执行裁剪、切分、删除、重排、静音和调音量。用户可预览改动并应用，应用后可整批撤销。 |
| MCP | 提供本地 MCP 服务。外部助手可在已连接的工程中查看时间线、预览一批剪辑并应用。 |
| 多语言 | 界面首发简体中文和英语，默认跟随系统，可在设置中切换。语言设置独立于工程，切换界面语言后仍可打开同一份工程。 |
| 保存 | 保存原始媒体和本地工程。工程记录素材引用、片段起止点、顺序、音轨设置和字幕，关闭后可重新打开继续剪辑。 |
| 导出 | 提供一个 MP4（H.264/AAC）预设，显示进度，允许取消，完成后打开文件所在目录。发布前在目标安装包上验收实际可用的编码器。 |

系统声音、摄像头画中画和区域录制进入后续迭代。根据使用反馈评估自动字幕、光标高亮和多视频轨。手动录制、剪辑、保存和导出始终可以离线使用。内置 AI 助手的可用性取决于已配置的模型。

## 技术方案

- **桌面端统一用 Rust**。用 `eframe` 创建桌面应用并处理窗口事件，用 `egui` 编写录制控制、单轨时间线、字幕编辑和 AI 面板。两者通过 Cargo 管理。首版播放器解码本地素材或代理视频，把当前画面交给界面显示，按工程时间线显示字幕。先在 Windows 验证播放、跨片段定位和中文输入法。 [eframe 文档](https://docs.rs/eframe/latest/eframe/) · [TraceDraw 桌面端依赖](https://github.com/tracedraw/tracedraw/blob/main/apps/tracedraw/Cargo.toml)
- **屏幕采集**在 Rust 中按平台接入。Windows 使用 Windows.Graphics.Capture，macOS 使用 ScreenCaptureKit，Linux Wayland 使用 XDG Desktop Portal 选择录制源，经 PipeWire 获取画面。Linux 先选一套 GNOME Wayland 环境作为验收基线，再扩展其他桌面环境。 [Microsoft 采集文档](https://learn.microsoft.com/en-us/windows/uwp/audio-video-camera/screen-capture) · [Apple ScreenCaptureKit](https://developer.apple.com/documentation/screencapturekit) · [ScreenCast Portal](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.ScreenCast.html)
- **时间戳与录制恢复**。采集层把视频帧和音频采样映射到同一单调时间轴，使用有界缓冲并记录丢帧。验证分段写盘与索引恢复，应用意外退出后能找回已经完整写入的片段。停止录制时将会话封装成可预览素材。 [FFmpeg 容器文档](https://ffmpeg.org/ffmpeg-formats.html)
- **剪辑与工程**。采用带版本号的 JSON 工程、稳定素材 ID 和相对路径。内部用整数时间戳表达素材入点、出点与轨道偏移。预览与导出共享同一份剪辑数据，保存时用原子替换保护已有工程。
- **媒体工具**。原型使用外部 FFmpeg 与 ffprobe 处理探测、代理生成和导出。录制时的编码与复用路径由 P0 原型验证后确定。发布时拟随安装包分发经验证的独立可执行文件，并记录准确版本、构建选项及许可证。 [FFmpeg 许可说明](https://ffmpeg.org/legal.html)
- **国际化**。界面文案和错误提示通过资源键维护，Rust 返回稳定错误码和参数。界面按语言格式化日期、数字和单位，工程数据与 MCP 保持统一的时间单位和字段。随包提供覆盖中英文的界面字体，配套发布两种语言的快速上手说明。

## AI 剪辑与 MCP 接入

用户可输入「剪掉开头 3 秒，把麦克风降低 6 dB」。助手读取当前时间线，把指令转换为结构化剪辑操作。时间线先显示受影响的片段和音轨，应用后立即更新预览。一次操作批次对应一条撤销记录。

- **统一执行**。手动操作、内置助手和外部 MCP 助手共用 Rust 剪辑命令。核心校验片段 ID、时间范围和工程修订号。若预览后工程发生变化，重新生成剪辑预览。应用以整批为单位提交，重试可返回原结果，避免重复剪切。
- **MCP 服务**。首版采用本地 `stdio`。MCP 子进程把结构化请求转给桌面进程，由桌面端统一写入工程。首批工具为 `get_timeline`、`preview_edits`、`apply_edits`，分别用于读取时间线、验证并预览操作、应用预览结果。工具只操作已连接的工程和其中的素材 ID。 [MCP 工具文档](https://modelcontextprotocol.io/specification/2025-11-25/server/tools) · [MCP stdio 文档](https://ts.sdk.modelcontextprotocol.io/v2/serving/stdio)
- **模型接入**。首版接入一个支持结构化输出的模型，先验证中英文剪辑指令。发送给在线模型的默认上下文限于指令、必要的片段元数据与时间码。录屏和音频保留在本地，后续扩展本地模型与转写服务。
- **内容理解**。后续基于带时间戳的转写支持「删掉我说错版本号的那段」，基于音频检测支持「去掉超过两秒的静音」。定位结果复用同一套剪辑预览、应用和撤销流程。

## 开发里程碑

| 阶段 | 交付 | 验收条件 |
| --- | --- | --- |
| **P0 Windows 原型** | 在 Windows 11 上采集一个屏幕或窗口及可选麦克风，写出可独立播放的文件。 | 在参考设备上录制 30 分钟 1080p / 30 fps，记录 CPU、内存和丢帧。用同步的画面与声音标记测量，开头和结尾的音画偏差均以不超过 100 ms 为目标。 |
| **P1 剪辑核心原型** | 用 P0 素材打通录制结束进入工程、手动剪辑、加字幕、预览和 MP4 导出，同时定义结构化剪辑命令并验证 Rust 桌面界面。 | 拖动时间线可定位预览，连续播放可跨过剪辑点，字幕输入框可用中文输入法。预览与成片的片段顺序和字幕时序一致，关闭重开工程后能继续编辑。 |
| **v0.1 Windows 预览版** | 完成录剪导出流程、简体中文和英语界面、内置 AI 剪辑助手及本地 MCP。 | 同一工程切换中英界面后可继续编辑。中英文指令和外部 MCP 调用均能应用相同的剪辑并撤销。验证过期预览、重复请求、权限拒绝、磁盘不足、取消导出及强制退出后的完整片段恢复。 |
| **v0.2 macOS 预览版** | 先在 Apple Silicon 上复用工程和编辑器，接入原生录制与权限提示。 | 在实机上跑通 v0.1 工作流，验证屏幕权限、麦克风权限及拒绝后重新授权。 |
| **v0.3 Linux 预览版** | 先适配 GNOME Wayland，并确定一种首发分发格式。 | 在 GNOME Wayland 实机上跑通 v0.1 工作流，验证选取录制源、取消选择和终止录制会话。 |

## 开源发布

项目代码建议采用 Apache-2.0。首个公开版本提供构建说明、贡献指南、平台能力表和第三方许可清单。发布安装包前按实际 FFmpeg 构建配置核对再分发条件，并单独核对 H.264/AAC 的分发条件和各平台的编码能力。 [Apache-2.0 许可证](https://www.apache.org/licenses/LICENSE-2.0) · [FFmpeg 许可与专利说明](https://ffmpeg.org/legal.html)

## 旧版 Logo 生成记录

使用内置 ImageGen 生成字形草稿并精修透明底标志。

<details>
<summary>生成与精修提示词</summary>

Use case: logo-brand. Design exactly one original standalone symbol for TakeSeam, an independent open-source desktop screen recorder and lightweight video timeline editor. The concept is the seam where two trimmed clips slide together. Create a bespoke uppercase S monogram: two broad angular ribbons, rotated 180 degrees relative to each other, with short square-ended horizontal arms and one distinct stepped negative-space cut that aligns them. The two forms interlock optically, without crossing each other or touching. Cohesive compact, almost square silhouette, deliberate optical spacing; think custom cast-metal maker's mark translated into simple flat vector geometry, not a literal letter in a font. Slight chamfers where a cut is implied, a few eased outer corners. Use exactly one saturated burnt vermilion color (#F25D46 approx). Background and internal gaps genuinely transparent. One centered symbol on a square canvas, fills about 65–70% of the canvas; readable as a confident silhouette at 24 px, gaps comfortably wide. Crisp edge and perfectly flat color. No application tile, record dot, camera, play triangle, film perforations, scissors, frame box, stripes, letters other than the emblem itself, tagline, mockup, metallic effect, drop shadow, glow, texture or gradients. Do not imitate known video editor logos.

Refine this specific TakeSeam brand symbol for a production logo proof. Preserve its strong squared-off S silhouette, diagonal chamfers at upper left and lower right, and the unique stepped middle seam. Rebuild the mark as hard, clean geometric shapes with mathematically straight edges, small intentional outer corner rounding, and one entirely uniform, flat warm vermilion fill, target #EF513B. Slightly clarify the center join with a single bold diagonal cut through the middle, like two clips meeting on a timeline. Keep all negative spaces open and generous enough to read at 24 pixels. Deliver only the one corrected mark, centered at about 65% of the square canvas. All outside areas and cutouts must have alpha = 0, completely transparent, with antialiasing only on the immediate outline. The output should look like a clean export from a vector drawing tool. Remove every trace of speckle, grain, residue or mottling around the mark and inside every gap. No frame, tile, words, effects, texture, layered colors, gradient, highlights, shadows or mockup.

</details>
