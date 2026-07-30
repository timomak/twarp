//! twarp 23b: the Plugins automation page — a management UI over the plugin
//! registry ([`crate::plugin_registry`]), replacing the former Skills and
//! MCPs pages. A plugin is a named bundle of MCP servers and/or skills with
//! plugin-level and component-level per-provider toggles.
//!
//! Layout: a centered chrome-class column with a "Quick add" preset gallery,
//! the plugins listed as cards (name, description, component summary,
//! Claude/Codex switches, Edit / Delete with a two-click confirm), an inline
//! expanding multi-component editor (no modals), the built-in plugins as
//! read-only cards, and an Import section offering to adopt real
//! `~/.claude/skills` directories (each landing as a single-skill plugin).

use std::collections::HashMap;

use twarp_core::features::FeatureFlag;
use twarp_core::ui::external_product_icon::ExternalProductIcon;
use twarp_core::ui::tokens::{radius, spacing, type_ramp};
use twarp_core::ui::Icon;
use twarpui::{
    elements::{
        new_scrollable::{NewScrollable, ScrollableAppearance, SingleAxisConfig},
        Align, Border, ChildView, ClippedScrollStateHandle, ConstrainedBox, Container,
        CornerRadius, CrossAxisAlignment, Element, Expanded, Flex, Hoverable, MainAxisSize,
        MouseStateHandle, ParentElement, Radius, ScrollbarWidth, Shrinkable, Text,
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
use crate::mcp_oauth::{McpAuthStatus, McpOauthModel};
use crate::mcp_registry::{
    is_bearer_authorization, McpAuth, McpRegistryModel, McpServerEntry, McpTransport,
};
use crate::plugin_registry::{PluginEntry, PluginRegistryModel};
use crate::skills_store::{is_valid_skill_name, AdoptableSkill, SkillsStoreModel};
use crate::view_components::{
    action_button::{ActionButton, DangerSecondaryTheme, PrimaryTheme, SecondaryTheme},
    dropdown::DropdownItem,
    Dropdown,
};

use super::view::{AutomationView, AutomationViewAction};

/// Content column width; matches the conversation measure used app-wide.
const CONTENT_MAX_WIDTH: f32 = 720.;

/// A quick-add template: one click opens the Add form prefilled as a
/// single-server plugin with the service's known transport/command/URL and
/// the env keys it needs, leaving only credentials for the user to paste in.
/// The record shape allows presets to bundle skills later; none do yet.
struct PluginPreset {
    /// Stable key used in actions and as the base for generated names.
    key: &'static str,
    label: &'static str,
    /// One-line gallery-card description.
    description: &'static str,
    transport: McpTransport,
    command: Option<&'static str>,
    args: &'static [&'static str],
    url: Option<&'static str>,
    /// Env keys prefilled as `KEY=` lines for the user to complete.
    env_keys: &'static [&'static str],
}

impl PluginPreset {
    fn entry(&self, name: String) -> McpServerEntry {
        McpServerEntry {
            name,
            transport: self.transport,
            command: self.command.map(str::to_owned),
            args: self.args.iter().map(|a| (*a).to_owned()).collect(),
            url: self.url.map(str::to_owned),
            env: self
                .env_keys
                .iter()
                .map(|k| ((*k).to_owned(), String::new()))
                .collect(),
            enabled_claude: true,
            enabled_codex: true,
            ..Default::default()
        }
    }
}

const PRESETS: &[PluginPreset] = &[
    PluginPreset {
        key: "slack",
        label: "Slack",
        description: "Read and post to Slack workspaces.",
        transport: McpTransport::Stdio,
        command: Some("npx"),
        args: &["-y", "@modelcontextprotocol/server-slack"],
        url: None,
        env_keys: &["SLACK_BOT_TOKEN", "SLACK_TEAM_ID"],
    },
    PluginPreset {
        key: "composio",
        label: "Composio",
        description: "Bridge to hundreds of Composio tool integrations.",
        transport: McpTransport::Http,
        command: None,
        args: &[],
        url: Some("https://mcp.composio.dev/YOUR_SERVER_ID"),
        env_keys: &[],
    },
    PluginPreset {
        key: "notion",
        label: "Notion",
        description: "Search and edit Notion pages and databases.",
        transport: McpTransport::Http,
        command: None,
        args: &[],
        url: Some("https://mcp.notion.com/mcp"),
        env_keys: &[],
    },
    PluginPreset {
        key: "linear",
        label: "Linear",
        description: "Manage Linear issues, projects, and cycles.",
        transport: McpTransport::Http,
        command: None,
        args: &[],
        url: Some("https://mcp.linear.app/mcp"),
        env_keys: &[],
    },
    PluginPreset {
        key: "github",
        label: "GitHub",
        description: "Work with GitHub repos, issues, and pull requests.",
        transport: McpTransport::Http,
        command: None,
        args: &[],
        url: Some("https://api.githubcopilot.com/mcp/"),
        env_keys: &[],
    },
    PluginPreset {
        key: "cloudflare",
        label: "Cloudflare",
        description: "Inspect and manage Cloudflare Workers bindings.",
        transport: McpTransport::Http,
        command: None,
        args: &[],
        url: Some("https://bindings.mcp.cloudflare.com/mcp"),
        env_keys: &[],
    },
    PluginPreset {
        key: "gmail",
        label: "Gmail",
        description: "Read, search, and draft Gmail messages.",
        transport: McpTransport::Stdio,
        command: Some("npx"),
        args: &["-y", "@gongrzhe/server-gmail-autoauth-mcp"],
        url: None,
        env_keys: &[],
    },
];

/// Actions dispatched by the Plugins page's controls, wrapped in
/// [`AutomationViewAction::Plugins`]. Per-sub-form actions are keyed by a
/// stable per-form UUID (not an index) so Remove doesn't invalidate the
/// callbacks baked into sibling forms' dropdowns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginsPageAction {
    /// Open the inline editor with a blank form.
    OpenAdd,
    /// Open the inline editor prefilled from the named quick-add preset.
    OpenPreset(String),
    /// Open the inline editor prefilled from the given plugin.
    OpenEdit(String),
    /// First click of the two-click delete affordance.
    RequestDelete(String),
    /// Second click: actually delete the plugin and its components.
    ConfirmDelete(String),
    ToggleClaude(String),
    ToggleCodex(String),
    /// Adopt a real `~/.claude/skills/<name>` dir into the store as a
    /// single-skill plugin.
    Adopt(String),
    // Inline-editor controls:
    AddServer,
    RemoveServer(String),
    /// Transport selected in a server sub-form ("stdio" | "http").
    SetServerTransport(String, String),
    ToggleServerClaude(String),
    ToggleServerCodex(String),
    /// twarp 24c: show/hide a server sub-form's Advanced section (transport,
    /// stdio fields, env, headers).
    ToggleServerAdvanced(String),
    /// twarp 24c: save the plugin, then probe/authorize this server.
    ConnectServer(String),
    /// twarp 24c: drop this server's stored OAuth tokens.
    DisconnectServer(String),
    /// twarp 24c: abandon a consent flow that is waiting on the browser.
    CancelServerConnect(String),
    AddSkill,
    RemoveSkill(String),
    ToggleSkillClaude(String),
    ToggleSkillCodex(String),
    ToggleFormClaude,
    ToggleFormCodex,
    Save,
    Cancel,
}

/// Persistent per-card UI handles so hover/switch state survives re-renders.
struct RowUi {
    claude_switch: SwitchStateHandle,
    codex_switch: SwitchStateHandle,
    edit_button: ViewHandle<ActionButton>,
    delete_button: ViewHandle<ActionButton>,
    confirm_delete_button: ViewHandle<ActionButton>,
}

/// One MCP-server sub-form in the inline editor.
struct ServerForm {
    /// Stable action key for this form instance.
    key: String,
    /// `Some` when editing an existing registry entry.
    existing_id: Option<String>,
    name_editor: ViewHandle<EditorView>,
    command_editor: ViewHandle<EditorView>,
    args_editor: ViewHandle<EditorView>,
    url_editor: ViewHandle<EditorView>,
    env_editor: ViewHandle<EditorView>,
    /// twarp 24a: `Header-Name: value`, one per line.
    headers_editor: ViewHandle<EditorView>,
    transport_dropdown: ViewHandle<Dropdown<AutomationViewAction>>,
    transport: McpTransport,
    /// twarp 24a: preserved across edits — the OAuth grant belongs to the
    /// server, not to whatever the form last rendered.
    auth: McpAuth,
    /// twarp 24c: whether the Advanced section is expanded. Collapsed by
    /// default so a hosted server is just Name + URL + Connect.
    advanced_open: bool,
    /// Separate handles because an [`ActionButton`]'s label is fixed at
    /// construction and only one of each pair is ever mounted.
    show_advanced_button: ViewHandle<ActionButton>,
    hide_advanced_button: ViewHandle<ActionButton>,
    connect_button: ViewHandle<ActionButton>,
    disconnect_button: ViewHandle<ActionButton>,
    cancel_connect_button: ViewHandle<ActionButton>,
    enabled_claude: bool,
    enabled_codex: bool,
    claude_switch: SwitchStateHandle,
    codex_switch: SwitchStateHandle,
    remove_button: ViewHandle<ActionButton>,
}

