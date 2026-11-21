use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use anyhow::{Context, Result, ensure};
use fast_image_resize::images::{Image, ImageRef};
use fast_image_resize::{FilterType, PixelType, ResizeAlg, ResizeOptions, Resizer};
use windows_capture::frame::Frame;

use crate::click_follow::PointerSample;
use crate::zoom_follow::ZoomFollow;

pub(crate) struct ZoomFrames {
    input: Arc<Mutex<PointerSample>>,
    motion: ZoomFollow,
    input_pixels: Vec<u8>,
    output: Option<Image<'static>>,
    resizer: Resizer,
    options: ResizeOptions,
}

impl ZoomFrames {
    pub(crate) fn new(input: Arc<Mutex<PointerSample>>) -> Self {
        Self {
            input,
            motion: ZoomFollow::default(),
            input_pixels: Vec::new(),
            output: None,
            resizer: Resizer::new(),
            options: ResizeOptions::new()
                .resize_alg(ResizeAlg::Interpolation(FilterType::Bilinear))
                .use_alpha(false),
        }
    }

    pub(crate) fn render(
        &mut self,
        frame: &mut Frame<'_>,
        now: Instant,
        output_size: (u32, u32),
    ) -> Result<Option<&[u8]>> {
        let pointer = {
            let mut sample = self.input.lock().unwrap_or_else(PoisonError::into_inner);
            PointerSample {
                position: sample.position,
                click: sample.click.take(),
            }
        };
        let Some(crop) =
            self.motion
                .sample(now, pointer, (frame.width(), frame.height()), output_size)
        else {
            return Ok(None);
        };

        let mut cropped = frame
            .buffer_crop(crop.left, crop.top, crop.right, crop.bottom)
            .context("无法读取点击跟随的采集区域")?;
        let (width, height) = (cropped.width(), cropped.height());
        let pitch = cropped.row_pitch() as usize;
        let row_bytes = width as usize * 4;
        let pixels = cropped.as_raw_buffer();
        ensure!(
            pitch >= row_bytes && pixels.len() >= pitch.saturating_mul(height as usize),
            "点击跟随的采集缓冲区尺寸不正确"
        );
        self.input_pixels.clear();
        self.input_pixels.reserve(row_bytes * height as usize);
        // Media Foundation expects the encoder's CPU BGRA input to be bottom-up.
        for row in pixels.chunks_exact(pitch).take(height as usize).rev() {
            self.input_pixels.extend_from_slice(&row[..row_bytes]);
        }
        let input = ImageRef::new(width, height, &self.input_pixels, PixelType::U8x4)
            .context("点击跟随的采集区域尺寸不正确")?;
        let output = self
            .output
            .get_or_insert_with(|| Image::new(output_size.0, output_size.1, PixelType::U8x4));
        self.resizer
            .resize(&input, output, &self.options)
            .context("无法缩放点击跟随的画面")?;
        Ok(Some(output.buffer()))
    }
}
