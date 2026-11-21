use std::fmt::Write as _;
use std::fs::File;
use std::io::{self, BufWriter, Read, Write};
use std::path::Path;

use anyhow::{Context, Result, ensure};

use crate::editor::background::{
    BackgroundFrame, BackgroundMotion, BackgroundSettings, BackgroundShadow, BackgroundSource,
};
use crate::editor::dynamic_recipe::{DynamicPattern, DynamicRecipe};

use super::local_media;
use super::overlays::VIDEO_FPS;

const TEXTURE_SIDE: u32 = 384;
const PATTERN_SIDE: u32 = 768;
const MAX_IMAGE_BYTES: u64 = 64 * 1024 * 1024;
const GENERATED_IMAGE: &str = "composition-background.ppm";
const METEOR_PLANES: [&str; 2] = [
    "composition-meteors-far.pam",
    "composition-meteors-near.pam",
];

#[derive(Clone, Copy)]
enum ImageKind {
    Png,
    Jpeg,
    Webp,
}

impl ImageKind {
    fn for_path(path: &Path) -> Result<Self> {
        match path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("png") => Ok(Self::Png),
            Some("jpg" | "jpeg") => Ok(Self::Jpeg),
            Some("webp") => Ok(Self::Webp),
            _ => anyhow::bail!("背景图片需为 PNG、JPEG 或 WebP"),
        }
    }

    fn filename(self) -> &'static str {
        match self {
            Self::Png => "composition-background.png",
            Self::Jpeg => "composition-background.jpg",
            Self::Webp => "composition-background.webp",
        }
    }

    fn matches(self, header: &[u8; 12]) -> bool {
        match self {
            Self::Png => header.starts_with(b"\x89PNG\r\n\x1A\n"),
            Self::Jpeg => header.starts_with(&[0xff, 0xd8, 0xff]),
            Self::Webp => header.starts_with(b"RIFF") && &header[8..] == b"WEBP",
        }
    }
}

pub(in crate::editor) fn stage(
    settings: &BackgroundSettings,
    directory: &Path,
    composition_size: (u32, u32),
) -> Result<()> {
    let classic_size = (TEXTURE_SIDE, TEXTURE_SIDE);
    match &settings.source {
        BackgroundSource::Off | BackgroundSource::Solid(_) => Ok(()),
        BackgroundSource::Gradient(preset) => {
            write_texture(directory, classic_size, |u, v| preset.sample(u, v, false))
        }
        BackgroundSource::Wallpaper(preset) => {
            write_texture(directory, classic_size, |u, v| preset.sample(u, v, true))
        }
        BackgroundSource::Dynamic(recipe) => {
            ensure!(recipe.is_valid(), "动态背景配方无效");
            let aspect = composition_size.0.max(1) as f32 / composition_size.1.max(1) as f32;
            match recipe.pattern {
                DynamicPattern::Grid | DynamicPattern::Dots => {
                    write_texture(directory, pattern_size(composition_size), |u, v| {
                        recipe.sample_in(u, v, aspect)
                    })
                }
                DynamicPattern::Meteors => {
                    let size = pattern_size(composition_size);
                    write_texture(directory, size, |u, v| recipe.meteor_base(u, v, aspect))?;
                    for (layer, filename) in METEOR_PLANES.iter().enumerate() {
                        write_meteor_tile(&directory.join(filename), size, |u, v| {
                            recipe.meteor_layer(u, v, aspect, layer)
                        })?;
                    }
                    Ok(())
                }
                _ => write_texture(directory, classic_size, |u, v| recipe.sample(u, v)),
            }
        }
        BackgroundSource::Image(path) => {
            let kind = ImageKind::for_path(path)?;
            let mut source = File::open(local_media(path)?).context("无法读取背景图片")?;
            let mut header = [0; 12];
            source.read_exact(&mut header).context("背景图片不完整")?;
            ensure!(kind.matches(&header), "背景图片格式与扩展名不符");
            ensure!(
                source.metadata()?.len() <= MAX_IMAGE_BYTES,
                "背景图片超过 64 MB"
            );
            let mut output = File::create(directory.join(kind.filename()))?;
            output.write_all(&header)?;
            let capacity = MAX_IMAGE_BYTES - header.len() as u64;
            let copied = io::copy(&mut source.take(capacity + 1), &mut output)?;
            ensure!(copied <= capacity, "背景图片超过 64 MB");
            Ok(())
        }
    }
}

