//! twarp 20b: the real MCPs automation page — a management UI over the
//! user-editable MCP-server registry ([`crate::mcp_registry`]).
//!
//! Layout: a centered chrome-class column with the registry listed as rows
//! (name, transport summary, per-provider enable switches, Edit / Delete),
//! the built-in servers shown as read-only rows, and an inline expanding
//! editor form (no modals) for Add / Edit.

use std::collections::HashMap;

use twarp_core::features::FeatureFlag;
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
    EditorOptions, EditorView, PropagateAndNoOpNavigationKeys, SingleLineEditorOptions, TextOptions,
};
use crate::mcp_registry::{McpRegistryModel, McpServerEntry, McpTransport};
use crate::view_components::{
    action_button::{ActionButton, DangerSecondaryTheme, PrimaryTheme, SecondaryTheme},
    dropdown::DropdownItem,
    Dropdown,
};

use super::view::{AutomationView, AutomationViewAction};

/// Content column width; matches the conversation measure used app-wide.
const CONTENT_MAX_WIDTH: f32 = 720.;

/// Actions dispatched by the MCPs page's controls, wrapped in
/// [`AutomationViewAction::Mcps`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpsPageAction {
    /// Open the inline editor with a blank form.
    OpenAdd,
    /// Open the inline editor pre-filled from the given registry entry.
    OpenEdit(String),
    Delete(String),
    ToggleClaude(String),
    ToggleCodex(String),
    /// Transport selected in the editor form ("stdio" | "http").
    SetTransport(String),
    ToggleFormClaude,
    ToggleFormCodex,
    Save,
    Cancel,
}

/// Persistent per-row UI handles so hover/switch state survives re-renders.
struct RowUi {
    claude_switch: SwitchStateHandle,
    codex_switch: SwitchStateHandle,
    edit_button: ViewHandle<ActionButton>,
    delete_button: ViewHandle<ActionButton>,
}

/// The inline Add / Edit form.
struct McpEditor {
    /// `Some` when editing an existing entry, `None` when adding.
    existing_id: Option<String>,
    name_editor: ViewHandle<EditorView>,
    command_editor: ViewHandle<EditorView>,
    args_editor: ViewHandle<EditorView>,
    url_editor: ViewHandle<EditorView>,
    env_editor: ViewHandle<EditorView>,
    transport_dropdown: ViewHandle<Dropdown<AutomationViewAction>>,
    transport: McpTransport,
    enabled_claude: bool,
    enabled_codex: bool,
    claude_switch: SwitchStateHandle,
    codex_switch: SwitchStateHandle,
    save_button: ViewHandle<ActionButton>,
    cancel_button: ViewHandle<ActionButton>,
    /// Validation error surfaced above the Save row.
    error: Option<String>,
}

/// State backing the MCPs page, owned by [`AutomationView`] when its page is
/// [`super::AutomationPage::Mcps`].
pub struct McpsPageState {
    scroll_state: ClippedScrollStateHandle,
    add_button: ViewHandle<ActionButton>,
    rows: HashMap<String, RowUi>,
    editor: Option<McpEditor>,
}

