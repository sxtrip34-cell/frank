// Voice chat, entirely on this machine: whisper.cpp turns what the user says
// into text and Piper reads the answer aloud. No audio leaves the computer and
// neither tool costs anything; the chat turn in between is the usual one.
//
// The tools are not bundled with the app. They live in %LOCALAPPDATA%\Frank\voice:
//   whisper\whisper-server.exe (+ whisper-cli.exe)    models\ggml-*.bin
//   piper\piper.exe                                   piper-voices\*.onnx (+ .onnx.json)
//
// Both are kept running once started — whisper-server with its model loaded,
// one Piper per voice — because loading is most of the wait: a sentence takes
// ~0.3 s to transcribe this way against ~1.4 s for a fresh whisper-cli. If a
// long-running helper fails, the one-shot command line takes over.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};

use crate::{log, platform, settings};

const TRANSCRIBE_TIMEOUT: Duration = Duration::from_secs(90);
const SPEAK_TIMEOUT: Duration = Duration::from_secs(90);
/// A minute of 16 kHz mono PCM is under 2 MB; anything far bigger is not speech.
const MAX_WAV: usize = 8 * 1024 * 1024;
/// Piper reads a long answer for minutes; past this it is cut at a word.
const MAX_SPOKEN_CHARS: usize = 3_000;
/// The languages the user speaks. Whisper detects the language of each
/// utterance; anything else it comes up with is a misheard Turkish one (a short
/// sentence taken for Azerbaijani, say), so it is transcribed again as Turkish.
const LANGUAGES: &[&str] = &["tr", "ru", "en"];
const FALLBACK_LANGUAGE: &str = "tr";

/// What Whisper "hears" in silence or noise, learned from subtitled videos.
/// A transcript that is only one of these is treated as nothing said.
const HALLUCINATIONS: &[&str] = &[
    "altyazı m.k.",
    "altyazı m.k",
    "izlediğiniz için teşekkürler.",
    "izlediğiniz için teşekkür ederim.",
    "abone olmayı unutmayın.",
    "thank you.",
    "thanks for watching!",
];

/// Loading a model onto the GPU takes a few seconds — but right after Windows
/// starts (Frank starts with it), with the disk busy and the GPU driver still
/// waking up, it can take minutes. A server that is not up by then is given
/// up on, and the command line used instead.
const SERVER_START_TIMEOUT: Duration = Duration::from_secs(180);
/// After a server failed to start, the command line is used this long before
/// another server is tried, so a broken one is not restarted per utterance.
const SERVER_RETRY_AFTER: Duration = Duration::from_secs(120);

/// Common English words: an answer with no Turkish or Cyrillic letters that is
/// made of these is read with the English voice.
const ENGLISH_WORDS: &[&str] = &[
    "the", "a", "an", "is", "are", "was", "were", "to", "of", "and", "you", "your", "it", "in",
    "on", "for", "with", "that", "this", "i", "we", "what", "how", "can", "be", "have", "do",
    "not", "yes", "no", "will", "my", "me", "there", "here", "it's", "i'm", "sure", "today",
];

static COUNTER: AtomicU64 = AtomicU64::new(1);

/// whisper-server, loading its model or ready.
struct WhisperServer {
    child: tokio::process::Child,
    port: u16,
    model: PathBuf,
    started: Instant,
    /// It has answered HTTP: the model is loaded.
    ready: bool,
}

#[derive(Default)]
struct WhisperSlot {
    server: Option<WhisperServer>,
    /// No new server before this: the last one failed to start.
    retry_at: Option<Instant>,
}

/// Held only for quick looks, never while the model loads: a second utterance
/// must not queue behind a server start.
static WHISPER: LazyLock<tokio::sync::Mutex<WhisperSlot>> =
    LazyLock::new(|| tokio::sync::Mutex::new(WhisperSlot::default()));

/// One Piper per voice, reading a line of text at a time and printing the path
/// of the WAV it wrote for it.
struct PiperProcess {
    child: tokio::process::Child,
    stdin: tokio::process::ChildStdin,
    lines: Lines<BufReader<tokio::process::ChildStdout>>,
    out_dir: PathBuf,
}

