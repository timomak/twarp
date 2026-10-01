use std::{cell::Cell, rc::Rc};

use pathfinder_geometry::vector::{vec2f, Vector2F};

use crate::{
    elements::{ConstrainedBox, Flex, ParentElement, Rect, SavePosition, Stack},
    platform::WindowStyle,
    App, AppContext, Element, Entity, Presenter, TypedActionView, View, WindowInvalidation,
};

struct WindowSizedView {
    render_count: Rc<Cell<usize>>,
    parent_size: Option<Vector2F>,
}

impl Entity for WindowSizedView {
    type Event = ();
}

impl TypedActionView for WindowSizedView {
    type Action = ();
}

impl View for WindowSizedView {
    fn ui_name() -> &'static str {
        "WindowSizedView"
    }

    fn render(&self, _: &AppContext) -> Box<dyn Element> {
        self.render_count.set(self.render_count.get() + 1);
        let viewport =
            ConstrainedBox::new(SavePosition::new(Rect::new().finish(), "viewport").finish())
                .with_window_size()
                .finish();
        let content = match self.parent_size {
            Some(size) => ConstrainedBox::new(viewport)
                .with_width(size.x())
                .with_height(size.y())
                .finish(),
            // A fixed child in a column is measured with an infinite height.
            // The window constraint must still bound that intrinsic pass.
            None => Flex::column().with_child(viewport).finish(),
        };
        Stack::new().with_child(content).finish()
    }
}

#[test]
fn window_constraint_follows_resize_and_zoom_without_rebuilding_the_element() {
    App::test((), |mut app| async move {
        let count = Rc::new(Cell::new(0));
        let (window_id, view) = app.add_window(WindowStyle::NotStealFocus, |_| WindowSizedView {
            render_count: count.clone(),
            parent_size: None,
        });
        app.update(|ctx| {
            let mut presenter = Presenter::new(window_id);
            presenter.invalidate(
                WindowInvalidation {
                    updated: [view.id()].into(),
                    ..Default::default()
                },
                ctx,
            );
            let rendered = count.get();
            for window_size in [vec2f(800., 600.), vec2f(1200., 900.), vec2f(600., 400.)] {
                presenter.build_scene(window_size, 1., None, ctx);
                assert_eq!(
                    presenter
                        .position_cache()
                        .get_position("viewport")
                        .unwrap()
                        .size(),
                    window_size,
                );
                assert_eq!(count.get(), rendered);
            }

            ctx.set_zoom_factor(2.0);
            presenter.build_scene(vec2f(1200., 800.), 1., None, ctx);
            assert_eq!(
                presenter
                    .position_cache()
                    .get_position("viewport")
                    .unwrap()
                    .size(),
                vec2f(600., 400.),
            );
            assert_eq!(count.get(), rendered);
        });
    });
}

#[test]
fn window_constraint_respects_a_smaller_parent() {
    App::test((), |mut app| async move {
        let (window_id, view) = app.add_window(WindowStyle::NotStealFocus, |_| WindowSizedView {
            render_count: Rc::new(Cell::new(0)),
            parent_size: Some(vec2f(300., 200.)),
        });
        app.update(|ctx| {
            let mut presenter = Presenter::new(window_id);
            presenter.invalidate(
                WindowInvalidation {
                    updated: [view.id()].into(),
                    ..Default::default()
                },
                ctx,
            );
            presenter.build_scene(vec2f(1200., 800.), 1., None, ctx);
            assert_eq!(
                presenter
                    .position_cache()
                    .get_position("viewport")
                    .unwrap()
                    .size(),
                vec2f(300., 200.),
            );
        });
    });
}