/// One skill sub-form in the inline editor: either an existing store skill
/// (name fixed) or a new inline-created one (name + description editable).
struct SkillForm {
    /// Stable action key for this form instance.
    key: String,
    /// `Some(name)` when this row is an existing store skill.
    existing_name: Option<String>,
    name_editor: ViewHandle<EditorView>,
    description_editor: ViewHandle<EditorView>,
    enabled_claude: bool,
    enabled_codex: bool,
    claude_switch: SwitchStateHandle,
    codex_switch: SwitchStateHandle,
    remove_button: ViewHandle<ActionButton>,
}

/// The inline Add / Edit form.
struct PluginEditor {
    /// `Some` when editing an existing plugin, `None` when adding.
    existing: Option<PluginEntry>,
    name_editor: ViewHandle<EditorView>,
    description_editor: ViewHandle<EditorView>,
    servers: Vec<ServerForm>,
    skills: Vec<SkillForm>,
    enabled_claude: bool,
    enabled_codex: bool,
    claude_switch: SwitchStateHandle,
    codex_switch: SwitchStateHandle,
    add_server_button: ViewHandle<ActionButton>,
    add_skill_button: ViewHandle<ActionButton>,
    save_button: ViewHandle<ActionButton>,
    cancel_button: ViewHandle<ActionButton>,
    /// Validation error surfaced above the Save row.
    error: Option<String>,
}

/// State backing the Plugins page, owned by [`AutomationView`] when its page
/// is [`super::AutomationPage::Plugins`].
pub struct PluginsPageState {
    scroll_state: ClippedScrollStateHandle,
    add_button: ViewHandle<ActionButton>,
    /// Dedicated CTA for the empty state — a view handle can't be mounted
    /// both in the header and in the empty state at once.
    empty_add_button: ViewHandle<ActionButton>,
    /// One button per [`PRESETS`] entry, in the same order.
    /// One persistent hover state per quick-add gallery card, keyed by
    /// [`PRESETS`] key.
    preset_hover: HashMap<&'static str, MouseStateHandle>,
    rows: HashMap<String, RowUi>,
    adopt_buttons: HashMap<String, ViewHandle<ActionButton>>,
    editor: Option<PluginEditor>,
    /// Plugin id whose Delete button is currently in its Confirm stage.
    pending_delete: Option<String>,
}

