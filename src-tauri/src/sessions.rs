// What the chat is told about the coding sessions running on this computer.
//
// The island hears every hook event and keeps a note per session (project,
// folder, state, last steps, the agent's last message). Each chat turn carries
// those notes here; this turns them into a short briefing for Claude, adds each
// project's git state, and returns the folders the chat may *read* — never
// write: the chat's tools stay Read, Glob and Grep.
//
// The briefing also lists the project folders the user works in, so that
// "tell the X folder to …" can find X even when no session runs there.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, SystemTime};

use serde::Deserialize;
use serde_json::Value;

use crate::{platform, settings};

/// More than this and the briefing stops being short.
const MAX_SESSIONS: usize = 6;
const MAX_STEPS: usize = 8;
const MAX_FIELD: usize = 400;
const GIT_TIMEOUT: Duration = Duration::from_secs(3);
const GIT_MAX_FILES: usize = 10;
/// Projects Claude Code worked in longer ago than this are left out.
const PROJECT_MAX_AGE: Duration = Duration::from_secs(60 * 24 * 60 * 60);
const MAX_FOLDERS: usize = 40;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionNote {
    pub project: String,
    pub cwd: String,
    pub agent: String,
    pub state: String,
    #[serde(default)]
    pub steps: Vec<String>,
    pub last_message: Option<String>,
    #[serde(default)]
    pub minutes_ago: u64,
}

pub struct Briefing {
    /// Goes in front of the user's message.
    pub text: String,
    /// Project folders the chat may read.
    pub dirs: Vec<PathBuf>,
}

/// `folders` is `known_folders()`, passed in so tests stay off the real disk.
pub async fn brief(notes: Vec<SessionNote>, folders: Vec<PathBuf>) -> Option<Briefing> {
    if notes.is_empty() && folders.is_empty() {
        return None;
    }
    let mut text = String::new();
    let mut dirs: Vec<PathBuf> = Vec::new();
    if !notes.is_empty() {
        text.push_str(
            "[Coding sessions open on this computer, as Frank sees them through their hooks. \
Every one listed is still open and can take an instruction; a session that closes drops off this list. \
Use this when the user asks about their projects; otherwise ignore it.]\n",
        );
    }

    for note in notes.into_iter().take(MAX_SESSIONS) {
        let agent = if note.agent == "claude" { "Claude Code".to_string() } else { clip(&note.agent) };
        text.push_str(&format!(
            "\n- {} ({agent}) working in {}: {}, last activity {} min ago\n",
            clip(&note.project),
            clip(&note.cwd),
            describe_state(&note.state),
            note.minutes_ago
        ));
        let steps: Vec<String> = note.steps.iter().rev().take(MAX_STEPS).rev().map(|s| clip(s)).collect();
        if !steps.is_empty() {
            text.push_str(&format!("  Recent steps: {}\n", steps.join(" | ")));
        }
        if let Some(said) = note.last_message.as_deref().filter(|s| !s.trim().is_empty()) {
            text.push_str(&format!("  Agent's last message: {}\n", clip(said)));
        }

        // Only a real, existing folder given by its full path is ever opened up.
        let dir = PathBuf::from(&note.cwd);
        if !(dir.is_absolute() && dir.is_dir()) {
            continue;
        }
        if let Some(git) = git_state(&dir).await {
            text.push_str(&format!("  Git: {git}\n"));
        }
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }

    if !dirs.is_empty() {
        text.push_str(
            "\nYou may read files in these project folders with Read, Glob and Grep. \
You cannot change anything in them or run commands.\n",
        );
    }
    if !folders.is_empty() {
        let list: Vec<String> = folders.iter().map(|f| clip(&f.to_string_lossy())).collect();
        text.push_str(&format!(
            "\n[The user's project folders, most recently used first: {}]\n",
            list.join(" | ")
        ));
    }
    Some(Briefing { text, dirs })
}