static PIPERS: LazyLock<tokio::sync::Mutex<HashMap<PathBuf, PiperProcess>>> =
    LazyLock::new(|| tokio::sync::Mutex::new(HashMap::new()));

pub fn voice_dir() -> PathBuf {
    settings::local_dir().join("voice")
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceStatus {
    pub ready: bool,
    pub missing: Vec<String>,
}

pub fn status() -> VoiceStatus {
    let mut missing = Vec::new();
    if !whisper_exe().is_file() {
        missing.push("whisper-cli".to_string());
    }
    if whisper_model().is_none() {
        missing.push("a Whisper model".to_string());
    }
    if !piper_exe().is_file() {
        missing.push("piper".to_string());
    }
    if piper_voice().is_none() {
        missing.push("a Piper voice".to_string());
    }
    VoiceStatus { ready: missing.is_empty(), missing }
}

fn whisper_exe() -> PathBuf {
    voice_dir()
        .join("whisper")
        .join(format!("whisper-cli{}", std::env::consts::EXE_SUFFIX))
}

fn piper_exe() -> PathBuf {
    voice_dir().join("piper").join(format!("piper{}", std::env::consts::EXE_SUFFIX))
}

/// The largest ggml model present: a bigger download is a better model.
fn whisper_model() -> Option<PathBuf> {
    largest(&voice_dir().join("models"), |name| name.starts_with("ggml-") && name.ends_with(".bin"))
}

fn piper_voice() -> Option<PathBuf> {
    largest(&voice_dir().join("piper-voices"), |name| name.ends_with(".onnx"))
}

fn largest(dir: &Path, wanted: impl Fn(&str) -> bool) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter(|e| wanted(&e.file_name().to_string_lossy()))
        .filter_map(|e| Some((e.metadata().ok().filter(|m| m.is_file())?.len(), e.path())))
        .max_by_key(|(len, _)| *len)
        .map(|(_, path)| path)
}

/// A fresh scratch file under voice\tmp; the caller removes it.
fn scratch(prefix: &str, ext: &str) -> Result<PathBuf, String> {
    let dir = voice_dir().join("tmp");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    Ok(dir.join(format!("{prefix}-{}-{n}.{ext}", std::process::id())))
}

/// Speech → text. `wav` is the 16 kHz mono recording the island made.
pub async fn transcribe(wav: Vec<u8>) -> Result<String, String> {
    if wav.len() < 44 || &wav[..4] != b"RIFF" || &wav[8..12] != b"WAVE" {
        return Err("Not a WAV recording.".into());
    }
    if wav.len() > MAX_WAV {
        return Err("That recording is too long.".into());
    }
    let audio_ctx = audio_ctx_for(&wav);

    if let Some(port) = whisper_server_port().await {
        match transcribe_on_server(port, &wav, audio_ctx).await {
            Ok(text) => return Ok(clean_transcript(&text)),
            Err(why) => {
                // Forget the server: the next utterance starts a fresh one.
                log::line(format!("voice: whisper-server failed ({why}), using whisper-cli"));
                WHISPER.lock().await.server = None;
            }
        }
    }

    let exe = whisper_exe();
    let model = whisper_model().ok_or("No Whisper model installed.")?;
    if !exe.is_file() {
        return Err("whisper-cli is not installed.".into());
    }

    let input = scratch("in", "wav")?;
    std::fs::write(&input, &wav).map_err(|e| e.to_string())?;

    let result: Result<Vec<u8>, String> = async {
        let first = whisper(&exe, &model, &input, "auto", audio_ctx).await?;
        match detected_language(&first.stderr) {
            Some(lang) if LANGUAGES.contains(&lang.as_str()) => Ok(first.stdout),
            _ => Ok(whisper(&exe, &model, &input, FALLBACK_LANGUAGE, audio_ctx).await?.stdout),
        }
    }
    .await;
    let _ = std::fs::remove_file(&input);
    Ok(clean_transcript(&String::from_utf8_lossy(&result?)))
}

