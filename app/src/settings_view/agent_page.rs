use std::{cell::RefCell, collections::HashMap};

use ::settings::Setting as _;
use claude_code::driver::PermissionMode;
use twarpui::{
    elements::{
        ChildView, Container, CornerRadius, CrossAxisAlignment, Element, Flex, MainAxisAlignment,
        ParentElement, Radius, Shrinkable,
    },
    ui_components::components::{Coords, UiComponent, UiComponentStyles},
    AppContext, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};
use twarpui_extras::secure_storage::{self, Error as SecureStorageError};

use super::{
    settings_page::{
        render_dropdown_item, render_dropdown_item_label, render_sub_header, MatchData, PageType,
        SettingsPageMeta, SettingsPageViewHandle, SettingsWidget,
    },
    LocalOnlyIconState, SettingsSection,
};
use crate::{
    app_state::{AgentLocalAuthProbe, CLIAgent},
    appearance::Appearance,
    editor::{
        EditorView, Event as EditorEvent, PropagateAndNoOpNavigationKeys, SingleLineEditorOptions,
        TextOptions,
    },
    menu::{MenuItem, MenuItemFields},
    report_if_error,
    settings::{self, AgentSettings},
    view_components::{
        action_button::{ActionButton, DangerSecondaryTheme, SecondaryTheme},
        dropdown::{DropdownAction, DropdownItem},
        Dropdown,
    },
};

const PAGE_TITLE: &str = "Agent";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentSettingsPageAction {
    SetBackend(CLIAgent),
    SetChatProvider(CLIAgent),
    SetChatModel(String),
    SetChatEffort(String),
    SetChatPermissionMode(String),
    ShowApiKeyEditor,
    SaveApiKey,
    RemoveApiKey,
}

pub enum AgentSettingsPageEvent {}

