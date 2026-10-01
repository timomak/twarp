use super::{ease_out_cubic, paint_shift, PanelSlide, SlideEdge};
use twarpui::{
    elements::Empty, platform::WindowStyle, App, AppContext, Element, Entity, TypedActionView, View,
};

#[test]
fn cubic_ease_clamps_and_reaches_expected_midpoint() {
    assert_eq!(ease_out_cubic(-1.0), 0.0);
    assert_eq!(ease_out_cubic(0.0), 0.0);
    assert_eq!(ease_out_cubic(0.5), 0.875);
    assert_eq!(ease_out_cubic(1.0), 1.0);
    assert_eq!(ease_out_cubic(2.0), 1.0);
}

#[test]
fn paint_shift_moves_each_rail_toward_its_window_edge() {
    assert_eq!(paint_shift(SlideEdge::Left, 25.0, 100.0), -75.0);
    assert_eq!(paint_shift(SlideEdge::Right, 25.0, 100.0), 0.0);
}

#[derive(Default)]
struct AnimatedView {
    slide: Option<PanelSlide>,
    ticks: usize,
}

impl Entity for AnimatedView {
    type Event = ();
}

impl TypedActionView for AnimatedView {
    type Action = ();
}

impl View for AnimatedView {
    fn ui_name() -> &'static str {
        "PanelSlideTest"
    }

    fn render(&self, _: &AppContext) -> Box<dyn Element> {
        Empty::new().finish()
    }
}

#[test]
fn rapid_reversals_only_run_the_current_transitions_tick() {
    App::test((), |mut app| async move {
        let (_, view) = app.add_window(WindowStyle::NotStealFocus, |_| AnimatedView::default());
        let pending = view.update(&mut app, |view, ctx| {
            (0..10)
                .map(|index| {
                    let (from, to) = if index % 2 == 0 { (0., 1.) } else { (1., 0.) };
                    view.slide = Some(PanelSlide::new(from, to));
                    view.slide.as_ref().unwrap().schedule_tick(ctx, |view, _| {
                        view.ticks += 1;
                    })
                })
                .collect::<Vec<_>>()
        });
        // Await each queued callback, including the superseded transitions.
        // No sleep or assumption about the executor's scheduling is needed.
        for tick in pending {
            app.update(|ctx| ctx.await_spawned_future(tick.future_id()))
                .await;
        }
        view.read(&app, |view, _| assert_eq!(view.ticks, 1));
    });
}

#[test]
fn clearing_a_slide_discards_its_pending_tick() {
    App::test((), |mut app| async move {
        let (_, view) = app.add_window(WindowStyle::NotStealFocus, |_| AnimatedView::default());
        let tick = view.update(&mut app, |view, ctx| {
            view.slide = Some(PanelSlide::new(1., 0.));
            let tick = view.slide.as_ref().unwrap().schedule_tick(ctx, |view, _| {
                view.ticks += 1;
            });
            view.slide = None;
            tick
        });
        app.update(|ctx| ctx.await_spawned_future(tick.future_id()))
            .await;
        view.read(&app, |view, _| assert_eq!(view.ticks, 0));
    });
}
