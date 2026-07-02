//! Tool-call cards for the Claude Code pane (feature 07, sub-phase 7d;
//! PRODUCT §16–§19).
//!
//! Bridges [`TranscriptItem::Tool`] onto the ported `inline_action` chrome
//! ([`super::inline_action`]): every card is a [`RenderableAction`] — never a
//! fresh primitive tree. Two ported row shapes are used, matching how Agent
//! Mode used them (the "overloaded row" comment in the original source):
//!
//! - **Compact card** (path/pattern tools, §17): a single action row — tool
//!   glyph + name + input summary, with the result label, status icon, and
//!   chevron as the row's trailing cluster.
//! - **Header card** (`Bash`, and unmapped tools with a multi-line input,
//!   §17–§18): a [`HeaderConfig`] header (title + cluster + chevron) above the
//!   command/input body — the old `requested_command` shape, rebuilt read-only
//!   from the ported chrome per TECH.md's matrix.
//!
//! Results (§19) collapse to a short derived label and expand into the card's
//! footer (truncated past a cap so expanding never blocks the UI). A `Task`
//! card renders its sub-agent's nested calls inside that footer, visually
//! grouping the child activity it spawned.
//!
//! The pure helpers (`tool_input_summary`, `generic_input_summary`,
//! `derive_result_label`, …) are unit-tested at the bottom of this file — the
//! same headless-formatting test pattern the original `requested_command_test`
//! used.

use std::borrow::Cow;
use std::collections::HashMap;

use claude_code::{ToolOutput, ToolStatus, TranscriptItem};
use serde_json::Value;
use twarpui::{
    elements::{
        Clipped, Container, CrossAxisAlignment, Flex, MainAxisSize, MouseStateHandle,
        ParentElement, Shrinkable, Text,
    },
    AppContext, Element, SingletonEntity,
};

use twarp_core::ui::theme::AnsiColorIdentifier;

use super::inline_action::{
    green_check_icon, red_x_icon, render_requested_action_body_text, running_icon, Disclosure,
    RightCluster,
};
use super::ClaudeCodeViewAction;
use crate::appearance::Appearance;
use crate::ui_components::blended_colors;
use crate::ui_components::icons::Icon as WarpIcon;

/// Expanded output is capped so a huge tool result can't stall layout
/// (PRODUCT §19: expanding never blocks the UI).
const EXPANDED_OUTPUT_MAX_LINES: usize = 200;
const EXPANDED_OUTPUT_MAX_BYTES: usize = 16 * 1024;
/// A single-line result no longer than this renders verbatim as the collapsed
/// label; anything bigger falls back to a line count.
const INLINE_RESULT_LABEL_MAX_CHARS: usize = 48;
/// Input summaries are clipped to one line of at most this many characters.
const INPUT_SUMMARY_MAX_CHARS: usize = 120;

/// Per-card UI state owned by the view: a stable mouse handle (so a click's
/// down/up hit the same handle across renders) and the user's expansion
/// choice, when they've made one (otherwise [`default_expanded`] applies).
#[derive(Default)]
pub(super) struct ToolCardUi {
    pub mouse_state: MouseStateHandle,
    pub expanded_override: Option<bool>,
}

/// Whether a card starts expanded: failures show their error without a click
/// (PRODUCT §19), and a running `Task` shows its child activity live.
pub(super) fn default_expanded(status: ToolStatus, has_children: bool) -> bool {
    match status {
        ToolStatus::Failed => true,
        ToolStatus::Running => has_children,
        ToolStatus::Completed => false,
    }
}

/// Display name for a card title (§16/§18). MCP tools (`mcp__server__tool`)
/// prettify to `server · tool`; everything else renders as reported.
pub(super) fn tool_display_name(name: &str) -> String {
    match name.strip_prefix("mcp__") {
        Some(rest) => rest.split("__").collect::<Vec<_>>().join(" · "),
        None => name.to_owned(),
    }
}