pub struct AgentSettingsPageView {
    page: PageType<Self>,
    backend_dropdown: ViewHandle<Dropdown<AgentSettingsPageAction>>,
    chat_provider_dropdown: ViewHandle<Dropdown<AgentSettingsPageAction>>,
    chat_model_dropdown: ViewHandle<Dropdown<AgentSettingsPageAction>>,
    chat_effort_dropdown: ViewHandle<Dropdown<AgentSettingsPageAction>>,
    chat_permission_mode_dropdown: ViewHandle<Dropdown<AgentSettingsPageAction>>,
    api_key_editor: ViewHandle<EditorView>,
    save_api_key_button: ViewHandle<ActionButton>,
    replace_api_key_button: ViewHandle<ActionButton>,
    remove_api_key_button: ViewHandle<ActionButton>,
    auth_probe_generation: u64,
    auth_probe_agent: CLIAgent,
    auth_probe_state: AuthProbeState,
    show_api_key_editor: bool,
    local_only_icon_states: RefCell<HashMap<String, twarpui::elements::MouseStateHandle>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AuthProbeState {
    Checking,
    Ready(AgentLocalAuthProbe),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AuthStatus {
    Checking,
    LoggedInLocalCli,
    UsingApiKey,
    NotAuthenticated,
    CliNotInstalled,
}

impl AgentSettingsPageView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let font_family = Appearance::as_ref(ctx).ui_font_family();
        let backend_dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);
            dropdown.set_rich_items(backend_items(), ctx);
            dropdown
                .set_selected_by_action(AgentSettingsPageAction::SetBackend(CLIAgent::Claude), ctx);
            dropdown.set_top_bar_max_width(220.);
            dropdown
        });
        let chat_provider_dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);
            dropdown.set_rich_items(chat_provider_items(), ctx);
            dropdown.set_selected_by_action(
                AgentSettingsPageAction::SetChatProvider(CLIAgent::Claude),
                ctx,
            );
            dropdown.set_top_bar_max_width(180.);
            dropdown
        });
        let chat_model_dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);
            dropdown.set_items(chat_model_items(), ctx);
            dropdown
                .set_selected_by_action(AgentSettingsPageAction::SetChatModel(String::new()), ctx);
            dropdown.set_top_bar_max_width(220.);
            dropdown
        });
        let chat_effort_dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);
            dropdown.set_items(chat_effort_items(), ctx);
            dropdown
                .set_selected_by_action(AgentSettingsPageAction::SetChatEffort(String::new()), ctx);
            dropdown.set_top_bar_max_width(180.);
            dropdown
        });
        let chat_permission_mode_dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);
            dropdown.set_items(chat_permission_mode_items(), ctx);
            dropdown.set_selected_by_action(
                AgentSettingsPageAction::SetChatPermissionMode(
                    PermissionMode::Default.as_cli_arg().to_owned(),
                ),
                ctx,
            );
            dropdown.set_top_bar_max_width(220.);
            dropdown
        });
        let api_key_editor = ctx.add_typed_action_view(move |ctx| {
            let options = SingleLineEditorOptions {
                text: TextOptions {
                    font_family_override: Some(font_family),
                    ..Default::default()
                },
                propagate_and_no_op_vertical_navigation_keys:
                    PropagateAndNoOpNavigationKeys::Always,
                ..Default::default()
            };
            let mut editor = EditorView::single_line(options, ctx);
            editor.set_placeholder_text("Paste API key", ctx);
            editor
        });
        ctx.subscribe_to_view(&api_key_editor, |_me, _, event, ctx| {
            if matches!(event, EditorEvent::Edited(_)) {
                ctx.notify();
            }
        });
        let save_api_key_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Save", SecondaryTheme).on_click(|ctx| {
                ctx.dispatch_typed_action(AgentSettingsPageAction::SaveApiKey);
            })
        });
        let replace_api_key_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Replace", SecondaryTheme).on_click(|ctx| {
                ctx.dispatch_typed_action(AgentSettingsPageAction::ShowApiKeyEditor);
            })
        });
        let remove_api_key_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Remove", DangerSecondaryTheme).on_click(|ctx| {
                ctx.dispatch_typed_action(AgentSettingsPageAction::RemoveApiKey);
            })
        });

        let mut view = Self {
            page: PageType::new_monolith(AgentSettingsWidget, Some(PAGE_TITLE), true),
            backend_dropdown,
            chat_provider_dropdown,
            chat_model_dropdown,
            chat_effort_dropdown,
            chat_permission_mode_dropdown,
            api_key_editor,
            save_api_key_button,
            replace_api_key_button,
            remove_api_key_button,
            auth_probe_generation: 0,
            auth_probe_agent: CLIAgent::Claude,
            auth_probe_state: AuthProbeState::Checking,
            show_api_key_editor: true,
            local_only_icon_states: RefCell::new(HashMap::new()),
        };
        view.refresh_dropdowns(ctx);
        view.refresh_auth_status(ctx);
        view
    }

    fn refresh_dropdowns(&mut self, ctx: &mut ViewContext<Self>) {
        let settings = AgentSettings::as_ref(ctx);
        let backend = settings.backend_agent();
        let chat_provider = settings.chat_provider_agent();
        let chat_model =
            settings::valid_chat_model(settings.chat_model.value()).unwrap_or_default();
        let chat_effort =
            settings::valid_chat_effort(settings.chat_effort.value()).unwrap_or_default();
        let permission_mode = PermissionMode::from_cli_arg(settings.chat_permission_mode.value())
            .unwrap_or(PermissionMode::Default)
            .as_cli_arg()
            .to_owned();

        self.backend_dropdown.update(ctx, |dropdown, ctx| {
            dropdown.set_rich_items(backend_items(), ctx);
            dropdown.set_selected_by_action(AgentSettingsPageAction::SetBackend(backend), ctx);
        });
        self.chat_provider_dropdown.update(ctx, |dropdown, ctx| {
            dropdown.set_rich_items(chat_provider_items(), ctx);
            dropdown.set_selected_by_action(
                AgentSettingsPageAction::SetChatProvider(chat_provider),
                ctx,
            );
        });
        self.chat_model_dropdown.update(ctx, |dropdown, ctx| {
            dropdown.set_items(chat_model_items(), ctx);
            dropdown.set_selected_by_action(AgentSettingsPageAction::SetChatModel(chat_model), ctx);
        });
        self.chat_effort_dropdown.update(ctx, |dropdown, ctx| {
            dropdown
                .set_selected_by_action(AgentSettingsPageAction::SetChatEffort(chat_effort), ctx);
        });
        self.chat_permission_mode_dropdown
            .update(ctx, |dropdown, ctx| {
                dropdown.set_selected_by_action(
                    AgentSettingsPageAction::SetChatPermissionMode(permission_mode),
                    ctx,
                );
            });
    }

    fn selected_auth_agent(&self, ctx: &AppContext) -> CLIAgent {
        AgentSettings::as_ref(ctx).backend_agent()
    }

    fn refresh_auth_status(&mut self, ctx: &mut ViewContext<Self>) {
        let agent = self.selected_auth_agent(ctx);
        self.sync_api_key_presence_flag(agent, ctx);

        let has_key = settings::api_key_presence(AgentSettings::as_ref(ctx), agent);
        self.show_api_key_editor = !has_key;
        self.auth_probe_agent = agent;
        self.auth_probe_state = AuthProbeState::Checking;
        self.auth_probe_generation += 1;
        let generation = self.auth_probe_generation;

        ctx.spawn(
            async move { agent.local_auth_probe() },
            move |view, probe, ctx| {
                if view.auth_probe_generation == generation && view.auth_probe_agent == agent {
                    view.auth_probe_state = AuthProbeState::Ready(probe);
                    ctx.notify();
                }
            },
        );
    }

    fn auth_status(&self, app: &AppContext) -> AuthStatus {
        let agent = self.selected_auth_agent(app);
        if settings::api_key_presence(AgentSettings::as_ref(app), agent) {
            return AuthStatus::UsingApiKey;
        }

        match self.auth_probe_state {
            AuthProbeState::Checking => AuthStatus::Checking,
            AuthProbeState::Ready(AgentLocalAuthProbe {
                cli_installed: false,
                ..
            }) => AuthStatus::CliNotInstalled,
            AuthProbeState::Ready(AgentLocalAuthProbe {
                cli_installed: true,
                logged_in: true,
            }) => AuthStatus::LoggedInLocalCli,
            AuthProbeState::Ready(AgentLocalAuthProbe {
                cli_installed: true,
                logged_in: false,
            }) => AuthStatus::NotAuthenticated,
        }
    }

    fn save_api_key(&mut self, ctx: &mut ViewContext<Self>) {
        let agent = self.selected_auth_agent(ctx);
        let Some(storage_key) = settings::api_key_storage_key(agent) else {
            return;
        };
        let api_key = self
            .api_key_editor
            .as_ref(ctx)
            .buffer_text(ctx)
            .trim()
            .to_owned();
        if api_key.is_empty() {
            return;
        }

        match secure_storage::Model::handle(ctx)
            .as_ref(ctx)
            .write_value(&storage_key, &api_key)
        {
            Ok(()) => {
                AgentSettings::handle(ctx).update(ctx, |settings, ctx| match agent {
                    CLIAgent::Claude => {
                        report_if_error!(settings.claude_api_key_set.set_value(true, ctx));
                    }
                    CLIAgent::Codex | CLIAgent::Gemini | CLIAgent::Unknown => {}
                });
                self.api_key_editor
                    .update(ctx, |editor, ctx| editor.clear_buffer(ctx));
                self.show_api_key_editor = false;
                self.refresh_auth_status(ctx);
            }
            Err(err) => {
                log::error!(
                    "Failed to store {} API key in secure storage: {err}",
                    agent.display_name()
                );
            }
        }
    }

    fn remove_api_key(&mut self, ctx: &mut ViewContext<Self>) {
        let agent = self.selected_auth_agent(ctx);
        let Some(storage_key) = settings::api_key_storage_key(agent) else {
            return;
        };

        let result = secure_storage::Model::handle(ctx)
            .as_ref(ctx)
            .remove_value(&storage_key);
        match result {
            Ok(()) | Err(SecureStorageError::NotFound) => {
                AgentSettings::handle(ctx).update(ctx, |settings, ctx| match agent {
                    CLIAgent::Claude => {
                        report_if_error!(settings.claude_api_key_set.set_value(false, ctx));
                    }
                    CLIAgent::Codex | CLIAgent::Gemini | CLIAgent::Unknown => {}
                });
                self.api_key_editor
                    .update(ctx, |editor, ctx| editor.clear_buffer(ctx));
                self.show_api_key_editor = true;
                self.refresh_auth_status(ctx);
            }
            Err(err) => {
                log::error!(
                    "Failed to remove {} API key from secure storage: {err}",
                    agent.display_name()
                );
            }
        }
    }

    fn sync_api_key_presence_flag(&self, agent: CLIAgent, ctx: &mut ViewContext<Self>) {
        if !settings::api_key_presence(AgentSettings::as_ref(ctx), agent) {
            return;
        }

        let Some(storage_key) = settings::api_key_storage_key(agent) else {
            return;
        };
        match secure_storage::Model::handle(ctx)
            .as_ref(ctx)
            .read_value(&storage_key)
        {
            Ok(_) => {}
            Err(SecureStorageError::NotFound) => {
                AgentSettings::handle(ctx).update(ctx, |settings, ctx| match agent {
                    CLIAgent::Claude => {
                        report_if_error!(settings.claude_api_key_set.set_value(false, ctx));
                    }
                    CLIAgent::Codex | CLIAgent::Gemini | CLIAgent::Unknown => {}
                });
            }
            Err(err) => {
                log::error!(
                    "Failed to verify {} API key presence in secure storage: {err}",
                    agent.display_name()
                );
            }
        }
    }
}

