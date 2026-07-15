use std::{future::Future, path::PathBuf, pin::Pin, time::Duration};

use claude_code::{Transcript, TranscriptItem};
use command::{blocking::Command, Stdio};
use instant::Instant;
use serde_json::json;

use crate::{
    app_state::CLIAgent,
    settings::{AgentActionProvider, AgentSuggestionActionConfig},
};

const REPLY_SUGGESTION_MAX_EXCHANGES: usize = 3;
const API_TIMEOUT: Duration = Duration::from_secs(15);
const CLI_TIMEOUT: Duration = Duration::from_secs(30);

pub type SuggestionFuture = Pin<Box<dyn Future<Output = Option<String>> + Send>>;

pub trait SuggestionProvider {
    fn suggest(&self, request: SuggestionRequest) -> SuggestionFuture;
}

#[derive(Clone, Debug)]
pub struct SuggestionRequest {
    pub config: AgentSuggestionActionConfig,
    pub chat_provider: CLIAgent,
    pub api_key: Option<String>,
    pub cwd: PathBuf,
    pub path_env: Option<String>,
    pub context: SuggestionContext,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SuggestionContext {
    Reply(ReplySuggestionContext),
    TerminalCommand(TerminalSuggestionContext),
    ComposerPlaceholder(ComposerPlaceholderSuggestionContext),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplySuggestionContext {
    exchanges: Vec<ReplyExchange>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComposerPlaceholderSuggestionContext {
    cwd: Option<String>,
    repo: Option<String>,
    exchanges: Vec<ReplyExchange>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalSuggestionContext {
    prefix: String,
    cwd: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ReplyExchange {
    user: String,
    assistant: String,
}

pub struct DefaultSuggestionProvider;

impl SuggestionProvider for DefaultSuggestionProvider {
    fn suggest(&self, request: SuggestionRequest) -> SuggestionFuture {
        Box::pin(async move { suggest(request).await })
    }
}

impl SuggestionContext {
    fn prompt(&self) -> String {
        match self {
            Self::Reply(context) => context.prompt(),
            Self::TerminalCommand(context) => context.prompt(),
            Self::ComposerPlaceholder(context) => context.prompt(),
        }
    }
}

impl ReplySuggestionContext {
    pub fn from_transcript(transcript: &Transcript) -> Option<Self> {
        let exchanges = recent_completed_exchanges(transcript);
        if exchanges.is_empty() {
            return None;
        }

        Some(Self { exchanges })
    }

    fn prompt(&self) -> String {
        let mut prompt = String::from(
            "Suggest exactly one concise next user message for this Claude Code chat.\n\
             Return only the message the user could send. Do not include quotes, labels, markdown, or explanation.\n\
             Keep it under 160 characters unless the next useful reply needs a short question.\n\n\
             Recent conversation:\n",
        );
        for exchange in &self.exchanges {
            prompt.push_str("\nUser: ");
            prompt.push_str(&exchange.user);
            prompt.push_str("\nAssistant: ");
            prompt.push_str(&exchange.assistant);
            prompt.push('\n');
        }
        prompt.push_str("\nNext user message:");
        prompt
    }
}

impl ComposerPlaceholderSuggestionContext {
    pub fn new(transcript: &Transcript, cwd: Option<String>, repo: Option<String>) -> Option<Self> {
        let cwd = cwd.filter(|cwd| !cwd.trim().is_empty());
        let repo = repo.filter(|repo| !repo.trim().is_empty());
        let exchanges = recent_completed_exchanges(transcript);
        (cwd.is_some() || repo.is_some() || !exchanges.is_empty()).then_some(Self {
            cwd,
            repo,
            exchanges,
        })
    }

    fn prompt(&self) -> String {
        let mut prompt = String::from(
            "Suggest exactly one concise prompt the user could type into an empty Claude Code composer.\n\
             It should be useful for the current project context and safe to insert as editable text.\n\
             Return only the prompt text. Do not include quotes, labels, markdown, or explanation.\n\
             Keep it under 140 characters.\n\n",
        );
        if let Some(cwd) = &self.cwd {
            prompt.push_str("Current directory: ");
            prompt.push_str(cwd);
            prompt.push('\n');
        }
        if let Some(repo) = &self.repo {
            prompt.push_str("Repo context: ");
            prompt.push_str(repo);
            prompt.push('\n');
        }
        if !self.exchanges.is_empty() {
            prompt.push_str("\nRecent conversation:\n");
            for exchange in &self.exchanges {
                prompt.push_str("\nUser: ");
                prompt.push_str(&exchange.user);
                prompt.push_str("\nAssistant: ");
                prompt.push_str(&exchange.assistant);
                prompt.push('\n');
            }
        }
        prompt.push_str("\nSuggested prompt:");
        prompt
    }
}

fn recent_completed_exchanges(transcript: &Transcript) -> Vec<ReplyExchange> {
    let mut pending_assistant: Option<String> = None;
    let mut exchanges = Vec::new();

    for item in transcript.items().iter().rev() {
        match item {
            TranscriptItem::Assistant { text, done } if *done && !text.trim().is_empty() => {
                if pending_assistant.is_none() {
                    pending_assistant = Some(text.trim().to_owned());
                }
            }
            TranscriptItem::User(text) if !text.trim().is_empty() => {
                if let Some(assistant) = pending_assistant.take() {
                    exchanges.push(ReplyExchange {
                        user: text.trim().to_owned(),
                        assistant,
                    });
                    if exchanges.len() >= REPLY_SUGGESTION_MAX_EXCHANGES {
                        break;
                    }
                }
            }
            _ => {}
        }
    }

    exchanges.reverse();
    exchanges
}

impl TerminalSuggestionContext {
    pub fn new(prefix: String, cwd: Option<String>) -> Option<Self> {
        (!prefix.trim().is_empty()).then_some(Self { prefix, cwd })
    }

    fn prompt(&self) -> String {
        let mut prompt = String::from(
            "Suggest exactly one shell command that completes the command prefix below.\n\
             Return only the full command. Do not include markdown, labels, quotes, or explanation.\n\
             The command must start with the prefix exactly and must be safe to place in a terminal input buffer.\n\
             Do not include a trailing newline and do not run the command.\n\n",
        );
        if let Some(cwd) = &self.cwd {
            prompt.push_str("Current directory: ");
            prompt.push_str(cwd);
            prompt.push('\n');
        }
        prompt.push_str("Command prefix: ");
        prompt.push_str(&self.prefix);
        prompt.push_str("\nFull command:");
        prompt
    }
}

async fn suggest(request: SuggestionRequest) -> Option<String> {
    let provider = request.config.provider.resolve(request.chat_provider);
    if provider != CLIAgent::Claude {
        return None;
    }

    let prompt = request.context.prompt();
    let suggestion = match request.api_key {
        Some(api_key) => {
            suggest_with_anthropic_api(api_key, request.config.model.as_deref(), prompt).await
        }
        None => {
            suggest_with_claude_cli(
                prompt,
                request.config.model,
                request.config.effort,
                request.cwd,
                request.path_env,
            )
            .await
        }
    }?;
    sanitize_suggestion(&suggestion, &request.context)
}

async fn suggest_with_anthropic_api(
    api_key: String,
    model: Option<&str>,
    prompt: String,
) -> Option<String> {
    let model = model.map(str::to_owned).or_else(|| {
        crate::claude_code_models::discovered()
            .and_then(|models| models.first())
            .map(|model| model.id.clone())
    })?;
    let client = reqwest::Client::builder()
        .https_only(true)
        .connect_timeout(Duration::from_secs(5))
        .timeout(API_TIMEOUT)
        .build()
        .ok()?;
    let response = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .json(&json!({
            "model": model,
            "max_tokens": 80,
            "temperature": 0.2,
            "messages": [
                {
                    "role": "user",
                    "content": prompt,
                }
            ],
        }))
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        log::info!(
            "reply suggestion API request returned {}; suppressing suggestion",
            response.status()
        );
        return None;
    }
    let body: serde_json::Value = response.json().await.ok()?;
    body.get("content")?.as_array()?.iter().find_map(|block| {
        (block.get("type")?.as_str()? == "text")
            .then(|| block.get("text")?.as_str().map(str::to_owned))?
    })
}

#[cfg(not(target_family = "wasm"))]
async fn suggest_with_claude_cli(
    prompt: String,
    model: Option<String>,
    effort: Option<String>,
    cwd: PathBuf,
    path_env: Option<String>,
) -> Option<String> {
    tokio::task::spawn_blocking(move || {
        let mut command = Command::new("claude");
        command
            .arg("-p")
            .arg(prompt)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .current_dir(cwd);
        if let Some(model) = model {
            command.arg("--model").arg(model);
        }
        if let Some(effort) = effort {
            command.arg("--effort").arg(effort);
        }
        if let Some(path_env) = path_env {
            command.env("PATH", path_env);
        }

        let mut child = command.spawn().ok()?;
        let deadline = Instant::now() + CLI_TIMEOUT;
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    let output = child.wait_with_output().ok()?;
                    return status
                        .success()
                        .then(|| String::from_utf8(output.stdout).ok())?;
                }
                Ok(None) if Instant::now() >= deadline => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(50)),
                Err(_) => return None,
            }
        }
    })
    .await
    .ok()
    .flatten()
}

