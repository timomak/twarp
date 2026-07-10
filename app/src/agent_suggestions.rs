use std::{future::Future, path::PathBuf, pin::Pin, time::Duration};

use claude_code::{Transcript, TranscriptItem};
use command::{Stdio, blocking::Command};
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
    pub context: ReplySuggestionContext,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplySuggestionContext {
    exchanges: Vec<ReplyExchange>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ReplyExchange {
    user: String,
    assistant: String,
}

pub struct DefaultSuggestionProvider;

impl SuggestionProvider for DefaultSuggestionProvider {
    fn suggest(&self, request: SuggestionRequest) -> SuggestionFuture {
        Box::pin(async move { suggest_reply(request).await })
    }
}

impl ReplySuggestionContext {
    pub fn from_transcript(transcript: &Transcript) -> Option<Self> {
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

        if exchanges.is_empty() {
            return None;
        }

        exchanges.reverse();
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

async fn suggest_reply(request: SuggestionRequest) -> Option<String> {
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
    sanitize_suggestion(&suggestion)
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

fn sanitize_suggestion(suggestion: &str) -> Option<String> {
    let mut suggestion = suggestion.trim().trim_matches('"').trim().to_owned();
    if let Some(stripped) = suggestion.strip_prefix("User:") {
        suggestion = stripped.trim().to_owned();
    }
    if let Some(stripped) = suggestion.strip_prefix("Next user message:") {
        suggestion = stripped.trim().to_owned();
    }
    suggestion = suggestion.lines().next().unwrap_or("").trim().to_owned();
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
            sanitize_suggestion("\"User: Can you run the tests?\""),
            Some("Can you run the tests?".to_owned())
        );
        assert_eq!(sanitize_suggestion(""), None);
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
            context: ReplySuggestionContext {
                exchanges: vec![ReplyExchange {
                    user: "hi".to_owned(),
                    assistant: "hello".to_owned(),
                }],
            },
        };
        assert_eq!(
            request.config.provider.resolve(request.chat_provider),
            CLIAgent::Claude
        );
    }
}