impl Entity for AgentSettingsPageView {
    type Event = AgentSettingsPageEvent;
}

impl TypedActionView for AgentSettingsPageView {
    type Action = AgentSettingsPageAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            AgentSettingsPageAction::SetBackend(agent) => {
                if agent.is_agent_settings_enabled() {
                    AgentSettings::handle(ctx).update(ctx, |settings, ctx| {
                        report_if_error!(settings
                            .backend
                            .set_value(agent.serialized_name().to_owned(), ctx));
                    });
                    self.refresh_auth_status(ctx);
                }
            }
            AgentSettingsPageAction::SetChatProvider(agent) => {
                if agent.is_agent_settings_enabled() {
                    AgentSettings::handle(ctx).update(ctx, |settings, ctx| {
                        report_if_error!(settings
                            .chat_provider
                            .set_value(agent.serialized_name().to_owned(), ctx));
                    });
                }
            }
            AgentSettingsPageAction::SetChatModel(model) => {
                AgentSettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(settings.chat_model.set_value(model.clone(), ctx));
                });
            }
            AgentSettingsPageAction::SetChatEffort(effort) => {
                AgentSettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(settings.chat_effort.set_value(effort.clone(), ctx));
                });
            }
            AgentSettingsPageAction::SetChatPermissionMode(mode) => {
                if PermissionMode::from_cli_arg(mode).is_some() {
                    AgentSettings::handle(ctx).update(ctx, |settings, ctx| {
                        report_if_error!(settings
                            .chat_permission_mode
                            .set_value(mode.clone(), ctx));
                    });
                }
            }
            AgentSettingsPageAction::ShowApiKeyEditor => {
                self.show_api_key_editor = true;
                self.api_key_editor.update(ctx, |editor, ctx| {
                    editor.clear_buffer(ctx);
                    editor.set_placeholder_text("Paste replacement API key", ctx);
                });
                ctx.focus(&self.api_key_editor);
            }
            AgentSettingsPageAction::SaveApiKey => {
                self.save_api_key(ctx);
            }
            AgentSettingsPageAction::RemoveApiKey => {
                self.remove_api_key(ctx);
            }
        }
        self.refresh_dropdowns(ctx);
        ctx.notify();
    }
}

