//! twarp 20c: the real Skills automation page — a management UI over the
//! twarp-managed shared-skills store ([`crate::skills_store`]).
//!
//! Layout: a centered chrome-class column with the store listed as rows
//! (`/name`, description, per-provider enable switches, Reveal / Delete with a
//! two-click confirm), an inline New-skill form (no modals), and an Import
//! section offering to adopt real `~/.claude/skills` directories into the
//! store. v1 imports user-level skills only (project-level `.claude/skills`
//! dirs are out of scope).

use std::collections::HashMap;

use twarp_core::ui::tokens::{radius, spacing, type_ramp};
use twarpui::{
    elements::{
        new_scrollable::{NewScrollable, ScrollableAppearance, SingleAxisConfig},
        Align, Border, ChildView, ClippedScrollStateHandle, ConstrainedBox, Container,
        CornerRadius, CrossAxisAlignment, Element, Flex, MainAxisSize, ParentElement, Radius,
        ScrollbarWidth, Shrinkable, Text,
    },
    ui_components::{
        components::{UiComponent, UiComponentStyles},
        switch::SwitchStateHandle,
    },
    AppContext, SingletonEntity, ViewContext, ViewHandle,
};

use crate::appearance::Appearance;
use crate::editor::{
    EditorView, PropagateAndNoOpNavigationKeys, SingleLineEditorOptions, TextOptions,
};
use crate::skills_store::{is_valid_skill_name, AdoptableSkill, SkillEntry, SkillsStoreModel};
use crate::view_components::action_button::{
    ActionButton, DangerSecondaryTheme, PrimaryTheme, SecondaryTheme,
};

use super::view::{AutomationView, AutomationViewAction};

/// Content column width; matches the conversation measure used app-wide.
const CONTENT_MAX_WIDTH: f32 = 720.;

/// Actions dispatched by the Skills page's controls, wrapped in
/// [`AutomationViewAction::Skills`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillsPageAction {
    /// Open the inline New-skill form.
    OpenNew,
    ToggleClaude(String),
    ToggleCodex(String),
    /// First click of the two-click delete affordance.
    RequestDelete(String),
    /// Second click: actually delete.
    ConfirmDelete(String),
    /// Reveal the skill's store directory in Finder.
    Reveal(String),
    /// Adopt a real `~/.claude/skills/<name>` directory into the store.
    Adopt(String),
    Save,
    Cancel,
}

/// Persistent per-row UI handles so hover/switch state survives re-renders.
struct RowUi {
    claude_switch: SwitchStateHandle,
    codex_switch: SwitchStateHandle,
    reveal_button: ViewHandle<ActionButton>,
    delete_button: ViewHandle<ActionButton>,
    confirm_delete_button: ViewHandle<ActionButton>,
}

/// The inline New-skill form.
struct SkillForm {
    name_editor: ViewHandle<EditorView>,
    description_editor: ViewHandle<EditorView>,
    save_button: ViewHandle<ActionButton>,
    cancel_button: ViewHandle<ActionButton>,
    /// Validation error surfaced above the Save row.
    error: Option<String>,
}

/// State backing the Skills page, owned by [`AutomationView`] when its page is
/// [`super::AutomationPage::Skills`].
pub struct SkillsPageState {
    scroll_state: ClippedScrollStateHandle,
    new_button: ViewHandle<ActionButton>,
    /// Dedicated CTA for the empty state (20e) — a view handle can't be
    /// mounted both in the header and in the empty state at once.
    empty_new_button: ViewHandle<ActionButton>,
    rows: HashMap<String, RowUi>,
    adopt_buttons: HashMap<String, ViewHandle<ActionButton>>,
    form: Option<SkillForm>,
    /// Skill name whose Delete button is currently in its Confirm stage.
    pending_delete: Option<String>,
}

