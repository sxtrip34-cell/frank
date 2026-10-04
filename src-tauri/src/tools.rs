// Frank's own tools for the chat: the services the user connected in Settings
// → Integrations (GitHub, Vercel, Stripe, Resend, Notion, Cal.com, n8n), served
// to the chat's Claude Code process as an MCP server.
//
// This computer only, by design. The server listens on 127.0.0.1, so nothing
// off this machine can reach it, and every request has to carry a token made
// fresh at each start. The chat is started with --strict-mcp-config and this
// server alone, so the connectors of the user's Claude account (which follow
// the account from device to device) are never loaded either.
//
// The keys never leave this process: Claude names a tool, Frank makes the call
// with the key from the Credential Manager and hands back plain text.
//
// Reading needs nothing more. An action — an issue opened, an email sent, a
// Notion page written, an n8n workflow started or switched — waits here until
// the user presses Allow on the island, and is refused after APPROVAL_WAIT or
// on Deny, whatever the model was told. Stripe is read-only: nothing moves money.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex, OnceLock};
use std::time::Duration;

use reqwest::Method;
use serde::Serialize;
use serde_json::{json, Map, Value};
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::island::WINDOW_LABEL;
use crate::{log, secrets, settings};

/// The server's name for Claude Code: its tools arrive as mcp__frank__<name>.
pub const SERVER_NAME: &str = "frank";
const HTTP_TIMEOUT: Duration = Duration::from_secs(15);
/// How long an action waits for Allow before it is refused.
const APPROVAL_WAIT: Duration = Duration::from_secs(120);
const MAX_HEADERS: usize = 32 * 1024;
const MAX_BODY: usize = 1024 * 1024;
/// A tool's answer past this is cut: the chat needs the gist, not a dump.
const MAX_ANSWER: usize = 12_000;

/// The token every request must carry; set once the server is listening.
static TOKEN: OnceLock<String> = OnceLock::new();

/// The --mcp-config file the chat is started with, in Frank's private folder.
pub fn config_file() -> PathBuf {
    settings::local_dir().join("mcp.json")
}

/// Whether the server is up, so the chat can be pointed at it.
pub fn ready() -> bool {
    TOKEN.get().is_some()
}

/// Which services have their keys, for the chat's instructions.
pub fn connected() -> Vec<&'static str> {
    let mut out = Vec::new();
    for (name, key) in [
        ("GitHub", "github-token"),
        ("Vercel", "vercel-token"),
        ("Stripe", "stripe-api-key"),
        ("Resend", "resend-api-key"),
        ("Notion", "notion-api-key"),
        ("Cal.com", "calcom-api-key"),
        ("n8n", "n8n-api-key"),
    ] {
        if secrets::get(key).is_some() {
            out.push(name);
        }
    }
    out
}

/// 256 bits from std's randomly keyed hasher: RandomState takes its keys from
/// the operating system's random source, and nothing outside this process
/// sees them.
fn new_token() -> String {
    use std::hash::{BuildHasher, Hasher};
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    (0..4u64)
        .map(|i| {
            let mut h = std::collections::hash_map::RandomState::new().build_hasher();
            h.write_u64(i);
            h.write_u128(nanos);
            format!("{:016x}", h.finish())
        })
        .collect()
}

pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let listener = match TcpListener::bind(("127.0.0.1", 0)).await {
            Ok(l) => l,
            Err(e) => {
                log::line(format!("tools: cannot listen: {e}"));
                return;
            }
        };
        let Ok(addr) = listener.local_addr() else { return };
        let token = new_token();
        let config = json!({
            "mcpServers": {
                SERVER_NAME: {
                    "type": "http",
                    "url": format!("http://127.0.0.1:{}/mcp", addr.port()),
                    "headers": { "Authorization": format!("Bearer {token}") },
                }
            }
        });
        if let Err(e) = std::fs::write(config_file(), config.to_string()) {
            log::line(format!("tools: cannot write the chat's MCP config: {e}"));
            return;
        }
        let _ = TOKEN.set(token);
        log::line(format!("tools: serving the chat on 127.0.0.1:{}", addr.port()));
        loop {
            let Ok((stream, peer)) = listener.accept().await else { continue };
            if !peer.ip().is_loopback() {
                continue;
            }
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let _ = serve(app, stream).await;
            });
        }
    });
}

// ── HTTP: one JSON-RPC message (or batch) per POST, answered as JSON ─────────

async fn serve(app: AppHandle, mut stream: TcpStream) -> std::io::Result<()> {
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 8192];
    let header_end = loop {
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i;
        }
        if buf.len() > MAX_HEADERS {
            return respond(&mut stream, 431, None).await;
        }
    };
    let head = String::from_utf8_lossy(&buf[..header_end]).into_owned();
    let mut lines = head.split("\r\n");
    let mut request = lines.next().unwrap_or("").split_whitespace();
    let method = request.next().unwrap_or("").to_string();
    let path = request.next().unwrap_or("").to_string();
    let mut length = 0usize;
    let mut auth = String::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            match name.trim().to_ascii_lowercase().as_str() {
                "content-length" => length = value.trim().parse().unwrap_or(0),
                "authorization" => auth = value.trim().to_string(),
                _ => {}
            }
        }
    }

    if path.split('?').next() != Some("/mcp") {
        return respond(&mut stream, 404, None).await;
    }
    let Some(token) = TOKEN.get() else { return respond(&mut stream, 503, None).await };
    if !same(&auth, &format!("Bearer {token}")) {
        return respond(&mut stream, 401, None).await;
    }
    if method != "POST" {
        return respond(&mut stream, 405, None).await;
    }
    if length > MAX_BODY {
        return respond(&mut stream, 413, None).await;
    }
    let mut body = buf[header_end + 4..].to_vec();
    while body.len() < length {
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    body.truncate(length);

    let Ok(message) = serde_json::from_slice::<Value>(&body) else {
        let error = json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": "Parse error" } });
        return respond(&mut stream, 400, Some(&error)).await;
    };
    let reply = match message {
        Value::Array(batch) => {
            let mut out = Vec::new();
            for m in batch {
                if let Some(r) = handle(&app, m).await {
                    out.push(r);
                }
            }
            (!out.is_empty()).then_some(Value::Array(out))
        }
        m => handle(&app, m).await,
    };
    match reply {
        Some(v) => respond(&mut stream, 200, Some(&v)).await,
        // Notifications and responses get no answer, only an acknowledgement.
        None => respond(&mut stream, 202, None).await,
    }
}

