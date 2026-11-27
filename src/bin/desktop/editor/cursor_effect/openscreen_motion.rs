//! Camera easing and spring chase adapted from OpenScreen (MIT).
//! https://github.com/siddharthvaddem/openscreen/tree/ca8104c279da0d4b4481ec992ff103949e22c772/src/components/video-editor/videoPlayback

const STIFFNESS: f64 = 320.0;
const DAMPING: f64 = 40.0;
const MASS: f64 = 0.92;
const REST_DELTA: f64 = 0.0005;
const REST_SPEED: f64 = 0.015;
const MAX_STEP_US: u64 = 80_000;

pub(super) fn zoom_ease(progress: f64) -> f64 {
    cubic_bezier([0.16, 1.0], [0.3, 1.0], progress)
}

pub(super) fn pan_ease(progress: f64) -> f64 {
    cubic_bezier([0.1, 0.0], [0.2, 1.0], progress)
}

fn cubic_bezier(first: [f64; 2], second: [f64; 2], progress: f64) -> f64 {
    let target = progress.clamp(0.0, 1.0);
    let mut t = target;
    const TOLERANCE: f64 = 1e-6;
    let [x1, y1] = first;
    let [x2, y2] = second;

    for _ in 0..8 {
        let error = sample_bezier(x1, x2, t) - target;
        let remainder = 1.0 - t;
        let derivative = 3.0 * x1 * remainder * remainder
            + 6.0 * (x2 - x1) * remainder * t
            + 3.0 * (1.0 - x2) * t * t;
        if error.abs() < TOLERANCE || derivative.abs() < TOLERANCE {
            break;
        }
        t -= error / derivative;
    }

    let mut lower = 0.0;
    let mut upper = 1.0;
    t = t.clamp(lower, upper);
    for _ in 0..10 {
        let x = sample_bezier(x1, x2, t);
        if (x - target).abs() < TOLERANCE {
            break;
        }
        if x < target {
            lower = t;
        } else {
            upper = t;
        }
        t = (lower + upper) / 2.0;
    }
    sample_bezier(y1, y2, t)
}

fn sample_bezier(first: f64, second: f64, t: f64) -> f64 {
    let remainder = 1.0 - t;
    3.0 * first * remainder * remainder * t + 3.0 * second * remainder * t * t + t * t * t
}

#[derive(Clone, Default)]
pub(in crate::editor) struct ZoomSpring {
    axes: [SpringAxis; 3],
    last_frame: Option<(u64, [u32; 2])>,
    focus: Option<([f64; 2], u64)>,
}

impl ZoomSpring {
    /// Adaptive smoothing from OpenScreen's cursorFollowUtils. Measure elapsed
    /// time on the timeline so 30 fps and 60 fps follow at comparable speeds.
    pub(super) fn follow_focus(
        &mut self,
        raw: Option<[f64; 2]>,
        time_us: u64,
        smoothing: f64,
        animate: bool,
    ) -> Option<[f64; 2]> {
        let previous = self.focus.take();
        let raw = raw?;
        let mut focus = raw;
        if animate && smoothing > 0.0
            && let Some((previous, previous_us)) = previous
            && let Some(elapsed) = time_us.checked_sub(previous_us)
            && (1..=MAX_STEP_US).contains(&elapsed)
        {
            let distance = (raw[0] - previous[0]).hypot(raw[1] - previous[1]);
            let base_factor = 0.1 + 0.15 * (distance / 0.15).min(1.0);
            let reference_us = 25_000.0 * (0.3 + smoothing);
            let factor = 1.0 - (1.0 - base_factor).powf(elapsed as f64 / reference_us);
            focus = std::array::from_fn(|axis| previous[axis] + (raw[axis] - previous[axis]) * factor);
        }
        self.focus = Some((focus, time_us));
        Some(focus)
    }

    /// Chase a normalized camera transform in content time. The translation
    /// axes use pixels internally to match OpenScreen's rest thresholds.
    pub(in crate::editor) fn step(
        &mut self,
        target: [f64; 3],
        surface: [u32; 2],
        time_us: u64,
        animate: bool,
    ) -> [f64; 3] {
        let previous = self.last_frame.replace((time_us, surface));
        let axes_scale = [
            1.0,
            f64::from(surface[0].max(1)),
            f64::from(surface[1].max(1)),
        ];
        let in_pixels: [f64; 3] = std::array::from_fn(|index| target[index] * axes_scale[index]);
        let elapsed = previous
            .filter(|&(_, previous_surface)| animate && previous_surface == surface)
            .and_then(|(previous_us, _)| time_us.checked_sub(previous_us))
            .filter(|delta| (1..=MAX_STEP_US).contains(delta));
        let Some(elapsed) = elapsed else {
            for (axis, target) in self.axes.iter_mut().zip(in_pixels) {
                axis.reset(target);
            }
            return target;
        };

        let delta = elapsed.max(1_000) as f64 / 1_000_000.0;
        let response = SpringResponse::new(delta);
        std::array::from_fn(|index| {
            self.axes[index].step(in_pixels[index], &response) / axes_scale[index]
        })
    }
}

#[derive(Clone, Default)]
struct SpringAxis {
    value: f64,
    velocity: f64,
}

impl SpringAxis {
    fn reset(&mut self, target: f64) -> f64 {
        self.value = target;
        self.velocity = 0.0;
        target
    }

    fn step(&mut self, target: f64, response: &SpringResponse) -> f64 {
        let displacement = self.value - target;
        if displacement.abs() <= REST_DELTA && self.velocity.abs() <= REST_SPEED {
            return self.reset(target);
        }
        let slow = (self.velocity + response.fast_rate * displacement)
            / (response.fast_rate - response.slow_rate);
        let fast = displacement - slow;
        let remaining = slow * response.slow_decay + fast * response.fast_decay;
        let speed = -response.slow_rate * slow * response.slow_decay
            - response.fast_rate * fast * response.fast_decay;
        let reached_rest = remaining.abs() <= REST_DELTA && speed.abs() <= REST_SPEED;
        let crossed_target =
            (displacement <= 0.0 && remaining > 0.0) || (displacement >= 0.0 && remaining < 0.0);
        if reached_rest || crossed_target {
            return self.reset(target);
        }
        let next = target + remaining;
        // OpenScreen estimates the inherited velocity from the previous rendered step.
        self.velocity = (next - self.value) / response.delta;
        self.value = next;
        next
    }
}

struct SpringResponse {
    delta: f64,
    slow_rate: f64,
    fast_rate: f64,
    slow_decay: f64,
    fast_decay: f64,
}

impl SpringResponse {
    fn new(delta: f64) -> Self {
        // Solve the overdamped mass-spring equation as the sum of two exponential modes.
        let decay = DAMPING / (2.0 * MASS);
        let damping_frequency = (decay * decay - STIFFNESS / MASS).sqrt();
        let slow_rate = decay - damping_frequency;
        let fast_rate = decay + damping_frequency;
        Self {
            delta,
            slow_rate,
            fast_rate,
            slow_decay: (-slow_rate * delta).exp(),
            fast_decay: (-fast_rate * delta).exp(),
        }
    }
}
