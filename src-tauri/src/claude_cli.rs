// Chat through the Claude Code CLI the user is already signed in to — no API
// key anywhere. One headless `claude -p` process in stream-json mode carries
// the conversation: each turn is a line on its stdin, answered by a result
// event. If it dies or must restart, the next one resumes the same session.
//
// The CLI runs in --restricted mode: no tool that runs commands or writes,
// read-only file tools confined to the inbox (where dropped files land) and
// the folders of live coding sessions, and the user's own settings files are
// not loaded — their hooks included, so the chat never shows up in the island
// as if it were a coding session.
//
// The one way the chat reaches beyond reading: when the user asks it to, it
// passes an instruction to one of their other Claude Code sessions (ListAgents
// + SendMessage). That session then works under its own permission settings.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};

use crate::claude::{ChatContext, ChatReply};
use crate::{files, platform, settings};

/// Model alias handed to `--model`. Sonnet answers fast and well without eating
/// through the plan the way Opus does.
pub const DEFAULT_MODEL: &str = "sonnet";

/// A web search can take a while; a stuck CLI must not hold the chat forever.
const TIMEOUT: Duration = Duration::from_secs(180);

/// Read for dropped files (PDFs and images included) and, with Glob and Grep,
/// the project folders of live sessions; the web for the rest; ListAgents and
/// SendMessage to hand an instruction to another session. Nothing that writes
/// a file or runs a command.
const TOOLS: &str = "Read,Glob,Grep,WebSearch,WebFetch,ListAgents,SendMessage";

/// Replaces Claude Code's own coding-agent prompt: shorter, so every turn costs
/// less of the plan, and written for a chat at the top of the screen.
const SYSTEM_PROMPT: &str = "You are Frank, a personal AI assistant living at the top of the user's screen. \
You can search the web and help with anything: research, coding, recommendations, questions. \
Messages may start with a briefing on the coding sessions running on this computer; \
when the user asks how a project is going, answer from it and read the project's files if needed. \
You can also pass instructions to the user's other Claude Code sessions, in whatever folder or project the \
user names. When the user explicitly asks you to have a folder, project or session do something, call \
ListAgents, pick the session that matches that folder (if several match, the most recently started one), \
and send it one clear, self-contained instruction with SendMessage, written as the user's request in the user's \
language. ListAgents names a session after the folder it was started in (spaces become dashes, plus a short \
suffix), and that folder can be a parent of the one the briefing shows it working in: a session named \
my-shop-8c that the briefing shows in C:\\Users\\me\\Projects\\my shop\\web\\frontend is that \
session. So match on any folder along the briefing's path, not just the last one. Then tell the user in one \
sentence which session you sent it to and what you asked. Only if no session in ListAgents matches at all, say \
so and ask the user to open Claude Code in that folder; the briefing's list of project folders helps you tell \
which folder they mean. Never message another session unless the user asked you to. \
Always answer in the language of the user's own words (Turkish, Russian or any other), \
never in the language of the briefing or of these instructions. Be concise. \
No markdown formatting (no **, no ##, no bullet dashes). Use plain text with line breaks.";

/// The chat's Claude Code process, kept running between turns: starting the
/// CLI costs more than a short answer takes (about 1.7 s here), so each turn
/// is one more line on its stdin rather than a fresh process.
struct Live {
    child: tokio::process::Child,
    stdin: tokio::process::ChildStdin,
    lines: Lines<BufReader<tokio::process::ChildStdout>>,
    /// What the process was started with. A turn that needs something else
    /// restarts it, resuming the same conversation.
    model: String,
    dirs: Vec<PathBuf>,
}

#[derive(Default)]
pub struct CliChat {
    live: tokio::sync::Mutex<Option<Live>>,
    /// The CLI session to resume; None until the first answer arrives.
    session: Mutex<Option<String>>,
    /// Set by reset(): the next turn starts a new process and conversation.
    fresh: AtomicBool,
}