/// Starts whisper-server in the background so the first utterance does not
/// wait for the model to load, along with Piper for the usual voice.
pub async fn warm_up() {
    let _ = whisper_server_port().await;
    if let Some(voice) = piper_voice_for("") {
        let mut pipers = PIPERS.lock().await;
        if !pipers.contains_key(&voice) {
            match start_piper(&voice) {
                Ok(process) => {
                    pipers.insert(voice, process);
                }
                Err(why) => log::line(format!("voice: piper did not start: {why}")),
            }
        }
    }
}

/// The port of a running whisper-server, starting one if needed and waiting
/// for it to load its model. None when it is not installed or will not start:
/// the caller falls back to whisper-cli.
///
/// A server still loading is waited for, never killed and started over: on a
/// cold start the load can outlast any one utterance, and starting from
/// scratch each time meant it never finished.
async fn whisper_server_port() -> Option<u16> {
    let client = reqwest::Client::new();
    loop {
        let port = {
            let mut slot = WHISPER.lock().await;
            let model = whisper_model()?;
            let usable = match slot.server.as_mut() {
                Some(s) => s.model == model && matches!(s.child.try_wait(), Ok(None)),
                None => false,
            };
            match slot.server.as_ref().filter(|_| usable) {
                Some(server) if server.ready => return Some(server.port),
                Some(server) if server.started.elapsed() <= SERVER_START_TIMEOUT => server.port,
                Some(_) => {
                    log::line("voice: whisper-server did not come up in time");
                    slot.server = None;
                    slot.retry_at = Some(Instant::now() + SERVER_RETRY_AFTER);
                    return None;
                }
                None => {
                    if let Some(old) = slot.server.take() {
                        // It exited — unless the model changed under it.
                        if old.model == model {
                            log::line("voice: whisper-server stopped");
                            slot.retry_at = Some(Instant::now() + SERVER_RETRY_AFTER);
                        }
                    }
                    if slot.retry_at.is_some_and(|at| Instant::now() < at) {
                        return None;
                    }
                    let server = spawn_whisper_server(model)?;
                    let port = server.port;
                    slot.server = Some(server);
                    port
                }
            }
        };

        // It answers HTTP once the model is loaded. Asked without the lock, so
        // a caller arriving meanwhile waits on the same server, not behind us.
        let url = format!("http://127.0.0.1:{port}/");
        if client.get(&url).timeout(Duration::from_secs(1)).send().await.is_ok() {
            let mut guard = WHISPER.lock().await;
            let slot = &mut *guard;
            if let Some(server) = slot.server.as_mut().filter(|s| s.port == port) {
                if !server.ready {
                    server.ready = true;
                    slot.retry_at = None;
                    log::line(format!(
                        "voice: whisper-server ready in {} ms",
                        server.started.elapsed().as_millis()
                    ));
                }
                return Some(port);
            }
            continue; // replaced meanwhile: look again
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

fn spawn_whisper_server(model: PathBuf) -> Option<WhisperServer> {
    let exe = voice_dir()
        .join("whisper")
        .join(format!("whisper-server{}", std::env::consts::EXE_SUFFIX));
    if !exe.is_file() {
        return None;
    }
    // Port 0 asks the OS for a free one; the server takes it over right after.
    let port = std::net::TcpListener::bind("127.0.0.1:0").ok()?.local_addr().ok()?.port();
    let threads = std::thread::available_parallelism().map(|n| n.get().min(8)).unwrap_or(4);
    let mut cmd = std::process::Command::new(&exe);
    cmd.arg("-m")
        .arg(&model)
        .args(["--host", "127.0.0.1", "--port", &port.to_string(), "-l", "auto", "-nt"])
        .args(["-t", &threads.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(dir) = exe.parent() {
        cmd.current_dir(dir);
    }
    platform::no_console(&mut cmd);
    let child = match tokio::process::Command::from(cmd).kill_on_drop(true).spawn() {
        Ok(child) => child,
        Err(why) => {
            log::line(format!("voice: whisper-server did not start: {why}"));
            return None;
        }
    };
    platform::tie_to_app(&child);
    log::line("voice: whisper-server starting");
    Some(WhisperServer { child, port, model, started: Instant::now(), ready: false })
}

/// Detects the language, and transcribes again as Turkish when Whisper's
/// guess is none of the user's languages.
async fn transcribe_on_server(port: u16, wav: &[u8], audio_ctx: u32) -> Result<String, String> {
    let (text, language) = server_pass(port, wav, audio_ctx, "auto").await?;
    if language.is_some_and(|l| LANGUAGES.contains(&l)) {
        return Ok(text);
    }
    Ok(server_pass(port, wav, audio_ctx, FALLBACK_LANGUAGE).await?.0)
}

async fn server_pass(
    port: u16,
    wav: &[u8],
    audio_ctx: u32,
    language: &str,
) -> Result<(String, Option<&'static str>), String> {
    let file = reqwest::multipart::Part::bytes(wav.to_vec())
        .file_name("speech.wav")
        .mime_str("audio/wav")
        .map_err(|e| e.to_string())?;
    let form = reqwest::multipart::Form::new()
        .part("file", file)
        .text("response_format", "verbose_json")
        .text("audio_ctx", audio_ctx.to_string())
        .text("language", language.to_string());
    let response = reqwest::Client::new()
        .post(format!("http://127.0.0.1:{port}/inference"))
        .multipart(form)
        .timeout(TRANSCRIBE_TIMEOUT)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status()));
    }
    let json: Value = response.json().await.map_err(|e| e.to_string())?;
    let text = json.get("text").and_then(Value::as_str).unwrap_or("").to_string();
    let language = json.get("language").and_then(Value::as_str).and_then(language_code);
    Ok((text, language))
}

/// whisper-server names languages in full ("russian"); whisper-cli uses codes.
fn language_code(name: &str) -> Option<&'static str> {
    match name.to_lowercase().as_str() {
        "turkish" | "tr" => Some("tr"),
        "russian" | "ru" => Some("ru"),
        "english" | "en" => Some("en"),
        _ => None,
    }
}

async fn whisper(
    exe: &Path,
    model: &Path,
    input: &Path,
    language: &str,
    audio_ctx: u32,
) -> Result<std::process::Output, String> {
    let threads = std::thread::available_parallelism()
        .map(|n| n.get().min(8))
        .unwrap_or(4)
        .to_string();
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("-m")
        .arg(model)
        .arg("-f")
        .arg(input)
        // No timestamps: stdout is the transcript and nothing else. The log,
        // detected language included, goes to stderr.
        .args(["-l", language, "-nt", "-t", &threads, "-ac", &audio_ctx.to_string()]);
    if let Some(dir) = exe.parent() {
        cmd.current_dir(dir);
    }
    run(cmd, None, TRANSCRIBE_TIMEOUT).await
}

/// "whisper_full_with_state: auto-detected language: ru (p = 0.998917)" → "ru".
fn detected_language(stderr: &[u8]) -> Option<String> {
    let log = String::from_utf8_lossy(stderr);
    let rest = log.lines().find_map(|l| l.split("auto-detected language:").nth(1))?;
    rest.split_whitespace().next().map(str::to_string)
}

/// Whisper's encoder always works on a 30-second window (1500 frames), which is
/// most of the wait for a five-second sentence. Sizing the window to the
/// recording, with room to spare, cut that from 4.5 s to 2.5 s here with the
/// same transcript; below 512 frames Turkish words started to slip.
fn audio_ctx_for(wav: &[u8]) -> u32 {
    const FULL: u32 = 1500; // 30 s
    const FLOOR: u32 = 512;
    let rate = u32::from_le_bytes([wav[24], wav[25], wav[26], wav[27]]);
    let block = u16::from_le_bytes([wav[32], wav[33]]) as u32;
    if rate == 0 || block == 0 {
        return FULL;
    }
    let seconds = (wav.len().saturating_sub(44) as f64) / (rate * block) as f64;
    // 50 frames a second, half again as many for margin, rounded up to 64.
    let frames = ((seconds * 75.0).ceil() as u32 + 128).div_ceil(64) * 64;
    frames.clamp(FLOOR, FULL)
}

/// Text → a WAV of Frank saying it.
pub async fn speak(text: &str) -> Result<Vec<u8>, String> {
    // Piper reads stdin line by line, one utterance each: one line keeps it one
    // recording, with Piper's own pauses between sentences.
    let mut line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.is_empty() {
        return Err("Nothing to say.".into());
    }
    if line.len() > MAX_SPOKEN_CHARS {
        let mut end = MAX_SPOKEN_CHARS;
        while !line.is_char_boundary(end) {
            end -= 1;
        }
        let cut = line[..end].rfind(' ').unwrap_or(end);
        line.truncate(cut);
    }
    line.push('\n');

    let exe = piper_exe();
    let voice = piper_voice_for(&line).ok_or("No Piper voice installed.")?;
    if !exe.is_file() {
        return Err("Piper is not installed.".into());
    }

    match speak_on_running_piper(&voice, &line).await {
        Ok(audio) => return Ok(audio),
        Err(why) => log::line(format!("voice: running piper failed ({why}), using a fresh one")),
    }

    let output = scratch("out", "wav")?;
    let mut cmd = std::process::Command::new(&exe);
    cmd.arg("--model").arg(&voice).arg("--output_file").arg(&output);
    // espeak-ng-data sits next to piper.exe and is found from there.
    if let Some(dir) = exe.parent() {
        cmd.current_dir(dir);
    }
    let result = run(cmd, Some(line.as_bytes()), SPEAK_TIMEOUT).await;
    let audio = result.and_then(|_| std::fs::read(&output).map_err(|e| e.to_string()));
    let _ = std::fs::remove_file(&output);
    audio
}

/// Speaks `line` on this voice's long-running Piper, starting it if needed.
async fn speak_on_running_piper(voice: &Path, line: &str) -> Result<Vec<u8>, String> {
    let mut pipers = PIPERS.lock().await;
    let alive = pipers
        .get_mut(voice)
        .is_some_and(|p| matches!(p.child.try_wait(), Ok(None)));
    if !alive {
        pipers.remove(voice);
        pipers.insert(voice.to_path_buf(), start_piper(voice)?);
    }
    let Some(piper) = pipers.get_mut(voice) else { return Err("piper did not start".into()) };

    let written = tokio::time::timeout(SPEAK_TIMEOUT, async {
        piper.stdin.write_all(line.as_bytes()).await.map_err(|e| e.to_string())?;
        piper.stdin.flush().await.map_err(|e| e.to_string())?;
        piper
            .lines
            .next_line()
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "piper stopped".to_string())
    })
    .await;
    let path = match written {
        Ok(Ok(path)) => PathBuf::from(path.trim()),
        Ok(Err(why)) => {
            pipers.remove(voice);
            return Err(why);
        }
        Err(_) => {
            pipers.remove(voice);
            return Err("piper took too long".into());
        }
    };
    // Only ever read (and delete) a file in the folder this Piper writes to.
    if path.parent() != Some(piper.out_dir.as_path()) {
        pipers.remove(voice);
        return Err("piper answered with an unexpected path".into());
    }
    let audio = std::fs::read(&path).map_err(|e| e.to_string());
    let _ = std::fs::remove_file(&path);
    audio
}