impl PluginsPageState {
    pub fn new(ctx: &mut ViewContext<AutomationView>) -> Self {
        let add_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Add plugin", PrimaryTheme).on_click(|ctx| {
                ctx.dispatch_typed_action(AutomationViewAction::Plugins(
                    PluginsPageAction::OpenAdd,
                ));
            })
        });
        let empty_add_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Add plugin", PrimaryTheme).on_click(|ctx| {
                ctx.dispatch_typed_action(AutomationViewAction::Plugins(
                    PluginsPageAction::OpenAdd,
                ));
            })
        });
        let preset_hover = PRESETS
            .iter()
            .map(|preset| (preset.key, MouseStateHandle::default()))
            .collect();
        let mut state = Self {
            scroll_state: Default::default(),
            add_button,
            empty_add_button,
            preset_hover,
            rows: HashMap::new(),
            adopt_buttons: HashMap::new(),
            editor: None,
            pending_delete: None,
        };
        state.sync_rows(ctx);
        state
    }

    /// Keep one [`RowUi`] / Adopt button alive per plugin / adoptable skill
    /// (created on demand, dropped when the entry goes away). Called from
    /// actions and whenever [`SkillsStoreModel`] notifies.
    pub fn sync_rows(&mut self, ctx: &mut ViewContext<AutomationView>) {
        let ids: Vec<String> = PluginRegistryModel::as_ref(ctx)
            .plugins()
            .iter()
            .map(|p| p.id.clone())
            .collect();
        let adoptable: Vec<String> = SkillsStoreModel::as_ref(ctx)
            .adoptable()
            .iter()
            .map(|s| s.name.clone())
            .collect();

        self.rows.retain(|id, _| ids.iter().any(|i| i == id));
        if self
            .pending_delete
            .as_ref()
            .is_some_and(|id| !ids.iter().any(|i| i == id))
        {
            self.pending_delete = None;
        }
        for id in ids {
            if self.rows.contains_key(&id) {
                continue;
            }
            let edit_id = id.clone();
            let edit_button = ctx.add_typed_action_view(|_| {
                ActionButton::new("Edit", SecondaryTheme).on_click(move |ctx| {
                    ctx.dispatch_typed_action(AutomationViewAction::Plugins(
                        PluginsPageAction::OpenEdit(edit_id.clone()),
                    ));
                })
            });
            let delete_id = id.clone();
            let delete_button = ctx.add_typed_action_view(|_| {
                ActionButton::new("Delete", DangerSecondaryTheme).on_click(move |ctx| {
                    ctx.dispatch_typed_action(AutomationViewAction::Plugins(
                        PluginsPageAction::RequestDelete(delete_id.clone()),
                    ));
                })
            });
            let confirm_id = id.clone();
            let confirm_delete_button = ctx.add_typed_action_view(|_| {
                ActionButton::new("Confirm delete", DangerSecondaryTheme).on_click(move |ctx| {
                    ctx.dispatch_typed_action(AutomationViewAction::Plugins(
                        PluginsPageAction::ConfirmDelete(confirm_id.clone()),
                    ));
                })
            });
            self.rows.insert(
                id,
                RowUi {
                    claude_switch: Default::default(),
                    codex_switch: Default::default(),
                    edit_button,
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
                    ctx.dispatch_typed_action(AutomationViewAction::Plugins(
                        PluginsPageAction::Adopt(adopt_name.clone()),
                    ));
                })
            });
            self.adopt_buttons.insert(name, button);
        }
    }

    pub fn handle_action(
        &mut self,
        action: &PluginsPageAction,
        ctx: &mut ViewContext<AutomationView>,
    ) {
        match action {
            PluginsPageAction::OpenAdd => {
                self.editor = Some(self.new_editor(None, None, ctx));
            }
            PluginsPageAction::OpenPreset(key) => {
                if let Some(preset) = PRESETS.iter().find(|p| p.key == key) {
                    let name = PluginRegistryModel::as_ref(ctx).unique_name(preset.key);
                    let server = preset.entry(name.clone());
                    let mut editor = self.new_editor(None, Some(preset.description), ctx);
                    set_editor_text(&editor.name_editor, &name, ctx);
                    editor.servers = vec![self.new_server_form(Some(server), ctx)];
                    self.editor = Some(editor);
                }
            }
            PluginsPageAction::OpenEdit(id) => {
                let entry = PluginRegistryModel::as_ref(ctx).get(id).cloned();
                if let Some(entry) = entry {
                    self.editor = Some(self.new_editor(Some(entry), None, ctx));
                }
            }
            PluginsPageAction::RequestDelete(id) => {
                self.pending_delete = Some(id.clone());
            }
            PluginsPageAction::ConfirmDelete(id) => {
                self.pending_delete = None;
                // Deleting the plugin currently loaded in the editor would
                // leave the form saving a ghost; close it too.
                if self
                    .editor
                    .as_ref()
                    .is_some_and(|e| e.existing.as_ref().map(|p| p.id.as_str()) == Some(id))
                {
                    self.editor = None;
                }
                self.delete_plugin(id, ctx);
            }
            PluginsPageAction::ToggleClaude(id) => {
                PluginRegistryModel::handle(ctx).update(ctx, |m, mctx| {
                    m.toggle_enabled(id, true, mctx);
                });
                // Re-materialize skills under the new effective enablement.
                SkillsStoreModel::handle(ctx).update(ctx, |m, mctx| m.request_refresh(mctx));
            }
            PluginsPageAction::ToggleCodex(id) => {
                PluginRegistryModel::handle(ctx).update(ctx, |m, mctx| {
                    m.toggle_enabled(id, false, mctx);
                });
                SkillsStoreModel::handle(ctx).update(ctx, |m, mctx| m.request_refresh(mctx));
            }
            PluginsPageAction::Adopt(name) => self.adopt_skill(name, ctx),
            PluginsPageAction::AddServer => {
                let form = self.new_server_form(None, ctx);
                if let Some(editor) = self.editor.as_mut() {
                    editor.servers.push(form);
                }
            }
            PluginsPageAction::RemoveServer(key) => {
                if let Some(editor) = self.editor.as_mut() {
                    editor.servers.retain(|form| &form.key != key);
                }
            }
            PluginsPageAction::SetServerTransport(key, value) => {
                if let (Some(editor), Some(transport)) =
                    (self.editor.as_mut(), McpTransport::from_str(value))
                {
                    if let Some(form) = editor.servers.iter_mut().find(|f| &f.key == key) {
                        form.transport = transport;
                        // Command/Arguments live behind Advanced, so switching
                        // to stdio has to reveal them or Save would fail on
                        // fields the user can't see (twarp 24c).
                        if transport == McpTransport::Stdio {
                            form.advanced_open = true;
                        }
                    }
                }
            }
            PluginsPageAction::ToggleServerClaude(key) => {
                if let Some(form) = self.server_form_mut(key) {
                    form.enabled_claude = !form.enabled_claude;
                }
            }
            PluginsPageAction::ToggleServerCodex(key) => {
                if let Some(form) = self.server_form_mut(key) {
                    form.enabled_codex = !form.enabled_codex;
                }
            }
            PluginsPageAction::ToggleServerAdvanced(key) => {
                if let Some(form) = self.server_form_mut(key) {
                    form.advanced_open = !form.advanced_open;
                }
            }
            PluginsPageAction::ConnectServer(key) => {
                // Connecting needs a persisted id for the OAuth callback to
                // route back to, so save first (P5) — a validation failure
                // leaves the editor open with its error and connects nothing.
                self.save(Some(key.clone()), ctx);
            }
            PluginsPageAction::DisconnectServer(key) => {
                if let Some(id) = self.server_form_id(key) {
                    McpOauthModel::handle(ctx).update(ctx, |m, mctx| m.disconnect(&id, mctx));
                }
            }
            PluginsPageAction::CancelServerConnect(key) => {
                if let Some(id) = self.server_form_id(key) {
                    McpOauthModel::handle(ctx).update(ctx, |m, mctx| m.cancel(&id, mctx));
                }
            }
            PluginsPageAction::AddSkill => {
                let form = self.new_skill_form(None, ctx);
                if let Some(editor) = self.editor.as_mut() {
                    editor.skills.push(form);
                }
            }
            PluginsPageAction::RemoveSkill(key) => {
                if let Some(editor) = self.editor.as_mut() {
                    editor.skills.retain(|form| &form.key != key);
                }
            }
            PluginsPageAction::ToggleSkillClaude(key) => {
                if let Some(form) = self.skill_form_mut(key) {
                    form.enabled_claude = !form.enabled_claude;
                }
            }
            PluginsPageAction::ToggleSkillCodex(key) => {
                if let Some(form) = self.skill_form_mut(key) {
                    form.enabled_codex = !form.enabled_codex;
                }
            }
            PluginsPageAction::ToggleFormClaude => {
                if let Some(editor) = self.editor.as_mut() {
                    editor.enabled_claude = !editor.enabled_claude;
                }
            }
            PluginsPageAction::ToggleFormCodex => {
                if let Some(editor) = self.editor.as_mut() {
                    editor.enabled_codex = !editor.enabled_codex;
                }
            }
            PluginsPageAction::Save => self.save(None, ctx),
            PluginsPageAction::Cancel => self.editor = None,
        }
        self.sync_rows(ctx);
        ctx.notify();
    }

    fn server_form_mut(&mut self, key: &str) -> Option<&mut ServerForm> {
        self.editor
            .as_mut()
            .and_then(|e| e.servers.iter_mut().find(|f| f.key == key))
    }

    fn skill_form_mut(&mut self, key: &str) -> Option<&mut SkillForm> {
        self.editor
            .as_mut()
            .and_then(|e| e.skills.iter_mut().find(|f| f.key == key))
    }

    /// Delete a plugin and its components: member servers leave the registry,
    /// member skills leave the store (and their materialized artifacts).
    fn delete_plugin(&mut self, id: &str, ctx: &mut ViewContext<AutomationView>) {
        let Some(entry) = PluginRegistryModel::as_ref(ctx).get(id).cloned() else {
            return;
        };
        for server_id in &entry.server_ids {
            McpRegistryModel::handle(ctx).update(ctx, |m, mctx| m.delete(server_id, mctx));
            // twarp 24b: a deleted server's keychain material goes with it.
            McpOauthModel::handle(ctx).update(ctx, |m, mctx| m.forget(server_id, mctx));
        }
        for skill_name in &entry.skill_names {
            SkillsStoreModel::handle(ctx).update(ctx, |m, mctx| m.delete(skill_name, mctx));
        }
        PluginRegistryModel::handle(ctx).update(ctx, |m, mctx| m.delete(id, mctx));
    }

    /// Adopt a real `~/.claude/skills` dir into the store; it lands as a
    /// single-skill plugin named after the skill.
    fn adopt_skill(&mut self, name: &str, ctx: &mut ViewContext<AutomationView>) {
        let plugin_id = uuid::Uuid::new_v4().to_string();
        let plugin_name = PluginRegistryModel::as_ref(ctx).unique_name(name);
        SkillsStoreModel::handle(ctx).update(ctx, |m, mctx| {
            m.adopt(name, mctx);
            m.set_component(name, true, true, Some(plugin_id.clone()), mctx);
        });
        let entry = PluginEntry {
            id: plugin_id,
            name: plugin_name,
            description: String::new(),
            enabled_claude: true,
            enabled_codex: true,
            server_ids: Vec::new(),
            skill_names: vec![name.to_owned()],
        };
        PluginRegistryModel::handle(ctx).update(ctx, |m, mctx| m.upsert(entry, mctx));
    }

    fn new_editor(
        &self,
        entry: Option<PluginEntry>,
        description: Option<&str>,
        ctx: &mut ViewContext<AutomationView>,
    ) -> PluginEditor {
        let name = entry.as_ref().map(|e| e.name.clone()).unwrap_or_default();
        let description = entry
            .as_ref()
            .map(|e| e.description.clone())
            .unwrap_or_else(|| description.unwrap_or_default().to_owned());
        let enabled_claude = entry.as_ref().map(|e| e.enabled_claude).unwrap_or(true);
        let enabled_codex = entry.as_ref().map(|e| e.enabled_codex).unwrap_or(true);

        let name_editor = new_single_line_editor("e.g. github", &name, ctx);
        let description_editor =
            new_single_line_editor("What this plugin is for", &description, ctx);

        let servers = entry
            .as_ref()
            .map(|e| {
                e.server_ids
                    .iter()
                    .filter_map(|id| McpRegistryModel::as_ref(ctx).get(id).cloned())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
            .into_iter()
            .map(|server| self.new_server_form(Some(server), ctx))
            .collect();
        let skills = entry
            .as_ref()
            .map(|e| e.skill_names.clone())
            .unwrap_or_default()
            .into_iter()
            .map(|skill_name| self.new_skill_form(Some(skill_name), ctx))
            .collect();

        let add_server_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Add server", SecondaryTheme).on_click(|ctx| {
                ctx.dispatch_typed_action(AutomationViewAction::Plugins(
                    PluginsPageAction::AddServer,
                ));
            })
        });
        let add_skill_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Add skill", SecondaryTheme).on_click(|ctx| {
                ctx.dispatch_typed_action(AutomationViewAction::Plugins(
                    PluginsPageAction::AddSkill,
                ));
            })
        });
        let save_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Save", PrimaryTheme).on_click(|ctx| {
                ctx.dispatch_typed_action(AutomationViewAction::Plugins(PluginsPageAction::Save));
            })
        });
        let cancel_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Cancel", SecondaryTheme).on_click(|ctx| {
                ctx.dispatch_typed_action(AutomationViewAction::Plugins(PluginsPageAction::Cancel));
            })
        });

        PluginEditor {
            existing: entry,
            name_editor,
            description_editor,
            servers,
            skills,
            enabled_claude,
            enabled_codex,
            claude_switch: Default::default(),
            codex_switch: Default::default(),
            add_server_button,
            add_skill_button,
            save_button,
            cancel_button,
            error: None,
        }
    }

    fn new_server_form(
        &self,
        entry: Option<McpServerEntry>,
        ctx: &mut ViewContext<AutomationView>,
    ) -> ServerForm {
        let entry = entry.unwrap_or_else(|| McpServerEntry {
            enabled_claude: true,
            enabled_codex: true,
            ..Default::default()
        });
        let key = uuid::Uuid::new_v4().to_string();
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
        let headers_text = entry
            .headers
            .iter()
            .map(|(k, v)| format!("{k}: {v}"))
            .collect::<Vec<_>>()
            .join("\n");
        let headers_editor =
            new_multiline_editor("Header-Name: value, one per line", &headers_text, ctx);

        let transport = entry.transport;
        let dropdown_key = key.clone();
        let transport_dropdown = ctx.add_typed_action_view(move |ctx| {
            let mut dropdown = Dropdown::new(ctx);
            dropdown.set_items(
                [McpTransport::Stdio, McpTransport::Http]
                    .into_iter()
                    .map(|t| {
                        DropdownItem::new(
                            t.label(),
                            AutomationViewAction::Plugins(PluginsPageAction::SetServerTransport(
                                dropdown_key.clone(),
                                t.as_str().to_owned(),
                            )),
                        )
                    })
                    .collect(),
                ctx,
            );
            dropdown.set_selected_by_action(
                AutomationViewAction::Plugins(PluginsPageAction::SetServerTransport(
                    dropdown_key.clone(),
                    transport.as_str().to_owned(),
                )),
                ctx,
            );
            dropdown.set_top_bar_max_width(220.);
            dropdown
        });

        let remove_key = key.clone();
        let remove_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Remove", DangerSecondaryTheme).on_click(move |ctx| {
                ctx.dispatch_typed_action(AutomationViewAction::Plugins(
                    PluginsPageAction::RemoveServer(remove_key.clone()),
                ));
            })
        });

        let show_advanced_button = server_action_button(
            "Advanced",
            SecondaryTheme,
            &key,
            PluginsPageAction::ToggleServerAdvanced,
            ctx,
        );
        let hide_advanced_button = server_action_button(
            "Hide advanced",
            SecondaryTheme,
            &key,
            PluginsPageAction::ToggleServerAdvanced,
            ctx,
        );
        let connect_button = server_action_button(
            "Connect",
            PrimaryTheme,
            &key,
            PluginsPageAction::ConnectServer,
            ctx,
        );
        let disconnect_button = server_action_button(
            "Disconnect",
            SecondaryTheme,
            &key,
            PluginsPageAction::DisconnectServer,
            ctx,
        );
        let cancel_connect_button = server_action_button(
            "Cancel",
            SecondaryTheme,
            &key,
            PluginsPageAction::CancelServerConnect,
            ctx,
        );

        ServerForm {
            key,
            existing_id,
            name_editor,
            command_editor,
            args_editor,
            url_editor,
            env_editor,
            headers_editor,
            transport_dropdown,
            transport,
            auth: entry.auth,
            // A stdio server has nothing to connect to, so its fields are the
            // point — open Advanced for it and leave remote servers minimal.
            advanced_open: transport == McpTransport::Stdio,
            show_advanced_button,
            hide_advanced_button,
            connect_button,
            disconnect_button,
            cancel_connect_button,
            enabled_claude: entry.enabled_claude,
            enabled_codex: entry.enabled_codex,
            claude_switch: Default::default(),
            codex_switch: Default::default(),
            remove_button,
        }
    }

    /// The registry id behind a server sub-form, once it has been saved.
    fn server_form_id(&self, key: &str) -> Option<String> {
        self.editor
            .as_ref()?
            .servers
            .iter()
            .find(|form| form.key == key)?
            .existing_id
            .clone()
    }

    fn new_skill_form(
        &self,
        existing_name: Option<String>,
        ctx: &mut ViewContext<AutomationView>,
    ) -> SkillForm {
        let key = uuid::Uuid::new_v4().to_string();
        let (enabled_claude, enabled_codex) = existing_name
            .as_deref()
            .and_then(|name| {
                SkillsStoreModel::as_ref(ctx)
                    .skills()
                    .iter()
                    .find(|s| s.name == name)
                    .map(|s| (s.enabled_claude, s.enabled_codex))
            })
            .unwrap_or((true, true));
        let name_editor = new_single_line_editor("e.g. deploy-checklist", "", ctx);
        let description_editor = new_single_line_editor("What this skill is for", "", ctx);
        let remove_key = key.clone();
        let remove_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Remove", DangerSecondaryTheme).on_click(move |ctx| {
                ctx.dispatch_typed_action(AutomationViewAction::Plugins(
                    PluginsPageAction::RemoveSkill(remove_key.clone()),
                ));
            })
        });
        SkillForm {
            key,
            existing_name,
            name_editor,
            description_editor,
            enabled_claude,
            enabled_codex,
            claude_switch: Default::default(),
            codex_switch: Default::default(),
            remove_button,
        }
    }

    /// Validate the form and commit it: upsert the plugin, upsert / create
    /// its components with membership, and spin removed components out into
    /// plugins of their own (they stay usable rather than silently orphaned).
    /// Commit the editor. `connect_form_key`, when set, is the server
    /// sub-form whose Connect button triggered the save: after the commit its
    /// saved entry is handed to [`McpOauthModel::connect`] (twarp 24c).
    fn save(
        &mut self,
        connect_form_key: Option<String>,
        ctx: &mut ViewContext<AutomationView>,
    ) {
        let Some(editor) = self.editor.as_mut() else {
            return;
        };
        let text =
            |handle: &ViewHandle<EditorView>| handle.as_ref(ctx).buffer_text(ctx).trim().to_owned();
        let name = text(&editor.name_editor);
        let description = text(&editor.description_editor);
        let existing_id = editor.existing.as_ref().map(|e| e.id.clone());

        // Plugin-level validation.
        let mut error = if name.is_empty() {
            Some("Name is required.".to_owned())
        } else if PluginRegistryModel::as_ref(ctx).name_taken(&name, existing_id.as_deref()) {
            Some("A plugin with this name already exists.".to_owned())
        } else if editor.servers.is_empty() && editor.skills.is_empty() {
            Some("A plugin needs at least one server or skill.".to_owned())
        } else {
            None
        };

        // Server sub-form validation.
        let mut server_entries = Vec::new();
        // twarp 24c: sub-form key -> saved registry id, so Connect can find
        // the entry it just committed.
        let mut form_ids: Vec<(String, String)> = Vec::new();
        if error.is_none() {
            let own_server_ids: Vec<String> = editor
                .servers
                .iter()
                .filter_map(|f| f.existing_id.clone())
                .collect();
            for form in &editor.servers {
                let server_name = text(&form.name_editor);
                let command = text(&form.command_editor);
                let args_text = text(&form.args_editor);
                let url = text(&form.url_editor);
                let env_text = form.env_editor.as_ref(ctx).buffer_text(ctx);
                let headers_text = form.headers_editor.as_ref(ctx).buffer_text(ctx);
                let headers = parse_headers(&headers_text);

                if server_name.is_empty() {
                    error = Some("Every server needs a name.".to_owned());
                } else if server_entries
                    .iter()
                    .any(|e: &McpServerEntry| e.name == server_name)
                {
                    error = Some(format!("Duplicate server name \"{server_name}\"."));
                } else if McpRegistryModel::as_ref(ctx)
                    .name_taken(&server_name, form.existing_id.as_deref())
                    && !own_server_ids.iter().any(|id| {
                        McpRegistryModel::as_ref(ctx)
                            .get(id)
                            .is_some_and(|s| s.name == server_name)
                    })
                {
                    error = Some(format!("A server named \"{server_name}\" already exists."));
                } else if form.transport == McpTransport::Stdio && command.is_empty() {
                    error = Some(format!(
                        "Server \"{server_name}\": command is required for stdio."
                    ));
                } else if form.transport == McpTransport::Http && url.is_empty() {
                    error = Some(format!("Server \"{server_name}\": URL is required."));
                } else if form.transport == McpTransport::Http && !valid_http_url(&url) {
                    error = Some(format!(
                        "Server \"{server_name}\": URL must start with http:// or https://."
                    ));
                } else if headers.is_none() {
                    error = Some(format!(
                        "Server \"{server_name}\": each header must read \"Name: value\"."
                    ));
                }
                if error.is_some() {
                    break;
                }

                server_entries.push(McpServerEntry {
                    id: form
                        .existing_id
                        .clone()
                        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                    name: server_name,
                    transport: form.transport,
                    command: (form.transport == McpTransport::Stdio && !command.is_empty())
                        .then_some(command),
                    args: (form.transport == McpTransport::Stdio)
                        .then(|| parse_args(&args_text))
                        .unwrap_or_default(),
                    url: (form.transport == McpTransport::Http).then_some(url),
                    env: env_text
                        .lines()
                        .filter_map(|line| {
                            let line = line.trim();
                            let (key, value) = line.split_once('=')?;
                            let key = key.trim();
                            (!key.is_empty()).then(|| (key.to_owned(), value.trim().to_owned()))
                        })
                        .collect(),
                    enabled_claude: form.enabled_claude,
                    enabled_codex: form.enabled_codex,
                    plugin_id: None, // set below once the plugin id is final
                    headers: headers.clone().unwrap_or_default(),
                    // An existing OAuth grant survives an edit; otherwise the
                    // presence of headers is what distinguishes static auth
                    // from none.
                    auth: match form.auth {
                        McpAuth::Oauth => McpAuth::Oauth,
                        _ if headers.as_ref().is_some_and(|h| !h.is_empty()) => McpAuth::Headers,
                        _ => McpAuth::None,
                    },
                });
                if let Some(entry) = server_entries.last() {
                    form_ids.push((form.key.clone(), entry.id.clone()));
                }
            }
        }

        // Skill sub-form validation. (name, description, existing?, claude, codex)
        let mut skill_specs: Vec<(String, String, bool, bool, bool)> = Vec::new();
        if error.is_none() {
            for form in &editor.skills {
                if let Some(existing) = &form.existing_name {
                    skill_specs.push((
                        existing.clone(),
                        String::new(),
                        true,
                        form.enabled_claude,
                        form.enabled_codex,
                    ));
                    continue;
                }
                let skill_name = text(&form.name_editor);
                let skill_description = text(&form.description_editor);
                if !is_valid_skill_name(&skill_name) {
                    error = Some(
                        "Skill names must be kebab-case: lowercase letters, digits, hyphens."
                            .to_owned(),
                    );
                } else if skill_specs.iter().any(|(n, ..)| n == &skill_name)
                    || SkillsStoreModel::as_ref(ctx).name_taken(&skill_name)
                {
                    error = Some(format!("A skill named \"{skill_name}\" already exists."));
                }
                if error.is_some() {
                    break;
                }
                skill_specs.push((
                    skill_name,
                    skill_description,
                    false,
                    form.enabled_claude,
                    form.enabled_codex,
                ));
            }
        }

        if let Some(error) = error {
            editor.error = Some(error);
            return;
        }

        let plugin_id = existing_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        // Spin removed components out into single-component plugins so they
        // stay visible and manageable instead of dangling.
        if let Some(previous) = editor.existing.clone() {
            let kept_ids: Vec<&String> = server_entries.iter().map(|e| &e.id).collect();
            for removed_id in previous
                .server_ids
                .iter()
                .filter(|id| !kept_ids.contains(id))
            {
                Self::orphan_server_into_plugin(removed_id, ctx);
            }
            let kept_names: Vec<&String> = skill_specs.iter().map(|(n, ..)| n).collect();
            for removed_name in previous
                .skill_names
                .iter()
                .filter(|n| !kept_names.contains(n))
            {
                Self::orphan_skill_into_plugin(removed_name, ctx);
            }
        }

        // Commit servers.
        let server_ids: Vec<String> = server_entries.iter().map(|e| e.id.clone()).collect();
        for mut entry in server_entries {
            entry.plugin_id = Some(plugin_id.clone());
            McpRegistryModel::handle(ctx).update(ctx, |m, mctx| m.upsert(entry, mctx));
        }

        // Commit skills.
        let skill_names: Vec<String> = skill_specs.iter().map(|(n, ..)| n.clone()).collect();
        for (skill_name, skill_description, existing, claude, codex) in skill_specs {
            let pid = Some(plugin_id.clone());
            SkillsStoreModel::handle(ctx).update(ctx, |m, mctx| {
                if !existing {
                    m.create(skill_name.clone(), skill_description.clone(), mctx);
                }
                m.set_component(&skill_name, claude, codex, pid.clone(), mctx);
            });
        }

        let entry = PluginEntry {
            id: plugin_id,
            name,
            description,
            enabled_claude: editor.enabled_claude,
            enabled_codex: editor.enabled_codex,
            server_ids,
            skill_names,
        };
        PluginRegistryModel::handle(ctx).update(ctx, |m, mctx| m.upsert(entry, mctx));
        self.editor = None;
        self.sync_rows(ctx);

        // twarp 24c: Connect probes the server that was just committed, so the
        // OAuth callback has a persisted id to route back to.
        if let Some(server_id) = connect_form_key.and_then(|key| {
            form_ids
                .into_iter()
                .find(|(form_key, _)| *form_key == key)
                .map(|(_, id)| id)
        }) {
            if let Some(saved) = McpRegistryModel::as_ref(ctx).get(&server_id).cloned() {
                McpOauthModel::handle(ctx).update(ctx, |m, mctx| m.connect(&saved, mctx));
            }
        }
    }

    /// A server removed from a plugin becomes a single-server plugin of its
    /// own (mirroring the load-time orphan migration).
    fn orphan_server_into_plugin(server_id: &str, ctx: &mut ViewContext<AutomationView>) {
        let Some(mut server) = McpRegistryModel::as_ref(ctx).get(server_id).cloned() else {
            return;
        };
        let plugin_id = uuid::Uuid::new_v4().to_string();
        let plugin_name = PluginRegistryModel::as_ref(ctx).unique_name(&server.name);
        server.plugin_id = Some(plugin_id.clone());
        McpRegistryModel::handle(ctx).update(ctx, |m, mctx| m.upsert(server.clone(), mctx));
        let entry = PluginEntry {
            id: plugin_id,
            name: plugin_name,
            description: String::new(),
            enabled_claude: true,
            enabled_codex: true,
            server_ids: vec![server_id.to_owned()],
            skill_names: Vec::new(),
        };
        PluginRegistryModel::handle(ctx).update(ctx, |m, mctx| m.upsert(entry, mctx));
    }

    /// A skill removed from a plugin becomes a single-skill plugin of its own.
    fn orphan_skill_into_plugin(skill_name: &str, ctx: &mut ViewContext<AutomationView>) {
        let plugin_id = uuid::Uuid::new_v4().to_string();
        let plugin_name = PluginRegistryModel::as_ref(ctx).unique_name(skill_name);
        let (claude, codex) = SkillsStoreModel::as_ref(ctx)
            .skills()
            .iter()
            .find(|s| s.name == skill_name)
            .map(|s| (s.enabled_claude, s.enabled_codex))
            .unwrap_or((true, true));
        SkillsStoreModel::handle(ctx).update(ctx, |m, mctx| {
            m.set_component(skill_name, claude, codex, Some(plugin_id.clone()), mctx);
        });
        let entry = PluginEntry {
            id: plugin_id,
            name: plugin_name,
            description: String::new(),
            enabled_claude: true,
            enabled_codex: true,
            server_ids: Vec::new(),
            skill_names: vec![skill_name.to_owned()],
        };
        PluginRegistryModel::handle(ctx).update(ctx, |m, mctx| m.upsert(entry, mctx));
    }

    pub fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let registry = PluginRegistryModel::as_ref(app);

        let mut column = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);

        // While the empty state (with its own CTA) shows, the header's Add
        // button is hidden — one clear next action, not two.
        let show_empty_state = registry.plugins().is_empty() && self.editor.is_none();

        // Header: title left, Add button right.
        let mut header = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                Shrinkable::new(
                    1.,
                    Align::new(
                        Text::new_inline(
                            "Plugins",
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
            header.add_child(ChildView::new(&self.add_button).finish());
        }
        column.add_child(header.finish());
        column.add_child(
            Container::new(
                Text::new_inline(
                    "A plugin bundles MCP servers and skills; every new Claude and Codex session picks them up.",
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

        // Quick add: one card per preset; clicking opens the Add form
        // prefilled as a single-server plugin, so only credentials are left.
        column.add_child(
            Container::new(
                Text::new_inline(
                    "QUICK ADD",
                    appearance.ui_font_family(),
                    type_ramp::CAPTION.size,
                )
                .with_line_height_ratio(type_ramp::CAPTION.line_height)
                .with_color(theme.sub_text_color(theme.background()).into())
                .finish(),
            )
            .with_margin_bottom(spacing::SM)
            .finish(),
        );
        // Two-column gallery of brand-icon cards (the reference layout's
        // Featured grid); an odd trailing preset gets an empty filler cell so
        // it keeps half-width.
        let mut gallery = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
        for pair in PRESETS.chunks(2) {
            let mut row = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_spacing(spacing::SM);
            for preset in pair {
                row.add_child(Expanded::new(1., self.render_preset_card(preset, app)).finish());
            }
            if pair.len() == 1 {
                row.add_child(
                    Expanded::new(
                        1.,
                        Flex::row().with_main_axis_size(MainAxisSize::Min).finish(),
                    )
                    .finish(),
                );
            }
            gallery.add_child(
                Container::new(row.finish())
                    .with_margin_bottom(spacing::SM)
                    .finish(),
            );
        }
        column.add_child(
            Container::new(gallery.finish())
                .with_margin_bottom(spacing::MD)
                .finish(),
        );

        if let Some(editor) = &self.editor {
            column.add_child(self.render_editor(editor, app));
        }

        if show_empty_state {
            column.add_child(super::render_empty_state(
                twarp_core::ui::Icon::Dataflow,
                "Work with your favorite tools",
                "A plugin bundles MCP servers and skills for every new Claude and Codex \
                 session — pick one from Quick Add above or build your own.",
                &self.empty_add_button,
                app,
            ));
        }
        for entry in registry.plugins() {
            column.add_child(self.render_card(entry, app));
        }

        // Built-in plugins, read-only. Names mirror the private SERVER_NAME
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
        column.add_child(render_builtin_card(
            "twarp-browser",
            "Drives the in-app browser pane for the active session.",
            Icon::Globe4,
            appearance,
        ));
        if crate::computer_control::platform_supported() {
            column.add_child(render_builtin_card(
                "twarp-computer-control",
                "Injects mouse/keyboard events for UI debugging.",
                Icon::Navigation,
                appearance,
            ));
        }

        // Import: real ~/.claude/skills dirs adoptable as single-skill
        // plugins.
        let store = SkillsStoreModel::as_ref(app);
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

    /// One plugin card: name + description + component summary left;
    /// plugin-level C/X switches, Edit, Delete right.
    /// One quick-add gallery card: brand-icon chip + label + description,
    /// hoverable; clicking opens the Add form prefilled from the preset.
    fn render_preset_card(
        &self,
        preset: &'static PluginPreset,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let Some(state) = self.preset_hover.get(preset.key) else {
            return Flex::row().with_main_axis_size(MainAxisSize::Min).finish();
        };

        let family = appearance.ui_font_family();
        let main = theme.main_text_color(theme.background());
        let sub = theme.sub_text_color(theme.background());
        let key = preset.key;
        let card = Hoverable::new(state.clone(), move |hover| {
            let icon_color = main;
            let chip = match ExternalProductIcon::from_string(key) {
                Some(product) => super::render_chip(
                    product.to_warpui_icon(icon_color).finish(),
                    theme.surface_overlay_1(),
                    spacing::XL,
                    spacing::SM,
                ),
                None => super::render_icon_chip(
                    Icon::Dataflow,
                    icon_color,
                    theme.surface_overlay_1(),
                    spacing::XL,
                    spacing::SM,
                ),
            };
            let labels = Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_spacing(spacing::XXS)
                .with_child(
                    Text::new_inline(preset.label, family, type_ramp::UI.size)
                        .with_line_height_ratio(type_ramp::UI.line_height)
                        .with_color(main.into())
                        .finish(),
                )
                .with_child(
                    Text::new(preset.description, family, type_ramp::CAPTION.size)
                        .with_line_height_ratio(type_ramp::CAPTION.line_height)
                        .with_color(sub.into())
                        .finish(),
                )
                .finish();
            let mut container = Container::new(
                Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_spacing(spacing::MD)
                    .with_child(chip)
                    .with_child(Shrinkable::new(1., Align::new(labels).left().finish()).finish())
                    .finish(),
            )
            .with_uniform_padding(spacing::MD)
            .with_border(Border::all(1.).with_border_fill(theme.outline()))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius::CARD)));
            if hover.is_hovered() {
                container = container.with_background(theme.surface_overlay_1());
            }
            container.finish()
        });
        card.on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(AutomationViewAction::Plugins(
                PluginsPageAction::OpenPreset(key.to_owned()),
            ));
        })
        .finish()
    }

    fn render_card(&self, entry: &PluginEntry, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let Some(row_ui) = self.rows.get(&entry.id) else {
            // A plugin created outside this view's actions; drawn without
            // controls until the next sync.
            return render_builtin_card(
                &entry.name,
                &entry.component_summary(),
                Icon::Dataflow,
                appearance,
            );
        };

        let mut labels = Flex::column()
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
            );
        if !entry.description.is_empty() {
            labels = labels.with_child(
                Text::new_inline(
                    entry.description.clone(),
                    appearance.ui_font_family(),
                    type_ramp::CAPTION.size,
                )
                .with_line_height_ratio(type_ramp::CAPTION.line_height)
                .with_color(theme.sub_text_color(theme.background()).into())
                .finish(),
            );
        }
        labels = labels.with_child(
            Text::new_inline(
                entry.component_summary(),
                appearance.ui_font_family(),
                type_ramp::CAPTION.size,
            )
            .with_line_height_ratio(type_ramp::CAPTION.line_height)
            .with_color(theme.sub_text_color(theme.background()).into())
            .finish(),
        );
        // twarp 24c: connection status for the card's remote servers, so a
        // server needing authorization is visible without opening the editor
        // (P9). Stdio-only plugins show nothing.
        for status in remote_server_statuses(entry, app) {
            labels = labels.with_child(render_status_chip(&status, appearance));
        }
        if let Some(conflict) = plugin_conflict_note(entry, app) {
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

        let claude_id = entry.id.clone();
        let codex_id = entry.id.clone();
        let delete_button = if self.pending_delete.as_deref() == Some(entry.id.as_str()) {
            &row_ui.confirm_delete_button
        } else {
            &row_ui.delete_button
        };
        let controls = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(spacing::SM)
            .with_child(render_labeled_switch(
                "Claude",
                entry.enabled_claude,
                row_ui.claude_switch.clone(),
                move |ctx| {
                    ctx.dispatch_typed_action(AutomationViewAction::Plugins(
                        PluginsPageAction::ToggleClaude(claude_id.clone()),
                    ));
                },
                appearance,
            ))
            .with_child(render_labeled_switch(
                "Codex",
                entry.enabled_codex,
                row_ui.codex_switch.clone(),
                move |ctx| {
                    ctx.dispatch_typed_action(AutomationViewAction::Plugins(
                        PluginsPageAction::ToggleCodex(codex_id.clone()),
                    ));
                },
                appearance,
            ))
            .with_child(ChildView::new(&row_ui.edit_button).finish())
            .with_child(ChildView::new(delete_button).finish())
            .finish();

        // Brand chip when the plugin is named after a known product,
        // otherwise the generic dataflow mark.
        let icon_color = theme.main_text_color(theme.background());
        let chip = match ExternalProductIcon::from_string(&entry.name) {
            Some(product) => super::render_chip(
                product.to_warpui_icon(icon_color).finish(),
                theme.surface_overlay_1(),
                spacing::XL,
                spacing::SM,
            ),
            None => super::render_icon_chip(
                Icon::Dataflow,
                icon_color,
                theme.surface_overlay_1(),
                spacing::XL,
                spacing::SM,
            ),
        };

        Container::new(
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(spacing::MD)
                .with_child(chip)
                // Align::left inside the Shrinkable makes the label column
                // greedily fill the row, pushing the controls to the right
                // edge (a bare Shrinkable is flex-loose and leaves the
                // controls hugging the text).
                .with_child(
                    Shrinkable::new(1., Align::new(labels.finish()).left().finish()).finish(),
                )
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
                // Soft-wrapped: long store paths must never paint under the
                // Adopt button (Text::new_inline overflows its box).
                Text::new(
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
            .with_spacing(spacing::MD)
            .with_child(super::render_icon_chip(
                Icon::BookOpen,
                theme.sub_text_color(theme.background()),
                theme.surface_overlay_1(),
                spacing::LG,
                spacing::SM,
            ))
            .with_child(Shrinkable::new(1., Align::new(labels).left().finish()).finish());
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

    /// The inline Add / Edit form: plugin metadata, repeatable server
    /// sub-forms, repeatable skill sub-forms, plugin-level toggles, Save row.
    fn render_editor(&self, editor: &PluginEditor, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let mut form = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
        form.add_child(render_form_label(
            if editor.existing.is_some() {
                "Edit plugin"
            } else {
                "New plugin"
            },
            type_ramp::UI.size,
            appearance,
        ));

        form.add_child(render_form_field("Name", &editor.name_editor, appearance));
        form.add_child(render_form_field(
            "Description",
            &editor.description_editor,
            appearance,
        ));

        // Server sub-forms.
        form.add_child(
            Container::new(render_form_label(
                "MCP SERVERS",
                type_ramp::CAPTION.size,
                appearance,
            ))
            .with_margin_top(spacing::SM)
            .finish(),
        );
        for server in &editor.servers {
            let status = McpOauthModel::as_ref(app).status_for_form(
                server.existing_id.as_deref(),
                server.transport,
            );
            form.add_child(self.render_server_form(server, &status, appearance, app));
        }
        form.add_child(
            Container::new(
                Flex::row()
                    .with_child(ChildView::new(&editor.add_server_button).finish())
                    .finish(),
            )
            .with_margin_bottom(spacing::SM)
            .finish(),
        );

        // Skill sub-forms.
        form.add_child(
            Container::new(render_form_label(
                "SKILLS",
                type_ramp::CAPTION.size,
                appearance,
            ))
            .with_margin_top(spacing::SM)
            .finish(),
        );
        for skill in &editor.skills {
            form.add_child(self.render_skill_form(skill, appearance));
        }
        form.add_child(
            Container::new(
                Flex::row()
                    .with_child(ChildView::new(&editor.add_skill_button).finish())
                    .finish(),
            )
            .with_margin_bottom(spacing::SM)
            .finish(),
        );

        // Plugin-level provider toggles.
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
                            ctx.dispatch_typed_action(AutomationViewAction::Plugins(
                                PluginsPageAction::ToggleFormClaude,
                            ));
                        },
                        appearance,
                    ))
                    .with_child(render_labeled_switch(
                        "Codex",
                        editor.enabled_codex,
                        editor.codex_switch.clone(),
                        |ctx| {
                            ctx.dispatch_typed_action(AutomationViewAction::Plugins(
                                PluginsPageAction::ToggleFormCodex,
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

    /// twarp 24c: the status chip plus whichever of Connect / Cancel /
    /// Disconnect applies to the server's current state.
    fn render_connect_row(
        &self,
        form: &ServerForm,
        status: &McpAuthStatus,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let button = match status {
            McpAuthStatus::WaitingForBrowser => Some(&form.cancel_connect_button),
            // A probe in flight has nothing to cancel — it settles on its own.
            McpAuthStatus::Connecting => None,
            McpAuthStatus::Connected { .. } => Some(&form.disconnect_button),
            _ => Some(&form.connect_button),
        };

        let mut row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(spacing::MD);
        if let Some(button) = button {
            row = row.with_child(ChildView::new(button).finish());
        }
        row = row.with_child(
            Shrinkable::new(1., render_status_chip(status, appearance)).finish(),
        );

        Container::new(row.finish())
            .with_margin_bottom(spacing::SM)
            .finish()
    }

    /// One server sub-form: name, remote URL + Connect, an Advanced
    /// disclosure holding transport / stdio fields / headers / env, then the
    /// provider toggles and Remove (twarp 24c).
    fn render_server_form(
        &self,
        form: &ServerForm,
        status: &McpAuthStatus,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let mut column = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);

        column.add_child(
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(
                    Shrinkable::new(
                        1.,
                        render_form_label("Server", type_ramp::LABEL.size, appearance),
                    )
                    .finish(),
                )
                .with_child(ChildView::new(&form.remove_button).finish())
                .finish(),
        );

        column.add_child(render_form_field("Name", &form.name_editor, appearance));

        // twarp 24c: remote-first. A hosted server is Name + URL + Connect;
        // everything a local process needs lives behind Advanced.
        if form.transport == McpTransport::Http {
            column.add_child(render_form_field(
                "Server URL",
                &form.url_editor,
                appearance,
            ));
            column.add_child(self.render_connect_row(form, status, appearance));
        }

        column.add_child(
            Container::new(
                ChildView::new(if form.advanced_open {
                    &form.hide_advanced_button
                } else {
                    &form.show_advanced_button
                })
                .finish(),
            )
            .with_margin_bottom(spacing::SM)
            .finish(),
        );

        if form.advanced_open {
            column.add_child(
                Container::new(
                    Flex::column()
                        .with_cross_axis_alignment(CrossAxisAlignment::Start)
                        .with_child(render_form_label(
                            "Transport",
                            type_ramp::LABEL.size,
                            appearance,
                        ))
                        .with_child(ChildView::new(&form.transport_dropdown).finish())
                        .finish(),
                )
                .with_margin_bottom(spacing::SM)
                .finish(),
            );

            match form.transport {
                McpTransport::Stdio => {
                    column.add_child(render_form_field(
                        "Command",
                        &form.command_editor,
                        appearance,
                    ));
                    column.add_child(render_form_field(
                        "Arguments",
                        &form.args_editor,
                        appearance,
                    ));
                }
                McpTransport::Http => {
                    column.add_child(render_form_field(
                        "Headers",
                        &form.headers_editor,
                        appearance,
                    ));
                }
            }
            column.add_child(render_form_field(
                "Environment variables",
                &form.env_editor,
                appearance,
            ));
        }

        // twarp 24a: Codex's mcp_servers config can only carry a bearer token.
        if form.enabled_codex && codex_unsupported_headers(form, app) {
            column.add_child(
                Container::new(
                    Text::new(
                        "Codex can't send custom headers; only an Authorization: Bearer \
                         header reaches it."
                            .to_owned(),
                        appearance.ui_font_family(),
                        type_ramp::CAPTION.size,
                    )
                    .with_line_height_ratio(type_ramp::CAPTION.line_height)
                    .with_color(theme.ui_warning_color())
                    .finish(),
                )
                .with_margin_bottom(spacing::SM)
                .finish(),
            );
        }

        let claude_key = form.key.clone();
        let codex_key = form.key.clone();
        column.add_child(
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(spacing::MD)
                .with_child(render_labeled_switch(
                    "Claude",
                    form.enabled_claude,
                    form.claude_switch.clone(),
                    move |ctx| {
                        ctx.dispatch_typed_action(AutomationViewAction::Plugins(
                            PluginsPageAction::ToggleServerClaude(claude_key.clone()),
                        ));
                    },
                    appearance,
                ))
                .with_child(render_labeled_switch(
                    "Codex",
                    form.enabled_codex,
                    form.codex_switch.clone(),
                    move |ctx| {
                        ctx.dispatch_typed_action(AutomationViewAction::Plugins(
                            PluginsPageAction::ToggleServerCodex(codex_key.clone()),
                        ));
                    },
                    appearance,
                ))
                .finish(),
        );

        Container::new(column.finish())
            .with_uniform_padding(spacing::MD)
            .with_margin_bottom(spacing::SM)
            .with_border(Border::all(1.).with_border_fill(theme.outline()))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius::CARD)))
            .finish()
    }

    /// One skill sub-form: fixed `/name` for existing skills, name +
    /// description editors for inline-new ones; toggles + Remove.
    fn render_skill_form(&self, form: &SkillForm, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let mut column = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);

        column.add_child(
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(
                    Shrinkable::new(
                        1.,
                        render_form_label("Skill", type_ramp::LABEL.size, appearance),
                    )
                    .finish(),
                )
                .with_child(ChildView::new(&form.remove_button).finish())
                .finish(),
        );

        if let Some(existing) = &form.existing_name {
            column.add_child(
                Container::new(
                    Text::new_inline(
                        format!("/{existing}"),
                        appearance.monospace_font_family(),
                        type_ramp::UI.size,
                    )
                    .with_line_height_ratio(type_ramp::UI.line_height)
                    .with_color(theme.main_text_color(theme.background()).into())
                    .finish(),
                )
                .with_margin_bottom(spacing::SM)
                .finish(),
            );
        } else {
            column.add_child(render_form_field("Name", &form.name_editor, appearance));
            column.add_child(render_form_field(
                "Description",
                &form.description_editor,
                appearance,
            ));
        }

        let claude_key = form.key.clone();
        let codex_key = form.key.clone();
        column.add_child(
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(spacing::MD)
                .with_child(render_labeled_switch(
                    "Claude",
                    form.enabled_claude,
                    form.claude_switch.clone(),
                    move |ctx| {
                        ctx.dispatch_typed_action(AutomationViewAction::Plugins(
                            PluginsPageAction::ToggleSkillClaude(claude_key.clone()),
                        ));
                    },
                    appearance,
                ))
                .with_child(render_labeled_switch(
                    "Codex",
                    form.enabled_codex,
                    form.codex_switch.clone(),
                    move |ctx| {
                        ctx.dispatch_typed_action(AutomationViewAction::Plugins(
                            PluginsPageAction::ToggleSkillCodex(codex_key.clone()),
                        ));
                    },
                    appearance,
                ))
                .finish(),
        );

        Container::new(column.finish())
            .with_uniform_padding(spacing::MD)
            .with_margin_bottom(spacing::SM)
            .with_border(Border::all(1.).with_border_fill(theme.outline()))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius::CARD)))
            .finish()
    }
}

