use std::{cell::RefCell, collections::HashMap};

use ::settings::Setting as _;
use claude_code::driver::PermissionMode;
use twarpui::{
    elements::{
        ChildView, Container, CornerRadius, CrossAxisAlignment, Element, Flex, MainAxisAlignment,
        ParentElement, Radius, Shrinkable,
    },
    ui_components::{
        components::{Coords, UiComponent, UiComponentStyles},
        switch::SwitchStateHandle,
    },
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
    SetTerminalProvider(String),
    SetTerminalModel(String),
    SetTerminalEffort(String),
    SetReplyProvider(String),
    SetReplyModel(String),
    SetReplyEffort(String),
    ToggleReplySuggestions,
    ToggleTerminalSuggestions,
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
    terminal_provider_dropdown: ViewHandle<Dropdown<AgentSettingsPageAction>>,
    terminal_model_dropdown: ViewHandle<Dropdown<AgentSettingsPageAction>>,
    terminal_effort_dropdown: ViewHandle<Dropdown<AgentSettingsPageAction>>,
    reply_provider_dropdown: ViewHandle<Dropdown<AgentSettingsPageAction>>,
    reply_model_dropdown: ViewHandle<Dropdown<AgentSettingsPageAction>>,
    reply_effort_dropdown: ViewHandle<Dropdown<AgentSettingsPageAction>>,
    reply_suggestions_switch_state: SwitchStateHandle,
    terminal_suggestions_switch_state: SwitchStateHandle,
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

#[derive(Clone, Copy)]
enum SuggestionAction {
    Terminal,
    Reply,
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
            dropdown.set_items(chat_model_items(CLIAgent::Claude), ctx);
            dropdown
                .set_selected_by_action(AgentSettingsPageAction::SetChatModel(String::new()), ctx);
            dropdown.set_top_bar_max_width(220.);
            dropdown
        });
        let chat_effort_dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);
            dropdown.set_items(chat_effort_items(CLIAgent::Claude), ctx);
            dropdown
                .set_selected_by_action(AgentSettingsPageAction::SetChatEffort(String::new()), ctx);
            dropdown.set_top_bar_max_width(180.);
            dropdown
        });
        let chat_permission_mode_dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);
            dropdown.set_items(chat_permission_mode_items(CLIAgent::Claude), ctx);
            dropdown.set_selected_by_action(
                AgentSettingsPageAction::SetChatPermissionMode(
                    PermissionMode::Default.as_cli_arg().to_owned(),
                ),
                ctx,
            );
            dropdown.set_top_bar_max_width(220.);
            dropdown
        });
        let terminal_provider_dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);
            dropdown.set_rich_items(
                suggestion_provider_items(AgentSettingsPageAction::SetTerminalProvider),
                ctx,
            );
            dropdown.set_selected_by_action(
                AgentSettingsPageAction::SetTerminalProvider(String::new()),
                ctx,
            );
            dropdown.set_top_bar_max_width(180.);
            dropdown
        });
        let terminal_model_dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);
            dropdown.set_items(
                model_items(CLIAgent::Claude, AgentSettingsPageAction::SetTerminalModel),
                ctx,
            );
            dropdown.set_selected_by_action(
                AgentSettingsPageAction::SetTerminalModel(String::new()),
                ctx,
            );
            dropdown.set_top_bar_max_width(220.);
            dropdown
        });
        let terminal_effort_dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);
            dropdown.set_items(
                effort_items(CLIAgent::Claude, AgentSettingsPageAction::SetTerminalEffort),
                ctx,
            );
            dropdown.set_selected_by_action(
                AgentSettingsPageAction::SetTerminalEffort(String::new()),
                ctx,
            );
            dropdown.set_top_bar_max_width(180.);
            dropdown
        });
        let reply_provider_dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);
            dropdown.set_rich_items(
                suggestion_provider_items(AgentSettingsPageAction::SetReplyProvider),
                ctx,
            );
            dropdown.set_selected_by_action(
                AgentSettingsPageAction::SetReplyProvider(String::new()),
                ctx,
            );
            dropdown.set_top_bar_max_width(180.);
            dropdown
        });
        let reply_model_dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);
            dropdown.set_items(
                model_items(CLIAgent::Claude, AgentSettingsPageAction::SetReplyModel),
                ctx,
            );
            dropdown
                .set_selected_by_action(AgentSettingsPageAction::SetReplyModel(String::new()), ctx);
            dropdown.set_top_bar_max_width(220.);
            dropdown
        });
        let reply_effort_dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);
            dropdown.set_items(
                effort_items(CLIAgent::Claude, AgentSettingsPageAction::SetReplyEffort),
                ctx,
            );
            dropdown.set_selected_by_action(
                AgentSettingsPageAction::SetReplyEffort(String::new()),
                ctx,
            );
            dropdown.set_top_bar_max_width(180.);
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
            terminal_provider_dropdown,
            terminal_model_dropdown,
            terminal_effort_dropdown,
            reply_provider_dropdown,
            reply_model_dropdown,
            reply_effort_dropdown,
            reply_suggestions_switch_state: Default::default(),
            terminal_suggestions_switch_state: Default::default(),
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
            settings::valid_model_for_provider(chat_provider, settings.chat_model.value())
                .unwrap_or_default();
        let chat_effort =
            settings::valid_effort_for_provider(chat_provider, settings.chat_effort.value())
                .unwrap_or_default();
        let permission_mode = settings::valid_permission_mode_for_provider(
            chat_provider,
            settings.chat_permission_mode.value(),
        )
        .unwrap_or(PermissionMode::Default)
        .as_cli_arg()
        .to_owned();
        let terminal_provider =
            settings::valid_suggestion_provider_value(settings.terminal_suggest_provider.value());
        let terminal_resolved_provider =
            settings::suggestion_provider(settings.terminal_suggest_provider.value())
                .resolve(chat_provider);
        let terminal_model = settings::valid_model_for_provider(
            terminal_resolved_provider,
            settings.terminal_suggest_model.value(),
        )
        .unwrap_or_default();
        let terminal_effort = settings::valid_effort_for_provider(
            terminal_resolved_provider,
            settings.terminal_suggest_effort.value(),
        )
        .unwrap_or_default();
        let reply_provider =
            settings::valid_suggestion_provider_value(settings.reply_suggest_provider.value());
        let reply_resolved_provider =
            settings::suggestion_provider(settings.reply_suggest_provider.value())
                .resolve(chat_provider);
        let reply_model = settings::valid_model_for_provider(
            reply_resolved_provider,
            settings.reply_suggest_model.value(),
        )
        .unwrap_or_default();
        let reply_effort = settings::valid_effort_for_provider(
            reply_resolved_provider,
            settings.reply_suggest_effort.value(),
        )
        .unwrap_or_default();

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
            dropdown.set_items(chat_model_items(chat_provider), ctx);
            dropdown.set_selected_by_action(AgentSettingsPageAction::SetChatModel(chat_model), ctx);
        });
        self.chat_effort_dropdown.update(ctx, |dropdown, ctx| {
            dropdown.set_items(chat_effort_items(chat_provider), ctx);
            dropdown
                .set_selected_by_action(AgentSettingsPageAction::SetChatEffort(chat_effort), ctx);
        });
        self.chat_permission_mode_dropdown
            .update(ctx, |dropdown, ctx| {
                dropdown.set_items(chat_permission_mode_items(chat_provider), ctx);
                dropdown.set_selected_by_action(
                    AgentSettingsPageAction::SetChatPermissionMode(permission_mode),
                    ctx,
                );
            });
        self.terminal_provider_dropdown
            .update(ctx, |dropdown, ctx| {
                dropdown.set_rich_items(
                    suggestion_provider_items(AgentSettingsPageAction::SetTerminalProvider),
                    ctx,
                );
                dropdown.set_selected_by_action(
                    AgentSettingsPageAction::SetTerminalProvider(terminal_provider),
                    ctx,
                );
            });
        self.terminal_model_dropdown.update(ctx, |dropdown, ctx| {
            dropdown.set_items(
                model_items(
                    terminal_resolved_provider,
                    AgentSettingsPageAction::SetTerminalModel,
                ),
                ctx,
            );
            dropdown.set_selected_by_action(
                AgentSettingsPageAction::SetTerminalModel(terminal_model),
                ctx,
            );
        });
        self.terminal_effort_dropdown.update(ctx, |dropdown, ctx| {
            dropdown.set_items(
                effort_items(
                    terminal_resolved_provider,
                    AgentSettingsPageAction::SetTerminalEffort,
                ),
                ctx,
            );
            dropdown.set_selected_by_action(
                AgentSettingsPageAction::SetTerminalEffort(terminal_effort),
                ctx,
            );
        });
        self.reply_provider_dropdown.update(ctx, |dropdown, ctx| {
            dropdown.set_rich_items(
                suggestion_provider_items(AgentSettingsPageAction::SetReplyProvider),
                ctx,
            );
            dropdown.set_selected_by_action(
                AgentSettingsPageAction::SetReplyProvider(reply_provider),
                ctx,
            );
        });
        self.reply_model_dropdown.update(ctx, |dropdown, ctx| {
            dropdown.set_items(
                model_items(
                    reply_resolved_provider,
                    AgentSettingsPageAction::SetReplyModel,
                ),
                ctx,
            );
            dropdown
                .set_selected_by_action(AgentSettingsPageAction::SetReplyModel(reply_model), ctx);
        });
        self.reply_effort_dropdown.update(ctx, |dropdown, ctx| {
            dropdown.set_items(
                effort_items(
                    reply_resolved_provider,
                    AgentSettingsPageAction::SetReplyEffort,
                ),
                ctx,
            );
            dropdown
                .set_selected_by_action(AgentSettingsPageAction::SetReplyEffort(reply_effort), ctx);
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
        self.auth_status_for_agent(agent, app)
    }

    fn auth_status_for_agent(&self, agent: CLIAgent, app: &AppContext) -> AuthStatus {
        if settings::api_key_presence(AgentSettings::as_ref(app), agent) {
            return AuthStatus::UsingApiKey;
        }

        if self.auth_probe_agent != agent {
            return AuthStatus::Checking;
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

    fn resolved_provider_for_suggestion(
        &self,
        action: SuggestionAction,
        app: &AppContext,
    ) -> (CLIAgent, bool) {
        let settings = AgentSettings::as_ref(app);
        let config = match action {
            SuggestionAction::Terminal => settings.terminal_suggest_config(),
            SuggestionAction::Reply => settings.reply_suggest_config(),
        };
        let inherited = config.provider.is_inherit();
        (
            config.provider.resolve(settings.chat_provider_agent()),
            inherited,
        )
    }

    fn is_valid_suggestion_provider_selection(&self, provider: &str) -> bool {
        let provider = provider.trim();
        provider.is_empty() || CLIAgent::from_serialized_name(provider).is_agent_settings_enabled()
    }

    fn resolved_provider_for_pending_suggestion_selection(
        &self,
        provider: &str,
        app: &AppContext,
    ) -> CLIAgent {
        let chat_provider = AgentSettings::as_ref(app).chat_provider_agent();
        settings::suggestion_provider(provider).resolve(chat_provider)
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
                let provider = AgentSettings::as_ref(ctx).chat_provider_agent();
                if model.is_empty() || settings::valid_model_for_provider(provider, model).is_some()
                {
                    AgentSettings::handle(ctx).update(ctx, |settings, ctx| {
                        report_if_error!(settings.chat_model.set_value(model.clone(), ctx));
                    });
                }
            }
            AgentSettingsPageAction::SetChatEffort(effort) => {
                let provider = AgentSettings::as_ref(ctx).chat_provider_agent();
                if effort.is_empty()
                    || settings::valid_effort_for_provider(provider, effort).is_some()
                {
                    AgentSettings::handle(ctx).update(ctx, |settings, ctx| {
                        report_if_error!(settings.chat_effort.set_value(effort.clone(), ctx));
                    });
                }
            }
            AgentSettingsPageAction::SetChatPermissionMode(mode) => {
                let provider = AgentSettings::as_ref(ctx).chat_provider_agent();
                if settings::valid_permission_mode_for_provider(provider, mode).is_some() {
                    AgentSettings::handle(ctx).update(ctx, |settings, ctx| {
                        report_if_error!(settings
                            .chat_permission_mode
                            .set_value(mode.clone(), ctx));
                    });
                }
            }
            AgentSettingsPageAction::SetTerminalProvider(provider) => {
                if self.is_valid_suggestion_provider_selection(provider) {
                    AgentSettings::handle(ctx).update(ctx, |settings, ctx| {
                        report_if_error!(settings
                            .terminal_suggest_provider
                            .set_value(provider.trim().to_owned(), ctx));
                    });
                }
            }
            AgentSettingsPageAction::SetTerminalModel(model) => {
                let provider = self.resolved_provider_for_pending_suggestion_selection(
                    AgentSettings::as_ref(ctx).terminal_suggest_provider.value(),
                    ctx,
                );
                if model.is_empty() || settings::valid_model_for_provider(provider, model).is_some()
                {
                    AgentSettings::handle(ctx).update(ctx, |settings, ctx| {
                        report_if_error!(settings
                            .terminal_suggest_model
                            .set_value(model.clone(), ctx));
                    });
                }
            }
            AgentSettingsPageAction::SetTerminalEffort(effort) => {
                let provider = self.resolved_provider_for_pending_suggestion_selection(
                    AgentSettings::as_ref(ctx).terminal_suggest_provider.value(),
                    ctx,
                );
                if effort.is_empty()
                    || settings::valid_effort_for_provider(provider, effort).is_some()
                {
                    AgentSettings::handle(ctx).update(ctx, |settings, ctx| {
                        report_if_error!(settings
                            .terminal_suggest_effort
                            .set_value(effort.clone(), ctx));
                    });
                }
            }
            AgentSettingsPageAction::SetReplyProvider(provider) => {
                if self.is_valid_suggestion_provider_selection(provider) {
                    AgentSettings::handle(ctx).update(ctx, |settings, ctx| {
                        report_if_error!(settings
                            .reply_suggest_provider
                            .set_value(provider.trim().to_owned(), ctx));
                    });
                }
            }
            AgentSettingsPageAction::SetReplyModel(model) => {
                let provider = self.resolved_provider_for_pending_suggestion_selection(
                    AgentSettings::as_ref(ctx).reply_suggest_provider.value(),
                    ctx,
                );
                if model.is_empty() || settings::valid_model_for_provider(provider, model).is_some()
                {
                    AgentSettings::handle(ctx).update(ctx, |settings, ctx| {
                        report_if_error!(settings
                            .reply_suggest_model
                            .set_value(model.clone(), ctx));
                    });
                }
            }
            AgentSettingsPageAction::SetReplyEffort(effort) => {
                let provider = self.resolved_provider_for_pending_suggestion_selection(
                    AgentSettings::as_ref(ctx).reply_suggest_provider.value(),
                    ctx,
                );
                if effort.is_empty()
                    || settings::valid_effort_for_provider(provider, effort).is_some()
                {
                    AgentSettings::handle(ctx).update(ctx, |settings, ctx| {
                        report_if_error!(settings
                            .reply_suggest_effort
                            .set_value(effort.clone(), ctx));
                    });
                }
            }
            AgentSettingsPageAction::ToggleReplySuggestions => {
                AgentSettings::handle(ctx).update(ctx, |settings, ctx| {
                    let enabled = *settings.enable_reply_suggestions.value();
                    report_if_error!(settings.enable_reply_suggestions.set_value(!enabled, ctx));
                });
            }
            AgentSettingsPageAction::ToggleTerminalSuggestions => {
                AgentSettings::handle(ctx).update(ctx, |settings, ctx| {
                    let enabled = *settings.enable_terminal_suggestions.value();
                    report_if_error!(settings
                        .enable_terminal_suggestions
                        .set_value(!enabled, ctx));
                });
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
        "agent claude codex gemini backend chat history terminal reply suggestions model effort permission mode"
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

        Flex::column()
            .with_child(backend)
            .with_child(auth)
            .with_child(render_sub_header(appearance, "Models by action", None))
            .with_child(
                Container::new(
                    Flex::column()
                        .with_child(render_chat_action_row(view, appearance, app))
                        .with_child(render_suggestion_action_row(
                            view,
                            appearance,
                            app,
                            SuggestionAction::Terminal,
                        ))
                        .with_child(render_suggestion_action_row(
                            view,
                            appearance,
                            app,
                            SuggestionAction::Reply,
                        ))
                        .finish(),
                )
                .with_padding_left(8.)
                .finish(),
            )
            .with_child(render_sub_header(appearance, "Suggestions", None))
            .with_child(render_reply_suggestions_toggle(view, appearance, app))
            .with_child(render_terminal_suggestions_toggle(view, appearance, app))
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

fn local_only_icon_state(
    view: &AgentSettingsPageView,
    storage_key: &str,
    sync_to_cloud: ::settings::SyncToCloud,
    app: &AppContext,
) -> LocalOnlyIconState {
    LocalOnlyIconState::for_setting(
        storage_key,
        sync_to_cloud,
        &mut view.local_only_icon_states.borrow_mut(),
        app,
    )
}

fn render_chat_action_row(
    view: &AgentSettingsPageView,
    appearance: &Appearance,
    app: &AppContext,
) -> Box<dyn Element> {
    let chat_provider = AgentSettings::as_ref(app).chat_provider_agent();
    let mut rows = vec![render_dropdown_item(
        appearance,
        "Provider",
        None,
        None,
        local_only_icon_state(
            view,
            settings::AgentChatProvider::storage_key(),
            settings::AgentChatProvider::sync_to_cloud(),
            app,
        ),
        None,
        &view.chat_provider_dropdown,
    )];

    if chat_provider.supports_models() {
        rows.push(render_dropdown_item(
            appearance,
            "Model",
            None,
            None,
            local_only_icon_state(
                view,
                settings::AgentChatModel::storage_key(),
                settings::AgentChatModel::sync_to_cloud(),
                app,
            ),
            None,
            &view.chat_model_dropdown,
        ));
    }

    if chat_provider.supports_effort() {
        rows.push(render_dropdown_item(
            appearance,
            "Effort",
            None,
            None,
            local_only_icon_state(
                view,
                settings::AgentChatEffort::storage_key(),
                settings::AgentChatEffort::sync_to_cloud(),
                app,
            ),
            None,
            &view.chat_effort_dropdown,
        ));
    }

    if chat_provider.supports_permission_modes() {
        rows.push(render_dropdown_item(
            appearance,
            "Permission mode",
            None,
            None,
            local_only_icon_state(
                view,
                settings::AgentChatPermissionMode::storage_key(),
                settings::AgentChatPermissionMode::sync_to_cloud(),
                app,
            ),
            None,
            &view.chat_permission_mode_dropdown,
        ));
    }

    rows.push(render_auth_source_row(
        view,
        appearance,
        app,
        chat_provider,
        None,
    ));

    render_action_group(appearance, "Chat & history", rows)
}

fn render_suggestion_action_row(
    view: &AgentSettingsPageView,
    appearance: &Appearance,
    app: &AppContext,
    action: SuggestionAction,
) -> Box<dyn Element> {
    let (provider, inherited) = view.resolved_provider_for_suggestion(action, app);
    let inherited_note = inherited.then(|| {
        render_inheritance_note(
            format!("Inherits Chat & history ({})", agent_display_name(provider)),
            appearance,
        )
    });

    let (title, provider_dropdown, model_dropdown, effort_dropdown) = match action {
        SuggestionAction::Terminal => (
            "Terminal suggestions",
            &view.terminal_provider_dropdown,
            &view.terminal_model_dropdown,
            &view.terminal_effort_dropdown,
        ),
        SuggestionAction::Reply => (
            "Chat reply suggestions",
            &view.reply_provider_dropdown,
            &view.reply_model_dropdown,
            &view.reply_effort_dropdown,
        ),
    };

    let (provider_key, provider_sync, model_key, model_sync, effort_key, effort_sync) = match action
    {
        SuggestionAction::Terminal => (
            settings::AgentTerminalSuggestProvider::storage_key(),
            settings::AgentTerminalSuggestProvider::sync_to_cloud(),
            settings::AgentTerminalSuggestModel::storage_key(),
            settings::AgentTerminalSuggestModel::sync_to_cloud(),
            settings::AgentTerminalSuggestEffort::storage_key(),
            settings::AgentTerminalSuggestEffort::sync_to_cloud(),
        ),
        SuggestionAction::Reply => (
            settings::AgentReplySuggestProvider::storage_key(),
            settings::AgentReplySuggestProvider::sync_to_cloud(),
            settings::AgentReplySuggestModel::storage_key(),
            settings::AgentReplySuggestModel::sync_to_cloud(),
            settings::AgentReplySuggestEffort::storage_key(),
            settings::AgentReplySuggestEffort::sync_to_cloud(),
        ),
    };

    let mut rows = vec![render_dropdown_item(
        appearance,
        "Provider",
        None,
        inherited_note,
        local_only_icon_state(view, provider_key, provider_sync, app),
        None,
        provider_dropdown,
    )];

    if provider.supports_models() {
        rows.push(render_dropdown_item(
            appearance,
            "Model",
            None,
            None,
            local_only_icon_state(view, model_key, model_sync, app),
            None,
            model_dropdown,
        ));
    }

    if provider.supports_effort() {
        rows.push(render_dropdown_item(
            appearance,
            "Effort",
            None,
            None,
            local_only_icon_state(view, effort_key, effort_sync, app),
            None,
            effort_dropdown,
        ));
    }

    rows.push(render_auth_source_row(
        view,
        appearance,
        app,
        provider,
        inherited.then(|| {
            format!(
                "Inherited from Chat & history ({})",
                agent_display_name(provider)
            )
        }),
    ));

    render_action_group(appearance, title, rows)
}

fn render_action_group(
    appearance: &Appearance,
    title: &'static str,
    rows: Vec<Box<dyn Element>>,
) -> Box<dyn Element> {
    let mut column = Flex::column().with_child(render_sub_header(appearance, title, None));
    for row in rows {
        column.add_child(row);
    }

    Container::new(column.finish())
        .with_margin_bottom(12.)
        .finish()
}

fn render_auth_source_row(
    view: &AgentSettingsPageView,
    appearance: &Appearance,
    app: &AppContext,
    provider: CLIAgent,
    inherited_detail: Option<String>,
) -> Box<dyn Element> {
    let status = view.auth_status_for_agent(provider, app);
    let warning = matches!(
        status,
        AuthStatus::NotAuthenticated | AuthStatus::CliNotInstalled
    );
    let color = if warning {
        appearance.theme().ui_error_color()
    } else {
        appearance.theme().active_ui_text_color().into()
    };
    let detail = inherited_detail
        .unwrap_or_else(|| format!("Resolved for {}.", agent_display_name(provider)));

    render_settings_row(
        render_dropdown_item_label(
            "Auth source".to_owned(),
            Some(detail),
            LocalOnlyIconState::Hidden,
            None,
            appearance,
        ),
        appearance
            .ui_builder()
            .span(auth_source_label(status))
            .with_style(UiComponentStyles {
                font_color: Some(color),
                font_size: Some(12.),
                ..Default::default()
            })
            .build()
            .finish(),
    )
}

fn render_reply_suggestions_toggle(
    view: &AgentSettingsPageView,
    appearance: &Appearance,
    app: &AppContext,
) -> Box<dyn Element> {
    let settings = AgentSettings::as_ref(app);
    render_suggestion_toggle(
        view,
        appearance,
        app,
        "Suggest a reply after each response",
        "Uses the Chat reply suggestions row when available.",
        *settings.enable_reply_suggestions.value(),
        settings::AgentEnableReplySuggestions::storage_key(),
        settings::AgentEnableReplySuggestions::sync_to_cloud(),
        view.reply_suggestions_switch_state.clone(),
        AgentSettingsPageAction::ToggleReplySuggestions,
    )
}

fn render_terminal_suggestions_toggle(
    view: &AgentSettingsPageView,
    appearance: &Appearance,
    app: &AppContext,
) -> Box<dyn Element> {
    let settings = AgentSettings::as_ref(app);
    render_suggestion_toggle(
        view,
        appearance,
        app,
        "AI command suggestions in the terminal",
        "Uses the Terminal suggestions row when available.",
        *settings.enable_terminal_suggestions.value(),
        settings::AgentEnableTerminalSuggestions::storage_key(),
        settings::AgentEnableTerminalSuggestions::sync_to_cloud(),
        view.terminal_suggestions_switch_state.clone(),
        AgentSettingsPageAction::ToggleTerminalSuggestions,
    )
}

fn render_suggestion_toggle(
    view: &AgentSettingsPageView,
    appearance: &Appearance,
    app: &AppContext,
    label: &str,
    detail: &str,
    checked: bool,
    storage_key: &str,
    sync_to_cloud: ::settings::SyncToCloud,
    switch_state: SwitchStateHandle,
    action: AgentSettingsPageAction,
) -> Box<dyn Element> {
    let label = render_dropdown_item_label(
        label.to_owned(),
        Some(detail.to_owned()),
        local_only_icon_state(view, storage_key, sync_to_cloud, app),
        None,
        appearance,
    );
    let switch = appearance
        .ui_builder()
        .switch(switch_state)
        .check(checked)
        .build()
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(action.clone());
        })
        .finish();

    Container::new(render_settings_row(label, switch))
        .with_margin_bottom(8.)
        .finish()
}

fn render_inheritance_note(text: String, appearance: &Appearance) -> Box<dyn Element> {
    appearance
        .ui_builder()
        .span(text)
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
        .finish()
}

fn auth_source_label(status: AuthStatus) -> &'static str {
    match status {
        AuthStatus::Checking => "Checking...",
        AuthStatus::LoggedInLocalCli => "Logged in (local CLI)",
        AuthStatus::UsingApiKey => "Using API key",
        AuthStatus::NotAuthenticated => "Not authenticated",
        AuthStatus::CliNotInstalled => "CLI not installed",
    }
}

fn agent_display_name(agent: CLIAgent) -> &'static str {
    agent.display_name()
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
    CLIAgent::SETTINGS_OPTIONS
        .into_iter()
        .map(|agent| agent_dropdown_item(agent, AgentSettingsPageAction::SetBackend(agent)))
        .collect()
}