fn start_piper(voice: &Path) -> Result<PiperProcess, String> {
    let exe = piper_exe();
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let out_dir = voice_dir().join("tmp").join(format!("piper-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
    let mut cmd = std::process::Command::new(&exe);
    cmd.arg("--model")
        .arg(voice)
        .arg("--output_dir")
        .arg(&out_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    // espeak-ng-data sits next to piper.exe and is found from there.
    if let Some(dir) = exe.parent() {
        cmd.current_dir(dir);
    }
    platform::no_console(&mut cmd);
    let mut child = tokio::process::Command::from(cmd)
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("Could not start piper: {e}"))?;
    platform::tie_to_app(&child);
    let stdin = child.stdin.take().ok_or("piper has no stdin")?;
    let stdout = child.stdout.take().ok_or("piper has no stdout")?;
    Ok(PiperProcess { child, stdin, lines: BufReader::new(stdout).lines(), out_dir })
}

/// The voice for this answer — Russian, English or Turkish — or whichever
/// voice there is.
fn piper_voice_for(text: &str) -> Option<PathBuf> {
    let prefix = match spoken_language(text) {
        "ru" => "ru_",
        "en" => "en_",
        _ => "tr_",
    };
    largest(&voice_dir().join("piper-voices"), |name| {
        name.starts_with(prefix) && name.ends_with(".onnx")
    })
    .or_else(|| {
        largest(&voice_dir().join("piper-voices"), |name| {
            name.starts_with("tr_") && name.ends_with(".onnx")
        })
    })
    .or_else(piper_voice)
}

/// Cyrillic is Russian; Turkish letters are Turkish; plain Latin text made of
/// common English words is English; anything else is taken as Turkish.
fn spoken_language(text: &str) -> &'static str {
    if mostly_cyrillic(text) {
        return "ru";
    }
    if text.chars().any(|c| "çğıöşüÇĞİÖŞÜ".contains(c)) {
        return "tr";
    }
    let words: Vec<String> = text
        .split(|c: char| !(c.is_alphabetic() || c == '\''))
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect();
    if words.is_empty() {
        return "tr";
    }
    let english = words.iter().filter(|w| ENGLISH_WORDS.contains(&w.as_str())).count();
    if english * 5 >= words.len() { "en" } else { "tr" }
}

fn mostly_cyrillic(text: &str) -> bool {
    let (mut cyrillic, mut other) = (0usize, 0usize);
    for c in text.chars().filter(|c| c.is_alphabetic()) {
        if ('\u{0400}'..='\u{04FF}').contains(&c) {
            cyrillic += 1;
        } else {
            other += 1;
        }
    }
    cyrillic > other
}

/// Runs a helper without a console window, feeding `stdin` if given, and
/// returns what it printed. A helper that overruns `timeout` is killed.
async fn run(
    mut cmd: std::process::Command,
    stdin: Option<&[u8]>,
    timeout: Duration,
) -> Result<std::process::Output, String> {
    let name = Path::new(cmd.get_program())
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    cmd.stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    platform::no_console(&mut cmd);

    let mut child = tokio::process::Command::from(cmd)
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("Could not start {name}: {e}"))?;
    if let (Some(data), Some(mut pipe)) = (stdin, child.stdin.take()) {
        pipe.write_all(data).await.map_err(|e| format!("{name}: {e}"))?;
    }
    let output = match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(result) => result.map_err(|e| format!("{name} failed: {e}"))?,
        Err(_) => return Err(format!("{name} took too long.")),
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let why = stderr.lines().map(str::trim).rfind(|l| !l.is_empty()).unwrap_or("no details");
        return Err(format!("{name} failed: {}", why.chars().take(200).collect::<String>()));
    }
    Ok(output)
}