/// Folders the user is likely to name: the projects Claude Code worked in
/// lately — each transcript records its folder — then the Desktop's own
/// folders. Existing folders only, most recent first.
pub fn known_folders() -> Vec<PathBuf> {
    let now = SystemTime::now();
    let ours = settings::local_dir();
    let mut recent: Vec<(SystemTime, PathBuf)> = Vec::new();
    let projects = platform::home_dir().join(".claude").join("projects");
    if let Ok(entries) = std::fs::read_dir(&projects) {
        for entry in entries.flatten() {
            let Some((when, cwd)) = newest_transcript_folder(&entry.path()) else { continue };
            let fresh = now.duration_since(when).map(|age| age < PROJECT_MAX_AGE).unwrap_or(true);
            // The chat's own sessions run under Frank's folder: not a project.
            if fresh && cwd.is_absolute() && cwd.is_dir() && !cwd.starts_with(&ours) {
                recent.push((when, cwd));
            }
        }
    }
    recent.sort_by(|a, b| b.0.cmp(&a.0));

    let mut out: Vec<PathBuf> = Vec::new();
    for (_, dir) in recent {
        if !out.contains(&dir) {
            out.push(dir);
        }
    }
    let home = platform::home_dir();
    for desktop in [home.join("Desktop"), home.join("OneDrive").join("Desktop")] {
        let Ok(entries) = std::fs::read_dir(&desktop) else { continue };
        let mut names: Vec<PathBuf> = entries
            .flatten()
            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
            .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
            .map(|e| e.path())
            .collect();
        names.sort();
        for dir in names {
            if !out.contains(&dir) {
                out.push(dir);
            }
        }
    }
    out.truncate(MAX_FOLDERS);
    out
}

/// The folder recorded in a project's newest transcript, and when it was
/// written. Every line carries `cwd`; the first few kilobytes are enough.
fn newest_transcript_folder(project: &Path) -> Option<(SystemTime, PathBuf)> {
    let newest = std::fs::read_dir(project)
        .ok()?
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "jsonl"))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .max_by_key(|(when, _)| *when)?;
    let mut head = vec![0u8; 64 * 1024];
    let read = std::fs::File::open(&newest.1).ok()?.read(&mut head).ok()?;
    let cwd = String::from_utf8_lossy(&head[..read])
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find_map(|v| v.get("cwd").and_then(Value::as_str).map(PathBuf::from))?;
    Some((newest.0, cwd))
}

/// The island's state names, worded so they cannot be misread: "finished"
/// was taken for "the session closed" when it only means the turn is done.
fn describe_state(state: &str) -> String {
    match state {
        "finished" | "idle" => "open and waiting for an instruction (its last task is done)".into(),
        "thinking" | "working" => "busy on a task right now".into(),
        "waiting for permission" => "waiting for the user to allow a step".into(),
        "error" => "stopped on an error, still open".into(),
        other => clip(other),
    }
}

/// Collapses whitespace and caps a field the island sent.
fn clip(s: &str) -> String {
    let one_line = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() <= MAX_FIELD {
        return one_line;
    }
    let mut cut: String = one_line.chars().take(MAX_FIELD).collect();
    cut.push('…');
    cut
}