/// The tool glyph for a card (§32: icon + name + summary + status). Unmapped
/// tools share the generic wrench.
pub(super) fn tool_icon(name: &str) -> WarpIcon {
    match name {
        "Read" | "Write" => WarpIcon::File,
        "Edit" | "MultiEdit" | "NotebookEdit" => WarpIcon::Pencil,
        "Bash" => WarpIcon::Terminal,
        "Grep" => WarpIcon::Search,
        "Glob" => WarpIcon::Folder,
        "WebFetch" | "WebSearch" => WarpIcon::Globe,
        "Task" | "Agent" => WarpIcon::ArrowSplit,
        "TodoWrite" => WarpIcon::BulletedListBlock,
        _ => WarpIcon::Tool,
    }
}

/// Whether a tool name is the sub-agent dispatch tool. Claude Code has shipped
/// this under two names — the historical `Task` and the current `Agent`; both
/// spawn a sub-agent whose child activity nests under the card (PRODUCT §19), so
/// the card chrome (icon, label, live status, prose result) treats them alike.
pub(super) fn is_subagent_tool(name: &str) -> bool {
    matches!(name, "Task" | "Agent")
}

/// Per-tool key-input summary (PRODUCT §17). `None` for tools twarp doesn't
/// map — the generic card renders the input instead (§18).
pub(super) fn tool_input_summary(name: &str, input: &Value) -> Option<String> {
    let str_field = |key: &str| {
        input
            .get(key)
            .and_then(|v| v.as_str())
            .map(format_command_text)
    };
    match name {
        "Read" | "Write" | "Edit" | "MultiEdit" => str_field("file_path"),
        "NotebookEdit" => str_field("notebook_path"),
        "Bash" => str_field("command"),
        "Grep" | "Glob" => str_field("pattern"),
        "WebFetch" => str_field("url"),
        "WebSearch" => str_field("query"),
        "Task" | "Agent" => {
            // The `Agent`/`Task` input carries a short `description` and the
            // `subagent_type` it dispatches to; show "<type>: <description>" so
            // the row says *which* agent is doing *what* (matching the Claude
            // app), falling back to whichever field is present.
            let desc = str_field("description");
            let kind = input
                .get("subagent_type")
                .and_then(|v| v.as_str())
                .filter(|k| !k.is_empty());
            match (kind, desc) {
                (Some(kind), Some(desc)) => Some(format!("{kind}: {desc}")),
                (None, desc) => desc,
                (Some(kind), None) => Some(kind.to_owned()),
            }
        }
        // Routed to the §22 task list in 7f; until then the card summarizes.
        "TodoWrite" => {
            let count = input.get("todos").and_then(|v| v.as_array()).map(Vec::len);
            count.map(|n| format!("{n} task{}", if n == 1 { "" } else { "s" }))
        }
        _ => None,
    }
}

/// The `description` Claude Code sends with `Bash` calls; shown as the header
/// title when present (PRODUCT §17: the command *and* its description).
pub(super) fn bash_description(input: &Value) -> Option<String> {
    input
        .get("description")
        .and_then(|v| v.as_str())
        .map(format_command_text)
}

/// Formats text to truncate at the first newline and add an ellipsis.
/// (Port of `requested_command::format_command_text`.)
pub(super) fn format_command_text(text: &str) -> String {
    if let Some(newline_pos) = text.find('\n') {
        let first_line = &text[..newline_pos];
        if text[newline_pos..].trim().is_empty() {
            first_line.to_string()
        } else {
            format!("{first_line}…")
        }
    } else {
        text.to_string()
    }
}

/// Truncate to at most `max` characters (on a char boundary), appending an
/// ellipsis when something was cut.
fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(max).collect();
    out.push('…');
    out
}