/// Whisper's stdout as one line, without its non-speech tags ("[BLANK_AUDIO]",
/// "(müzik)") or the hallucinations it tacks onto the silence after speech.
fn clean_transcript(raw: &str) -> String {
    raw.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !is_tag(l))
        .map(without_hallucination)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// A segment with a known hallucination removed, whether it is the whole
/// segment or stuck onto the end of a real sentence.
fn without_hallucination(line: &str) -> &str {
    // Lowercasing "İ" leaves a combining dot behind; dropping it keeps one
    // char per char, so a suffix length in the folded text is one here too.
    let folded: Vec<char> = line.to_lowercase().replace('\u{307}', "").chars().collect();
    for h in HALLUCINATIONS {
        let h: Vec<char> = h.chars().collect();
        if folded == h {
            return "";
        }
        let tail = folded.len().saturating_sub(h.len());
        if folded.len() > h.len() && folded[tail..] == h[..] && folded[tail - 1] == ' ' {
            let cut = line.char_indices().nth(tail).map(|(i, _)| i).unwrap_or(line.len());
            return line[..cut].trim_end();
        }
    }
    line
}

fn is_tag(line: &str) -> bool {
    let wrapped = |open: char, close: char| line.starts_with(open) && line.ends_with(close);
    wrapped('[', ']') || wrapped('(', ')') || wrapped('*', '*')
}