/// Combined conflict caption for a plugin card, from its member skills'
/// existing conflict states, or `None` when healthy.
fn plugin_conflict_note(entry: &PluginEntry, app: &AppContext) -> Option<String> {
    let store = SkillsStoreModel::as_ref(app);
    let (mut claude, mut codex) = (false, false);
    for name in &entry.skill_names {
        if let Some(skill) = store.skills().iter().find(|s| &s.name == name) {
            claude |= skill.claude_conflict;
            codex |= skill.codex_conflict;
        }
    }
    match (claude, codex) {
        (true, true) => {
            Some("⚠ unmanaged files block the Claude and Codex materializations".to_owned())
        }
        (true, false) => Some("⚠ a real ~/.claude/skills directory blocks the symlink".to_owned()),
        (false, true) => Some("⚠ an unmanaged ~/.codex/prompts file blocks the prompt".to_owned()),
        (false, false) => None,
    }
}

/// twarp 24a: parse the Headers field. `None` means at least one non-blank
/// line isn't `Name: value`, which blocks Save (P4).
fn parse_headers(text: &str) -> Option<std::collections::BTreeMap<String, String>> {
    let mut headers = std::collections::BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let (name, value) = line.split_once(':')?;
        let name = name.trim();
        let value = value.trim();
        if name.is_empty() || value.is_empty() {
            return None;
        }
        headers.insert(name.to_owned(), value.to_owned());
    }
    Some(headers)
}