/// Compact, readable rendering of an unmapped tool's input (PRODUCT §18):
/// `key: value` pairs for objects, the string itself for strings, JSON
/// otherwise. Never panics on any shape; empty for `null`.
pub(super) fn generic_input_summary(input: &Value) -> String {
    let text = match input {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Object(map) => map
            .iter()
            .map(|(key, value)| {
                let rendered = match value {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                format!("{key}: {}", format_command_text(&rendered))
            })
            .collect::<Vec<_>>()
            .join("\n"),
        other => other.to_string(),
    };
    text.lines()
        .map(|line| truncate_chars(line, INPUT_SUMMARY_MAX_CHARS))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The collapsed result label (PRODUCT §19): the driver's summary when it set
/// one; otherwise derived — a short single-line result verbatim, a line count
/// for anything bigger, "failed" for errors. A permission denial (the §26
/// degradation path: headless `claude` refuses un-granted tools with an
/// error tool_result) reads "needs permission" so the remedy — the composer's
/// mode selector — is discoverable.
pub(super) fn derive_result_label(
    status: ToolStatus,
    output: Option<&ToolOutput>,
) -> Option<String> {
    match status {
        ToolStatus::Running => None,
        ToolStatus::Failed
            if output.is_some_and(|o| {
                o.text.contains("requested permissions") && o.text.contains("haven't granted")
            }) =>
        {
            Some("needs permission".to_owned())
        }
        ToolStatus::Failed => Some("failed".to_owned()),
        ToolStatus::Completed => {
            let output = output?;
            if let Some(summary) = &output.summary {
                return Some(summary.clone());
            }
            let text = output.text.trim();
            if text.is_empty() {
                return Some("no output".to_owned());
            }
            let mut lines = text.lines();
            let first = lines.next().unwrap_or_default();
            match lines.count() {
                0 if first.chars().count() <= INLINE_RESULT_LABEL_MAX_CHARS => {
                    Some(first.to_owned())
                }
                more => Some(format!(
                    "{} line{}",
                    more + 1,
                    if more == 0 { "" } else { "s" }
                )),
            }
        }
    }
}

/// The collapsed-row status for a *running* sub-agent (PRODUCT §19, UI
/// cleanup): the verb of the child tool it's currently running (e.g. "Read…"),
/// the step count once children have completed, else "working…" before any
/// child appears — so a collapsed `Agent` row still says what it's doing
/// without expanding, the way the Claude app surfaces the sub-agent's step.
fn subagent_running_label(children: &[TranscriptItem]) -> String {
    let tools: Vec<&TranscriptItem> = children
        .iter()
        .filter(|c| matches!(c, TranscriptItem::Tool { .. }))
        .collect();
    if let Some(TranscriptItem::Tool {
        name,
        input,
        status: ToolStatus::Running,
        ..
    }) = tools.last().copied()
    {
        let (verb, _) = tool_verb_label(name, input);
        return format!("{verb}…");
    }
    match tools.len() {
        0 => "working…".to_owned(),
        n => format!("{n} step{}", if n == 1 { "" } else { "s" }),
    }
}

/// Cap expanded output (PRODUCT §19). Returns the shown text and how many
/// lines were cut.
pub(super) fn truncate_output(text: &str) -> (String, usize) {
    let total_lines = text.lines().count();
    let mut shown = String::new();
    let mut shown_lines = 0;
    for line in text.lines().take(EXPANDED_OUTPUT_MAX_LINES) {
        if shown.len() + line.len() > EXPANDED_OUTPUT_MAX_BYTES {
            break;
        }
        if shown_lines > 0 {
            shown.push('\n');
        }
        shown.push_str(line);
        shown_lines += 1;
    }
    (shown, total_lines.saturating_sub(shown_lines))
}

/// Whether the card has anything to expand into a footer.
fn is_expandable(output: Option<&ToolOutput>, children: &[TranscriptItem]) -> bool {
    !children.is_empty() || output.is_some_and(|o| !o.text.trim().is_empty())
}

pub(super) fn status_icon(status: ToolStatus, appearance: &Appearance) -> twarpui::elements::Icon {
    match status {
        ToolStatus::Running => running_icon(appearance),
        ToolStatus::Completed => green_check_icon(appearance),
        ToolStatus::Failed => red_x_icon(appearance),
    }
}

/// The leading verb + argument for a card's label (UI cleanup: rows read as
/// "Ran …", "Read …", "Searched …" to match the Claude app, instead of the raw
/// tool name). Unmapped tools keep their (prettified) name as the verb with no
/// argument; the input still renders in the expanded box.
pub(super) fn tool_verb_label(name: &str, input: &Value) -> (Cow<'static, str>, Option<String>) {
    let verb: Cow<'static, str> = match name {
        "Read" => "Read".into(),
        "Write" => "Wrote".into(),
        "Edit" | "MultiEdit" | "NotebookEdit" => "Edited".into(),
        "Bash" => "Ran".into(),
        "Grep" | "Glob" | "WebSearch" => "Searched".into(),
        "WebFetch" => "Fetched".into(),
        "Task" | "Agent" => "Agent".into(),
        "TodoWrite" => "Updated".into(),
        _ => Cow::Owned(tool_display_name(name)),
    };
    let arg = match name {
        // A `Bash` row reads best as its human description; fall back to the
        // command's first line so the row always says *something*.
        "Bash" => bash_description(input).or_else(|| tool_input_summary(name, input)),
        "TodoWrite" => Some("todos".to_owned()),
        _ => tool_input_summary(name, input),
    };
    (verb, arg)
}

/// A card's label row: the verb in the main text color (red on failure, so a
/// collapsed error still reads at a glance), the argument muted monospace
/// beside it, clipped to the row.
pub(super) fn verb_title_row(
    verb: &str,
    arg: Option<&str>,
    failed: bool,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let verb_color = if failed {
        AnsiColorIdentifier::Red
            .to_ansi_color(&theme.terminal_colors().normal)
            .into()
    } else {
        blended_colors::text_main(theme, blended_colors::neutral_2(theme))
    };
    let mut row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);
    row.add_child(
        Text::new_inline(
            verb.to_owned(),
            appearance.ui_font_family(),
            appearance.monospace_font_size(),
        )
        .with_color(verb_color)
        .with_selectable(false)
        .finish(),
    );
    if let Some(arg) = arg {
        row.add_child(
            Shrinkable::new(
                1.,
                Container::new(
                    Clipped::new(
                        Text::new_inline(
                            arg.to_owned(),
                            appearance.monospace_font_family(),
                            appearance.monospace_font_size() - 1.,
                        )
                        .with_color(
                            theme
                                .sub_text_color(blended_colors::neutral_2(theme).into())
                                .into(),
                        )
                        .with_selectable(false)
                        .finish(),
                    )
                    .finish(),
                )
                .with_margin_left(8.)
                .finish(),
            )
            .finish(),
        );
    }
    row.finish()
}

