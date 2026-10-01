use pathfinder_geometry::{rect::RectF, vector::Vector2F};

use super::Tracked;

/// Window geometry whose size participates in render dependency tracking.
/// Moving a window updates its origin without rebuilding its contents.
pub(super) struct CachedWindowBounds {
    origin: Vector2F,
    size: Tracked<Option<Vector2F>>,
}

impl CachedWindowBounds {
    pub(super) fn new(bounds: Option<RectF>) -> Self {
        Self {
            origin: bounds.map(|bounds| bounds.origin()).unwrap_or_default(),
            size: bounds.map(|bounds| bounds.size()).into(),
        }
    }

    pub(super) fn get(&self) -> Option<RectF> {
        Some(RectF::new(self.origin, (*self.size)?))
    }

    pub(super) fn update(&mut self, bounds: RectF) {
        self.origin = bounds.origin();
        let size = Some(bounds.size());
        // Tracked invalidates on mutable access, even when the value is equal.
        // Avoid rebuilding size-dependent views for repeated native callbacks.
        if *self.size != size {
            *self.size = size;
        }
    }
}

#[cfg(test)]
#[path = "window_bounds_tests.rs"]
mod tests;