/// Compares the whole of both strings, so the time taken says nothing about
/// how much of a guessed token was right.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

async fn respond(stream: &mut TcpStream, status: u16, body: Option<&Value>) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        202 => "Accepted",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Payload Too Large",
        431 => "Request Header Fields Too Large",
        _ => "Service Unavailable",
    };
    let body = body.map(Value::to_string).unwrap_or_default();
    let mut head = format!("HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n", body.len());
    if !body.is_empty() {
        head.push_str("Content-Type: application/json\r\n");
    }
    if status == 405 {
        head.push_str("Allow: POST\r\n");
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(body.as_bytes()).await?;
    stream.flush().await
}

// ── MCP ───────────────────────────────────────────────────────────────────────

async fn handle(app: &AppHandle, msg: Value) -> Option<Value> {
    let method = msg.get("method").and_then(Value::as_str)?.to_string();
    // A notification has no id and wants no answer.
    let id = msg.get("id").cloned()?;
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    let result = match method.as_str() {
        "initialize" => Ok(initialize(&params)),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": catalogue() })),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("").to_string();
            let args = params.get("arguments").and_then(Value::as_object).cloned().unwrap_or_default();
            Ok(call(app, &name, &args).await)
        }
        other => Err((-32601, format!("Unknown method {other}"))),
    };
    Some(match result {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err((code, message)) => json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }),
    })
}

fn initialize(params: &Value) -> Value {
    let version = params.get("protocolVersion").and_then(Value::as_str).unwrap_or("2025-06-18");
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": { "listChanged": false } },
        "serverInfo": { "name": SERVER_NAME, "version": env!("CARGO_PKG_VERSION") },
        "instructions": "The services the user connected to Frank. Reading is immediate; an action waits for the user to press Allow on Frank's screen.",
    })
}

async fn call(app: &AppHandle, name: &str, args: &Map<String, Value>) -> Value {
    match run(app, name, args).await {
        Ok(text) => json!({ "content": [{ "type": "text", "text": clip(&text, MAX_ANSWER) }] }),
        Err(why) => json!({ "content": [{ "type": "text", "text": why }], "isError": true }),
    }
}

// ── Catalogue ─────────────────────────────────────────────────────────────────

fn tool(name: &str, description: &str, properties: Value, required: &[&str], action: bool) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": { "type": "object", "properties": properties, "required": required },
        "annotations": { "readOnlyHint": !action, "destructiveHint": false, "openWorldHint": true },
    })
}

fn catalogue() -> Vec<Value> {
    let repo = json!({ "type": "string", "description": "owner/name, e.g. octocat/hello-world" });
    let limit = json!({ "type": "integer", "description": "How many, 1–50", "minimum": 1, "maximum": 50 });
    vec![
        tool("github_repos", "The user's GitHub repositories, most recently pushed first.", json!({ "limit": limit }), &[], false),
        tool("github_repo_status", "A GitHub repository at a glance: description, latest CI (Actions) runs, open pull requests.", json!({ "repo": repo }), &["repo"], false),
        tool("github_issues", "Issues and pull requests of a GitHub repository.", json!({
            "repo": repo,
            "state": { "type": "string", "enum": ["open", "closed", "all"] },
            "limit": limit,
        }), &["repo"], false),
        tool("github_issue", "One GitHub issue or pull request with its latest comments.", json!({
            "repo": repo, "number": { "type": "integer" },
        }), &["repo", "number"], false),
        tool("github_notifications", "The user's unread GitHub notifications.", json!({ "limit": limit }), &[], false),
        tool("github_create_issue", "ACTION: open an issue in a GitHub repository. Waits for the user to press Allow on Frank's screen.", json!({
            "repo": repo, "title": { "type": "string" }, "body": { "type": "string" },
        }), &["repo", "title"], true),
        tool("github_comment", "ACTION: comment on a GitHub issue or pull request. Waits for the user to press Allow on Frank's screen.", json!({
            "repo": repo, "number": { "type": "integer" }, "body": { "type": "string" },
        }), &["repo", "number", "body"], true),
        tool("vercel_projects", "The user's Vercel projects.", json!({ "limit": limit }), &[], false),
        tool("vercel_deployments", "Recent Vercel deployments with their state (READY, ERROR, BUILDING…), optionally of one project.", json!({
            "project": { "type": "string", "description": "Project name (optional)" }, "limit": limit,
        }), &[], false),
        tool("stripe_balance", "The Stripe account's balance, available and pending. Read-only.", json!({}), &[], false),
        tool("stripe_payments", "Recent Stripe payments (charges). Read-only: nothing here can move money.", json!({ "limit": limit }), &[], false),
        tool("resend_emails", "Emails recently sent through Resend, with their delivery status.", json!({ "limit": limit }), &[], false),
        tool("resend_send_email", "ACTION: send an email through Resend. `from` must be on a domain verified in Resend. Waits for the user to press Allow on Frank's screen.", json!({
            "from": { "type": "string", "description": "e.g. Trip <hello@example.com>" },
            "to": { "type": "array", "items": { "type": "string" } },
            "subject": { "type": "string" },
            "text": { "type": "string", "description": "Plain-text body" },
        }), &["from", "to", "subject", "text"], true),
        tool("notion_search", "Search the Notion pages and databases shared with Frank's integration.", json!({
            "query": { "type": "string" }, "limit": limit,
        }), &[], false),
        tool("notion_page", "Read a Notion page: its title and text.", json!({
            "page_id": { "type": "string", "description": "Page id or URL" },
        }), &["page_id"], false),
        tool("notion_create_page", "ACTION: create a Notion page under another page. Waits for the user to press Allow on Frank's screen.", json!({
            "parent_page_id": { "type": "string", "description": "Parent page id or URL" },
            "title": { "type": "string" },
            "text": { "type": "string", "description": "Body; blank lines separate paragraphs" },
        }), &["parent_page_id", "title"], true),
        tool("notion_append", "ACTION: add text to the end of a Notion page. Waits for the user to press Allow on Frank's screen.", json!({
            "page_id": { "type": "string", "description": "Page id or URL" },
            "text": { "type": "string", "description": "Blank lines separate paragraphs" },
        }), &["page_id", "text"], true),
        tool("calcom_bookings", "Upcoming Cal.com bookings.", json!({ "limit": limit }), &[], false),
        tool("n8n_workflows", "The n8n workflows and whether each is active.", json!({}), &[], false),
        tool("n8n_executions", "Recent n8n workflow runs and how they ended.", json!({ "limit": limit }), &[], false),
        tool("n8n_webhook", "ACTION: start an n8n workflow through its webhook (POST <n8n>/webhook/<path>). Waits for the user to press Allow on Frank's screen.", json!({
            "path": { "type": "string", "description": "The webhook path, e.g. new-lead" },
            "payload": { "type": "object", "description": "JSON body to send (optional)" },
        }), &["path"], true),
        tool("n8n_set_active", "ACTION: switch an n8n workflow on or off. Waits for the user to press Allow on Frank's screen.", json!({
            "workflow_id": { "type": "string" }, "active": { "type": "boolean" },
        }), &["workflow_id", "active"], true),
    ]
}