/// twarp 24c: a remote server's URL must be syntactically usable before we
/// try to connect to it (P4).
fn valid_http_url(url: &str) -> bool {
    url::Url::parse(url).is_ok_and(|parsed| matches!(parsed.scheme(), "http" | "https"))
}

/// twarp 24c: one server sub-form button whose action carries the form key.
fn server_action_button(
    label: &'static str,
    theme: impl crate::view_components::action_button::ActionButtonTheme + 'static,
    key: &str,
    action: fn(String) -> PluginsPageAction,
    ctx: &mut ViewContext<AutomationView>,
) -> ViewHandle<ActionButton> {
    let key = key.to_owned();
    ctx.add_typed_action_view(move |_| {
        ActionButton::new(label, theme).on_click(move |ctx| {
            ctx.dispatch_typed_action(AutomationViewAction::Plugins(action(key.clone())));
        })
    })
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

fn set_editor_text(
    handle: &ViewHandle<EditorView>,
    text: &str,
    ctx: &mut ViewContext<AutomationView>,
) {
    let text = text.to_owned();
    handle.update(ctx, |editor, ctx| {
        editor.set_buffer_text_ignoring_undo(&text, ctx);
    });
}

pub(super) fn new_single_line_editor(
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

pub(super) fn new_multiline_editor(
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
pub(super) fn render_form_field(
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

/// twarp 24c: one status per remote server in the plugin, prefixed with the
/// server name when the plugin has more than one.
fn remote_server_statuses(entry: &PluginEntry, app: &AppContext) -> Vec<McpAuthStatus> {
    let registry = McpRegistryModel::as_ref(app);
    let oauth = McpOauthModel::as_ref(app);
    let remote: Vec<&McpServerEntry> = entry
        .server_ids
        .iter()
        .filter_map(|id| registry.get(id))
        .filter(|server| server.transport == McpTransport::Http)
        .collect();
    let multiple = remote.len() > 1;
    remote
        .into_iter()
        .map(|server| {
            let status = oauth.status(server);
            if multiple {
                status.prefixed(&server.name)
            } else {
                status
            }
        })
        .collect()
}

/// twarp 24a: whether the Headers field currently holds something Codex
/// can't express. Reads the editor rather than the saved entry so the warning
/// tracks what the user is typing.
fn codex_unsupported_headers(form: &ServerForm, app: &AppContext) -> bool {
    if form.transport != McpTransport::Http {
        return false;
    }
    let text = form.headers_editor.as_ref(app).buffer_text(app);
    parse_headers(&text).is_some_and(|headers| {
        headers
            .iter()
            .any(|(name, value)| !is_bearer_authorization(name, value))
    })
}

/// twarp 24c: the connection-status chip shown on a server row — label plus,
/// for errors and static-auth prompts, the detail text underneath (P9).
fn render_status_chip(status: &McpAuthStatus, appearance: &Appearance) -> Box<dyn Element> {
    let theme = appearance.theme();
    let color: twarpui::color::ColorU = match status {
        McpAuthStatus::Error(_) => theme.ui_error_color(),
        McpAuthStatus::Connected { .. } => theme.ui_green_color(),
        _ => theme.sub_text_color(theme.background()).into(),
    };

    let mut column = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Start)
        .with_child(
            Text::new_inline(
                status.label(),
                appearance.ui_font_family(),
                type_ramp::CAPTION.size,
            )
            .with_line_height_ratio(type_ramp::CAPTION.line_height)
            .with_color(color)
            .finish(),
        );
    if let Some(detail) = status.detail() {
        column = column.with_child(
            Text::new(
                detail.to_owned(),
                appearance.ui_font_family(),
                type_ramp::CAPTION.size,
            )
            .with_line_height_ratio(type_ramp::CAPTION.line_height)
            .with_color(theme.sub_text_color(theme.background()).into())
            .finish(),
        );
    }
    column.finish()
}

pub(super) fn render_form_label(
    text: &str,
    size: f32,
    appearance: &Appearance,
) -> Box<dyn Element> {
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
pub(super) fn render_labeled_switch(
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

/// A read-only card for a built-in plugin (no switches, no Edit / Delete).
fn render_builtin_card(
    name: &str,
    summary: &str,
    icon: Icon,
    appearance: &Appearance,
) -> Box<dyn Element> {
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
            .with_spacing(spacing::MD)
            .with_child(super::render_icon_chip(
                icon,
                theme.main_text_color(theme.background()),
                theme.surface_overlay_1(),
                spacing::XL,
                spacing::SM,
            ))
            .with_child(Shrinkable::new(1., Align::new(labels).left().finish()).finish())
            .with_child(chip)
            .finish(),
    )
    .with_uniform_padding(spacing::MD)
    .with_margin_bottom(spacing::SM)
    .with_border(Border::all(1.).with_border_fill(theme.outline()))
    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius::CARD)))
    .finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headers_parse_from_name_colon_value_lines() {
        let headers = parse_headers("Authorization: Bearer abc\n\n  X-Api-Key:  secret  \n")
            .expect("valid header lines");
        assert_eq!(headers["Authorization"], "Bearer abc");
        // Surrounding whitespace is the user's, not the value's.
        assert_eq!(headers["X-Api-Key"], "secret");
        // A value may itself contain colons (e.g. a URL).
        let headers = parse_headers("Referer: https://example.com/x").expect("valid");
        assert_eq!(headers["Referer"], "https://example.com/x");
    }

    #[test]
    fn malformed_header_lines_block_save() {
        // twarp 24a P4: anything that isn't `Name: value` is rejected rather
        // than silently dropped, so a typo can't become a missing credential.
        assert!(parse_headers("Authorization Bearer abc").is_none());
        assert!(parse_headers(": no-name").is_none());
        assert!(parse_headers("No-Value:").is_none());
        // Blank input and blank lines are fine.
        assert_eq!(parse_headers("").unwrap().len(), 0);
        assert_eq!(parse_headers("\n  \n").unwrap().len(), 0);
    }

    #[test]
    fn only_http_urls_are_connectable() {
        assert!(valid_http_url("https://mcp.composio.dev/abc"));
        assert!(valid_http_url("http://localhost:8000/mcp"));
        assert!(!valid_http_url("mcp.composio.dev/abc"));
        assert!(!valid_http_url("file:///etc/passwd"));
        assert!(!valid_http_url(""));
    }
}
