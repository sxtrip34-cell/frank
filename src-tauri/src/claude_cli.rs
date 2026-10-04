// Chat through the Claude Code CLI the user is already signed in to — no API
// key anywhere. One headless `claude -p` process in stream-json mode carries
// the conversation: each turn is a line on its stdin, answered by a result
// event. If it dies or must restart, the next one resumes the same session.
//
// The CLI runs in --restricted mode: no tool that runs commands, file tools
// confined to its own folder (C:\Frank), the inbox (where dropped files land)
// and the project folders Claude Code works in, and the user's own settings
// files are not loaded — their hooks included, so the chat never shows up in
// the island as if it were a coding session.
//
// It writes in exactly one place: its memory, C:\Frank\memory. Writing is
// allowed by a single Edit(memory/**) rule and the permission mode refuses
// everything else, so the project folders it reads stay read-only.
//
// The one way the chat reaches further: when the user asks it to, it passes an
// instruction to one of their other Claude Code sessions (ListAgents +
// SendMessage). That session then works under its own permission settings.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};

use crate::claude::{ChatContext, ChatReply};
use crate::{files, office, platform, settings, tools};

/// Model alias handed to `--model`. Sonnet answers fast and well without eating
/// through the plan the way Opus does.
pub const DEFAULT_MODEL: &str = "sonnet";

/// A web search can take a while; a stuck CLI must not hold the chat forever.
const TIMEOUT: Duration = Duration::from_secs(180);

/// Read for dropped files (PDFs and images included) and, with Glob and Grep,
/// the project folders; Write and Edit for the memory; the web for the rest;
/// ListAgents and SendMessage to hand an instruction to another session.
/// Nothing that runs a command.
const TOOLS: &str = "Read,Glob,Grep,Write,Edit,WebSearch,WebFetch,ListAgents,SendMessage";

/// What runs without asking — and with --permission-mode dontAsk, nothing else
/// runs at all. Edit(path) rules cover every file-writing tool, and a relative
/// path is taken from the working folder: writes go to C:\Frank\memory only.
const ALLOWED: &str = "Read,Glob,Grep,Edit(memory/**),WebSearch,WebFetch,ListAgents,SendMessage";

/// The connected services (tools.rs), and what the chat may reach at all.
const SERVICES_PROMPT: &str = "Through your frank tools you can use the services the user connected to \
Frank: GitHub, Vercel, Stripe, Resend, Notion, Cal.com and n8n. Connected right now: {connected}. Use them \
whenever the user asks about these services. A service that is not connected says so when you call it: \
tell the user they can add its key in Frank's Settings, Integrations. Reading is immediate. An action \
(opening an issue or a comment, sending an email, creating or adding to a Notion page, starting or \
switching an n8n workflow) only happens after the user presses Allow on Frank's screen: call the tool \
with exactly what you mean to do, and if it comes back declined, say so and do not try again. Stripe is \
read-only. You work on this computer only: besides the web and these services, never reach anything \
elsewhere, and message only the Claude Code sessions ListAgents shows on this computer, never a cloud \
session or one on another machine.";

/// The memory index past this is cut short in the prompt; the notes it points
/// to can always be read in full.
const MAX_MEMORY_INDEX: usize = 8 * 1024;

