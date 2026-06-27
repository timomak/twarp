use std::f32::consts::TAU;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::{Element, Point};
use crate::{
    assets::asset_cache::{AssetCache, AssetSource, AssetState},
    event::DispatchedEvent,
    image_cache::{AnimatedImageBehavior, CacheOption, FitType, Image, ImageCache},
    AfterLayoutContext, AppContext, EventContext, LayoutContext, PaintContext, SingletonEntity,
    SizeConstraint,
};
use instant::Instant;
use pathfinder_color::ColorU;
use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::Vector2F;

/// Persistent animation phase for a [`PulsingIcon`]. Held by the owning view and
/// cloned into the element each render so the pulse keeps a continuous clock
/// instead of restarting every time `render()` re-runs.
#[derive(Clone)]
pub struct PulsingIconStateHandle(Arc<Mutex<Instant>>);

impl Default for PulsingIconStateHandle {
    fn default() -> Self {
        Self::new()
    }
}

impl PulsingIconStateHandle {
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(Instant::now())))
    }

    fn started_at(&self) -> Instant {
        *self.0.lock().expect("Mutex should not be poisoned")
    }
}

/// An icon that gently breathes its opacity to signal live, in-progress work
/// (e.g. the Claude glyph beside the streaming "Working…" status). Like [`Icon`],
/// it tints a monochrome SVG dynamically; unlike `Icon`, it animates opacity on a
/// sine wave and self-schedules ~30fps repaints, so the pulse runs in the paint
/// phase without the owning view re-running `render()`.
///
/// [`Icon`]: crate::elements::Icon
pub struct PulsingIcon {
    path: &'static str,
    size: Option<Vector2F>,
    origin: Option<Point>,
    color: ColorU,
    period: Duration,
    min_opacity: f32,
    max_opacity: f32,
    handle: PulsingIconStateHandle,
}

impl PulsingIcon {
    pub fn new(
        path: &'static str,
        color: impl Into<ColorU>,
        handle: PulsingIconStateHandle,
    ) -> Self {
        Self {
            path,
            size: None,
            origin: None,
            color: color.into(),
            period: Duration::from_millis(1100),
            min_opacity: 0.35,
            max_opacity: 1.0,
            handle,
        }
    }

    /// Length of one full dim→bright→dim cycle. Defaults to 1.1s.
    pub fn with_period(mut self, period: Duration) -> Self {
        self.period = period;
        self
    }

    /// Opacity bounds the pulse sweeps between. Defaults to `0.35..=1.0`.
    pub fn with_opacity_range(mut self, min: f32, max: f32) -> Self {
        self.min_opacity = min;
        self.max_opacity = max;
        self
    }

    fn current_opacity(&self) -> f32 {
        let period = self.period.as_secs_f32().max(0.001);
        let elapsed = self.handle.started_at().elapsed().as_secs_f32();
        let phase = (elapsed / period) * TAU;
        // cos maps [0, period] -> opacity that starts at `min`, rises to `max` at
        // the half cycle, and eases back: 0.5 - 0.5*cos(phase) is in [0, 1].
        let t = 0.5 - 0.5 * phase.cos();
        self.min_opacity + (self.max_opacity - self.min_opacity) * t
    }
}

impl Element for PulsingIcon {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        _: &mut LayoutContext,
        _: &AppContext,
    ) -> Vector2F {
        let size = constraint.max;
        self.size = Some(size);
        size
    }

    fn after_layout(&mut self, _: &mut AfterLayoutContext, _: &AppContext) {}

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        /// Duration, in ms, for which to repaint. Approximately 30fps.
        const REPAINT_DURATION: u64 = 32;

        let bounds = (self.size.unwrap() * ctx.scene.scale_factor()).to_i32();

        // If the x or y bounds are less than or equal to 0, don't attempt to paint the icon.
        if bounds.x() <= 0 || bounds.y() <= 0 {
            return;
        }

        let opacity = self.current_opacity();
        let asset_cache = AssetCache::as_ref(app);
        match ImageCache::as_ref(app).image(
            AssetSource::Bundled { path: self.path },
            bounds,
            FitType::Contain,
            AnimatedImageBehavior::FullAnimation,
            CacheOption::BySize,
            ctx.max_texture_dimension_2d,
            asset_cache,
        ) {
            AssetState::Loaded { data } => match data.as_ref() {
                Image::Static(image) => {
                    let logical_image_size = image.size().to_f32() / ctx.scene.scale_factor();
                    let origin = origin + ((self.size().unwrap() - logical_image_size) / 2.0);
                    self.origin = Some(Point::from_vec2f(origin, ctx.scene.z_index()));

                    ctx.scene.draw_icon(
                        RectF::new(origin, logical_image_size),
                        image.clone(),
                        opacity,
                        self.color,
                    );

                    // Keep the pulse running while the icon is on screen.
                    ctx.repaint_after(Duration::from_millis(REPAINT_DURATION));
                }
                Image::Animated(_image) => {
                    log::info!("Animated icons are currently not supported");
                }
            },
            AssetState::Loading { handle } => {
                ctx.repaint_after_load(handle);
            }
            AssetState::Evicted => {
                log::warn!("Unable to render svg because it was evicted");
            }
            AssetState::FailedToLoad(err) => {
                log::warn!("Unable to render svg: {err:#}");
            }
        }
    }

    fn size(&self) -> Option<Vector2F> {
        self.size
    }

    fn dispatch_event(
        &mut self,
        _: &DispatchedEvent,
        _: &mut EventContext,
        _: &AppContext,
    ) -> bool {
        false
    }

    fn origin(&self) -> Option<Point> {
        self.origin
    }
}