#[cfg(test)]
mod tests {
    use super::{
        audio_ctx_for, clean_transcript, detected_language, language_code, mostly_cyrillic,
        spoken_language,
    };

    #[test]
    fn each_answer_gets_the_voice_of_its_language() {
        assert_eq!(spoken_language("Привет! Сегодня солнечно."), "ru");
        assert_eq!(spoken_language("Bugün hava güneşli, şemsiyeye gerek yok."), "tr");
        assert_eq!(spoken_language("Sure, the weather is nice today. You won't need it."), "en");
        // Turkish without any Turkish letter is still Turkish.
        assert_eq!(spoken_language("Tamam, hemen bakarim."), "tr");
        assert_eq!(spoken_language(""), "tr");
    }

    #[test]
    fn server_language_names_become_codes() {
        assert_eq!(language_code("Russian"), Some("ru"));
        assert_eq!(language_code("turkish"), Some("tr"));
        assert_eq!(language_code("english"), Some("en"));
        assert_eq!(language_code("azerbaijani"), None);
    }

    #[test]
    fn the_detected_language_is_read_from_whisper_log() {
        let log = b"whisper_full_with_state: auto-detected language: ru (p = 0.998917)\n";
        assert_eq!(detected_language(log).as_deref(), Some("ru"));
        assert_eq!(detected_language(b"whisper_print_timings: total time = 1 ms\n"), None);
    }