#[cfg(target_family = "wasm")]
async fn suggest_with_claude_cli(
    _prompt: String,
    _model: Option<String>,
    _effort: Option<String>,
    _cwd: PathBuf,
    _path_env: Option<String>,
) -> Option<String> {
    None
}

fn sanitize_suggestion(suggestion: &str, context: &SuggestionContext) -> Option<String> {
    let mut suggestion = suggestion.trim().trim_matches('"').trim().to_owned();
    if let Some(stripped) = suggestion.strip_prefix("User:") {
        suggestion = stripped.trim().to_owned();
    }
    if let Some(stripped) = suggestion.strip_prefix("Next user message:") {
        suggestion = stripped.trim().to_owned();
    }
    if let Some(stripped) = suggestion.strip_prefix("Suggested prompt:") {
        suggestion = stripped.trim().to_owned();
    }
    if let Some(stripped) = suggestion.strip_prefix("Command:") {
        suggestion = stripped.trim().to_owned();
    }
    if let Some(stripped) = suggestion.strip_prefix("$ ") {
        suggestion = stripped.trim().to_owned();
    }
    suggestion = suggestion.lines().next().unwrap_or("").trim().to_owned();
    if let SuggestionContext::TerminalCommand(context) = context {
        if !suggestion.starts_with(&context.prefix) {
            return None;
        }
    }
    (!suggestion.is_empty()).then_some(suggestion)
}

