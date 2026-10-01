use std::{cell::Cell, rc::Rc};

use pathfinder_geometry::{rect::RectF, vector::vec2f};

use crate::{
    elements::Empty,
    platform::{WindowBounds, WindowStyle},
    App, AppContext, Element, Entity, TypedActionView, View, WindowId,
};

struct BoundsView {
    window_id: WindowId,
    reads_bounds: bool,
    render_count: Rc<Cell<usize>>,
    wide_layout: Rc<Cell<bool>>,
}

impl Entity for BoundsView {
    type Event = ();
}

impl TypedActionView for BoundsView {
    type Action = ();
}

impl View for BoundsView {
    fn ui_name() -> &'static str {
        "BoundsView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        self.render_count.set(self.render_count.get() + 1);
        if self.reads_bounds {
            self.wide_layout.set(
                app.window_bounds(&self.window_id)
                    .is_some_and(|bounds| bounds.width() >= 1000.),
            );
        }
        Empty::new().finish()
    }
}

#[test]
fn resize_rerenders_only_views_reading_that_windows_bounds() {
    App::test((), |mut app| async move {
        let bounds = RectF::new(vec2f(10., 20.), vec2f(1200., 800.));
        let count = Rc::new(Cell::new(0));
        let wide_layout = Rc::new(Cell::new(false));
        let (window_id, reader) = app.add_window_with_bounds(
            WindowStyle::NotStealFocus,
            WindowBounds::ExactPosition(bounds),
            |ctx| BoundsView {
                window_id: ctx.window_id(),
                reads_bounds: true,
                render_count: count.clone(),
                wide_layout: wide_layout.clone(),
            },
        );
        let unrelated_count = Rc::new(Cell::new(0));
        let unrelated = app.add_view(window_id, |_| BoundsView {
            window_id,
            reads_bounds: false,
            render_count: unrelated_count.clone(),
            wide_layout: Rc::new(Cell::new(false)),
        });
        let other_count = Rc::new(Cell::new(0));
        let (_, other_window) = app.add_window_with_bounds(
            WindowStyle::NotStealFocus,
            WindowBounds::ExactPosition(bounds),
            |ctx| BoundsView {
                window_id: ctx.window_id(),
                reads_bounds: true,
                render_count: other_count.clone(),
                wide_layout: Rc::new(Cell::new(false)),
            },
        );
        reader.update(&mut app, |_, _| {});
        unrelated.update(&mut app, |_, _| {});
        other_window.update(&mut app, |_, _| {});
        let before = (count.get(), unrelated_count.get(), other_count.get());
        assert!(wide_layout.get());

        let resized = RectF::new(bounds.origin(), vec2f(800., 600.));
        app.update(|ctx| ctx.update_window_bounds(window_id, resized));
        assert!(!wide_layout.get());
        assert_eq!(count.get(), before.0 + 1);
        assert_eq!(unrelated_count.get(), before.1);
        assert_eq!(other_count.get(), before.2);

        // Native move notifications and repeated resize notifications must not
        // reconstruct size-dependent chrome or unrelated views.
        app.update(|ctx| ctx.update_window_bounds(window_id, resized));
        let moved = RectF::new(vec2f(40., 60.), resized.size());
        app.update(|ctx| ctx.update_window_bounds(window_id, moved));
        assert_eq!(count.get(), before.0 + 1);
        assert_eq!(app.window_bounds(&window_id), Some(moved));

        app.update(|ctx| ctx.update_window_bounds(window_id, bounds));
        assert!(wide_layout.get());
        assert_eq!(count.get(), before.0 + 2);
        assert_eq!(unrelated_count.get(), before.1);
        assert_eq!(other_count.get(), before.2);
    });
}

#[test]
fn initial_native_bounds_invalidate_a_view_that_rendered_without_bounds() {
    App::test((), |mut app| async move {
        let count = Rc::new(Cell::new(0));
        let wide_layout = Rc::new(Cell::new(false));
        let (window_id, reader) = app.add_window(WindowStyle::NotStealFocus, |ctx| BoundsView {
            window_id: ctx.window_id(),
            reads_bounds: true,
            render_count: count.clone(),
            wide_layout: wide_layout.clone(),
        });
        reader.update(&mut app, |_, _| {});
        assert_eq!(app.window_bounds(&window_id), None);
        assert!(!wide_layout.get());
        let before = count.get();

        let bounds = RectF::new(vec2f(0., 0.), vec2f(1200., 800.));
        app.update(|ctx| ctx.update_window_bounds(window_id, bounds));
        assert_eq!(count.get(), before + 1);
        assert!(wide_layout.get());
    });
}