fn chat_provider_items() -> Vec<MenuItem<DropdownAction<AgentSettingsPageAction>>> {
    CLIAgent::SETTINGS_OPTIONS
        .into_iter()
        .map(|agent| agent_dropdown_item(agent, AgentSettingsPageAction::SetChatProvider(agent)))
        .collect()
}

fn suggestion_provider_items(
    action: fn(String) -> AgentSettingsPageAction,
) -> Vec<MenuItem<DropdownAction<AgentSettingsPageAction>>> {
    let mut items = vec![MenuItemFields::new("Default")
        .with_on_select_action(DropdownAction::SelectActionAndClose(action(String::new())))
        .with_disabled(false)
        .into_item()];
    items.extend(
        CLIAgent::SETTINGS_OPTIONS
            .into_iter()
            .map(|agent| agent_dropdown_item(agent, action(agent.serialized_name().to_owned()))),
    );
    items
}

fn agent_dropdown_item(
    agent: CLIAgent,
    action: AgentSettingsPageAction,
) -> MenuItem<DropdownAction<AgentSettingsPageAction>> {
    let disabled = !agent.is_agent_settings_enabled();
    let label = if disabled {
        format!("{} (coming soon)", agent.display_name())
    } else {
        agent.display_name().to_owned()
    };
    MenuItemFields::new(label)
        .with_on_select_action(DropdownAction::SelectActionAndClose(action))
        .with_disabled(disabled)
        .into_item()
}