    #[test]
    fn russian_answers_get_the_russian_voice() {
        assert!(mostly_cyrillic("Привет! Сегодня в Москве солнечно."));
        assert!(!mostly_cyrillic("Merhaba! Bugün İstanbul güneşli."));
        // A Turkish answer naming a Russian city stays Turkish.
        assert!(!mostly_cyrillic("Bugün Москва'da hava güneşli ve sıcak olacak."));
    }

    /// A 16 kHz mono 16-bit WAV of `seconds` of silence.
    fn wav(seconds: f64) -> Vec<u8> {
        let data = (seconds * 16_000.0) as usize * 2;
        let mut v = vec![0u8; 44 + data];
        v[..4].copy_from_slice(b"RIFF");
        v[8..12].copy_from_slice(b"WAVE");
        v[24..28].copy_from_slice(&16_000u32.to_le_bytes());
        v[32..34].copy_from_slice(&2u16.to_le_bytes());
        v
    }

    #[test]
    fn the_whisper_window_fits_the_recording_with_room_to_spare() {
        assert_eq!(audio_ctx_for(&wav(2.0)), 512); // never below the floor
        let ten = audio_ctx_for(&wav(10.0));
        assert!(ten >= 500 + 128 && ten % 64 == 0, "10 s got {ten}");
        assert_eq!(audio_ctx_for(&wav(29.0)), 1500); // never past 30 s
    }

    #[test]
    fn transcripts_lose_tags_and_known_hallucinations() {
        assert_eq!(clean_transcript(" Merhaba Frank.\n Nasılsın?\n"), "Merhaba Frank. Nasılsın?");
        assert_eq!(clean_transcript("[BLANK_AUDIO]\n"), "");
        assert_eq!(clean_transcript("(müzik)\nSelam\n"), "Selam");
        assert_eq!(clean_transcript("Altyazı M.K.\n"), "");
        assert_eq!(clean_transcript("İzlediğiniz için teşekkürler.\n"), "");
        // Tacked onto the end of what was really said, as its own segment or not.
        assert_eq!(clean_transcript("Şemsiye almalı mıyım?\nAltyazı M.K.\n"), "Şemsiye almalı mıyım?");
        assert_eq!(clean_transcript("Şemsiye almalı mıyım? Altyazı M.K.\n"), "Şemsiye almalı mıyım?");
        // A real sentence that merely contains the words stays.
        assert_eq!(
            clean_transcript("İzlediğiniz için teşekkürler dedi ve gitti.\n"),
            "İzlediğiniz için teşekkürler dedi ve gitti."
        );
    }
}