impl SkillsPageState {
    pub fn new(ctx: &mut ViewContext<AutomationView>) -> Self {
        let new_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("New skill", PrimaryTheme).on_click(|ctx| {
                ctx.dispatch_typed_action(AutomationViewAction::Skills(SkillsPageAction::OpenNew));
            })
        });
        let empty_new_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("New skill", PrimaryTheme).on_click(|ctx| {
                ctx.dispatch_typed_action(AutomationViewAction::Skills(SkillsPageAction::OpenNew));
            })
        });
        let mut state = Self {
            scroll_state: Default::default(),
            new_button,
            empty_new_button,
            rows: HashMap::new(),
            adopt_buttons: HashMap::new(),
            form: None,
            pending_delete: None,
        };
        state.sync_rows(ctx);
        state
    }

    /// Keep one [`RowUi`] / Adopt button alive per store / adoptable skill
    /// (created on demand, dropped when the skill goes away). Called from
    /// actions and whenever [`SkillsStoreModel`] notifies.
    pub fn sync_rows(&mut self, ctx: &mut ViewContext<AutomationView>) {
        let store = SkillsStoreModel::as_ref(ctx);
        let names: Vec<String> = store.skills().iter().map(|s| s.name.clone()).collect();
        let adoptable: Vec<String> = store.adoptable().iter().map(|s| s.name.clone()).collect();

        self.rows.retain(|name, _| names.iter().any(|n| n == name));
        if self
            .pending_delete
            .as_ref()
            .is_some_and(|name| !names.iter().any(|n| n == name))
        {
            self.pending_delete = None;
        }
        for name in names {
            if self.rows.contains_key(&name) {
                continue;
            }
            let reveal_name = name.clone();
            let reveal_button = ctx.add_typed_action_view(|_| {
                ActionButton::new("Reveal", SecondaryTheme).on_click(move |ctx| {
                    ctx.dispatch_typed_action(AutomationViewAction::Skills(
                        SkillsPageAction::Reveal(reveal_name.clone()),
                    ));
                })
            });
            let delete_name = name.clone();
            let delete_button = ctx.add_typed_action_view(|_| {
                ActionButton::new("Delete", DangerSecondaryTheme).on_click(move |ctx| {
                    ctx.dispatch_typed_action(AutomationViewAction::Skills(
                        SkillsPageAction::RequestDelete(delete_name.clone()),
                    ));
                })
            });
            let confirm_name = name.clone();
            let confirm_delete_button = ctx.add_typed_action_view(|_| {
                ActionButton::new("Confirm delete", DangerSecondaryTheme).on_click(move |ctx| {
                    ctx.dispatch_typed_action(AutomationViewAction::Skills(
                        SkillsPageAction::ConfirmDelete(confirm_name.clone()),
                    ));
                })
            });
            self.rows.insert(
                name,
                RowUi {
                    claude_switch: Default::default(),
                    codex_switch: Default::default(),
                    reveal_button,
                    delete_button,
                    confirm_delete_button,
                },
            );
        }

        self.adopt_buttons
            .retain(|name, _| adoptable.iter().any(|n| n == name));
        for name in adoptable {
            if self.adopt_buttons.contains_key(&name) {
                continue;
            }
            let adopt_name = name.clone();
            let button = ctx.add_typed_action_view(|_| {
                ActionButton::new("Adopt", PrimaryTheme).on_click(move |ctx| {
                    ctx.dispatch_typed_action(AutomationViewAction::Skills(
                        SkillsPageAction::Adopt(adopt_name.clone()),
                    ));
                })
            });
            self.adopt_buttons.insert(name, button);
        }
    }

    pub fn handle_action(
        &mut self,
        action: &SkillsPageAction,
        ctx: &mut ViewContext<AutomationView>,
    ) {
        match action {
            SkillsPageAction::OpenNew => {
                self.form = Some(self.new_form(ctx));
            }
            SkillsPageAction::ToggleClaude(name) => {
                SkillsStoreModel::handle(ctx).update(ctx, |m, mctx| {
                    m.toggle_enabled(name, true, mctx);
                });
            }
            SkillsPageAction::ToggleCodex(name) => {
                SkillsStoreModel::handle(ctx).update(ctx, |m, mctx| {
                    m.toggle_enabled(name, false, mctx);
                });
            }
            SkillsPageAction::RequestDelete(name) => {
                self.pending_delete = Some(name.clone());
            }
            SkillsPageAction::ConfirmDelete(name) => {
                self.pending_delete = None;
                SkillsStoreModel::handle(ctx).update(ctx, |m, mctx| m.delete(name, mctx));
            }
            SkillsPageAction::Reveal(name) => {
                if let Some(dir) = SkillsStoreModel::as_ref(ctx).skill_dir(name) {
                    reveal_in_finder(&dir);
                }
            }
            SkillsPageAction::Adopt(name) => {
                SkillsStoreModel::handle(ctx).update(ctx, |m, mctx| m.adopt(name, mctx));
            }
            SkillsPageAction::Save => self.save(ctx),
            SkillsPageAction::Cancel => self.form = None,
        }
        self.sync_rows(ctx);
        ctx.notify();
    }

    fn new_form(&self, ctx: &mut ViewContext<AutomationView>) -> SkillForm {
        let name_editor = new_single_line_editor("e.g. deploy-checklist", ctx);
        let description_editor = new_single_line_editor("What this skill is for", ctx);
        let save_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Create", PrimaryTheme).on_click(|ctx| {
                ctx.dispatch_typed_action(AutomationViewAction::Skills(SkillsPageAction::Save));
            })
        });
        let cancel_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Cancel", SecondaryTheme).on_click(|ctx| {
                ctx.dispatch_typed_action(AutomationViewAction::Skills(SkillsPageAction::Cancel));
            })
        });
        SkillForm {
            name_editor,
            description_editor,
            save_button,
            cancel_button,
            error: None,
        }
    }

    /// Validate the New-skill form and commit it to the store.
    fn save(&mut self, ctx: &mut ViewContext<AutomationView>) {
        let Some(form) = self.form.as_mut() else {
            return;
        };
        let name = form
            .name_editor
            .as_ref(ctx)
            .buffer_text(ctx)
            .trim()
            .to_owned();
        let description = form
            .description_editor
            .as_ref(ctx)
            .buffer_text(ctx)
            .trim()
            .to_owned();

        let error = if !is_valid_skill_name(&name) {
            Some("Name must be kebab-case: lowercase letters, digits, and hyphens.".to_owned())
        } else if SkillsStoreModel::as_ref(ctx).name_taken(&name) {
            Some("A skill with this name already exists.".to_owned())
        } else {
            None
        };
        if let Some(error) = error {
            form.error = Some(error);
            return;
        }

        SkillsStoreModel::handle(ctx).update(ctx, |m, mctx| m.create(name, description, mctx));
        self.form = None;
    }

    pub fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let store = SkillsStoreModel::as_ref(app);

        let mut column = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);

        // 20e: while the empty state (with its own CTA) shows, the header's
        // New-skill button is hidden — one clear next action, not two.
        let show_empty_state = store.skills().is_empty() && self.form.is_none();

        // Header: title left, New-skill button right.
        let mut header = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                Shrinkable::new(
                    1.,
                    Align::new(
                        Text::new_inline(
                            "Skills",
                            appearance.ui_font_family(),
                            type_ramp::HEADING.size,
                        )
                        .with_line_height_ratio(type_ramp::HEADING.line_height)
                        .with_color(theme.main_text_color(theme.background()).into())
                        .finish(),
                    )
                    .left()
                    .finish(),
                )
                .finish(),
            );
        if !show_empty_state {
            header.add_child(ChildView::new(&self.new_button).finish());
        }
        column.add_child(header.finish());
        column.add_child(
            Container::new(
                Text::new_inline(
                    "Skills here live in ~/.twarp/skills and are shared with both Claude and Codex.",
                    appearance.ui_font_family(),
                    type_ramp::UI.size,
                )
                .with_line_height_ratio(type_ramp::UI.line_height)
                .with_color(theme.sub_text_color(theme.background()).into())
                .finish(),
            )
            .with_margin_top(spacing::XS)
            .with_margin_bottom(spacing::LG)
            .finish(),
        );

        if let Some(form) = &self.form {
            column.add_child(self.render_form(form, app));
        }

        let skills = store.skills();
        if show_empty_state {
            column.add_child(super::render_empty_state(
                twarp_core::ui::Icon::BookOpen,
                "No skills yet.",
                &self.empty_new_button,
                app,
            ));
        }
        for skill in skills {
            column.add_child(self.render_row(skill, app));
        }

        let adoptable = store.adoptable();
        if !adoptable.is_empty() {
            column.add_child(
                Container::new(
                    Text::new_inline(
                        "IMPORT FROM ~/.claude/skills",
                        appearance.ui_font_family(),
                        type_ramp::CAPTION.size,
                    )
                    .with_line_height_ratio(type_ramp::CAPTION.line_height)
                    .with_color(theme.sub_text_color(theme.background()).into())
                    .finish(),
                )
                .with_margin_top(spacing::XL)
                .with_margin_bottom(spacing::SM)
                .finish(),
            );
            for skill in adoptable {
                column.add_child(self.render_adopt_row(skill, app));
            }
        }

        let content = Container::new(
            ConstrainedBox::new(column.finish())
                .with_max_width(CONTENT_MAX_WIDTH)
                .finish(),
        )
        .with_uniform_padding(spacing::XL)
        .finish();

        let centered = Align::new(content).top_center().finish();

        let scrollable = NewScrollable::vertical(
            SingleAxisConfig::Clipped {
                handle: self.scroll_state.clone(),
                child: centered,
            },
            theme.nonactive_ui_detail().into(),
            theme.active_ui_detail().into(),
            twarpui::elements::Fill::None,
        )
        .with_vertical_scrollbar(ScrollableAppearance::new(ScrollbarWidth::Auto, false))
        // The page nests editors (form fields); propagate deltas this vertical
        // scrollable can't act on so they still reach the inner editors.
        .with_propagate_mousewheel_if_not_handled(true)
        .finish();

        Container::new(scrollable)
            .with_background(theme.background())
            .finish()
    }

    /// One store row: /name + description left; conflict note, C/X switches,
    /// Reveal, Delete right.
    fn render_row(&self, skill: &SkillEntry, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let Some(row_ui) = self.rows.get(&skill.name) else {
            // A skill scanned in outside this view's actions; drawn without
            // controls until the next sync.
            return render_plain_row(&format!("/{}", skill.name), &skill.description, appearance);
        };

        let mut labels = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(
                Text::new_inline(
                    format!("/{}", skill.name),
                    appearance.monospace_font_family(),
                    type_ramp::UI.size,
                )
                .with_line_height_ratio(type_ramp::UI.line_height)
                .with_color(theme.main_text_color(theme.background()).into())
                .finish(),
            );
        if !skill.description.is_empty() {
            labels = labels.with_child(
                Text::new_inline(
                    skill.description.clone(),
                    appearance.ui_font_family(),
                    type_ramp::CAPTION.size,
                )
                .with_line_height_ratio(type_ramp::CAPTION.line_height)
                .with_color(theme.sub_text_color(theme.background()).into())
                .finish(),
            );
        }
        if let Some(conflict) = conflict_note(skill) {
            labels = labels.with_child(
                Text::new_inline(
                    conflict,
                    appearance.ui_font_family(),
                    type_ramp::CAPTION.size,
                )
                .with_line_height_ratio(type_ramp::CAPTION.line_height)
                .with_color(theme.ui_error_color().into())
                .finish(),
            );
        }

        let claude_name = skill.name.clone();
        let codex_name = skill.name.clone();
        let delete_button = if self.pending_delete.as_deref() == Some(skill.name.as_str()) {
            &row_ui.confirm_delete_button
        } else {
            &row_ui.delete_button
        };
        let controls = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(spacing::SM)
            .with_child(render_labeled_switch(
                "Claude",
                skill.enabled_claude,
                row_ui.claude_switch.clone(),
                move |ctx| {
                    ctx.dispatch_typed_action(AutomationViewAction::Skills(
                        SkillsPageAction::ToggleClaude(claude_name.clone()),
                    ));
                },
                appearance,
            ))
            .with_child(render_labeled_switch(
                "Codex",
                skill.enabled_codex,
                row_ui.codex_switch.clone(),
                move |ctx| {
                    ctx.dispatch_typed_action(AutomationViewAction::Skills(
                        SkillsPageAction::ToggleCodex(codex_name.clone()),
                    ));
                },
                appearance,
            ))
            .with_child(ChildView::new(&row_ui.reveal_button).finish())
            .with_child(ChildView::new(delete_button).finish())
            .finish();

        Container::new(
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(Shrinkable::new(1., labels.finish()).finish())
                .with_child(controls)
                .finish(),
        )
        .with_uniform_padding(spacing::MD)
        .with_margin_bottom(spacing::SM)
        .with_border(Border::all(1.).with_border_fill(theme.outline()))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius::CARD)))
        .finish()
    }

    /// One Import row: /name + source path left, Adopt button right.
    fn render_adopt_row(&self, skill: &AdoptableSkill, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let labels = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(
                Text::new_inline(
                    format!("/{}", skill.name),
                    appearance.monospace_font_family(),
                    type_ramp::UI.size,
                )
                .with_line_height_ratio(type_ramp::UI.line_height)
                .with_color(theme.main_text_color(theme.background()).into())
                .finish(),
            )
            .with_child(
                Text::new_inline(
                    skill.path.to_string_lossy().into_owned(),
                    appearance.monospace_font_family(),
                    type_ramp::CAPTION.size,
                )
                .with_line_height_ratio(type_ramp::CAPTION.line_height)
                .with_color(theme.sub_text_color(theme.background()).into())
                .finish(),
            )
            .finish();

        let mut row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(Shrinkable::new(1., labels).finish());
        if let Some(button) = self.adopt_buttons.get(&skill.name) {
            row = row.with_child(ChildView::new(button).finish());
        }

        Container::new(row.finish())
            .with_uniform_padding(spacing::MD)
            .with_margin_bottom(spacing::SM)
            .with_border(Border::all(1.).with_border_fill(theme.outline()))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius::CARD)))
            .finish()
    }

    /// The inline New-skill form.
    fn render_form(&self, form: &SkillForm, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let mut column = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
        column.add_child(render_form_label(
            "New skill",
            type_ramp::UI.size,
            appearance,
        ));
        column.add_child(render_form_field("Name", &form.name_editor, appearance));
        column.add_child(render_form_field(
            "Description",
            &form.description_editor,
            appearance,
        ));

        if let Some(error) = &form.error {
            column.add_child(
                Container::new(
                    Text::new_inline(
                        error.clone(),
                        appearance.ui_font_family(),
                        type_ramp::LABEL.size,
                    )
                    .with_line_height_ratio(type_ramp::LABEL.line_height)
                    .with_color(theme.ui_error_color().into())
                    .finish(),
                )
                .with_margin_bottom(spacing::SM)
                .finish(),
            );
        }

        column.add_child(
            Flex::row()
                .with_spacing(spacing::SM)
                .with_child(ChildView::new(&form.save_button).finish())
                .with_child(ChildView::new(&form.cancel_button).finish())
                .finish(),
        );

        Container::new(column.finish())
            .with_uniform_padding(spacing::LG)
            .with_margin_bottom(spacing::LG)
            .with_border(Border::all(1.).with_border_fill(theme.outline()))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius::CARD)))
            .finish()
    }
}