/// "branch main, ahead 1; 3 changed files: M src/a.ts, ?? notes.md; recent
/// commits: …" — or None when the folder is not a git repository.
async fn git_state(dir: &Path) -> Option<String> {
    let git = platform::find_on_path("git")?;
    let status = run_git(&git, dir, &["status", "--porcelain=v1", "--branch"]).await?;
    let mut lines = status.lines();
    let branch = lines
        .next()
        .and_then(|l| l.strip_prefix("## "))
        .unwrap_or("unknown branch")
        .to_string();
    let changed: Vec<&str> = lines.filter(|l| !l.trim().is_empty()).collect();

    let mut out = format!("branch {branch}");
    if changed.is_empty() {
        out.push_str("; working tree clean");
    } else {
        let shown: Vec<&str> = changed.iter().take(GIT_MAX_FILES).map(|l| l.trim()).collect();
        out.push_str(&format!("; {} changed file(s): {}", changed.len(), shown.join(", ")));
        if changed.len() > GIT_MAX_FILES {
            out.push_str(", …");
        }
    }
    if let Some(log) = run_git(&git, dir, &["log", "-3", "--format=%h %s (%cr)"]).await {
        let commits: Vec<&str> = log.lines().filter(|l| !l.trim().is_empty()).collect();
        if !commits.is_empty() {
            out.push_str(&format!("; recent commits: {}", commits.join(" | ")));
        }
    }
    Some(clip(&out))
}

/// Runs a read-only git command in `dir`. Nothing it does writes: optional
/// locks are off, so `status` does not refresh the index, and fsmonitor is off,
/// so no repository setting gets to run a program.
async fn run_git(git: &Path, dir: &Path, args: &[&str]) -> Option<String> {
    let mut cmd = std::process::Command::new(git);
    cmd.arg("-C")
        .arg(dir)
        .args(["-c", "core.fsmonitor=false", "-c", "core.quotepath=false"])
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    platform::no_console(&mut cmd);
    let child = tokio::process::Command::from(cmd).kill_on_drop(true).spawn().ok()?;
    let output = tokio::time::timeout(GIT_TIMEOUT, child.wait_with_output()).await.ok()?.ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(cwd: &str) -> SessionNote {
        SessionNote {
            project: "Website".into(),
            cwd: cwd.into(),
            agent: "claude".into(),
            state: "working".into(),
            steps: vec!["user asked: fix the header".into(), "Edit · src/header.ts".into()],
            last_message: Some("Done, the header\nis fixed.".into()),
            minutes_ago: 2,
        }
    }

    #[test]
    fn nothing_to_say_means_no_briefing() {
        assert!(tauri::async_runtime::block_on(brief(vec![], vec![])).is_none());
    }

    #[test]
    fn known_folders_are_listed_but_not_opened_up() {
        let folder = std::env::temp_dir();
        let b = tauri::async_runtime::block_on(brief(vec![], vec![folder.clone()])).unwrap();
        assert!(b.text.contains(&*folder.to_string_lossy()));
        // Naming a folder is not the same as letting the chat read it.
        assert!(b.dirs.is_empty());
    }

    #[test]
    fn a_transcript_gives_away_its_folder() {
        let project = std::env::temp_dir().join(format!("frank-proj-{}", std::process::id()));
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(
            project.join("s1.jsonl"),
            "{\"type\":\"summary\"}\n{\"cwd\":\"C:\\\\work\\\\site\",\"type\":\"user\"}\n",
        )
        .unwrap();
        let (_, cwd) = newest_transcript_folder(&project).unwrap();
        assert_eq!(cwd, PathBuf::from(r"C:\work\site"));
        let _ = std::fs::remove_dir_all(&project);
    }

    #[test]
    fn a_session_is_described_and_only_real_folders_open_up() {
        let real = std::env::temp_dir();
        let b = tauri::async_runtime::block_on(brief(
            vec![
                note(real.to_str().unwrap()),
                note("relative/path"),
                note("C:\\definitely\\not\\here\\frank"),
            ],
            vec![],
        ))
        .unwrap();
        assert!(b.text.contains("Website (Claude Code)"));
        assert!(b.text.contains("Edit · src/header.ts"));
        // Newlines from the agent's message never break the briefing's layout.
        assert!(b.text.contains("Done, the header is fixed."));
        assert_eq!(b.dirs, vec![real]);
    }

    #[test]
    fn long_fields_are_capped() {
        let long = "x".repeat(MAX_FIELD * 3);
        assert_eq!(clip(&long).chars().count(), MAX_FIELD + 1);
    }
}