fn pattern_size((width, height): (u32, u32)) -> (u32, u32) {
    let factor = f64::from(PATTERN_SIDE) / f64::from(width.max(height).max(1));
    let fit = |length| (f64::from(length) * factor).round().max(2.0) as u32;
    (fit(width), fit(height))
}

fn write_texture(
    directory: &Path,
    (width, height): (u32, u32),
    sample: impl Fn(f32, f32) -> [u8; 3],
) -> Result<()> {
    let mut file = BufWriter::new(File::create(directory.join(GENERATED_IMAGE))?);
    write!(file, "P6\n{width} {height}\n255\n")?;
    let mut row = Vec::with_capacity(width as usize * 3);
    let columns = (width - 1) as f32;
    let rows = (height - 1) as f32;
    for y in 0..height {
        row.clear();
        for x in 0..width {
            row.extend_from_slice(&sample(x as f32 / columns, y as f32 / rows));
        }
        file.write_all(&row)?;
    }
    file.flush()?;
    Ok(())
}

fn write_meteor_tile(
    path: &Path,
    (width, height): (u32, u32),
    sample: impl Fn(f32, f32) -> [u8; 4],
) -> Result<()> {
    let mut file = BufWriter::new(File::create(path)?);
    write!(
        file,
        "P7\nWIDTH {}\nHEIGHT {}\nDEPTH 4\nMAXVAL 255\nTUPLTYPE RGB_ALPHA\nENDHDR\n",
        width * 2,
        height * 2
    )?;
    let row_bytes = width as usize * 4;
    let mut pixels = Vec::with_capacity(row_bytes * height as usize);
    for y in 0..height {
        for x in 0..width {
            pixels.extend_from_slice(&sample(
                (x as f32 + 0.5) / width as f32,
                (y as f32 + 0.5) / height as f32,
            ));
        }
    }
    for _ in 0..2 {
        for row in pixels.chunks_exact(row_bytes) {
            file.write_all(row)?;
            file.write_all(row)?;
        }
    }
    file.flush()?;
    Ok(())
}

fn still_source(filename: &str) -> String {
    format!(
        "movie=filename={filename}:format_name=image2:dec_threads=1:format_opts=protocol_whitelist=file,loop=loop=-1:size=1:start=0,settb=expr=1/{VIDEO_FPS},setpts=N"
    )
}

fn accent_animation(motion: BackgroundMotion, solid: bool, size: (u32, u32), at: f64) -> String {
    let (width, height) = size;
    let time = format!("(on/{VIDEO_FPS}+{at:.9})");
    let position = |distance: &str, amplitude: &str, speed: &str, phase: &str| {
        format!("({distance})*(0.5+{amplitude}*sin({time}*{speed}+{phase}))")
    };
    let (zoom, x, y) = match motion {
        BackgroundMotion::None => return String::new(),
        BackgroundMotion::Drift => (
            "1.15".into(),
            position("iw-iw/zoom", "0.38", "0.42", "0"),
            position("ih-ih/zoom", "0.31", "0.29", "1.2"),
        ),
        BackgroundMotion::Aurora => (
            format!("1.14+0.045*sin({time}*0.52)"),
            position("iw-iw/zoom", "0.28", "0.34", "0.4"),
            position("ih-ih/zoom", "0.35", "0.41", "1.0"),
        ),
        BackgroundMotion::Wave => (
            format!("1.17+0.06*sin({time}*1.25)"),
            position("iw-iw/zoom", "0.40", "0.87", "0.3"),
            position("ih-ih/zoom", "0.22", "1.07", "1.5"),
        ),
    };
    let mut filters = if solid {
        String::new()
    } else {
        format!(",zoompan=z='{zoom}':x='{x}':y='{y}':d=1:s={width}x{height}:fps={VIDEO_FPS}")
    };
    if motion == BackgroundMotion::Aurora {
        let _ = write!(filters, ",hue=h='10*sin((t+{at:.9})*0.55)'");
    }
    if solid {
        let rate = if motion == BackgroundMotion::Wave {
            1.25
        } else {
            0.52
        };
        let _ = write!(
            filters,
            ",eq=brightness='0.035*(1-cos((t+{at:.9})*{rate}))':eval=frame"
        );
    }
    filters
}

