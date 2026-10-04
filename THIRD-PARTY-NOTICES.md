# Third-party notices

Frank's own code is MIT-licensed (see [LICENSE](LICENSE)). It builds on, or
works with, the following projects.

## Source code Frank is based on

| Project | Licence | Notes |
|---|---|---|
| [Coucou](https://github.com/Louis-CFM/coucou) by Louis Raillé | MIT | Frank started from Coucou's Windows source code. Coucou's name, its "Mochi" character, icon and sounds are **not** part of Frank: Frank's character, icon and sounds were made for Frank, in code (`src/character/orb.ts`, `scripts/gen-icons.mjs`, `scripts/gen-sounds.mjs`). Frank is an independent project, not affiliated with or endorsed by the author of Coucou. |

## Built with

| Project | Licence |
|---|---|
| [Tauri](https://tauri.app) | MIT or Apache-2.0 |
| [Vite](https://vite.dev), [TypeScript](https://www.typescriptlang.org) | MIT, Apache-2.0 |
| Rust crates listed in `Cargo.lock` (tokio, reqwest, serde, keyring, windows, …) | MIT and/or Apache-2.0 |

## Downloaded by `scripts/setup-voice.ps1` (not bundled with Frank)

These run on your own computer. The setup script downloads them from their
official sources the first time you set up voice; Frank does not redistribute
them.

| Component | Licence | Used for |
|---|---|---|
| [whisper.cpp](https://github.com/ggml-org/whisper.cpp) | MIT | Speech → text |
| [Whisper models](https://huggingface.co/ggerganov/whisper.cpp) (OpenAI Whisper weights, ggml format) | MIT | Speech → text |
| [Piper](https://github.com/rhasspy/piper) | MIT | Text → speech |
| espeak-ng (shipped inside the Piper download) | GPL-3.0 | Piper's pronunciation rules |
| Voice `en_US-joe-medium` | CC0 | English speech |
| Voice `ru_RU-dmitri-medium` | CC0 | Russian speech |
| Voice `tr_TR-dfki-medium` | **CC BY-NC-SA 4.0** | Turkish speech — **non-commercial use only**. Skip it with `setup-voice.ps1 -NoTurkish`. |

## Services

Frank's chat runs through [Claude Code](https://claude.com/claude-code) and
your own Claude account; Claude Code is installed separately and is subject to
Anthropic's terms. Integrations (GitHub, Vercel, Notion, …) only talk to the
services you configure, with your own keys.