/// The monospace block shown at the top of an expanded card's box: a `Bash`
/// command verbatim, or an unmapped tool's `key: value` input (§18). Mapped
/// path/pattern tools carry their key input in the label, so they get nothing
/// here — just their result.
fn input_block_text(name: &str, input: &Value) -> Option<String> {
    match name {
        "Bash" => input
            .get("command")
            .and_then(|v| v.as_str())
            .map(str::to_owned)
            .filter(|s| !s.trim().is_empty()),
        _ if tool_input_summary(name, input).is_none() => {
            let rendered = generic_input_summary(input);
            (!rendered.is_empty()).then_some(rendered)
        }
        _ => None,
    }
}

/// Whether the card has anything to reveal on expand: nested calls, a non-empty
/// result, or an input block.
fn body_present(
    name: &str,
    input: &Value,
    output: Option<&ToolOutput>,
    children: &[TranscriptItem],
) -> bool {
    !children.is_empty()
        || output.is_some_and(|o| !o.text.trim().is_empty())
        || input_block_text(name, input).is_some()
}

/// The expanded box content: the optional input block above the §19 footer
/// (nested `Task` calls + the result text).
#[allow(clippy::too_many_arguments)]
fn build_tool_body(
    name: &str,
    input: &Value,
    output: Option<&ToolOutput>,
    children: &[TranscriptItem],
    status: ToolStatus,
    ui: &HashMap<String, ToolCardUi>,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let mut column = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_main_axis_size(MainAxisSize::Min);
    if let Some(text) = input_block_text(name, input) {
        column.add_child(
            render_requested_action_body_text(
                Cow::Owned(text),
                appearance.monospace_font_family(),
                app,
            )
            .finish(),
        );
    }
    column.add_child(render_footer(
        output,
        children,
        status,
        is_subagent_tool(name),
        ui,
        app,
    ));
    column.finish()
}