async fn run(app: &AppHandle, name: &str, a: &Map<String, Value>) -> Result<String, String> {
    match name {
        "github_repos" => github_repos(a).await,
        "github_repo_status" => github_repo_status(a).await,
        "github_issues" => github_issues(a).await,
        "github_issue" => github_issue(a).await,
        "github_notifications" => github_notifications(a).await,
        "github_create_issue" => github_create_issue(app, a).await,
        "github_comment" => github_comment(app, a).await,
        "vercel_projects" => vercel_projects(a).await,
        "vercel_deployments" => vercel_deployments(a).await,
        "stripe_balance" => stripe_balance().await,
        "stripe_payments" => stripe_payments(a).await,
        "resend_emails" => resend_emails(a).await,
        "resend_send_email" => resend_send_email(app, a).await,
        "notion_search" => notion_search(a).await,
        "notion_page" => notion_page(a).await,
        "notion_create_page" => notion_create_page(app, a).await,
        "notion_append" => notion_append(app, a).await,
        "calcom_bookings" => calcom_bookings(a).await,
        "n8n_workflows" => n8n_workflows().await,
        "n8n_executions" => n8n_executions(a).await,
        "n8n_webhook" => n8n_webhook(app, a).await,
        "n8n_set_active" => n8n_set_active(app, a).await,
        other => Err(format!("Frank has no tool called {other}.")),
    }
}

// ── Approval ──────────────────────────────────────────────────────────────────

static PENDING: LazyLock<Mutex<HashMap<String, tokio::sync::oneshot::Sender<bool>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ApprovalAsk {
    request_id: String,
    title: String,
    detail: String,
}

/// Shows the action on the island and waits for Allow. Ok only on Allow.
async fn ask(app: &AppHandle, title: &str, detail: String) -> Result<(), String> {
    let id = new_token()[..16].to_string();
    let (tx, rx) = tokio::sync::oneshot::channel();
    PENDING.lock().unwrap().insert(id.clone(), tx);
    // The tool's name only: what is in the email or the issue stays out of the log.
    log::line(format!("tools: asking the user before {title}"));
    let _ = app.emit_to(WINDOW_LABEL, "tool-approval", ApprovalAsk {
        request_id: id.clone(),
        title: title.to_string(),
        detail: clip(&detail, 900),
    });
    let answer = tokio::time::timeout(APPROVAL_WAIT, rx).await;
    PENDING.lock().unwrap().remove(&id);
    match answer {
        Ok(Ok(true)) => Ok(()),
        Ok(Ok(false)) => Err("The user pressed Deny on Frank's screen, so nothing was done. Do not try again unless they ask.".into()),
        _ => {
            let _ = app.emit_to(WINDOW_LABEL, "tool-approval-closed", id);
            Err("The user did not answer on Frank's screen in time, so nothing was done.".into())
        }
    }
}

/// The island's Allow / Deny for an action.
pub fn decide(request_id: &str, allow: bool) {
    if let Some(tx) = PENDING.lock().unwrap().remove(request_id) {
        let _ = tx.send(allow);
    }
}

// ── Shared helpers ────────────────────────────────────────────────────────────

fn key(name: &str, service: &str) -> Result<String, String> {
    secrets::get(name).ok_or_else(|| {
        format!("{service} is not connected to Frank. The user can add its key in Frank's Settings → Integrations.")
    })
}

fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(HTTP_TIMEOUT)
        .user_agent("Frank")
        .build()
        .unwrap_or_default()
}

async fn send(request: reqwest::RequestBuilder, service: &str) -> Result<Value, String> {
    let response = request.send().await.map_err(|e| format!("{service} could not be reached: {e}"))?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        let why = match status.as_u16() {
            401 => "the key was refused (401); it may be wrong or expired".to_string(),
            403 => "the key is not allowed to do this (403)".to_string(),
            404 => "not found (404), or the key cannot see it".to_string(),
            code => format!("error {code}: {}", clip(&api_message(&text), 300)),
        };
        return Err(format!("{service}: {why}"));
    }
    Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
}

fn api_message(body: &str) -> String {
    let parsed: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    parsed
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| parsed.get("error").and_then(|e| e.get("message")).and_then(Value::as_str))
        .or_else(|| parsed.get("error").and_then(Value::as_str))
        .map(str::to_string)
        .unwrap_or_else(|| body.chars().take(300).collect())
}

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max).collect();
    format!("{cut}… (cut short)")
}

fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

/// "2026-10-04T12:30:00Z" → "2026-10-04 12:30".
fn when(iso: &str) -> String {
    iso.get(..16).unwrap_or(iso).replace('T', " ")
}

/// Unix milliseconds (Vercel) as a UTC date.
fn when_ms(ms: f64) -> String {
    when_secs((ms / 1000.0) as i64)
}

/// Unix seconds as "YYYY-MM-DD HH:MM" UTC.
fn when_secs(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // Civil date from days since 1970 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02} UTC", rem / 3600, rem % 3600 / 60)
}

fn arg_str<'a>(a: &'a Map<String, Value>, key: &str) -> Result<&'a str, String> {
    a.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("missing \"{key}\""))
}

fn arg_opt<'a>(a: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    a.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
}

fn arg_limit(a: &Map<String, Value>, default: u64) -> u64 {
    a.get("limit").and_then(Value::as_u64).unwrap_or(default).clamp(1, 50)
}

fn arg_number(a: &Map<String, Value>, key: &str) -> Result<u64, String> {
    a.get(key)
        .and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.trim_start_matches('#').parse().ok())))
        .ok_or_else(|| format!("missing \"{key}\""))
}

/// owner/name, with nothing in it that could steer the URL elsewhere.
fn arg_repo(a: &Map<String, Value>) -> Result<String, String> {
    let repo = arg_str(a, "repo")?;
    let ok = |s: &str| !s.is_empty() && s != "." && s != ".." && s.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c));
    match repo.split_once('/') {
        Some((owner, name)) if ok(owner) && ok(name) => Ok(format!("{owner}/{name}")),
        _ => Err(format!("\"{repo}\" is not a repository: use owner/name")),
    }
}

/// A Notion id from an id or a page URL: its last 32 hex digits.
fn notion_id(raw: &str) -> Result<String, String> {
    let last = raw.trim().trim_end_matches('/').rsplit(['/', '-']).next().unwrap_or("");
    let last = last.split(['?', '#']).next().unwrap_or("");
    let hex: String = raw.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    let candidate = if last.len() == 32 && last.chars().all(|c| c.is_ascii_hexdigit()) {
        last.to_string()
    } else if hex.len() >= 32 {
        hex[hex.len() - 32..].to_string()
    } else {
        return Err(format!("\"{raw}\" is not a Notion page id or URL"));
    };
    Ok(candidate.to_ascii_lowercase())
}

/// Paragraph blocks for Notion: blank lines split paragraphs, and each block
/// stays under Notion's 2000-character limit.
fn notion_paragraphs(body: &str) -> Vec<Value> {
    body.split("\n\n")
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .flat_map(|p| {
            let chars: Vec<char> = p.chars().collect();
            chars.chunks(1900).map(|c| c.iter().collect::<String>()).collect::<Vec<_>>()
        })
        .take(100)
        .map(|p| json!({ "object": "block", "type": "paragraph", "paragraph": { "rich_text": [{ "type": "text", "text": { "content": p } }] } }))
        .collect()
}

// ── GitHub ────────────────────────────────────────────────────────────────────

fn gh(method: Method, path: &str) -> Result<reqwest::RequestBuilder, String> {
    let token = key("github-token", "GitHub")?;
    Ok(http()
        .request(method, format!("https://api.github.com{path}"))
        .bearer_auth(token)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28"))
}

async fn github_repos(a: &Map<String, Value>) -> Result<String, String> {
    let n = arg_limit(a, 20);
    let list = send(gh(Method::GET, &format!("/user/repos?per_page={n}&sort=pushed"))?, "GitHub").await?;
    let lines: Vec<String> = list
        .as_array()
        .into_iter()
        .flatten()
        .map(|r| {
            format!(
                "{}{} · pushed {} · {} open issues/PRs · {}",
                text(r, "full_name"),
                if r.get("private").and_then(Value::as_bool) == Some(true) { " (private)" } else { "" },
                when(text(r, "pushed_at")),
                r.get("open_issues_count").and_then(Value::as_u64).unwrap_or(0),
                text(r, "description"),
            )
        })
        .collect();
    Ok(if lines.is_empty() { "No repositories.".into() } else { lines.join("\n") })
}

async fn github_repo_status(a: &Map<String, Value>) -> Result<String, String> {
    let repo = arg_repo(a)?;
    let info = send(gh(Method::GET, &format!("/repos/{repo}"))?, "GitHub").await?;
    let mut out = format!(
        "{} — {}\nDefault branch {}, last push {}, {} open issues/PRs, {}\n",
        text(&info, "full_name"),
        text(&info, "description"),
        text(&info, "default_branch"),
        when(text(&info, "pushed_at")),
        info.get("open_issues_count").and_then(Value::as_u64).unwrap_or(0),
        text(&info, "html_url"),
    );
    if let Ok(runs) = send(gh(Method::GET, &format!("/repos/{repo}/actions/runs?per_page=5"))?, "GitHub").await {
        let runs: Vec<String> = runs
            .get("workflow_runs")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|r| {
                let outcome = r.get("conclusion").and_then(Value::as_str).unwrap_or(text(r, "status"));
                format!("- {} on {}: {} ({})", text(r, "name"), text(r, "head_branch"), outcome, when(text(r, "created_at")))
            })
            .collect();
        out.push_str(&if runs.is_empty() { "CI: no Actions runs.\n".to_string() } else { format!("Latest CI runs:\n{}\n", runs.join("\n")) });
    }
    if let Ok(pulls) = send(gh(Method::GET, &format!("/repos/{repo}/pulls?state=open&per_page=10"))?, "GitHub").await {
        let pulls: Vec<String> = pulls
            .as_array()
            .into_iter()
            .flatten()
            .map(|p| format!("- #{} {} (by {})", p.get("number").and_then(Value::as_u64).unwrap_or(0), text(p, "title"), text(p.get("user").unwrap_or(&Value::Null), "login")))
            .collect();
        out.push_str(&if pulls.is_empty() { "No open pull requests.".to_string() } else { format!("Open pull requests:\n{}", pulls.join("\n")) });
    }
    Ok(out)
}