/// Short conflict/status caption for a row, or `None` when healthy.
fn conflict_note(skill: &SkillEntry) -> Option<String> {
    match (skill.claude_conflict, skill.codex_conflict) {
        (true, true) => {
            Some("⚠ unmanaged files block the Claude and Codex materializations".to_owned())
        }
        (true, false) => Some("⚠ a real ~/.claude/skills directory blocks the symlink".to_owned()),
        (false, true) => Some("⚠ an unmanaged ~/.codex/prompts file blocks the prompt".to_owned()),
        (false, false) => None,
    }
}

/// Reveal a path in Finder.
fn reveal_in_finder(path: &std::path::Path) {
    if let Err(err) = std::process::Command::new("open")
        .arg("-R")
        .arg(path)
        .spawn()
    {
        log::error!("failed to reveal {path:?} in Finder: {err}");
    }
}

fn new_single_line_editor(
    placeholder: &str,
    ctx: &mut ViewContext<AutomationView>,
) -> ViewHandle<EditorView> {
    let placeholder = placeholder.to_owned();
    ctx.add_typed_action_view(move |ctx| {
        let options = SingleLineEditorOptions {
            text: TextOptions::ui_font_size(Appearance::as_ref(ctx)),
            propagate_and_no_op_vertical_navigation_keys: PropagateAndNoOpNavigationKeys::Always,
            ..Default::default()
        };
        let mut editor = EditorView::single_line(options, ctx);
        editor.set_placeholder_text(placeholder, ctx);
        editor
    })
}