fn recipe_animation(
    recipe: &DynamicRecipe,
    motion: BackgroundMotion,
    size: (u32, u32),
    at: f64,
) -> String {
    let strength = f64::from(recipe.intensity) / 100.0;
    let speed = f64::from(recipe.speed) / 100.0;
    if strength == 0.0 || motion == BackgroundMotion::None {
        return String::new();
    }

    let (base, breath, tempo, sway) = match motion {
        BackgroundMotion::Drift => (0.23, 0.02, 0.54, 0.07),
        BackgroundMotion::Aurora => (0.28, 0.07, 0.68, 0.10),
        BackgroundMotion::Wave => (0.27, 0.06, 1.30, 0.12),
        BackgroundMotion::None => return String::new(),
    };
    let t = format!("((on/{VIDEO_FPS}+{at:.9})*{speed:.4})");
    let (dy, dx) = f64::from(recipe.angle).to_radians().sin_cos();
    let along = format!("0.36*sin({t}*{tempo})");
    let across = format!("{sway}*sin({t}*{tempo}*0.71+0.64)");
    let zoom = format!("1+{strength:.4}*({base}+{breath}*sin({t}*{tempo}*0.83))");
    let (width, height) = size;
    let mut filters = format!(
        ",zoompan=z='{zoom}':x='(iw-iw/zoom)*(0.5+({dx:.6})*({along})-({dy:.6})*({across}))':y='(ih-ih/zoom)*(0.5+({dy:.6})*({along})+({dx:.6})*({across}))':d=1:s={width}x{height}:fps={VIDEO_FPS}"
    );
    if motion == BackgroundMotion::Aurora {
        let hue = strength * 18.0;
        let _ = write!(
            filters,
            ",hue=h='{hue:.4}*sin((t+{at:.9})*{speed:.4}*0.67)'"
        );
    }
    filters
}

fn meteor_background(
    sky: &str,
    recipe: &DynamicRecipe,
    motion: BackgroundMotion,
    (width, height): (u32, u32),
    timeline_from: u64,
) -> String {
    let mode = match motion {
        BackgroundMotion::None => 0.0,
        BackgroundMotion::Drift => 0.72,
        BackgroundMotion::Aurora => 0.90,
        BackgroundMotion::Wave => 1.0,
    };
    let speed = mode * f64::from(recipe.speed) / 100.0 * f64::from(recipe.intensity) / 100.0;
    let (dy, dx) = f64::from(recipe.angle).to_radians().sin_cos();
    let first_frame = (u128::from(timeline_from) * u128::from(VIDEO_FPS) + 500_000) / 1_000_000;
    let mut graph = format!("{sky}[meteor_sky];");
    for (layer, filename) in METEOR_PLANES.iter().enumerate() {
        let layer_speed = if layer == 0 { 0.19 } else { 0.34 };
        let step = speed * layer_speed / VIDEO_FPS as f64;
        let horizontal = dx * step;
        let vertical = dy * step;
        // One quadrant of the tiled image stays visible. Derive its offset from the absolute
        // timeline frame so the same position renders the same pixels after a cut or seek.
        let _ = write!(
            graph,
            "{},format=yuva444p,crop=w=iw/2:h=ih/2:x='mod(-(n+{first_frame})*{horizontal:.12}*min(ow,oh),ow)':y='mod(-(n+{first_frame})*{vertical:.12}*min(ow,oh),oh)':exact=1,scale={width}:{height}:flags=bilinear,setsar=1[meteor_{layer}];",
            still_source(filename)
        );
    }
    let _ = write!(
        graph,
        "[meteor_sky][meteor_0]overlay=format=yuv444[meteor_mid];[meteor_mid][meteor_1]overlay=format=yuv444,fps={VIDEO_FPS}:start_time=0"
    );
    graph
}

