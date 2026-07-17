//! Slide open/close animation for the left sidebar.
//!
//! warpui has no transition framework (`render()` never re-runs on its own),
//! so the workspace view drives this with the self-rearming `notify()` timer
//! pattern (see `design/PHILOSOPHY.md` "Motion"): each tick computes an
//! eased progress from an `Instant` start time and calls `ctx.notify()` again
//! until the ~150ms ease is done. Only user-initiated toggles animate —
//! startup/restore paths assign `left_panel_open` directly and never create a
//! [`LeftPanelSlide`], so the panel appears at full width instantly there.

use std::time::{Duration, Instant};

use pathfinder_geometry::vector::{vec2f, Vector2F};
use twarpui::{
    elements::Point, event::DispatchedEvent, AfterLayoutContext, AppContext, ClipBounds, Element,
    EventContext, LayoutContext, PaintContext, SizeConstraint,
};

/// Total duration of the sidebar slide.
pub(super) const LEFT_PANEL_SLIDE_DURATION: Duration = Duration::from_millis(150);

/// Interval between animation frames (self-rearming timer ticks).
pub(super) const LEFT_PANEL_SLIDE_TICK: Duration = Duration::from_millis(8);

fn ease_out_cubic(t: f32) -> f32 {
    let inv = 1.0 - t.clamp(0.0, 1.0);
    1.0 - inv * inv * inv
}

/// In-flight slide state stored on the workspace view. `from`/`to` are
/// visible-width fractions (0.0 = fully hidden, 1.0 = fully open); starting
/// from the current fraction means a rapid re-toggle mid-animation reverses
/// smoothly instead of jumping.
pub(super) struct LeftPanelSlide {
    started_at: Instant,
    from: f32,
    to: f32,
}

impl LeftPanelSlide {
    pub(super) fn new(from: f32, to: f32) -> Self {
        Self {
            started_at: Instant::now(),
            from,
            to,
        }
    }

    /// Current visible fraction of the panel width, eased.
    pub(super) fn fraction(&self) -> f32 {
        let t = self.started_at.elapsed().as_secs_f32()
            / LEFT_PANEL_SLIDE_DURATION.as_secs_f32().max(f32::EPSILON);
        self.from + (self.to - self.from) * ease_out_cubic(t)
    }

    /// Whether the panel is animating towards open (1.0).
    pub(super) fn is_opening(&self) -> bool {
        self.to > self.from
    }

    pub(super) fn is_done(&self) -> bool {
        self.started_at.elapsed() >= LEFT_PANEL_SLIDE_DURATION
    }
}

/// Clips its child to `fraction` of the child's natural width and paints it
/// shifted left by the hidden remainder, so the panel slides in from (and out
/// to) the left edge instead of squishing. The child is laid out with the
/// unmodified (finite) incoming constraint, so the sidebar's own
/// `Resizable`-driven width is respected and no infinite-constraint
/// `debug_assert` in `Flex` can trip.
pub(super) struct SlideClip {
    child: Box<dyn Element>,
    /// Visible fraction of the child's width, in `[0, 1]`.
    fraction: f32,
    origin: Option<Point>,
    size: Option<Vector2F>,
    child_size: Option<Vector2F>,
}

impl SlideClip {
    pub(super) fn new(child: Box<dyn Element>, fraction: f32) -> Self {
        Self {
            child,
            fraction: fraction.clamp(0.0, 1.0),
            origin: None,
            size: None,
            child_size: None,
        }
    }
}

impl Element for SlideClip {
    fn layout(
        &mut self,
        mut constraint: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        // The child (the sidebar) sizes itself to its full width; we only
        // report the animated visible width upward. Relax the min so the row
        // can't force us wider than the animated width.
        constraint.min.set_x(0.0);
        let child_size = self.child.layout(constraint, ctx, app);
        self.child_size = Some(child_size);
        let size = vec2f((child_size.x() * self.fraction).round(), child_size.y());
        self.size = Some(size);
        size
    }

    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &AppContext) {
        self.child.after_layout(ctx, app);
    }

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        let (Some(size), Some(child_size)) = (self.size, self.child_size) else {
            return;
        };
        let origin_point = Point::from_vec2f(origin, ctx.scene.z_index());
        self.origin = Some(origin_point);

        if size.x() <= 0.0 {
            return;
        }

        // Clip to the visible strip, then paint the child shifted left by the
        // hidden remainder so its right edge tracks the strip's right edge
        // (a slide, not a reveal-in-place).
        let Some(bounds) = ctx.scene.visible_rect(origin_point, size) else {
            return;
        };
        let shift = size.x() - child_size.x();
        ctx.scene.start_layer(ClipBounds::BoundedBy(bounds));
        self.child.paint(origin + vec2f(shift, 0.0), ctx, app);
        ctx.scene.stop_layer();
    }

    fn dispatch_event(
        &mut self,
        event: &DispatchedEvent,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        // The slide lasts ~150ms; suppress child hit-testing while partially
        // hidden so clicks can't land on the shifted (mostly offscreen) panel.
        if self.fraction >= 1.0 {
            self.child.dispatch_event(event, ctx, app)
        } else {
            false
        }
    }

    fn size(&self) -> Option<Vector2F> {
        self.size
    }

    fn origin(&self) -> Option<Point> {
        self.origin
    }
}