fn chat_model_items(provider: CLIAgent) -> Vec<DropdownItem<AgentSettingsPageAction>> {
    model_items(provider, AgentSettingsPageAction::SetChatModel)
}

fn model_items(
    provider: CLIAgent,
    action: fn(String) -> AgentSettingsPageAction,
) -> Vec<DropdownItem<AgentSettingsPageAction>> {
    let mut items = vec![DropdownItem::new("Default", action(String::new()))];
    items.extend(
        provider
            .model_options()
            .into_iter()
            .map(|model| DropdownItem::new(model.display_name, action(model.id))),
    );
    items
}

fn chat_effort_items(provider: CLIAgent) -> Vec<DropdownItem<AgentSettingsPageAction>> {
    effort_items(provider, AgentSettingsPageAction::SetChatEffort)
}

fn effort_items(
    provider: CLIAgent,
    action: fn(String) -> AgentSettingsPageAction,
) -> Vec<DropdownItem<AgentSettingsPageAction>> {
    std::iter::once(DropdownItem::new("Default", action(String::new())))
        .chain(
            provider
                .effort_options()
                .iter()
                .map(|option| DropdownItem::new(option.label, action(option.value.to_owned()))),
        )
        .collect()
}

fn chat_permission_mode_items(provider: CLIAgent) -> Vec<DropdownItem<AgentSettingsPageAction>> {
    provider
        .permission_modes()
        .iter()
        .copied()
        .rev()
        .map(|mode| {
            DropdownItem::new(
                prettify_permission_mode(mode.as_cli_arg()),
                AgentSettingsPageAction::SetChatPermissionMode(mode.as_cli_arg().to_owned()),
            )
        })
        .collect()
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