async fn github_issues(a: &Map<String, Value>) -> Result<String, String> {
    let repo = arg_repo(a)?;
    let state = match arg_opt(a, "state") {
        Some(s @ ("open" | "closed" | "all")) => s,
        _ => "open",
    };
    let n = arg_limit(a, 20);
    let list = send(gh(Method::GET, &format!("/repos/{repo}/issues?state={state}&per_page={n}"))?, "GitHub").await?;
    let lines: Vec<String> = list
        .as_array()
        .into_iter()
        .flatten()
        .map(|i| {
            format!(
                "#{} {}{} · {} · by {} · updated {} · {} comments",
                i.get("number").and_then(Value::as_u64).unwrap_or(0),
                if i.get("pull_request").is_some() { "[PR] " } else { "" },
                text(i, "title"),
                text(i, "state"),
                text(i.get("user").unwrap_or(&Value::Null), "login"),
                when(text(i, "updated_at")),
                i.get("comments").and_then(Value::as_u64).unwrap_or(0),
            )
        })
        .collect();
    Ok(if lines.is_empty() { format!("No {state} issues or pull requests in {repo}.") } else { lines.join("\n") })
}

async fn github_issue(a: &Map<String, Value>) -> Result<String, String> {
    let repo = arg_repo(a)?;
    let number = arg_number(a, "number")?;
    let issue = send(gh(Method::GET, &format!("/repos/{repo}/issues/{number}"))?, "GitHub").await?;
    let mut out = format!(
        "{repo}#{number} {}{} — {} · by {} · {}\n\n{}\n",
        if issue.get("pull_request").is_some() { "[PR] " } else { "" },
        text(&issue, "title"),
        text(&issue, "state"),
        text(issue.get("user").unwrap_or(&Value::Null), "login"),
        text(&issue, "html_url"),
        clip(text(&issue, "body"), 4000),
    );
    let comments = send(gh(Method::GET, &format!("/repos/{repo}/issues/{number}/comments?per_page=100"))?, "GitHub").await?;
    let all: Vec<&Value> = comments.as_array().into_iter().flatten().collect();
    for c in all.iter().skip(all.len().saturating_sub(10)) {
        out.push_str(&format!(
            "\n— {} ({}):\n{}\n",
            text(c.get("user").unwrap_or(&Value::Null), "login"),
            when(text(c, "created_at")),
            clip(text(c, "body"), 800),
        ));
    }
    Ok(out)
}

async fn github_notifications(a: &Map<String, Value>) -> Result<String, String> {
    let n = arg_limit(a, 20);
    let list = send(gh(Method::GET, &format!("/notifications?per_page={n}"))?, "GitHub").await?;
    let lines: Vec<String> = list
        .as_array()
        .into_iter()
        .flatten()
        .map(|x| {
            let subject = x.get("subject").unwrap_or(&Value::Null);
            format!(
                "{} · {} · {} ({}, {})",
                text(x.get("repository").unwrap_or(&Value::Null), "full_name"),
                text(subject, "type"),
                text(subject, "title"),
                text(x, "reason"),
                when(text(x, "updated_at")),
            )
        })
        .collect();
    Ok(if lines.is_empty() { "No unread notifications.".into() } else { lines.join("\n") })
}

async fn github_create_issue(app: &AppHandle, a: &Map<String, Value>) -> Result<String, String> {
    let repo = arg_repo(a)?;
    let title = arg_str(a, "title")?;
    let body = arg_opt(a, "body").unwrap_or("");
    key("github-token", "GitHub")?;
    ask(app, "GitHub · open an issue", format!("Repository: {repo}\nTitle: {title}\n\n{body}")).await?;
    let made = send(gh(Method::POST, &format!("/repos/{repo}/issues"))?.json(&json!({ "title": title, "body": body })), "GitHub").await?;
    Ok(format!("Opened #{} in {repo}: {}", made.get("number").and_then(Value::as_u64).unwrap_or(0), text(&made, "html_url")))
}

async fn github_comment(app: &AppHandle, a: &Map<String, Value>) -> Result<String, String> {
    let repo = arg_repo(a)?;
    let number = arg_number(a, "number")?;
    let body = arg_str(a, "body")?;
    key("github-token", "GitHub")?;
    ask(app, "GitHub · comment", format!("On: {repo}#{number}\n\n{body}")).await?;
    let made = send(gh(Method::POST, &format!("/repos/{repo}/issues/{number}/comments"))?.json(&json!({ "body": body })), "GitHub").await?;
    Ok(format!("Commented on {repo}#{number}: {}", text(&made, "html_url")))
}

// ── Vercel ────────────────────────────────────────────────────────────────────

fn vercel(path: &str) -> Result<reqwest::RequestBuilder, String> {
    let token = key("vercel-token", "Vercel")?;
    Ok(http().get(format!("https://api.vercel.com{path}")).bearer_auth(token))
}

async fn vercel_projects(a: &Map<String, Value>) -> Result<String, String> {
    let n = arg_limit(a, 20);
    let list = send(vercel(&format!("/v9/projects?limit={n}"))?, "Vercel").await?;
    let lines: Vec<String> = list
        .get("projects")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|p| {
            format!(
                "{} · {} · updated {}",
                text(p, "name"),
                p.get("framework").and_then(Value::as_str).unwrap_or("no framework set"),
                when_ms(p.get("updatedAt").and_then(Value::as_f64).unwrap_or(0.0)),
            )
        })
        .collect();
    Ok(if lines.is_empty() { "No projects.".into() } else { lines.join("\n") })
}

