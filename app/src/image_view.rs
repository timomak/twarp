//! A read-only pane that renders an image file (PNG/JPEG/GIF/WebP) in-app.
//!
//! Opening a raster image from the file tree previously handed the path to
//! the system default application (macOS Preview) via
//! `FileTarget::SystemGeneric`. This view keeps the preview inside twarp:
//! the file is decoded by [`twarpui::ImageCache`] and drawn contain-fit,
//! centered in the pane. Animated GIF/WebP sources play from the moment the
//! pane is created.

use std::path::PathBuf;

use instant::Instant;
use twarpui::assets::asset_cache::AssetSource;
use twarpui::text_layout::ClipConfig;
use twarpui::{
    elements::{CacheOption, Container, Image},
    AppContext, Element, Entity, ModelHandle, TypedActionView, View, ViewContext,
};

use crate::pane_group::focus_state::PaneFocusHandle;
use crate::pane_group::{
    pane::view::{self, HeaderContent, StandardHeader, StandardHeaderOptions},
    BackingView, PaneConfiguration, PaneEvent,
};

/// Padding between the pane edges and the rendered image.
const IMAGE_PADDING: f32 = 12.;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageViewEvent {
    Pane(PaneEvent),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageViewAction {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageViewCustomAction {}

/// A pane view that renders a single local image file.
pub struct ImageView {
    path: PathBuf,
    pane_configuration: ModelHandle<PaneConfiguration>,
    focus_handle: Option<PaneFocusHandle>,
    /// Anchor for animated GIF/WebP playback (see
    /// [`Image::enable_animation_with_start_time`]).
    created_at: Instant,
}

impl ImageView {
    pub fn new(path: PathBuf, ctx: &mut ViewContext<Self>) -> Self {
        let pane_configuration = ctx.add_model(|_ctx| PaneConfiguration::new(title_for(&path)));

        Self {
            path,
            pane_configuration,
            focus_handle: None,
            created_at: Instant::now(),
        }
    }

    pub fn pane_configuration(&self) -> ModelHandle<PaneConfiguration> {
        self.pane_configuration.clone()
    }

    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    pub fn focus(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.focus_self();
    }
}

fn title_for(path: &std::path::Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

impl Entity for ImageView {
    type Event = ImageViewEvent;
}

impl View for ImageView {
    fn ui_name() -> &'static str {
        "ImageView"
    }

    fn render(&self, _app: &AppContext) -> Box<dyn Element> {
        // `CacheOption::Original` caches only the decoded asset and lets the
        // GPU scale it, so pane resizes don't re-run a CPU resize per size.
        let image = Image::new(
            AssetSource::LocalFile {
                path: self.path.display().to_string(),
            },
            CacheOption::Original,
        )
        .contain()
        .enable_animation_with_start_time(self.created_at)
        .finish();

        Container::new(image)
            .with_uniform_padding(IMAGE_PADDING)
            .finish()
    }
}

impl TypedActionView for ImageView {
    type Action = ImageViewAction;

    fn handle_action(&mut self, _action: &Self::Action, _ctx: &mut ViewContext<Self>) {
        // ImageViewAction is currently uninhabited.
    }
}

impl BackingView for ImageView {
    type PaneHeaderOverflowMenuAction = ImageViewAction;
    type CustomAction = ImageViewCustomAction;
    type AssociatedData = ();

    fn handle_pane_header_overflow_menu_action(
        &mut self,
        _action: &Self::PaneHeaderOverflowMenuAction,
        _ctx: &mut ViewContext<Self>,
    ) {
        // No overflow menu items are registered.
    }

    fn close(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.emit(ImageViewEvent::Pane(PaneEvent::Close));
    }

    fn focus_contents(&mut self, ctx: &mut ViewContext<Self>) {
        self.focus(ctx);
    }

    fn render_header_content(
        &self,
        _ctx: &view::HeaderRenderContext<'_>,
        _app: &AppContext,
    ) -> HeaderContent {
        HeaderContent::Standard(StandardHeader {
            title: title_for(&self.path),
            title_secondary: None,
            title_style: None,
            title_clip_config: ClipConfig::start(),
            title_max_width: None,
            left_of_title: None,
            right_of_title: None,
            left_of_overflow: None,
            options: StandardHeaderOptions::default(),
            title_on_double_click: None,
        })
    }

    fn set_focus_handle(&mut self, focus_handle: PaneFocusHandle, _ctx: &mut ViewContext<Self>) {
        self.focus_handle = Some(focus_handle);
    }
}