impl View for AgentSettingsPageView {
    fn ui_name() -> &'static str {
        "AgentSettingsPageView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        self.page.render(self, app)
    }
}

impl SettingsPageMeta for AgentSettingsPageView {
    fn section() -> SettingsSection {
        SettingsSection::Agent
    }

    fn should_render(&self, _ctx: &AppContext) -> bool {
        true
    }

    fn update_filter(&mut self, query: &str, ctx: &mut ViewContext<Self>) -> MatchData {
        self.page.update_filter(query, ctx)
    }

    fn scroll_to_widget(&mut self, widget_id: &'static str) {
        self.page.scroll_to_widget(widget_id)
    }

    fn clear_highlighted_widget(&mut self) {
        self.page.clear_highlighted_widget();
    }
}

impl From<ViewHandle<AgentSettingsPageView>> for SettingsPageViewHandle {
    fn from(view_handle: ViewHandle<AgentSettingsPageView>) -> Self {
        SettingsPageViewHandle::Agent(view_handle)
    }
}

struct AgentSettingsWidget;

impl SettingsWidget for AgentSettingsWidget {
    type View = AgentSettingsPageView;

    fn search_terms(&self) -> &str {
        "agent claude codex gemini backend chat history model effort permission mode"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let backend_note = appearance
            .ui_builder()
            .span("Codex and Gemini: coming soon")
            .with_style(UiComponentStyles {
                font_color: Some(appearance.theme().nonactive_ui_text_color().into()),
                font_size: Some(12.),
                margin: Some(Coords {
                    top: 2.,
                    ..Default::default()
                }),
                ..Default::default()
            })
            .build()
            .finish();

        let backend = render_dropdown_item(
            appearance,
            "Backend",
            None,
            Some(backend_note),
            LocalOnlyIconState::for_setting(
                settings::AgentBackend::storage_key(),
                settings::AgentBackend::sync_to_cloud(),
                &mut view.local_only_icon_states.borrow_mut(),
                app,
            ),
            None,
            &view.backend_dropdown,
        );
        let auth = render_auth_section(view, appearance, app);

        let chat_provider = render_dropdown_item(
            appearance,
            "Provider",
            None,
            None,
            LocalOnlyIconState::for_setting(
                settings::AgentChatProvider::storage_key(),
                settings::AgentChatProvider::sync_to_cloud(),
                &mut view.local_only_icon_states.borrow_mut(),
                app,
            ),
            None,
            &view.chat_provider_dropdown,
        );
        let chat_model = render_dropdown_item(
            appearance,
            "Model",
            None,
            None,
            LocalOnlyIconState::for_setting(
                settings::AgentChatModel::storage_key(),
                settings::AgentChatModel::sync_to_cloud(),
                &mut view.local_only_icon_states.borrow_mut(),
                app,
            ),
            None,
            &view.chat_model_dropdown,
        );
        let chat_effort = render_dropdown_item(
            appearance,
            "Effort",
            None,
            None,
            LocalOnlyIconState::for_setting(
                settings::AgentChatEffort::storage_key(),
                settings::AgentChatEffort::sync_to_cloud(),
                &mut view.local_only_icon_states.borrow_mut(),
                app,
            ),
            None,
            &view.chat_effort_dropdown,
        );
        let chat_permission_mode = render_dropdown_item(
            appearance,
            "Permission mode",
            None,
            None,
            LocalOnlyIconState::for_setting(
                settings::AgentChatPermissionMode::storage_key(),
                settings::AgentChatPermissionMode::sync_to_cloud(),
                &mut view.local_only_icon_states.borrow_mut(),
                app,
            ),
            None,
            &view.chat_permission_mode_dropdown,
        );

        Flex::column()
            .with_child(backend)
            .with_child(auth)
            .with_child(render_sub_header(appearance, "Models by action", None))
            .with_child(
                Container::new(
                    Flex::column()
                        .with_child(render_sub_header(appearance, "Chat & history", None))
                        .with_child(chat_provider)
                        .with_child(chat_model)
                        .with_child(chat_effort)
                        .with_child(chat_permission_mode)
                        .finish(),
                )
                .with_padding_left(8.)
                .finish(),
            )
            .finish()
    }
}