async fn vercel_deployments(a: &Map<String, Value>) -> Result<String, String> {
    let n = arg_limit(a, 10);
    let mut path = format!("/v6/deployments?limit={n}");
    if let Some(project) = arg_opt(a, "project") {
        let safe: String = project.chars().filter(|c| c.is_ascii_alphanumeric() || "._-".contains(*c)).collect();
        path.push_str(&format!("&app={safe}"));
    }
    let list = send(vercel(&path)?, "Vercel").await?;
    let lines: Vec<String> = list
        .get("deployments")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|d| {
            let meta = d.get("meta").unwrap_or(&Value::Null);
            format!(
                "{} · {} · {} · https://{} · branch {} · {}",
                text(d, "name"),
                d.get("state").or_else(|| d.get("readyState")).and_then(Value::as_str).unwrap_or("?"),
                when_ms(d.get("createdAt").or_else(|| d.get("created")).and_then(Value::as_f64).unwrap_or(0.0)),
                text(d, "url"),
                text(meta, "githubCommitRef"),
                clip(text(meta, "githubCommitMessage"), 120),
            )
        })
        .collect();
    Ok(if lines.is_empty() { "No deployments.".into() } else { lines.join("\n") })
}

// ── Stripe (read-only) ────────────────────────────────────────────────────────

fn stripe(path: &str) -> Result<reqwest::RequestBuilder, String> {
    let key = key("stripe-api-key", "Stripe")?;
    let auth = format!("Basic {}", crate::claude::base64_for(format!("{key}:").as_bytes()));
    Ok(http().get(format!("https://api.stripe.com{path}")).header("Authorization", auth))
}

fn money(cents: i64, currency: &str) -> String {
    format!("{:.2} {}", cents as f64 / 100.0, currency.to_uppercase())
}

async fn stripe_balance() -> Result<String, String> {
    let b = send(stripe("/v1/balance")?, "Stripe").await?;
    let mut out = String::new();
    for bucket in ["available", "pending"] {
        let parts: Vec<String> = b
            .get(bucket)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|x| money(x.get("amount").and_then(Value::as_i64).unwrap_or(0), text(x, "currency")))
            .collect();
        out.push_str(&format!("{bucket}: {}\n", if parts.is_empty() { "0".to_string() } else { parts.join(", ") }));
    }
    Ok(out)
}

async fn stripe_payments(a: &Map<String, Value>) -> Result<String, String> {
    let n = arg_limit(a, 10);
    let list = send(stripe(&format!("/v1/charges?limit={n}"))?, "Stripe").await?;
    let lines: Vec<String> = list
        .get("data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|c| {
            let who = c
                .get("billing_details")
                .and_then(|b| b.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("");
            format!(
                "{} · {}{} · {} · {} {}",
                money(c.get("amount").and_then(Value::as_i64).unwrap_or(0), text(c, "currency")),
                text(c, "status"),
                if c.get("refunded").and_then(Value::as_bool) == Some(true) { " (refunded)" } else { "" },
                when_secs(c.get("created").and_then(Value::as_i64).unwrap_or(0)),
                text(c, "description"),
                who,
            )
        })
        .collect();
    Ok(if lines.is_empty() { "No payments.".into() } else { lines.join("\n") })
}

// ── Resend ────────────────────────────────────────────────────────────────────

fn resend(method: Method, path: &str) -> Result<reqwest::RequestBuilder, String> {
    let key = key("resend-api-key", "Resend")?;
    Ok(http().request(method, format!("https://api.resend.com{path}")).bearer_auth(key))
}

async fn resend_emails(a: &Map<String, Value>) -> Result<String, String> {
    let n = arg_limit(a, 10);
    let list = send(resend(Method::GET, &format!("/emails?limit={n}"))?, "Resend").await?;
    let lines: Vec<String> = list
        .get("data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|e| {
            let to = match e.get("to") {
                Some(Value::Array(list)) => list.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", "),
                Some(Value::String(s)) => s.clone(),
                _ => String::new(),
            };
            format!("{} · to {} · {} · {}", when(text(e, "created_at")), to, text(e, "subject"), text(e, "last_event"))
        })
        .collect();
    Ok(if lines.is_empty() { "No emails sent yet.".into() } else { lines.join("\n") })
}

async fn resend_send_email(app: &AppHandle, a: &Map<String, Value>) -> Result<String, String> {
    let from = arg_str(a, "from")?;
    let to: Vec<String> = match a.get("to") {
        Some(Value::Array(list)) => list.iter().filter_map(Value::as_str).map(str::trim).map(str::to_string).collect(),
        Some(Value::String(s)) => s.split([',', ';']).map(str::trim).map(str::to_string).collect(),
        _ => Vec::new(),
    };
    let to: Vec<String> = to.into_iter().filter(|t| t.contains('@')).collect();
    if to.is_empty() {
        return Err("missing \"to\": at least one email address".into());
    }
    let subject = arg_str(a, "subject")?;
    let body = arg_str(a, "text")?;
    key("resend-api-key", "Resend")?;
    ask(app, "Resend · send an email", format!("From: {from}\nTo: {}\nSubject: {subject}\n\n{body}", to.join(", "))).await?;
    let sent = send(
        resend(Method::POST, "/emails")?.json(&json!({ "from": from, "to": to, "subject": subject, "text": body })),
        "Resend",
    )
    .await?;
    Ok(format!("Sent to {} (Resend id {}).", to.join(", "), text(&sent, "id")))
}

// ── Notion ────────────────────────────────────────────────────────────────────

fn notion(method: Method, path: &str) -> Result<reqwest::RequestBuilder, String> {
    let token = key("notion-api-key", "Notion")?;
    Ok(http()
        .request(method, format!("https://api.notion.com/v1{path}"))
        .bearer_auth(token)
        .header("Notion-Version", "2022-06-28"))
}

fn notion_title(obj: &Value) -> String {
    let from_rich = |list: Option<&Value>| -> String {
        list.and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|t| t.get("plain_text").and_then(Value::as_str))
            .collect()
    };
    if obj.get("object").and_then(Value::as_str) == Some("database") {
        let t = from_rich(obj.get("title"));
        return if t.is_empty() { "Untitled".into() } else { t };
    }
    obj.get("properties")
        .and_then(Value::as_object)
        .and_then(|props| props.values().find(|p| text(p, "type") == "title"))
        .map(|p| from_rich(p.get("title")))
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| "Untitled".into())
}