fn rounded_layer(foreground: &str, radius: f64) -> String {
    if radius < 0.5 {
        return format!("{foreground},format=yuva444p");
    }
    let radius = format!("min({radius:.4},min(W,H)/2)");
    let x = format!("max({radius}-min(X,W-1-X),0)");
    let y = format!("max({radius}-min(Y,H-1-Y),0)");
    // A segment has fixed foreground dimensions, so reuse the first frame's shape for every
    // subsequent image instead of evaluating a per-pixel corner expression on every frame.
    format!(
        "{foreground},split=2[body][shape];[shape]trim=end_frame=1,format=gray,geq=lum='255*clip({radius}+0.5-sqrt(pow({x},2)+pow({y},2)),0,1)'[alpha];[body][alpha]alphamerge"
    )
}

fn frame_trim(style: BackgroundFrame, scale: f64, bounds: (u32, u32)) -> ((u32, u32), String) {
    let (side, top, bottom, finish) = match style {
        BackgroundFrame::None => return (bounds, String::new()),
        BackgroundFrame::Window => (3.0, 34.0, 3.0, "0x343946"),
        BackgroundFrame::Laptop => (14.0, 20.0, 43.0, "0x373B45"),
        BackgroundFrame::Phone => (18.0, 33.0, 26.0, "0x242936"),
        BackgroundFrame::Monitor => (11.0, 11.0, 25.0, "0x292E38"),
    };
    let pixels = |native: f64| (native * scale).round().max(1.0) as u32;
    let (width, height) = bounds;
    let side = pixels(side).min(width.saturating_sub(2) / 2);
    let top = pixels(top).min(height.saturating_sub(2) / 2);
    let bottom = pixels(bottom).min(height.saturating_sub(top + 2));
    let body = (width - side * 2, height - top - bottom);
    let mut trim = format!(
        ",pad=iw+{}:ih+{}:{side}:{top}:color={finish}",
        side * 2,
        top + bottom
    );
    if top >= 4 && style == BackgroundFrame::Window {
        let dot = (top / 5).max(1);
        let y = (top - dot) / 2;
        for index in 0..3 {
            let x = side + dot * (2 + index * 2);
            let _ = write!(
                trim,
                ",drawbox=x={x}:y={y}:w={dot}:h={dot}:color=white@0.30:t=fill"
            );
        }
    } else if top >= 3 && style == BackgroundFrame::Phone {
        let lens = top / 3;
        let _ = write!(
            trim,
            ",drawbox=x=(iw-{lens})/2:y={lens}:w={lens}:h={lens}:color=black@0.65:t=fill"
        );
    } else if bottom >= 3 {
        let y = bottom / 2;
        let _ = write!(
            trim,
            ",drawbox=x=iw*0.43:y=ih-{y}:w=iw*0.14:h=1:color=white@0.21:t=fill"
        );
    }
    (body, trim)
}

fn shadow_filter(shadow: BackgroundShadow, scale: f64) -> Option<String> {
    let (opacity, sigma, offset) = match shadow {
        BackgroundShadow::None => return None,
        BackgroundShadow::Soft => (0.24, 8.0, 5.0),
        BackgroundShadow::Medium => (0.39, 15.0, 9.0),
        BackgroundShadow::Strong => (0.53, 23.0, 14.0),
    };
    let sigma = (sigma * scale).max(0.5);
    let margin = (sigma * 3.0).ceil() as u32;
    let shift = (offset * scale).round() as u32;
    let extra = margin * 2;
    Some(format!(
        "[fg][mask];[mask]trim=end_frame=1,lut=y=16:u=128:v=128:a='val*{opacity}',pad=iw+{extra}:ih+{extra}:{margin}:{margin}:color=black@0,gblur=sigma={sigma:.4}:planes=8[shadow];[bg][shadow]overlay=x='(main_w-overlay_w)/2':y='(main_h-overlay_h)/2+{shift}':format=yuv444[ground];[ground][fg]"
    ))
}

fn suffix(filters: &str) -> &str {
    if filters.is_empty() { "null" } else { filters }
}