impl McpsPageState {
    pub fn new(ctx: &mut ViewContext<AutomationView>) -> Self {
        let add_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Add MCP server", PrimaryTheme).on_click(|ctx| {
                ctx.dispatch_typed_action(AutomationViewAction::Mcps(McpsPageAction::OpenAdd));
            })
        });
        let mut state = Self {
            scroll_state: Default::default(),
            add_button,
            rows: HashMap::new(),
            editor: None,
        };
        state.sync_rows(ctx);
        state
    }

    /// Keep one [`RowUi`] alive per registry entry (created on demand,
    /// dropped when the entry goes away).
    fn sync_rows(&mut self, ctx: &mut ViewContext<AutomationView>) {
        let ids: Vec<String> = McpRegistryModel::as_ref(ctx)
            .servers()
            .iter()
            .map(|s| s.id.clone())
            .collect();
        self.rows.retain(|id, _| ids.iter().any(|i| i == id));
        for id in ids {
            if self.rows.contains_key(&id) {
                continue;
            }
            let edit_id = id.clone();
            let edit_button = ctx.add_typed_action_view(|_| {
                ActionButton::new("Edit", SecondaryTheme).on_click(move |ctx| {
                    ctx.dispatch_typed_action(AutomationViewAction::Mcps(
                        McpsPageAction::OpenEdit(edit_id.clone()),
                    ));
                })
            });
            let delete_id = id.clone();
            let delete_button = ctx.add_typed_action_view(|_| {
                ActionButton::new("Delete", DangerSecondaryTheme).on_click(move |ctx| {
                    ctx.dispatch_typed_action(AutomationViewAction::Mcps(McpsPageAction::Delete(
                        delete_id.clone(),
                    )));
                })
            });
            self.rows.insert(
                id,
                RowUi {
                    claude_switch: Default::default(),
                    codex_switch: Default::default(),
                    edit_button,
                    delete_button,
                },
            );
        }
    }

    pub fn handle_action(
        &mut self,
        action: &McpsPageAction,
        ctx: &mut ViewContext<AutomationView>,
    ) {
        match action {
            McpsPageAction::OpenAdd => {
                self.editor = Some(self.new_editor(None, ctx));
            }
            McpsPageAction::OpenEdit(id) => {
                let entry = McpRegistryModel::as_ref(ctx).get(id).cloned();
                if let Some(entry) = entry {
                    self.editor = Some(self.new_editor(Some(entry), ctx));
                }
            }
            McpsPageAction::Delete(id) => {
                // Deleting the entry currently loaded in the editor would
                // leave the form saving a ghost; close it too.
                if self
                    .editor
                    .as_ref()
                    .is_some_and(|e| e.existing_id.as_deref() == Some(id))
                {
                    self.editor = None;
                }
                McpRegistryModel::handle(ctx).update(ctx, |m, mctx| m.delete(id, mctx));
                self.sync_rows(ctx);
            }
            McpsPageAction::ToggleClaude(id) => {
                McpRegistryModel::handle(ctx).update(ctx, |m, mctx| {
                    m.toggle_enabled(id, true, mctx);
                });
            }
            McpsPageAction::ToggleCodex(id) => {
                McpRegistryModel::handle(ctx).update(ctx, |m, mctx| {
                    m.toggle_enabled(id, false, mctx);
                });
            }
            McpsPageAction::SetTransport(value) => {
                if let (Some(editor), Some(transport)) =
                    (self.editor.as_mut(), McpTransport::from_str(value))
                {
                    editor.transport = transport;
                }
            }
            McpsPageAction::ToggleFormClaude => {
                if let Some(editor) = self.editor.as_mut() {
                    editor.enabled_claude = !editor.enabled_claude;
                }
            }
            McpsPageAction::ToggleFormCodex => {
                if let Some(editor) = self.editor.as_mut() {
                    editor.enabled_codex = !editor.enabled_codex;
                }
            }
            McpsPageAction::Save => self.save(ctx),
            McpsPageAction::Cancel => self.editor = None,
        }
        ctx.notify();
    }

    fn new_editor(
        &self,
        entry: Option<McpServerEntry>,
        ctx: &mut ViewContext<AutomationView>,
    ) -> McpEditor {
        let entry = entry.unwrap_or_else(|| McpServerEntry {
            enabled_claude: true,
            enabled_codex: true,
            ..Default::default()
        });
        let existing_id = (!entry.id.is_empty()).then(|| entry.id.clone());

        let name_editor = new_single_line_editor("e.g. github", &entry.name, ctx);
        let command_editor = new_single_line_editor(
            "e.g. npx",
            entry.command.as_deref().unwrap_or_default(),
            ctx,
        );
        let args_editor = new_single_line_editor(
            "Space-separated, or a JSON array",
            &entry.args.join(" "),
            ctx,
        );
        let url_editor = new_single_line_editor(
            "e.g. https://example.com/mcp",
            entry.url.as_deref().unwrap_or_default(),
            ctx,
        );
        let env_text = entry
            .env
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("\n");
        let env_editor = new_multiline_editor("KEY=value, one per line", &env_text, ctx);

        let transport = entry.transport;
        let transport_dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);
            dropdown.set_items(
                [McpTransport::Stdio, McpTransport::Http]
                    .into_iter()
                    .map(|t| {
                        DropdownItem::new(
                            t.label(),
                            AutomationViewAction::Mcps(McpsPageAction::SetTransport(
                                t.as_str().to_owned(),
                            )),
                        )
                    })
                    .collect(),
                ctx,
            );
            dropdown.set_selected_by_action(
                AutomationViewAction::Mcps(McpsPageAction::SetTransport(
                    transport.as_str().to_owned(),
                )),
                ctx,
            );
            dropdown.set_top_bar_max_width(220.);
            dropdown
        });

        let save_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Save", PrimaryTheme).on_click(|ctx| {
                ctx.dispatch_typed_action(AutomationViewAction::Mcps(McpsPageAction::Save));
            })
        });
        let cancel_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Cancel", SecondaryTheme).on_click(|ctx| {
                ctx.dispatch_typed_action(AutomationViewAction::Mcps(McpsPageAction::Cancel));
            })
        });

        McpEditor {
            existing_id,
            name_editor,
            command_editor,
            args_editor,
            url_editor,
            env_editor,
            transport_dropdown,
            transport,
            enabled_claude: entry.enabled_claude,
            enabled_codex: entry.enabled_codex,
            claude_switch: Default::default(),
            codex_switch: Default::default(),
            save_button,
            cancel_button,
            error: None,
        }
    }

    /// Validate the form and commit it to the registry.
    fn save(&mut self, ctx: &mut ViewContext<AutomationView>) {
        let Some(editor) = self.editor.as_mut() else {
            return;
        };
        let text =
            |handle: &ViewHandle<EditorView>| handle.as_ref(ctx).buffer_text(ctx).trim().to_owned();
        let name = text(&editor.name_editor);
        let command = text(&editor.command_editor);
        let args_text = text(&editor.args_editor);
        let url = text(&editor.url_editor);
        let env_text = editor.env_editor.as_ref(ctx).buffer_text(ctx);

        let error = if name.is_empty() {
            Some("Name is required.".to_owned())
        } else if McpRegistryModel::as_ref(ctx).name_taken(&name, editor.existing_id.as_deref()) {
            Some("A server with this name already exists.".to_owned())
        } else if editor.transport == McpTransport::Stdio && command.is_empty() {
            Some("Command is required for a stdio server.".to_owned())
        } else if editor.transport == McpTransport::Http && url.is_empty() {
            Some("URL is required for an HTTP server.".to_owned())
        } else {
            None
        };
        if let Some(error) = error {
            editor.error = Some(error);
            return;
        }

        let entry = McpServerEntry {
            id: editor
                .existing_id
                .clone()
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            name,
            transport: editor.transport,
            command: (editor.transport == McpTransport::Stdio && !command.is_empty())
                .then_some(command),
            args: (editor.transport == McpTransport::Stdio)
                .then(|| parse_args(&args_text))
                .unwrap_or_default(),
            url: (editor.transport == McpTransport::Http).then_some(url),
            env: env_text
                .lines()
                .filter_map(|line| {
                    let line = line.trim();
                    let (key, value) = line.split_once('=')?;
                    let key = key.trim();
                    (!key.is_empty()).then(|| (key.to_owned(), value.trim().to_owned()))
                })
                .collect(),
            enabled_claude: editor.enabled_claude,
            enabled_codex: editor.enabled_codex,
        };
        McpRegistryModel::handle(ctx).update(ctx, |m, mctx| m.upsert(entry, mctx));
        self.editor = None;
        self.sync_rows(ctx);
    }

    pub fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let registry = McpRegistryModel::as_ref(app);

        let mut column = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);

        // Header: title left, Add button right.
        column.add_child(
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(
                    Shrinkable::new(
                        1.,
                        Align::new(
                            Text::new_inline(
                                "MCPs",
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
                )
                .with_child(ChildView::new(&self.add_button).finish())
                .finish(),
        );
        column.add_child(
            Container::new(
                Text::new_inline(
                    "Servers here are shared by every new Claude and Codex session.",
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

        if let Some(editor) = &self.editor {
            column.add_child(self.render_editor(editor, app));
        }

        let servers = registry.servers();
        if servers.is_empty() && self.editor.is_none() {
            column.add_child(
                Container::new(
                    Text::new_inline(
                        "No MCP servers configured yet.",
                        appearance.ui_font_family(),
                        type_ramp::PROSE.size,
                    )
                    .with_line_height_ratio(type_ramp::PROSE.line_height)
                    .with_color(theme.sub_text_color(theme.background()).into())
                    .finish(),
                )
                .with_margin_bottom(spacing::LG)
                .finish(),
            );
        }
        for entry in servers {
            column.add_child(self.render_row(entry, app));
        }

        // Built-in servers, read-only. Names mirror the private SERVER_NAME
        // constants in `browser_mcp` / `computer_control::mcp`.
        column.add_child(
            Container::new(
                Text::new_inline(
                    "BUILT-IN",
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
        column.add_child(render_builtin_row(
            "twarp-browser",
            "Drives the in-app browser pane for the active session.",
            appearance,
        ));
        if FeatureFlag::LocalComputerUse.is_enabled() {
            column.add_child(render_builtin_row(
                "twarp-computer-control",
                "Injects mouse/keyboard events for UI debugging.",
                appearance,
            ));
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

    /// One registry row: name + transport summary left; C/X switches, Edit,
    /// Delete right.
    fn render_row(&self, entry: &McpServerEntry, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let Some(row_ui) = self.rows.get(&entry.id) else {
            // A row created outside this view's actions; drawn without
            // controls until the next sync.
            return render_builtin_row(&entry.name, &entry.summary(), appearance);
        };

        let labels = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(
                Text::new_inline(
                    entry.name.clone(),
                    appearance.ui_font_family(),
                    type_ramp::UI.size,
                )
                .with_line_height_ratio(type_ramp::UI.line_height)
                .with_color(theme.main_text_color(theme.background()).into())
                .finish(),
            )
            .with_child(
                Text::new_inline(
                    entry.summary(),
                    appearance.monospace_font_family(),
                    type_ramp::CAPTION.size,
                )
                .with_line_height_ratio(type_ramp::CAPTION.line_height)
                .with_color(theme.sub_text_color(theme.background()).into())
                .finish(),
            )
            .finish();

        let claude_id = entry.id.clone();
        let codex_id = entry.id.clone();
        let controls = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(spacing::SM)
            .with_child(render_labeled_switch(
                "Claude",
                entry.enabled_claude,
                row_ui.claude_switch.clone(),
                move |ctx| {
                    ctx.dispatch_typed_action(AutomationViewAction::Mcps(
                        McpsPageAction::ToggleClaude(claude_id.clone()),
                    ));
                },
                appearance,
            ))
            .with_child(render_labeled_switch(
                "Codex",
                entry.enabled_codex,
                row_ui.codex_switch.clone(),
                move |ctx| {
                    ctx.dispatch_typed_action(AutomationViewAction::Mcps(
                        McpsPageAction::ToggleCodex(codex_id.clone()),
                    ));
                },
                appearance,
            ))
            .with_child(ChildView::new(&row_ui.edit_button).finish())
            .with_child(ChildView::new(&row_ui.delete_button).finish())
            .finish();

        Container::new(
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(Shrinkable::new(1., labels).finish())
                .with_child(controls)
                .finish(),
        )
        .with_uniform_padding(spacing::MD)
        .with_margin_bottom(spacing::SM)
        .with_border(Border::all(1.).with_border_fill(theme.outline()))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius::CARD)))
        .finish()
    }

    /// The inline Add / Edit form.
    fn render_editor(&self, editor: &McpEditor, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let mut form = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
        form.add_child(render_form_label(
            if editor.existing_id.is_some() {
                "Edit MCP server"
            } else {
                "New MCP server"
            },
            type_ramp::UI.size,
            appearance,
        ));

        form.add_child(render_form_field("Name", &editor.name_editor, appearance));
        form.add_child(
            Container::new(
                Flex::column()
                    .with_cross_axis_alignment(CrossAxisAlignment::Start)
                    .with_child(render_form_label(
                        "Transport",
                        type_ramp::LABEL.size,
                        appearance,
                    ))
                    .with_child(ChildView::new(&editor.transport_dropdown).finish())
                    .finish(),
            )
            .with_margin_bottom(spacing::SM)
            .finish(),
        );

        match editor.transport {
            McpTransport::Stdio => {
                form.add_child(render_form_field(
                    "Command",
                    &editor.command_editor,
                    appearance,
                ));
                form.add_child(render_form_field(
                    "Arguments",
                    &editor.args_editor,
                    appearance,
                ));
            }
            McpTransport::Http => {
                form.add_child(render_form_field("URL", &editor.url_editor, appearance));
            }
        }
        form.add_child(render_form_field(
            "Environment variables",
            &editor.env_editor,
            appearance,
        ));

        form.add_child(
            Container::new(
                Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_spacing(spacing::MD)
                    .with_child(render_labeled_switch(
                        "Claude",
                        editor.enabled_claude,
                        editor.claude_switch.clone(),
                        |ctx| {
                            ctx.dispatch_typed_action(AutomationViewAction::Mcps(
                                McpsPageAction::ToggleFormClaude,
                            ));
                        },
                        appearance,
                    ))
                    .with_child(render_labeled_switch(
                        "Codex",
                        editor.enabled_codex,
                        editor.codex_switch.clone(),
                        |ctx| {
                            ctx.dispatch_typed_action(AutomationViewAction::Mcps(
                                McpsPageAction::ToggleFormCodex,
                            ));
                        },
                        appearance,
                    ))
                    .finish(),
            )
            .with_margin_top(spacing::XS)
            .with_margin_bottom(spacing::SM)
            .finish(),
        );

        if let Some(error) = &editor.error {
            form.add_child(
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

        form.add_child(
            Flex::row()
                .with_spacing(spacing::SM)
                .with_child(ChildView::new(&editor.save_button).finish())
                .with_child(ChildView::new(&editor.cancel_button).finish())
                .finish(),
        );

        Container::new(form.finish())
            .with_uniform_padding(spacing::LG)
            .with_margin_bottom(spacing::LG)
            .with_border(Border::all(1.).with_border_fill(theme.outline()))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius::CARD)))
            .finish()
    }
}

fn parse_args(text: &str) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }
    if let Ok(args) = serde_json::from_str::<Vec<String>>(text) {
        return args;
    }
    text.split_whitespace().map(str::to_owned).collect()
}

fn new_single_line_editor(
    placeholder: &str,
    initial: &str,
    ctx: &mut ViewContext<AutomationView>,
) -> ViewHandle<EditorView> {
    let placeholder = placeholder.to_owned();
    let initial = initial.to_owned();
    ctx.add_typed_action_view(move |ctx| {
        let options = SingleLineEditorOptions {
            text: TextOptions::ui_font_size(Appearance::as_ref(ctx)),
            propagate_and_no_op_vertical_navigation_keys: PropagateAndNoOpNavigationKeys::Always,
            ..Default::default()
        };
        let mut editor = EditorView::single_line(options, ctx);
        editor.set_placeholder_text(placeholder, ctx);
        if !initial.is_empty() {
            editor.set_buffer_text_ignoring_undo(&initial, ctx);
        }
        editor
    })
}

fn new_multiline_editor(
    placeholder: &str,
    initial: &str,
    ctx: &mut ViewContext<AutomationView>,
) -> ViewHandle<EditorView> {
    let placeholder = placeholder.to_owned();
    let initial = initial.to_owned();
    ctx.add_typed_action_view(move |ctx| {
        let options = EditorOptions {
            autogrow: true,
            soft_wrap: true,
            text: TextOptions::ui_font_size(Appearance::as_ref(ctx)),
            ..Default::default()
        };
        let mut editor = EditorView::new(options, ctx);
        editor.set_placeholder_text(placeholder, ctx);
        if !initial.is_empty() {
            editor.set_buffer_text_ignoring_undo(&initial, ctx);
        }
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

/// A read-only row for a built-in server (no switches, no Edit / Delete).
fn render_builtin_row(name: &str, summary: &str, appearance: &Appearance) -> Box<dyn Element> {
    let theme = appearance.theme();
    let labels = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Start)
        .with_child(
            Text::new_inline(
                name.to_owned(),
                appearance.ui_font_family(),
                type_ramp::UI.size,
            )
            .with_line_height_ratio(type_ramp::UI.line_height)
            .with_color(theme.main_text_color(theme.background()).into())
            .finish(),
        )
        .with_child(
            Text::new_inline(
                summary.to_owned(),
                appearance.ui_font_family(),
                type_ramp::CAPTION.size,
            )
            .with_line_height_ratio(type_ramp::CAPTION.line_height)
            .with_color(theme.sub_text_color(theme.background()).into())
            .finish(),
        )
        .finish();

    let chip = Container::new(
        Text::new_inline(
            "Built-in",
            appearance.ui_font_family(),
            type_ramp::CAPTION.size,
        )
        .with_line_height_ratio(type_ramp::CAPTION.line_height)
        .with_color(theme.sub_text_color(theme.surface_1()).into())
        .finish(),
    )
    .with_background(theme.surface_1())
    .with_padding_left(spacing::SM)
    .with_padding_right(spacing::SM)
    .with_padding_top(spacing::XXS)
    .with_padding_bottom(spacing::XXS)
    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius::CHIP)))
    .finish();

    Container::new(
        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(Shrinkable::new(1., labels).finish())
            .with_child(chip)
            .finish(),
    )
    .with_uniform_padding(spacing::MD)
    .with_margin_bottom(spacing::SM)
    .with_border(Border::all(1.).with_border_fill(theme.outline()))
    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius::CARD)))
    .finish()
}