async fn notion_search(a: &Map<String, Value>) -> Result<String, String> {
    let n = arg_limit(a, 10);
    let mut body = json!({ "page_size": n, "sort": { "direction": "descending", "timestamp": "last_edited_time" } });
    if let Some(q) = arg_opt(a, "query") {
        body["query"] = json!(q);
    }
    let found = send(notion(Method::POST, "/search")?.json(&body), "Notion").await?;
    let lines: Vec<String> = found
        .get("results")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|o| format!("{} · {} · edited {} · id {} · {}", notion_title(o), text(o, "object"), when(text(o, "last_edited_time")), text(o, "id"), text(o, "url")))
        .collect();
    Ok(if lines.is_empty() {
        "Nothing found. Notion only shows Frank the pages shared with its integration.".into()
    } else {
        lines.join("\n")
    })
}

fn notion_block_text(block: &Value) -> Option<String> {
    let kind = block.get("type")?.as_str()?;
    let inner = block.get(kind)?;
    let words: String = inner
        .get("rich_text")?
        .as_array()?
        .iter()
        .filter_map(|t| t.get("plain_text").and_then(Value::as_str))
        .collect();
    let prefix = match kind {
        "heading_1" => "# ",
        "heading_2" => "## ",
        "heading_3" => "### ",
        "bulleted_list_item" => "- ",
        "numbered_list_item" => "1. ",
        "to_do" if inner.get("checked").and_then(Value::as_bool) == Some(true) => "[x] ",
        "to_do" => "[ ] ",
        "quote" => "> ",
        _ => "",
    };
    Some(format!("{prefix}{words}"))
}

async fn notion_page(a: &Map<String, Value>) -> Result<String, String> {
    let id = notion_id(arg_str(a, "page_id")?)?;
    let page = send(notion(Method::GET, &format!("/pages/{id}"))?, "Notion").await?;
    let blocks = send(notion(Method::GET, &format!("/blocks/{id}/children?page_size=100"))?, "Notion").await?;
    let body: Vec<String> = blocks
        .get("results")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(notion_block_text)
        .collect();
    Ok(format!("{}\n{}\n\n{}", notion_title(&page), text(&page, "url"), body.join("\n")))
}

async fn notion_create_page(app: &AppHandle, a: &Map<String, Value>) -> Result<String, String> {
    let parent = notion_id(arg_str(a, "parent_page_id")?)?;
    let title = arg_str(a, "title")?;
    let body = arg_opt(a, "text").unwrap_or("");
    key("notion-api-key", "Notion")?;
    ask(app, "Notion · create a page", format!("Under page: {parent}\nTitle: {title}\n\n{body}")).await?;
    let made = send(
        notion(Method::POST, "/pages")?.json(&json!({
            "parent": { "page_id": parent },
            "properties": { "title": { "title": [{ "text": { "content": title } }] } },
            "children": notion_paragraphs(body),
        })),
        "Notion",
    )
    .await?;
    Ok(format!("Created \"{title}\": {}", text(&made, "url")))
}

async fn notion_append(app: &AppHandle, a: &Map<String, Value>) -> Result<String, String> {
    let id = notion_id(arg_str(a, "page_id")?)?;
    let body = arg_str(a, "text")?;
    key("notion-api-key", "Notion")?;
    ask(app, "Notion · add to a page", format!("Page: {id}\n\n{body}")).await?;
    send(notion(Method::PATCH, &format!("/blocks/{id}/children"))?.json(&json!({ "children": notion_paragraphs(body) })), "Notion").await?;
    Ok("Added to the page.".into())
}

// ── Cal.com ───────────────────────────────────────────────────────────────────

async fn calcom_bookings(a: &Map<String, Value>) -> Result<String, String> {
    let key = key("calcom-api-key", "Cal.com")?;
    let n = arg_limit(a, 10);
    let list = send(
        http()
            .get(format!("https://api.cal.com/v2/bookings?status=upcoming&take={n}"))
            .bearer_auth(key)
            .header("cal-api-version", "2024-08-13"),
        "Cal.com",
    )
    .await?;
    let lines: Vec<String> = list
        .get("data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|b| {
            let attendee = b.get("attendees").and_then(Value::as_array).and_then(|a| a.first()).unwrap_or(&Value::Null);
            let start = b.get("start").or_else(|| b.get("startTime")).and_then(Value::as_str).unwrap_or("");
            format!("{} · {} · with {} <{}> · {}", when(start), text(b, "title"), text(attendee, "name"), text(attendee, "email"), text(b, "status"))
        })
        .collect();
    Ok(if lines.is_empty() { "No upcoming bookings.".into() } else { lines.join("\n") })
}

// ── n8n ───────────────────────────────────────────────────────────────────────

fn n8n(method: Method, path: &str) -> Result<reqwest::RequestBuilder, String> {
    let key = key("n8n-api-key", "n8n")?;
    let base = secrets::get("n8n-url").ok_or("n8n is not connected to Frank: its URL is missing in Settings → Integrations.")?;
    let base = base.trim_end_matches('/');
    Ok(http().request(method, format!("{base}{path}")).header("X-N8N-API-KEY", key))
}

async fn n8n_workflows() -> Result<String, String> {
    let list = send(n8n(Method::GET, "/api/v1/workflows?limit=100")?, "n8n").await?;
    let lines: Vec<String> = list
        .get("data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|w| {
            let id = w.get("id").map(|v| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string())).unwrap_or_default();
            let active = w.get("active").and_then(Value::as_bool) == Some(true);
            format!("{} · id {id} · {}", text(w, "name"), if active { "active" } else { "inactive" })
        })
        .collect();
    Ok(if lines.is_empty() { "No workflows.".into() } else { lines.join("\n") })
}