fn render_auth_section(
    view: &AgentSettingsPageView,
    appearance: &Appearance,
    app: &AppContext,
) -> Box<dyn Element> {
    let status = view.auth_status(app);
    let status_row = render_auth_status_row(status, appearance);
    let api_key_row = render_api_key_row(view, appearance, app);

    Container::new(
        Flex::column()
            .with_child(render_sub_header(appearance, "Authentication", None))
            .with_child(status_row)
            .with_child(api_key_row)
            .finish(),
    )
    .with_margin_top(12.)
    .with_margin_bottom(16.)
    .finish()
}

fn render_auth_status_row(status: AuthStatus, appearance: &Appearance) -> Box<dyn Element> {
    let (label, detail) = match status {
        AuthStatus::Checking => ("Checking...", "Verifying the selected CLI auth state."),
        AuthStatus::LoggedInLocalCli => (
            "Logged in (local CLI)",
            "Twarp will use your existing CLI login by default.",
        ),
        AuthStatus::UsingApiKey => ("Using API key", "The key is stored in the OS keychain."),
        AuthStatus::NotAuthenticated => (
            "Not authenticated",
            "Run `claude auth login`, or save an API key.",
        ),
        AuthStatus::CliNotInstalled => (
            "CLI not installed",
            "Install the selected CLI and make sure it is on PATH.",
        ),
    };

    render_settings_row(
        render_dropdown_item_label(
            "Auth status".to_owned(),
            Some(detail.to_owned()),
            LocalOnlyIconState::Hidden,
            None,
            appearance,
        ),
        appearance
            .ui_builder()
            .span(label)
            .with_style(UiComponentStyles {
                font_color: Some(appearance.theme().active_ui_text_color().into()),
                font_size: Some(12.),
                ..Default::default()
            })
            .build()
            .finish(),
    )
}

