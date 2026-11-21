use std::time::{Duration, Instant};

use crate::click_follow::PointerSample;

const MAX_ZOOM: f32 = 1.6;
const ZOOM_IN: Duration = Duration::from_millis(270);
const HOLD: Duration = Duration::from_millis(1_200);
const ZOOM_OUT: Duration = Duration::from_millis(420);
const FOLLOW_TIME: Duration = Duration::from_millis(140);

#[derive(Default)]
pub struct ZoomFollow {
    active: Option<ActiveZoom>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ZoomCrop {
    pub left: u32,
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
}

struct ActiveZoom {
    last_click: Instant,
    sampled_at: Instant,
    center: [f32; 2],
    progress: f32,
}

impl ZoomFollow {
    pub fn sample(
        &mut self,
        now: Instant,
        pointer: PointerSample,
        frame_size: (u32, u32),
        output_size: (u32, u32),
    ) -> Option<ZoomCrop> {
        let click = pointer.click.filter(|click| valid_position(click.position));
        if let Some(click) = click {
            let at = click.at.min(now);
            self.advance(at, None);
            let zoom = self.active.get_or_insert(ActiveZoom {
                last_click: at,
                sampled_at: at,
                center: click.position,
                progress: 0.0,
            });
            zoom.last_click = zoom.last_click.max(at);
        }

        let target = click.map(|click| click.position).or(pointer.position);
        self.advance(now, target.filter(|position| valid_position(*position)));

        let zoom = self.active.as_ref().filter(|zoom| zoom.progress > 0.0)?;
        let eased = zoom.progress * zoom.progress * (3.0 - 2.0 * zoom.progress);
        crop_at(
            zoom.center,
            1.0 + (MAX_ZOOM - 1.0) * eased,
            frame_size,
            output_size,
        )
    }

    fn advance(&mut self, now: Instant, target: Option<[f32; 2]>) {
        let Some(zoom) = &mut self.active else {
            return;
        };

        let now = now.max(zoom.sampled_at);
        let hold_until = zoom.last_click + HOLD;
        let rising = now
            .min(hold_until)
            .saturating_duration_since(zoom.sampled_at);
        let falling = now.saturating_duration_since(hold_until.max(zoom.sampled_at));
        zoom.progress = (zoom.progress + rising.as_secs_f32() / ZOOM_IN.as_secs_f32()).min(1.0);
        zoom.progress = (zoom.progress - falling.as_secs_f32() / ZOOM_OUT.as_secs_f32()).max(0.0);

        if let Some([x, y]) = target {
            let elapsed = now.duration_since(zoom.sampled_at);
            let follow = 1.0 - (-elapsed.as_secs_f32() / FOLLOW_TIME.as_secs_f32()).exp();
            zoom.center[0] += (x - zoom.center[0]) * follow;
            zoom.center[1] += (y - zoom.center[1]) * follow;
        }
        zoom.sampled_at = now;

        if now >= hold_until && zoom.progress == 0.0 {
            self.active = None;
        }
    }
}

fn valid_position([x, y]: [f32; 2]) -> bool {
    (0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y)
}

fn crop_at(
    [center_x, center_y]: [f32; 2],
    scale: f32,
    (frame_width, frame_height): (u32, u32),
    (output_width, output_height): (u32, u32),
) -> Option<ZoomCrop> {
    if scale <= 1.0 || [frame_width, frame_height, output_width, output_height].contains(&0) {
        return None;
    }

    // Fit the encoder's aspect ratio inside the captured frame before applying the zoom.
    let aspect = f64::from(output_width) / f64::from(output_height);
    let fit_width = f64::from(frame_width).min(f64::from(frame_height) * aspect);
    let crop_width = fit_width / f64::from(scale);
    let crop_height = crop_width / aspect;
    let width = (crop_width.round() as u32).clamp(1, frame_width);
    let height = (crop_height.round() as u32).clamp(1, frame_height);
    if width == frame_width && height == frame_height {
        return None;
    }

    let left = crop_origin(center_x, frame_width, width);
    let top = crop_origin(center_y, frame_height, height);
    Some(ZoomCrop {
        left,
        top,
        right: left + width,
        bottom: top + height,
    })
}

fn crop_origin(center: f32, frame_length: u32, crop_length: u32) -> u32 {
    let origin = f64::from(center) * f64::from(frame_length) - f64::from(crop_length) / 2.0;
    (origin.round() as u32).min(frame_length - crop_length)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::click_follow::Click;

    const SIZE: (u32, u32) = (1920, 1080);

    fn click(at: Instant, position: [f32; 2]) -> PointerSample {
        PointerSample {
            position: Some(position),
            click: Some(Click { at, position }),
        }
    }

    fn pointer(position: [f32; 2]) -> PointerSample {
        PointerSample {
            position: Some(position),
            click: None,
        }
    }

    #[test]
    fn zoom_starts_with_a_click_then_returns_to_the_full_frame() {
        let mut zoom = ZoomFollow::default();
        let start = Instant::now();
        let center = [0.5, 0.5];
        assert_eq!(zoom.sample(start, pointer(center), SIZE, SIZE), None);
        assert_eq!(zoom.sample(start, click(start, center), SIZE, SIZE), None);

        let halfway = zoom
            .sample(start + ZOOM_IN / 2, pointer(center), SIZE, SIZE)
            .unwrap();
        let full = zoom
            .sample(start + ZOOM_IN, pointer(center), SIZE, SIZE)
            .unwrap();
        assert_eq!(
            full,
            ZoomCrop {
                left: 360,
                top: 203,
                right: 1560,
                bottom: 878
            }
        );
        assert!(halfway.left < full.left);
        assert_eq!(
            zoom.sample(start + HOLD + ZOOM_OUT, pointer(center), SIZE, SIZE),
            None
        );
    }

    #[test]
    fn repeated_clicks_keep_the_crop_continuous_and_extend_the_hold() {
        let mut zoom = ZoomFollow::default();
        let start = Instant::now();
        zoom.sample(start, click(start, [0.5, 0.5]), SIZE, SIZE);
        let next = start + HOLD + ZOOM_OUT / 2;
        let before = zoom.sample(next, pointer([0.5, 0.5]), SIZE, SIZE);
        let after = zoom.sample(next, click(next, [0.8, 0.5]), SIZE, SIZE);
        assert_eq!(before, after);
        let regained = zoom
            .sample(next + ZOOM_IN, pointer([0.8, 0.5]), SIZE, SIZE)
            .unwrap();
        assert_eq!(regained.right, SIZE.0);
        assert!(regained.left > before.unwrap().left);
        assert!(
            zoom.sample(next + HOLD, pointer([0.8, 0.5]), SIZE, SIZE)
                .is_some()
        );
    }

    #[test]
    fn crop_stays_inside_odd_sized_frames_with_the_encoder_aspect_ratio() {
        let crop = crop_at([1.0, 0.0], MAX_ZOOM, (1919, 1079), SIZE).unwrap();
        assert_eq!(crop.top, 0);
        assert_eq!(crop.right, 1919);
        let width = (crop.right - crop.left) as f64;
        let height = (crop.bottom - crop.top) as f64;
        assert!((width - height * 16.0 / 9.0).abs() < 1.0);
        assert_eq!(crop_at([0.0, 0.0], MAX_ZOOM, (1, 1), SIZE), None);
        assert_eq!(crop_at([0.0, 0.0], MAX_ZOOM, (0, 1), SIZE), None);
    }
}