#[cfg(test)]
mod tests {
    use claude_code::TranscriptEvent;

    use super::*;

    #[test]
    fn reply_context_uses_last_completed_exchange() {
        let mut transcript = Transcript::new();
        transcript.apply(TranscriptEvent::UserMessage("what changed?".to_owned()));
        transcript.apply(TranscriptEvent::AssistantTextDelta {
            text: "I updated the tests.".to_owned(),
        });
        transcript.apply(TranscriptEvent::AssistantTextDone);

        let context = ReplySuggestionContext::from_transcript(&transcript).unwrap();
        assert_eq!(
            context.exchanges,
            vec![ReplyExchange {
                user: "what changed?".to_owned(),
                assistant: "I updated the tests.".to_owned(),
            }]
        );
    }

    #[test]
    fn sanitize_strips_common_wrappers() {
        assert_eq!(
            sanitize_suggestion(
                "\"User: Can you run the tests?\"",
                &SuggestionContext::Reply(ReplySuggestionContext { exchanges: vec![] })
            ),
            Some("Can you run the tests?".to_owned())
        );
        assert_eq!(
            sanitize_suggestion(
                "",
                &SuggestionContext::Reply(ReplySuggestionContext { exchanges: vec![] })
            ),
            None
        );
    }

    #[test]
    fn terminal_suggestion_must_keep_prefix() {
        let context = SuggestionContext::TerminalCommand(
            TerminalSuggestionContext::new("git st".to_owned(), None).unwrap(),
        );
        assert_eq!(
            sanitize_suggestion("git status --short", &context),
            Some("git status --short".to_owned())
        );
        assert_eq!(sanitize_suggestion("cargo test", &context), None);
    }

    #[test]
    fn inherit_provider_resolves_to_chat_provider() {
        let request = SuggestionRequest {
            config: AgentSuggestionActionConfig {
                provider: AgentActionProvider::Inherit,
                model: None,
                effort: None,
            },
            chat_provider: CLIAgent::Claude,
            api_key: None,
            cwd: PathBuf::new(),
            path_env: None,
            context: SuggestionContext::Reply(ReplySuggestionContext {
                exchanges: vec![ReplyExchange {
                    user: "hi".to_owned(),
                    assistant: "hello".to_owned(),
                }],
            }),
        };
        assert_eq!(
            request.config.provider.resolve(request.chat_provider),
            CLIAgent::Claude
        );
    }
}