fn render_api_key_row(
    view: &AgentSettingsPageView,
    appearance: &Appearance,
    app: &AppContext,
) -> Box<dyn Element> {
    let agent = view.selected_auth_agent(app);
    let has_key = settings::api_key_presence(AgentSettings::as_ref(app), agent);
    let local_only_icon_state = LocalOnlyIconState::for_setting(
        settings::AgentClaudeApiKeySet::storage_key(),
        settings::AgentClaudeApiKeySet::sync_to_cloud(),
        &mut view.local_only_icon_states.borrow_mut(),
        app,
    );
    let label = render_dropdown_item_label(
        "API key".to_owned(),
        Some("Stored in the OS keychain, never in plaintext settings.".to_owned()),
        local_only_icon_state,
        None,
        appearance,
    );

    let mut controls = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::End)
        .with_main_axis_alignment(MainAxisAlignment::Center);

    if has_key {
        controls.add_child(
            appearance
                .ui_builder()
                .span("**** key set")
                .with_style(UiComponentStyles {
                    font_color: Some(appearance.theme().active_ui_text_color().into()),
                    font_size: Some(12.),
                    margin: Some(Coords {
                        bottom: 6.,
                        ..Default::default()
                    }),
                    ..Default::default()
                })
                .build()
                .finish(),
        );
        controls.add_child(
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(ChildView::new(&view.replace_api_key_button).finish())
                .with_child(
                    Container::new(ChildView::new(&view.remove_api_key_button).finish())
                        .with_margin_left(8.)
                        .finish(),
                )
                .finish(),
        );
    }

    if view.show_api_key_editor {
        controls.add_child(
            Container::new(
                Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_child(
                        appearance
                            .ui_builder()
                            .text_input(view.api_key_editor.clone())
                            .with_style(UiComponentStyles {
                                width: Some(260.),
                                border_radius: Some(CornerRadius::with_all(Radius::Pixels(4.))),
                                border_width: Some(1.),
                                border_color: Some(appearance.theme().outline().into()),
                                background: Some(
                                    appearance.theme().surface_2().into_solid().into(),
                                ),
                                padding: Some(Coords::uniform(6.)),
                                ..Default::default()
                            })
                            .build()
                            .finish(),
                    )
                    .with_child(
                        Container::new(ChildView::new(&view.save_api_key_button).finish())
                            .with_margin_left(8.)
                            .finish(),
                    )
                    .finish(),
            )
            .with_margin_top(if has_key { 8. } else { 0. })
            .finish(),
        );
    }

    render_settings_row(label, controls.finish())
}