impl CliChat {
    pub fn reset(&self) {
        *self.session.lock().unwrap() = None;
        self.fresh.store(true, Ordering::Relaxed);
    }
}

/// One chat turn. Returns the assistant's text, or a message the island shows
/// in the note view. `read_dirs` are project folders the chat may read, on top
/// of the inbox; restricted mode keeps the file tools inside them.
pub async fn send(
    chat: &CliChat,
    model: &str,
    query: String,
    context: Option<ChatContext>,
    read_dirs: &[PathBuf],
) -> Result<ChatReply, String> {
    let exe = claude_exe()
        .ok_or_else(|| "Claude Code not found. Install it, then sign in once with `claude`.".to_string())?;

    // The CLI names a session after its working folder, and that name is what
    // other sessions see on a message from the chat — so it runs in "frank".
    // Dropped files are read from the inbox, opened up beside it.
    let dir = settings::local_dir().join("frank");
    let inbox = files::inbox_dir();
    platform::ensure_private_dir(&settings::local_dir()).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&inbox).map_err(|e| e.to_string())?;

    // The island sends a file once, with the first message after it was dropped
    // or pasted — which may be mid-conversation for a pasted screenshot.
    let mut prompt = String::new();
    match &context {
        Some(ChatContext::File { name, path }) => {
            prompt.push_str(&format!(
                "The user attached a file named \"{name}\". It is at {path}. Read it before answering.\n\n"
            ));
        }
        Some(ChatContext::Window { app_name, title, url }) => {
            prompt.push_str(&format!("Context — App: {app_name}, Window: {title}"));
            if let Some(url) = url {
                prompt.push_str(&format!(", URL: {url}"));
            }
            prompt.push_str("\n\n");
        }
        None => {}
    }
    prompt.push_str(&query);

    // One turn at a time: the lock is held until this turn's result is in.
    let mut live = chat.live.lock().await;
    if chat.fresh.swap(false, Ordering::Relaxed) {
        *live = None; // kill_on_drop ends the old conversation's process
    }
    let reusable = match live.as_mut() {
        Some(l) => {
            l.model == model && l.dirs == read_dirs && matches!(l.child.try_wait(), Ok(None))
        }
        None => false,
    };
    if !reusable {
        *live = None;
        let resume = chat.session.lock().unwrap().clone();
        *live = Some(start(&exe, &dir, &inbox, model, read_dirs, resume.as_deref())?);
    }
    let Some(process) = live.as_mut() else { return Err("Claude Code did not start.".into()) };

    // The prompt goes in on stdin, as a stream-json user message: no quoting
    // rules to get wrong, whatever the user typed.
    let message = json!({
        "type": "user",
        "message": { "role": "user", "content": [{ "type": "text", "text": prompt }] },
    });
    let turn = async {
        process
            .stdin
            .write_all(format!("{message}\n").as_bytes())
            .await
            .map_err(|e| format!("Could not talk to Claude Code: {e}"))?;
        process.stdin.flush().await.map_err(|e| format!("Could not talk to Claude Code: {e}"))?;
        while let Some(line) = process.lines.next_line().await.map_err(|e| e.to_string())? {
            if let Some(result) = result_event(&line) {
                return Ok(result);
            }
        }
        Err("Claude Code stopped unexpectedly.".to_string())
    };
    // A process that failed or hung is dropped (and so killed); the next turn
    // starts another and resumes the conversation.
    let result = match tokio::time::timeout(TIMEOUT, turn).await {
        Ok(Ok(result)) => result,
        Ok(Err(why)) => {
            *live = None;
            return Err(why);
        }
        Err(_) => {
            *live = None;
            return Err("Claude Code took too long to answer.".into());
        }
    };
    drop(live);

    let text = result.get("result").and_then(Value::as_str).unwrap_or("").trim().to_string();
    if result.get("is_error").and_then(Value::as_bool).unwrap_or(false) {
        // "Not logged in", a usage limit, an unknown model: the CLI's own words
        // are the most useful thing to show.
        return Err(if text.is_empty() { "Claude Code returned an error.".into() } else { text });
    }

    if let Some(id) = result.get("session_id").and_then(Value::as_str) {
        *chat.session.lock().unwrap() = Some(id.to_string());
    }
    if text.is_empty() {
        return Err("No response text.".into());
    }
    Ok(ChatReply { text })
}

