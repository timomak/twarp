use std::{cell::RefCell, collections::HashMap};

use ::settings::Setting as _;
use claude_code::driver::PermissionMode;
use twarpui::{
    elements::{Container, Element, Flex, ParentElement},
    ui_components::components::{Coords, UiComponent, UiComponentStyles},
    AppContext, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

use super::{
    settings_page::{
        render_dropdown_item, render_sub_header, MatchData, PageType, SettingsPageMeta,
        SettingsPageViewHandle, SettingsWidget,
    },
    LocalOnlyIconState, SettingsSection,
};
use crate::{
    app_state::CLIAgent,
    appearance::Appearance,
    report_if_error,
    settings::{self, AgentSettings},
    view_components::{Dropdown, DropdownItem},
};

const PAGE_TITLE: &str = "Agent";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentSettingsPageAction {
    SetBackend(CLIAgent),
    SetChatProvider(CLIAgent),
    SetChatModel(String),
    SetChatEffort(String),
    SetChatPermissionMode(String),
}

pub enum AgentSettingsPageEvent {}

pub struct AgentSettingsPageView {
    page: PageType<Self>,
    backend_dropdown: ViewHandle<Dropdown<AgentSettingsPageAction>>,
    chat_provider_dropdown: ViewHandle<Dropdown<AgentSettingsPageAction>>,
    chat_model_dropdown: ViewHandle<Dropdown<AgentSettingsPageAction>>,
    chat_effort_dropdown: ViewHandle<Dropdown<AgentSettingsPageAction>>,
    chat_permission_mode_dropdown: ViewHandle<Dropdown<AgentSettingsPageAction>>,
    local_only_icon_states: RefCell<HashMap<String, twarpui::elements::MouseStateHandle>>,
}

impl AgentSettingsPageView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let backend_dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);
            dropdown.set_items(backend_items(), ctx);
            dropdown
                .set_selected_by_action(AgentSettingsPageAction::SetBackend(CLIAgent::Claude), ctx);
            dropdown.set_top_bar_max_width(220.);
            dropdown
        });
        let chat_provider_dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);
            dropdown.set_items(chat_provider_items(), ctx);
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

        let mut view = Self {
            page: PageType::new_monolith(AgentSettingsWidget, Some(PAGE_TITLE), true),
            backend_dropdown,
            chat_provider_dropdown,
            chat_model_dropdown,
            chat_effort_dropdown,
            chat_permission_mode_dropdown,
            local_only_icon_states: RefCell::new(HashMap::new()),
        };
        view.refresh_dropdowns(ctx);
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
            dropdown.set_items(backend_items(), ctx);
            dropdown.set_selected_by_action(AgentSettingsPageAction::SetBackend(backend), ctx);
        });
        self.chat_provider_dropdown.update(ctx, |dropdown, ctx| {
            dropdown.set_items(chat_provider_items(), ctx);
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

fn backend_items() -> Vec<DropdownItem<AgentSettingsPageAction>> {
    vec![
        DropdownItem::new(
            "Claude",
            AgentSettingsPageAction::SetBackend(CLIAgent::Claude),
        ),
        DropdownItem::new(
            "Codex (coming soon)",
            AgentSettingsPageAction::SetBackend(CLIAgent::Codex),
        ),
        DropdownItem::new(
            "Gemini (coming soon)",
            AgentSettingsPageAction::SetBackend(CLIAgent::Gemini),
        ),
    ]
}

fn chat_provider_items() -> Vec<DropdownItem<AgentSettingsPageAction>> {
    vec![
        DropdownItem::new(
            "Claude",
            AgentSettingsPageAction::SetChatProvider(CLIAgent::Claude),
        ),
        DropdownItem::new(
            "Codex (coming soon)",
            AgentSettingsPageAction::SetChatProvider(CLIAgent::Codex),
        ),
        DropdownItem::new(
            "Gemini (coming soon)",
            AgentSettingsPageAction::SetChatProvider(CLIAgent::Gemini),
        ),
    ]
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