fn render_settings_row(label: Box<dyn Element>, control: Box<dyn Element>) -> Box<dyn Element> {
    Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_child(
            Shrinkable::new(
                1.0,
                Container::new(label)
                    .with_margin_bottom(4.)
                    .with_padding_right(16.)
                    .finish(),
            )
            .finish(),
        )
        .with_child(control)
        .finish()
}

fn backend_items() -> Vec<MenuItem<DropdownAction<AgentSettingsPageAction>>> {
    vec![
        agent_dropdown_item(
            "Claude",
            AgentSettingsPageAction::SetBackend(CLIAgent::Claude),
            false,
        ),
        agent_dropdown_item(
            "Codex (coming soon)",
            AgentSettingsPageAction::SetBackend(CLIAgent::Codex),
            true,
        ),
        agent_dropdown_item(
            "Gemini (coming soon)",
            AgentSettingsPageAction::SetBackend(CLIAgent::Gemini),
            true,
        ),
    ]
}

fn chat_provider_items() -> Vec<MenuItem<DropdownAction<AgentSettingsPageAction>>> {
    vec![
        agent_dropdown_item(
            "Claude",
            AgentSettingsPageAction::SetChatProvider(CLIAgent::Claude),
            false,
        ),
        agent_dropdown_item(
            "Codex (coming soon)",
            AgentSettingsPageAction::SetChatProvider(CLIAgent::Codex),
            true,
        ),
        agent_dropdown_item(
            "Gemini (coming soon)",
            AgentSettingsPageAction::SetChatProvider(CLIAgent::Gemini),
            true,
        ),
    ]
}

fn agent_dropdown_item(
    label: &'static str,
    action: AgentSettingsPageAction,
    disabled: bool,
) -> MenuItem<DropdownAction<AgentSettingsPageAction>> {
    MenuItemFields::new(label)
        .with_on_select_action(DropdownAction::SelectActionAndClose(action))
        .with_disabled(disabled)
        .into_item()
}

fn chat_model_items() -> Vec<DropdownItem<AgentSettingsPageAction>> {
    let mut items = vec![DropdownItem::new(
        "Default",
        AgentSettingsPageAction::SetChatModel(String::new()),
    )];
    match crate::claude_code_models::discovered() {
        Some(models) => {
            items.extend(models.iter().map(|model| {
                DropdownItem::new(
                    model.display_name.clone(),
                    AgentSettingsPageAction::SetChatModel(model.id.clone()),
                )
            }));
        }
        None => {
            items.extend(
                crate::claude_code_models::FALLBACK_MODEL_ALIASES
                    .iter()
                    .map(|alias| {
                        DropdownItem::new(
                            prettify_model(alias),
                            AgentSettingsPageAction::SetChatModel((*alias).to_owned()),
                        )
                    }),
            );
        }
    }
    items
}

fn chat_effort_items() -> Vec<DropdownItem<AgentSettingsPageAction>> {
    [
        ("Default", ""),
        ("Low", "low"),
        ("Medium", "medium"),
        ("High", "high"),
        ("Max", "max"),
    ]
    .into_iter()
    .map(|(label, value)| {
        DropdownItem::new(
            label,
            AgentSettingsPageAction::SetChatEffort(value.to_owned()),
        )
    })
    .collect()
}

fn chat_permission_mode_items() -> Vec<DropdownItem<AgentSettingsPageAction>> {
    PermissionMode::ALL
        .into_iter()
        .rev()
        .map(|mode| {
            DropdownItem::new(
                prettify_permission_mode(mode.as_cli_arg()),
                AgentSettingsPageAction::SetChatPermissionMode(mode.as_cli_arg().to_owned()),
            )
        })
        .collect()
}

fn prettify_model(model: &str) -> String {
    let mut chars = model.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

fn prettify_permission_mode(mode: &str) -> String {
    match mode {
        "default" => "Ask".to_owned(),
        "acceptEdits" => "Accept edits".to_owned(),
        "plan" => "Plan".to_owned(),
        "bypassPermissions" => "Bypass".to_owned(),
        other => other.to_owned(),
    }
}