/// Render one tool call as a flat disclosure (PRODUCT §16–§19, UI cleanup):
/// a borderless `glyph + verb + arg` row, with the result/status cluster and a
/// bordered content box revealed only on expand. `nested` marks a `Task` child
/// rendered inside its parent's box (tighter spacing, no column indent).
#[allow(clippy::too_many_arguments)]
pub(super) fn render_tool_card(
    id: &str,
    name: &str,
    input: &Value,
    status: ToolStatus,
    output: Option<&ToolOutput>,
    children: &[TranscriptItem],
    ui: &HashMap<String, ToolCardUi>,
    nested: bool,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();

    let card_ui = ui.get(id);
    let mouse_state = card_ui.map(|ui| ui.mouse_state.clone()).unwrap_or_default();
    let expandable = body_present(name, input, output, children);
    let expanded = expandable
        && card_ui
            .and_then(|ui| ui.expanded_override)
            .unwrap_or_else(|| default_expanded(status, !children.is_empty()));

    let (verb, arg) = tool_verb_label(name, input);
    let title = verb_title_row(
        &verb,
        arg.as_deref(),
        status == ToolStatus::Failed,
        appearance,
    );
    let glyph =
        twarpui::elements::Icon::new(tool_icon(name).into(), blended_colors::neutral_7(theme));

    // A running sub-agent shows its current step in the cluster so the
    // collapsed row stays legible (§19); every other card shows its result
    // label (which is `None` while running).
    let cluster_label = if is_subagent_tool(name) && status == ToolStatus::Running {
        Some(subagent_running_label(children))
    } else {
        derive_result_label(status, output)
    };

    let toggle_id = id.to_owned();
    let mut disclosure = Disclosure::new(title)
        .with_glyph(glyph)
        .expandable(expandable)
        .expanded(expanded)
        .with_cluster(RightCluster {
            label: cluster_label,
            icon: Some(status_icon(status, appearance)),
        })
        .with_mouse_state(mouse_state)
        .nested(nested)
        .on_toggle(move |ctx| {
            ctx.dispatch_typed_action(ClaudeCodeViewAction::ToggleToolCard(toggle_id.clone()));
        });
    if expanded {
        disclosure = disclosure.with_body(build_tool_body(
            name, input, output, children, status, ui, app,
        ));
    }
    disclosure.render(app)
}

