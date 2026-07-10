use pathfinder_geometry::vector::{vec2f, Vector2F};
use twarpui::{event::DispatchedEvent, AppContext, Element, EventContext, SizeConstraint};

/// The minimum width kept for the header's center (title) column when the
/// pane is too narrow to honour the full edge budget.
const MIN_CENTER_WIDTH: f32 = 96.;

/// The standard pane header's three-column layout: `[left | center | right]`
/// with both edge columns pinned to a shared fixed budget so the title stays
/// exactly centered — *when the pane is wide enough*. In a narrow pane (e.g.
/// three Claude panes side by side) the old fixed pinning laid both edges out
/// at the full budget regardless, clipping the right-edge controls off the
/// pane. This element keeps the pinned wide behaviour but degrades at layout
/// time: below `2 * edge_budget + MIN_CENTER_WIDTH` the left column drops to
/// its natural width and the right column gets whatever remains beyond the
/// minimum title room — so edge contents see a real (bounded, pane-derived)
/// width constraint and can adapt via `SizeConstraintSwitch`.
pub(super) struct EdgeBudgetRow {
    left: Box<dyn Element>,
    center: Box<dyn Element>,
    right: Box<dyn Element>,
    /// The per-edge width both columns are pinned to when the pane is wide
    /// enough (`StandardHeaderOptions::control_container_width`, floored to
    /// the required close/overflow icon width).
    edge_budget: f32,
    /// The floor for the right column in the narrow case — the close and
    /// overflow buttons must stay reachable even if the title gets nothing.
    min_right_width: f32,

    left_size: Option<Vector2F>,
    right_size: Option<Vector2F>,
    origin: Option<twarpui::elements::Point>,
    size: Option<Vector2F>,
}

impl EdgeBudgetRow {
    pub(super) fn new(
        left: Box<dyn Element>,
        center: Box<dyn Element>,
        right: Box<dyn Element>,
        edge_budget: f32,
        min_right_width: f32,
    ) -> Self {
        Self {
            left,
            center,
            right,
            edge_budget,
            min_right_width,
            left_size: None,
            right_size: None,
            origin: None,
            size: None,
        }
    }
}

impl Element for EdgeBudgetRow {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        ctx: &mut twarpui::LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        let width = constraint.max.x();
        let height = constraint.max.y();
        // Mirror the old flex row's `CrossAxisAlignment::Stretch`: children fill
        // the header height (when it is finite — transient intrinsic measuring
        // passes can hand an infinite constraint).
        let child_constraint = |min_w: f32, max_w: f32| {
            if height.is_finite() {
                SizeConstraint::new(vec2f(min_w, height), vec2f(max_w, height))
            } else {
                SizeConstraint::new(vec2f(min_w, constraint.min.y()), vec2f(max_w, height))
            }
        };

        // On an infinite-width measuring pass, behave like the pinned layout.
        let pinned = !width.is_finite() || width >= 2. * self.edge_budget + MIN_CENTER_WIDTH;
        let (left_size, right_size) = if pinned {
            // Both edge slots occupy exactly the budget even when their content
            // is narrower (the old `ConstrainedBox` min=max behaviour) — equal
            // edges are what keep the title exactly centered.
            let left = self.left.layout(
                child_constraint(self.edge_budget, self.edge_budget),
                ctx,
                app,
            );
            let right = self.right.layout(
                child_constraint(self.edge_budget, self.edge_budget),
                ctx,
                app,
            );
            (
                vec2f(self.edge_budget, left.y()),
                vec2f(self.edge_budget, right.y()),
            )
        } else {
            // Narrow: the left column (toolbelt) takes its natural width up to
            // half the squeezable space; the right column (controls) gets the
            // rest beyond the minimum title room, floored so close/overflow
            // stay usable. The title is no longer exactly centered here —
            // fitting the controls wins.
            let squeezable = (width - MIN_CENTER_WIDTH).max(0.);
            let left = self
                .left
                .layout(child_constraint(0., squeezable * 0.5), ctx, app);
            let right_max = (squeezable - left.x())
                .max(self.min_right_width.min(width))
                .max(0.);
            let right = self.right.layout(child_constraint(0., right_max), ctx, app);
            (left, right)
        };
        let center_width = if width.is_finite() {
            (width - left_size.x() - right_size.x()).max(0.)
        } else {
            0.
        };
        let center_size =
            self.center
                .layout(child_constraint(center_width, center_width), ctx, app);

        self.left_size = Some(left_size);
        self.right_size = Some(right_size);
        let row_height = if height.is_finite() {
            height
        } else {
            left_size
                .y()
                .max(right_size.y())
                .max(center_size.y())
                .max(0.)
        };
        let row_width = if width.is_finite() {
            width
        } else {
            left_size.x() + center_size.x() + right_size.x()
        };
        let size = vec2f(row_width, row_height);
        self.size = Some(size);
        size
    }

    fn after_layout(&mut self, ctx: &mut twarpui::AfterLayoutContext, app: &AppContext) {
        self.left.after_layout(ctx, app);
        self.center.after_layout(ctx, app);
        self.right.after_layout(ctx, app);
    }

    fn paint(&mut self, origin: Vector2F, ctx: &mut twarpui::PaintContext, app: &AppContext) {
        self.origin = Some(twarpui::elements::Point::from_vec2f(
            origin,
            ctx.scene.z_index(),
        ));
        let size = self.size.expect("size set during layout");
        let left_size = self.left_size.expect("left size set during layout");
        let right_size = self.right_size.expect("right size set during layout");
        self.left.paint(origin, ctx, app);
        self.center
            .paint(origin + vec2f(left_size.x(), 0.), ctx, app);
        self.right
            .paint(origin + vec2f(size.x() - right_size.x(), 0.), ctx, app);
    }

    fn dispatch_event(
        &mut self,
        event: &DispatchedEvent,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        // Same order the old flex row dispatched in: any child may handle.
        self.left.dispatch_event(event, ctx, app)
            | self.center.dispatch_event(event, ctx, app)
            | self.right.dispatch_event(event, ctx, app)
    }

    fn size(&self) -> Option<Vector2F> {
        self.size
    }

    fn origin(&self) -> Option<twarpui::elements::Point> {
        self.origin
    }
}