/// How the chat keeps its memory, added to the system prompt with the index.
const MEMORY_PROMPT: &str = "You have a memory that lasts from one conversation to the next: the memory \
folder in your working folder ({memory}). memory/MEMORY.md is its index, shown below. Save a memory when \
the user tells you something about themselves, the people and projects in their life, their preferences \
or plans, or asks you to remember something: write one short Markdown note per fact in the memory folder, \
named for it (for example memory/sister-birthday.md), then add a one-line pointer to it in \
memory/MEMORY.md. Check the index first and update a note rather than making a second one; correct or \
delete a note that turns out wrong. Read a note when it bears on what the user asks. Never save \
passwords, keys, card numbers or other secrets. When the user asks what you remember, answer from the \
index and the notes. Saving notes is the only writing you can do: every other file on this computer is \
read-only for you.";

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
    // other sessions see on a message from the chat — so it runs in "Frank",
    // C:\Frank, where its memory lives. Dropped files are read from the inbox,
    // opened up beside it.
    let dir = platform::frank_home();
    let inbox = files::inbox_dir();
    platform::ensure_private_dir(&settings::local_dir()).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(dir.join("memory"))
        .map_err(|e| format!("Could not create Frank's folder {}: {e}", dir.display()))?;
    std::fs::create_dir_all(&inbox).map_err(|e| e.to_string())?;

    // The island sends a file once, with the first message after it was dropped
    // or pasted — which may be mid-conversation for a pasted screenshot.
    let mut prompt = String::new();
    match &context {
        // A Word document is handed over as the text pulled out of it: the
        // Read tool takes text, images and PDFs, not .docx.
        Some(ChatContext::File { name, path }) => match office::readable_copy(Path::new(path)) {
            Some(Ok(text)) => prompt.push_str(&format!(
                "The user attached a Word document named \"{name}\" (at {path}). Its text has been \
                 extracted to {}. Read that file before answering.\n\n",
                text.display()
            )),
            Some(Err(err)) => prompt.push_str(&format!(
                "The user attached a file named \"{name}\" at {path}, but its text could not be read \
                 ({err}). Tell them so, and ask for a PDF or a .docx instead.\n\n"
            )),
            None => prompt.push_str(&format!(
                "The user attached a file named \"{name}\". It is at {path}. Read it before answering.\n\n"
            )),
        },
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
    ]);
    // Frank's own tool server, and only it: --strict-mcp-config keeps every
    // other MCP configuration out, the Claude account's connectors included.
    // Its tools are allowed as a whole; the actions among them ask the user
    // themselves, on the island, before they do anything.
    if tools::ready() {
        cmd.arg("--mcp-config").arg(tools::config_file());
        cmd.arg("--allowedTools").arg(format!("{ALLOWED},mcp__{}", tools::SERVER_NAME));
    } else {
        cmd.arg("--allowedTools").arg(ALLOWED);
    }
    cmd.arg("--system-prompt").arg(system_prompt(&dir.join("memory")));
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

/// The chat's instructions, its memory rules, and the memory index as it is
/// now: read again whenever the process starts, so a conversation begins with
/// everything saved in the ones before.
fn system_prompt(memory: &Path) -> String {
    let index = std::fs::read_to_string(memory.join("MEMORY.md")).unwrap_or_default();
    let index = index.trim();
    let index = if index.is_empty() {
        "(empty: nothing saved yet)".to_string()
    } else if index.len() > MAX_MEMORY_INDEX {
        let mut cut = MAX_MEMORY_INDEX;
        while !index.is_char_boundary(cut) {
            cut -= 1;
        }
        format!("{}\n(cut short here: read memory/MEMORY.md for the rest)", &index[..cut])
    } else {
        index.to_string()
    };
    let rules = MEMORY_PROMPT.replace("{memory}", &memory.display().to_string());
    let connected = tools::connected();
    let services = SERVICES_PROMPT.replace(
        "{connected}",
        &if connected.is_empty() { "none yet".to_string() } else { connected.join(", ") },
    );
    format!("{SYSTEM_PROMPT}\n\n{services}\n\n{rules}\n\nmemory/MEMORY.md now:\n{index}")
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
    use super::{result_event, system_prompt, MAX_MEMORY_INDEX};

    #[test]
    fn the_memory_index_rides_in_the_system_prompt() {
        let dir = std::env::temp_dir().join(format!("frank-memory-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(system_prompt(&dir).ends_with("(empty: nothing saved yet)"));

        std::fs::write(dir.join("MEMORY.md"), "- [Sister](sister.md) — birthday 12 May\n").unwrap();
        let prompt = system_prompt(&dir);
        assert!(prompt.contains("birthday 12 May"));
        assert!(prompt.contains(&*dir.display().to_string()), "the rules name the real folder");

        // A huge index is cut on a character boundary, never mid-letter.
        std::fs::write(dir.join("MEMORY.md"), "ş".repeat(MAX_MEMORY_INDEX)).unwrap();
        assert!(system_prompt(&dir).contains("(cut short here"));
        let _ = std::fs::remove_dir_all(&dir);
    }

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