/// The compositing branch is shared by preview and export. Its sole external pad is the video
/// stream, so it can be attached with `-vf` alongside independent audio filters.
pub(in crate::editor) fn filter(
    settings: &BackgroundSettings,
    size: (u32, u32),
    composition: (u32, u32),
    timeline_from: u64,
    has_video: bool,
    input_filter: &str,
    output_filter: &str,
) -> Result<String> {
    let (width, height) = size;
    let (composition_width, composition_height) = composition;
    ensure!(
        width >= 2 && height >= 2 && composition_width > 0 && composition_height > 0,
        "背景画幅尺寸无效"
    );
    let solid = matches!(settings.source, BackgroundSource::Solid(_));
    let modern = matches!(
        &settings.source,
        BackgroundSource::Dynamic(recipe)
            if matches!(recipe.pattern, DynamicPattern::Grid | DynamicPattern::Dots | DynamicPattern::Meteors)
    );
    let source = match &settings.source {
        BackgroundSource::Off => anyhow::bail!("背景合成尚未启用"),
        BackgroundSource::Solid([r, g, b]) => {
            format!("color=c=0x{r:02x}{g:02x}{b:02x}:s={width}x{height}:r={VIDEO_FPS}")
        }
        source => {
            let filename = match source {
                BackgroundSource::Image(path) => ImageKind::for_path(path)?.filename(),
                _ => GENERATED_IMAGE,
            };
            let still = still_source(filename);
            if modern {
                format!("{still},scale={width}:{height}:flags=bilinear")
            } else {
                format!(
                    "{still},scale={width}:{height}:force_original_aspect_ratio=increase,crop={width}:{height}"
                )
            }
        }
    };

    let time = timeline_from as f64 / 1_000_000.0;
    let short = width.min(height);
    let scale = f64::from(short) / 1080.0;
    let backdrop = format!("{source},setsar=1,format=yuv444p");
    let mut background = match &settings.source {
        BackgroundSource::Dynamic(recipe) if recipe.pattern == DynamicPattern::Meteors => {
            meteor_background(&backdrop, recipe, settings.motion, size, timeline_from)
        }
        BackgroundSource::Dynamic(recipe) => {
            let motion = recipe_animation(recipe, settings.motion, size, time);
            format!("{backdrop}{motion}")
        }
        _ => {
            let motion = accent_animation(settings.motion, solid, size, time);
            format!("{backdrop}{motion}")
        }
    };
    if settings.blur > 0 {
        let sigma = f64::from(settings.blur) * 0.38 * scale;
        let _ = write!(background, ",gblur=sigma={sigma:.4}");
    }
    let output = suffix(output_filter);
    if !has_video {
        return Ok(format!("[in]nullsink;{background},{output}[out]"));
    }

    let inset = (u64::from(short) * u64::from(settings.inset) / 100) as u32;
    let limit = |side: u32| side.saturating_sub(inset.saturating_mul(2)).max(2) & !1;
    let ((inner_width, inner_height), trim) =
        frame_trim(settings.frame, scale, (limit(width), limit(height)));
    let dar = format!("(dar*{width}*{composition_height}/({height}*{composition_width}))");
    let inner_width = inner_width & !1;
    let inner_height = inner_height & !1;
    let fit = format!(
        "scale=w='max(2,2*round(min({inner_width},{inner_height}*{dar})/2))':h='max(2,2*round(min({inner_height},{inner_width}/{dar})/2))':flags=bilinear,setsar=1,format=yuv444p{trim}"
    );
    let input = suffix(input_filter);
    let foreground = rounded_layer(
        &format!("[in]{input},{fit}"),
        f64::from(settings.radius) * scale,
    );
    let layers = match shadow_filter(settings.shadow, scale) {
        Some(shadow) => format!("{foreground},split=2{shadow}"),
        None => format!("{foreground}[fg];[bg][fg]"),
    };
    Ok(format!(
        "{background}[bg];{layers}overlay=x='(main_w-overlay_w)/2':y='(main_h-overlay_h)/2':format=yuv444:shortest=1,{output}[out]"
    ))
}
