<div align="center">

<img src="src-tauri/icons/128x128@2x.png" width="110" alt="Frank">

# Frank

**A small ball of light at the top of your screen that works with Claude Code — and talks with you.**

**English** · [Türkçe](README.tr.md) · [Русский](README.ru.md)

</div>

![Frank in every mood](docs/character-sheet.png)

## What Frank does

- **Watches your Claude Code sessions, live** — what each one reads, edits and runs, across all your windows.
- **Approves from the top of the screen** — when Claude Code asks for permission, Frank shows **Allow / Deny**. No need to go back to the terminal.
- **Chats with Claude** through your own Claude subscription — **no API key**.
- **Talks with you** — say **"Frank"** and he listens, answers out loud and keeps listening. Say his name again to interrupt him.
- **Speaks your language** — understands Turkish, Russian and English, and answers in the language you used. The interface is in all three too.
- **Knows your projects** — every folder Claude Code works in on your computer, whichever app runs it. Ask *"how is my website project doing?"*: he reads the project folder (read-only) and tells you.
- **Works with your services** — connect GitHub, Vercel, Stripe, Resend, Notion, Cal.com or n8n in Settings, and ask: *"did the last deploy go through?"*, *"what's open in my repo?"*. He can also act — open an issue, send an email, write to Notion, start an n8n workflow — but only after you press **Allow** on the card that shows exactly what he will do. Stripe is read-only.
- **Remembers you** — tell him something worth keeping, or say *"remember that …"*: he writes it down in his memory, `C:\Frank\memory`, and knows it in every conversation after.
- **Passes instructions on** — *"Frank, tell the website project to make the header bigger"*: he sends it to the Claude Code session working there.
- **Looks at things for you** — paste a screenshot (**Win + Shift + S**, then **Ctrl + V**) or drop a file on him, and ask about it.

## What it costs

Frank is free and open source (MIT). It uses:

- **Your Claude Code sign-in** (Claude Pro or Max) for the chat. Answers count toward your plan's usage limits. There is no API key and no extra charge, as long as *extra usage* stays off in your Claude account settings.
- **Free, open-source voice tools that run on your own computer**: whisper.cpp (speech → text) and Piper (text → speech). No audio ever leaves your PC.

## Requirements

- Windows 10 or 11, 64-bit
- [Claude Code](https://claude.com/claude-code), signed in with your Claude account: `npm install -g @anthropic-ai/claude-code`, then run `claude` once to sign in
- Optional: an NVIDIA graphics card makes voice recognition faster and more accurate
- Only to build Frank yourself (all free): [Rust](https://rustup.rs), [Node.js 20+](https://nodejs.org), and [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) with **Desktop development with C++**

## Install

### Download (easiest)

1. **Download [Frank-Windows-setup.exe](../../releases/latest/download/Frank-Windows-setup.exe)** from the [latest release](../../releases/latest).
2. **Run it.** It installs for your user only; no administrator rights needed. Windows may warn that the installer is not code-signed: choose *More info → Run anyway*.
3. **Say yes to the voice tools** when the installer offers them at the end. They download in their own window into `%LOCALAPPDATA%\Frank\voice`: about 1.5 GB with an NVIDIA card, about 0.7 GB without. Every file is pinned to one release and checked against its SHA-256 before it is used.
4. **Start Frank** from the Start menu. He appears at the top centre of your screen.

To check that your download is the real one, run `Get-FileHash Frank-Windows-setup.exe` in PowerShell and compare it with the SHA-256 on the release page.

### Build it yourself

1. **Get the code** — `git clone` this repository (or download it as a ZIP) and open PowerShell in its folder.
2. **Build:**
   ```powershell
   npm install
   npm run sounds
   npm run icons
   $env:CARGO_PROFILE_RELEASE_LTO = "thin"
   $env:CARGO_PROFILE_RELEASE_CODEGEN_UNITS = "16"
   npm run pack
   ```
   The installer appears in `release\`. The two `CARGO_…` lines keep the build from running out of memory on machines with 16–24 GB of RAM. The first build takes a few minutes.
3. **Install** — run `release\Frank-Windows-setup.exe` and carry on from step 3 of *Download* above.

## First steps

1. **Connect Claude Code** — right-click Frank's icon in the notification area → **Settings…** → **Claude Code** → **Install hooks…**. You see exactly what will change in your Claude Code settings; a dated backup is taken before anything is written. Then open a new Claude Code session.
2. **Hands-free voice** — Settings → **General** → **Wake word**: turn it on. You can change the word too.
3. **Language** — Settings → **General** → **Language** (automatic follows Windows).

## Using Frank

| Do this | Frank does that |
|---|---|
| Move the mouse to the top centre of the screen | He peeks out |
| Click him | The island opens |
| Drag the island with the mouse | It stays where you let go, always fully on screen; near an edge, that edge lights up |
| Let go near the top edge | It docks there, where you left it (in the middle it takes its own place) |
| Let go near the left or right edge | It docks there as a slim tab that slides into the edge; a line of light shows where, hover it to bring him back |
| Press **Esc** while dragging | It goes back where it was |
| Say **"Frank"** (wake word on) | He opens and listens |
| Say **"Frank, …"** with a request in one breath | He does it right away |
| Say **"Frank"** while he is talking | He stops and listens |
| Click the microphone in the chat | Voice conversation without the wake word |
| **Win + Shift + S**, then **Ctrl + V** in the chat | Attaches the screenshot |
| Drop a file on the island | He swallows it, then answers questions about it |
| Claude Code asks for permission | **Allow / Deny** appears at the top of the screen |

A spoken conversation ends by itself after about 15 seconds of silence.

## Privacy

- No telemetry, no account of its own.
- Voice is transcribed and spoken **on your computer**; only the text of your request goes to Claude, through your own Claude Code.
- Frank can read, never change, every folder Claude Code has worked in on your computer in the last 60 days, and looks into one when you ask about it. Instructions you give him for a project are carried out by your own Claude Code session there, under its own permission settings.
- Frank's memory is plain text in `C:\Frank\memory`: one short note per thing he remembers, and an index, `MEMORY.md`. It is the only place he can write. Read, edit or delete it whenever you like, or ask him *"what do you remember about me?"*. He never saves passwords or keys.
- Keys for optional integrations live in the Windows Credential Manager, never on disk. Claude never sees them: Frank makes the calls himself and hands back the answer.
- Frank's tools are served only on this computer (127.0.0.1, with a password made fresh at every start), and his chat loads no other MCP server — not even your Claude account's connectors. He reaches the web, the services you connected and the Claude Code sessions on this computer, nothing else.

## Troubleshooting

- **Windows warns about the installer** — it is not code-signed (signing costs money every year). Choose *More info → Run anyway*, after checking its SHA-256 if you downloaded it.
- **The build stops with "out of memory"** — use the two `CARGO_…` lines from the build step.
- **Frank doesn't hear you** — Windows Settings → Privacy & security → Microphone → *Let desktop apps access your microphone* must be on.
- **"Voice chat needs … in %LOCALAPPDATA%\Frank\voice"** — run the Frank installer again and say yes to the voice tools. A download that stopped halfway carries on where it left off.
- **Frank says he can't reach a project** — open Claude Code in that project's folder first; then he can pass instructions to it.

## Credits and licence

Frank is open source under the [MIT licence](LICENSE). Its code started from [Coucou](https://github.com/Louis-CFM/coucou) by Louis Raillé (MIT); Frank's character, icon and sounds are his own, drawn and synthesized in code. Frank is an independent project, not affiliated with or endorsed by the author of Coucou. Third-party components and their licences are listed in [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md).
