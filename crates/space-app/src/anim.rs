//! Lightweight UI transitions (nav zoom / view crossfade).

use std::time::{Duration, Instant};

#[derive(Clone, Copy)]
pub enum TransKind {
    /// Zoom into a child folder.
    ZoomIn,
    /// Zoom out to parent.
    ZoomOut,
    /// Switch visualization mode.
    ViewSwitch,
}

pub struct Transition {
    pub kind: TransKind,
    started: Instant,
    duration: Duration,
}

impl Transition {
    pub fn start(kind: TransKind) -> Self {
        let duration = match kind {
            TransKind::ZoomIn | TransKind::ZoomOut => Duration::from_millis(420),
            TransKind::ViewSwitch => Duration::from_millis(260),
        };
        Self {
            kind,
            started: Instant::now(),
            duration,
        }
    }

    pub fn linear(&self) -> f32 {
        (self.started.elapsed().as_secs_f32() / self.duration.as_secs_f32()).clamp(0.0, 1.0)
    }

    /// Cubic ease-out — snappy start, soft landing.
    pub fn ease(&self) -> f32 {
        ease_out_cubic(self.linear())
    }

    pub fn done(&self) -> bool {
        self.linear() >= 1.0
    }

    pub fn opacity(&self) -> f32 {
        let e = self.ease();
        match self.kind {
            TransKind::ViewSwitch => {
                // Soft dip then rise — never fully blank.
                let dip = if e < 0.45 {
                    1.0 - ease_in_out(e / 0.45) * 0.55
                } else {
                    0.45 + ease_out_cubic((e - 0.45) / 0.55) * 0.55
                };
                dip.clamp(0.35, 1.0)
            }
            TransKind::ZoomIn | TransKind::ZoomOut => (0.55 + 0.45 * e).clamp(0.4, 1.0),
        }
    }

    pub fn scale(&self) -> f32 {
        let e = self.ease();
        match self.kind {
            // Slight overshoot then settle for depth.
            TransKind::ZoomIn => lerp(0.78, 1.0, ease_out_back(e)),
            TransKind::ZoomOut => lerp(1.16, 1.0, ease_out_cubic(e)),
            TransKind::ViewSwitch => lerp(0.94, 1.0, e),
        }
    }
}

#[inline]
pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[inline]
pub fn ease_out_cubic(t: f32) -> f32 {
    let u = 1.0 - t;
    1.0 - u * u * u
}

#[inline]
pub fn ease_in_out(t: f32) -> f32 {
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
    }
}

/// Soft overshoot for zoom-in settle.
#[inline]
pub fn ease_out_back(t: f32) -> f32 {
    let c1 = 1.70158;
    let c3 = c1 + 1.0;
    1.0 + c3 * (t - 1.0).powi(3) + c1 * (t - 1.0).powi(2)
}