/// The expanded footer (PRODUCT §19): a `Task`'s nested child cards, then the
/// (truncated) result text — the error text when the tool failed.
fn render_footer(
    output: Option<&ToolOutput>,
    children: &[TranscriptItem],
    status: ToolStatus,
    subagent: bool,
    ui: &HashMap<String, ToolCardUi>,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let mut column = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_main_axis_size(MainAxisSize::Min);

    for child in children {
        if let TranscriptItem::Tool {
            id,
            name,
            input,
            status,
            output,
            children,
        } = child
        {
            column.add_child(render_tool_card(
                id,
                name,
                input,
                *status,
                output.as_ref(),
                children,
                ui,
                true,
                app,
            ));
        }
    }

    if let Some(output) = output {
        let text = output.text.trim();
        if !text.is_empty() {
            let (shown, hidden) = truncate_output(text);
            // A sub-agent's result is its natural-language answer, so render it
            // in the proportional UI font as prose; every other tool's output is
            // monospace machine text (§19).
            let result_font = if subagent {
                appearance.ui_font_family()
            } else {
                appearance.monospace_font_family()
            };
            column.add_child(
                Container::new(
                    render_requested_action_body_text(Cow::Owned(shown), result_font, app).finish(),
                )
                .with_vertical_padding(6.)
                .finish(),
            );
            if hidden > 0 {
                column.add_child(
                    Container::new(
                        Text::new_inline(
                            format!("… {hidden} more lines"),
                            appearance.ui_font_family(),
                            appearance.monospace_font_size() - 1.,
                        )
                        .with_color(theme.sub_text_color(theme.surface_1()).into())
                        .with_selectable(false)
                        .finish(),
                    )
                    .with_padding_bottom(6.)
                    .finish(),
                );
            }
        } else if status == ToolStatus::Failed {
            column.add_child(
                Container::new(
                    Text::new_inline(
                        "The tool reported an error with no output.".to_owned(),
                        appearance.ui_font_family(),
                        appearance.monospace_font_size() - 1.,
                    )
                    .with_color(theme.sub_text_color(theme.surface_1()).into())
                    .with_selectable(false)
                    .finish(),
                )
                .with_vertical_padding(6.)
                .finish(),
            );
        }
    }

    column.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn per_tool_summaries_show_the_key_input() {
        // PRODUCT §17, one per mapped tool family.
        let cases = [
            ("Read", json!({"file_path": "src/main.rs"}), "src/main.rs"),
            ("Write", json!({"file_path": "a.txt"}), "a.txt"),
            ("Edit", json!({"file_path": "b.rs"}), "b.rs"),
            ("MultiEdit", json!({"file_path": "c.rs"}), "c.rs"),
            (
                "NotebookEdit",
                json!({"notebook_path": "n.ipynb"}),
                "n.ipynb",
            ),
            ("Bash", json!({"command": "cargo test"}), "cargo test"),
            ("Grep", json!({"pattern": "fn main"}), "fn main"),
            ("Glob", json!({"pattern": "**/*.rs"}), "**/*.rs"),
            (
                "WebFetch",
                json!({"url": "https://example.com"}),
                "https://example.com",
            ),
            ("WebSearch", json!({"query": "rust gpui"}), "rust gpui"),
            (
                "Task",
                json!({"description": "explore the repo"}),
                "explore the repo",
            ),
            (
                "Agent",
                json!({"description": "explore the repo"}),
                "explore the repo",
            ),
            (
                "Agent",
                json!({"subagent_type": "Explore", "description": "trace capture flow"}),
                "Explore: trace capture flow",
            ),
        ];
        for (name, input, expected) in cases {
            assert_eq!(
                tool_input_summary(name, &input).as_deref(),
                Some(expected),
                "summary for {name}"
            );
        }
    }

    fn running_tool(name: &str, input: Value) -> TranscriptItem {
        TranscriptItem::Tool {
            id: name.to_owned(),
            name: name.to_owned(),
            input,
            status: ToolStatus::Running,
            output: None,
            children: Vec::new(),
        }
    }

    fn done_tool(name: &str) -> TranscriptItem {
        TranscriptItem::Tool {
            id: format!("{name}-done"),
            name: name.to_owned(),
            input: Value::Null,
            status: ToolStatus::Completed,
            output: None,
            children: Vec::new(),
        }
    }

    #[test]
    fn subagent_running_label_reports_current_step() {
        // Nothing started yet → generic.
        assert_eq!(subagent_running_label(&[]), "working…");
        // Currently running a child tool → that tool's verb.
        assert_eq!(
            subagent_running_label(&[
                done_tool("Read"),
                running_tool("Grep", json!({"pattern": "x"})),
            ]),
            "Searched…"
        );
        // Children all finished (between steps) → step count.
        assert_eq!(
            subagent_running_label(&[done_tool("Read"), done_tool("Grep")]),
            "2 steps"
        );
        assert_eq!(subagent_running_label(&[done_tool("Read")]), "1 step");
    }

    #[test]
    fn both_task_and_agent_count_as_subagents() {
        assert!(is_subagent_tool("Task"));
        assert!(is_subagent_tool("Agent"));
        assert!(!is_subagent_tool("Bash"));
    }

    #[test]
    fn todowrite_summarizes_task_count() {
        let input = json!({"todos": [{"content": "a"}, {"content": "b"}]});
        assert_eq!(
            tool_input_summary("TodoWrite", &input).as_deref(),
            Some("2 tasks")
        );
        let one = json!({"todos": [{"content": "a"}]});
        assert_eq!(
            tool_input_summary("TodoWrite", &one).as_deref(),
            Some("1 task")
        );
    }

    #[test]
    fn unmapped_tools_have_no_specific_summary() {
        // §18: they fall to the generic card.
        assert_eq!(tool_input_summary("SomeFutureTool", &json!({"x": 1})), None);
        assert_eq!(
            tool_input_summary("mcp__linear__create_issue", &json!({"title": "t"})),
            None
        );
    }

    #[test]
    fn missing_fields_do_not_panic_and_yield_no_summary() {
        // §29 defensive: a known tool with an unexpected input shape.
        assert_eq!(tool_input_summary("Read", &json!({})), None);
        assert_eq!(tool_input_summary("Read", &Value::Null), None);
        assert_eq!(tool_input_summary("Bash", &json!({"command": 42})), None);
    }

    #[test]
    fn mcp_names_prettify_and_others_pass_through() {
        assert_eq!(
            tool_display_name("mcp__linear__create_issue"),
            "linear · create_issue"
        );
        assert_eq!(tool_display_name("Read"), "Read");
        assert_eq!(tool_display_name("mcp__solo"), "solo");
    }

    #[test]
    fn generic_summary_renders_object_keys_compactly() {
        let input = json!({"title": "Fix bug", "labels": ["a", "b"], "count": 3});
        let summary = generic_input_summary(&input);
        assert!(summary.contains("title: Fix bug"), "got: {summary}");
        assert!(summary.contains("count: 3"), "got: {summary}");
        assert!(summary.contains(r#"labels: ["a","b"]"#), "got: {summary}");
    }

    #[test]
    fn generic_summary_handles_all_value_shapes() {
        // §18: never crash or render blank-by-panic on any input shape.
        assert_eq!(generic_input_summary(&Value::Null), "");
        assert_eq!(generic_input_summary(&json!("plain")), "plain");
        assert_eq!(generic_input_summary(&json!(7)), "7");
        assert_eq!(generic_input_summary(&json!([1, 2])), "[1,2]");
    }

    #[test]
    fn generic_summary_truncates_long_values() {
        let long = "x".repeat(500);
        let summary = generic_input_summary(&json!({ "data": long }));
        assert!(summary.chars().count() <= INPUT_SUMMARY_MAX_CHARS + "data: …".chars().count());
        assert!(summary.ends_with('…'));
    }

    #[test]
    fn bash_description_is_first_line_only() {
        let input = json!({"command": "ls", "description": "List files\nand more"});
        assert_eq!(bash_description(&input).as_deref(), Some("List files…"));
        assert_eq!(bash_description(&json!({"command": "ls"})), None);
    }

    // Ported `format_command_text` cases (requested_command_test.rs).
    #[test]
    fn format_command_text_single_line_unchanged() {
        assert_eq!(format_command_text("echo hello world"), "echo hello world");
    }

    #[test]
    fn format_command_text_truncates_at_newline_with_ellipsis() {
        assert_eq!(
            format_command_text("cargo build\n--release"),
            "cargo build…"
        );
    }

    #[test]
    fn format_command_text_no_ellipsis_when_rest_is_whitespace() {
        assert_eq!(format_command_text("git status\n   \t  "), "git status");
    }

    #[test]
    fn format_command_text_preserves_multibyte_at_boundary() {
        assert_eq!(format_command_text("echo 🧪\nthen more"), "echo 🧪…");
    }

    #[test]
    fn result_label_prefers_driver_summary() {
        let output = ToolOutput {
            text: "a\nb\nc".to_owned(),
            summary: Some("3 matches".to_owned()),
        };
        assert_eq!(
            derive_result_label(ToolStatus::Completed, Some(&output)).as_deref(),
            Some("3 matches")
        );
    }

    #[test]
    fn result_label_derives_from_text() {
        let short = ToolOutput {
            text: "ok".to_owned(),
            summary: None,
        };
        assert_eq!(
            derive_result_label(ToolStatus::Completed, Some(&short)).as_deref(),
            Some("ok")
        );
        let multi = ToolOutput {
            text: "a\nb\nc".to_owned(),
            summary: None,
        };
        assert_eq!(
            derive_result_label(ToolStatus::Completed, Some(&multi)).as_deref(),
            Some("3 lines")
        );
        let empty = ToolOutput {
            text: "  ".to_owned(),
            summary: None,
        };
        assert_eq!(
            derive_result_label(ToolStatus::Completed, Some(&empty)).as_deref(),
            Some("no output")
        );
    }

    #[test]
    fn result_label_by_status() {
        assert_eq!(derive_result_label(ToolStatus::Running, None), None);
        assert_eq!(
            derive_result_label(ToolStatus::Failed, None).as_deref(),
            Some("failed")
        );
        // Completed with no output yet (shouldn't happen, but §29 defensive).
        assert_eq!(derive_result_label(ToolStatus::Completed, None), None);
    }

    #[test]
    fn permission_denial_reads_needs_permission() {
        // The §26 degradation shape, verbatim from headless claude 2.1.173.
        let denial = ToolOutput {
            text: "Claude requested permissions to write to /tmp/x.txt, but you haven't \
                   granted it yet."
                .to_owned(),
            summary: None,
        };
        assert_eq!(
            derive_result_label(ToolStatus::Failed, Some(&denial)).as_deref(),
            Some("needs permission")
        );
        let other = ToolOutput {
            text: "boom".to_owned(),
            summary: None,
        };
        assert_eq!(
            derive_result_label(ToolStatus::Failed, Some(&other)).as_deref(),
            Some("failed")
        );
    }

    #[test]
    fn long_single_line_results_fall_back_to_line_count() {
        let long = ToolOutput {
            text: "x".repeat(INLINE_RESULT_LABEL_MAX_CHARS + 1),
            summary: None,
        };
        assert_eq!(
            derive_result_label(ToolStatus::Completed, Some(&long)).as_deref(),
            Some("1 line")
        );
    }

    #[test]
    fn truncate_output_caps_lines_and_reports_hidden() {
        let text = (0..500)
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let (shown, hidden) = truncate_output(&text);
        assert_eq!(shown.lines().count(), EXPANDED_OUTPUT_MAX_LINES);
        assert_eq!(hidden, 300);
        let (all, none) = truncate_output("a\nb");
        assert_eq!(all, "a\nb");
        assert_eq!(none, 0);
    }

    #[test]
    fn default_expansion_rules() {
        // §19: failures show their error; a running Task shows child activity.
        assert!(default_expanded(ToolStatus::Failed, false));
        assert!(default_expanded(ToolStatus::Running, true));
        assert!(!default_expanded(ToolStatus::Running, false));
        assert!(!default_expanded(ToolStatus::Completed, true));
    }
}
