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
            TransKind::ZoomIn | TransKind::ZoomOut => Duration::from_millis(280),
            TransKind::ViewSwitch => Duration::from_millis(180),
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

    /// Smoothstep ease.
    pub fn ease(&self) -> f32 {
        let t = self.linear();
        t * t * (3.0 - 2.0 * t)
    }

    pub fn done(&self) -> bool {
        self.linear() >= 1.0
    }

    pub fn opacity(&self) -> f32 {
        let e = self.ease();
        match self.kind {
            TransKind::ViewSwitch => {
                // dip then rise
                if e < 0.5 {
                    1.0 - e * 1.4
                } else {
                    (e - 0.5) * 2.0
                }
                .clamp(0.15, 1.0)
            }
            TransKind::ZoomIn | TransKind::ZoomOut => (0.35 + 0.65 * e).clamp(0.2, 1.0),
        }
    }

    pub fn scale(&self) -> f32 {
        let e = self.ease();
        match self.kind {
            TransKind::ZoomIn => 0.82 + 0.18 * e,
            TransKind::ZoomOut => 1.12 - 0.12 * e,
            TransKind::ViewSwitch => 0.96 + 0.04 * e,
        }
    }
}