/// Starts the chat's Claude Code process, resuming `resume` if given.
fn start(
    exe: &Path,
    dir: &Path,
    inbox: &Path,
    model: &str,
    read_dirs: &[PathBuf],
    resume: Option<&str>,
) -> Result<Live, String> {
    let mut cmd = std::process::Command::new(exe);
    cmd.args([
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--restricted",
        "--strict-mcp-config",
        "--permission-mode",
        "dontAsk",
        "--tools",
        TOOLS,
        "--allowedTools",
        TOOLS,
        "--system-prompt",
        SYSTEM_PROMPT,
    ]);
    if model != "default" {
        cmd.args(["--model", model]);
    }
    if let Some(id) = resume {
        cmd.args(["--resume", id]);
    }
    cmd.arg("--add-dir").arg(inbox);
    for extra in read_dirs {
        cmd.arg("--add-dir").arg(extra);
    }
    // With either of these set the CLI bills an API account instead of using
    // the claude.ai sign-in. This chat exists precisely to never do that.
    cmd.env_remove("ANTHROPIC_API_KEY").env_remove("ANTHROPIC_AUTH_TOKEN");
    cmd.current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    platform::no_console(&mut cmd);

    let mut child = tokio::process::Command::from(cmd)
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("Could not start Claude Code: {e}"))?;
    platform::tie_to_app(&child);
    let stdin = child.stdin.take().ok_or("Could not talk to Claude Code.")?;
    let stdout = child.stdout.take().ok_or("Could not talk to Claude Code.")?;
    Ok(Live {
        child,
        stdin,
        lines: BufReader::new(stdout).lines(),
        model: model.to_string(),
        dirs: read_dirs.to_vec(),
    })
}

/// The `{"type":"result",…}` event that ends a turn, among the stream-json
/// events the CLI prints along the way.
fn result_event(line: &str) -> Option<Value> {
    let event: Value = serde_json::from_str(line.trim()).ok()?;
    (event.get("type").and_then(Value::as_str) == Some("result")).then_some(event)
}

/// Where the `claude` CLI is.
///
/// npm puts a `claude.cmd` shim on PATH that only starts the native exe beside
/// its node_modules. Starting that exe directly skips cmd.exe, whose argument
/// rules a system prompt has no business going through, and lets a timeout end
/// the real process rather than just the shim.
fn claude_exe() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(found) = platform::find_on_path("claude") {
        if let Some(dir) = found.parent() {
            candidates.push(dir.join("node_modules/@anthropic-ai/claude-code/bin/claude.exe"));
        }
        candidates.push(found);
    }
    // An app started from the Start menu may not see the PATH a terminal does.
    #[cfg(windows)]
    {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            let npm = PathBuf::from(appdata).join("npm");
            candidates.push(npm.join("node_modules/@anthropic-ai/claude-code/bin/claude.exe"));
            candidates.push(npm.join("claude.cmd"));
        }
        candidates.push(platform::home_dir().join(".local/bin/claude.exe"));
    }
    candidates.into_iter().find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::result_event;

    #[test]
    fn only_the_result_event_ends_a_turn() {
        let result = r#"{"type":"result","is_error":false,"result":"hi","session_id":"s1"}"#;
        assert_eq!(result_event(result).unwrap()["result"], "hi");
        assert!(result_event(r#"{"type":"assistant","message":{}}"#).is_none());
        assert!(result_event(r#"{"type":"system","subtype":"init"}"#).is_none());
        assert!(result_event("").is_none());
        assert!(result_event("not json").is_none());
    }
}