/// A labeled form field: caption label above a hairline-bordered editor.
fn render_form_field(
    label: &str,
    editor: &ViewHandle<EditorView>,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let input = Container::new(
        appearance
            .ui_builder()
            .text_input(editor.clone())
            .with_style(UiComponentStyles::default())
            .build()
            .finish(),
    )
    .with_border(Border::all(1.).with_border_fill(theme.outline()))
    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius::CARD)))
    .finish();

    Container::new(
        Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(render_form_label(label, type_ramp::LABEL.size, appearance))
            .with_child(input)
            .finish(),
    )
    .with_margin_bottom(spacing::SM)
    .finish()
}

fn render_form_label(text: &str, size: f32, appearance: &Appearance) -> Box<dyn Element> {
    let theme = appearance.theme();
    Container::new(
        Text::new_inline(text.to_owned(), appearance.ui_font_family(), size)
            .with_line_height_ratio(type_ramp::LABEL.line_height)
            .with_color(theme.sub_text_color(theme.background()).into())
            .finish(),
    )
    .with_margin_bottom(spacing::XS)
    .finish()
}

/// A provider switch with its caption label to the left.
fn render_labeled_switch(
    label: &str,
    checked: bool,
    state: SwitchStateHandle,
    on_click: impl Fn(&mut twarpui::EventContext) + 'static,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_main_axis_size(MainAxisSize::Min)
        .with_spacing(spacing::XS)
        .with_child(
            Text::new_inline(
                label.to_owned(),
                appearance.ui_font_family(),
                type_ramp::CAPTION.size,
            )
            .with_line_height_ratio(type_ramp::CAPTION.line_height)
            .with_color(theme.sub_text_color(theme.background()).into())
            .finish(),
        )
        .with_child(
            appearance
                .ui_builder()
                .switch(state)
                .check(checked)
                .build()
                .on_click(move |ctx, _, _| on_click(ctx))
                .finish(),
        )
        .finish()
}

/// A controls-free row (used before this view's row UI has synced).
fn render_plain_row(name: &str, description: &str, appearance: &Appearance) -> Box<dyn Element> {
    let theme = appearance.theme();
    let labels = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Start)
        .with_child(
            Text::new_inline(
                name.to_owned(),
                appearance.monospace_font_family(),
                type_ramp::UI.size,
            )
            .with_line_height_ratio(type_ramp::UI.line_height)
            .with_color(theme.main_text_color(theme.background()).into())
            .finish(),
        )
        .with_child(
            Text::new_inline(
                description.to_owned(),
                appearance.ui_font_family(),
                type_ramp::CAPTION.size,
            )
            .with_line_height_ratio(type_ramp::CAPTION.line_height)
            .with_color(theme.sub_text_color(theme.background()).into())
            .finish(),
        )
        .finish();

    Container::new(labels)
        .with_uniform_padding(spacing::MD)
        .with_margin_bottom(spacing::SM)
        .with_border(Border::all(1.).with_border_fill(theme.outline()))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius::CARD)))
        .finish()
}