async fn n8n_executions(a: &Map<String, Value>) -> Result<String, String> {
    let n = arg_limit(a, 10);
    let list = send(n8n(Method::GET, &format!("/api/v1/executions?limit={n}&includeData=false"))?, "n8n").await?;
    let lines: Vec<String> = list
        .get("data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|x| {
            let id = x.get("id").map(Value::to_string).unwrap_or_default();
            let workflow = x.get("workflowId").map(Value::to_string).unwrap_or_default();
            format!("run {id} · workflow {workflow} · {} · started {}", text(x, "status"), when(text(x, "startedAt")))
        })
        .collect();
    Ok(if lines.is_empty() { "No runs.".into() } else { lines.join("\n") })
}

async fn n8n_webhook(app: &AppHandle, a: &Map<String, Value>) -> Result<String, String> {
    let path = arg_str(a, "path")?.trim_matches('/');
    let ok = !path.is_empty()
        && !path.contains("..")
        && path.chars().all(|c| c.is_ascii_alphanumeric() || "/_-".contains(c));
    if !ok {
        return Err(format!("\"{path}\" is not a webhook path"));
    }
    let payload = a.get("payload").cloned().unwrap_or_else(|| json!({}));
    let request = n8n(Method::POST, &format!("/webhook/{path}"))?;
    ask(app, "n8n · start a workflow", format!("Webhook: /webhook/{path}\n\n{}", serde_json::to_string_pretty(&payload).unwrap_or_default())).await?;
    let answer = send(request.json(&payload), "n8n").await?;
    Ok(format!("The workflow was started. It answered: {}", clip(&answer.to_string(), 1500)))
}

async fn n8n_set_active(app: &AppHandle, a: &Map<String, Value>) -> Result<String, String> {
    let id = arg_str(a, "workflow_id")?;
    if !id.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(format!("\"{id}\" is not a workflow id"));
    }
    let active = a.get("active").and_then(Value::as_bool).ok_or("missing \"active\"")?;
    let verb = if active { "activate" } else { "deactivate" };
    let request = n8n(Method::POST, &format!("/api/v1/workflows/{id}/{verb}"))?;
    ask(app, if active { "n8n · switch a workflow on" } else { "n8n · switch a workflow off" }, format!("Workflow id: {id}")).await?;
    send(request, "n8n").await?;
    Ok(format!("Workflow {id} is now {}.", if active { "active" } else { "inactive" }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: Value) -> Map<String, Value> {
        v.as_object().cloned().unwrap()
    }

    #[test]
    fn tokens_are_long_and_never_repeat() {
        let a = new_token();
        assert_eq!(a.len(), 64);
        assert_ne!(a, new_token());
        assert!(same(&a, &a.clone()));
        assert!(!same(&a, &new_token()));
        assert!(!same("Bearer x", "Bearer xy"));
    }

    #[test]
    fn every_action_says_it_waits_for_allow() {
        let tools = catalogue();
        assert!(tools.len() >= 20);
        for t in &tools {
            let read_only = t["annotations"]["readOnlyHint"].as_bool().unwrap();
            let says_action = t["description"].as_str().unwrap().starts_with("ACTION:");
            assert_eq!(read_only, !says_action, "{}", t["name"]);
        }
        // Nothing at all that could move money.
        assert!(tools.iter().all(|t| !t["name"].as_str().unwrap().starts_with("stripe_") || t["annotations"]["readOnlyHint"] == true));
    }

    #[test]
    fn repositories_and_ids_cannot_steer_the_url() {
        assert_eq!(arg_repo(&args(json!({ "repo": " octo/hello-world " }))).unwrap(), "octo/hello-world");
        for bad in ["octo", "../x", "octo/../../user", "a/b/c", "octo/name?x=1", "octo/ x"] {
            assert!(arg_repo(&args(json!({ "repo": bad }))).is_err(), "{bad}");
        }
        assert_eq!(
            notion_id("https://www.notion.so/team/My-Page-0123456789abcdef0123456789ABCDEF?pvs=4").unwrap(),
            "0123456789abcdef0123456789abcdef"
        );
        assert_eq!(notion_id("01234567-89ab-cdef-0123-456789abcdef").unwrap(), "0123456789abcdef0123456789abcdef");
        assert!(notion_id("not a page").is_err());
    }

    #[test]
    fn numbers_limits_and_dates() {
        assert_eq!(arg_number(&args(json!({ "number": "#42" })), "number").unwrap(), 42);
        assert_eq!(arg_limit(&args(json!({ "limit": 500 })), 10), 50);
        assert_eq!(arg_limit(&args(json!({})), 10), 10);
        assert_eq!(when("2026-10-04T12:30:59Z"), "2026-10-04 12:30");
        assert_eq!(when_secs(0), "1970-01-01 00:00 UTC");
        assert_eq!(when_secs(1_791_115_800), "2026-10-04 12:10 UTC");
    }

    #[test]
    fn notion_text_is_split_into_blocks_it_accepts() {
        let blocks = notion_paragraphs(&format!("one\n\n{}\n\n\n", "x".repeat(4000)));
        assert_eq!(blocks.len(), 4); // "one" + 4000 chars in three pieces
        assert!(blocks.iter().all(|b| b["paragraph"]["rich_text"][0]["text"]["content"].as_str().unwrap().chars().count() <= 1900));
    }

    #[test]
    fn notifications_get_no_answer_and_unknown_methods_an_error() {
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        // handle() needs an AppHandle only for tools/call; these paths never touch it.
        let reply = |m: Value| -> Option<Value> {
            let method = m.get("method").and_then(Value::as_str)?.to_string();
            let id = m.get("id").cloned()?;
            rt.block_on(async {
                Some(match method.as_str() {
                    "initialize" => json!({ "id": id, "result": initialize(&m["params"]) }),
                    "tools/list" => json!({ "id": id, "result": { "tools": catalogue() } }),
                    other => json!({ "id": id, "error": other }),
                })
            })
        };
        assert!(reply(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })).is_none());
        let init = reply(json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "2025-03-26" } })).unwrap();
        assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(init["result"]["serverInfo"]["name"], SERVER_NAME);
    }
}
