// Runs `tauri build` (which also builds frank-hook, through the npm prebuild
// script) with the build machine's paths taken out of the binaries. Rust keeps
// the source path of every crate in its panic messages, and without this the
// shipped exe would carry the builder's home folder, user name included.

import { spawnSync } from "node:child_process";
import { homedir } from "node:os";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const cargoHome = process.env.CARGO_HOME ? resolve(process.env.CARGO_HOME) : resolve(homedir(), ".cargo");

// The last matching prefix wins, so the narrower folders come after the home
// folder they sit in.
const remaps = [
  [homedir(), "~"],
  [cargoHome, "cargo"],
  [root, "frank"],
];

/** Both spellings of a Windows drive letter: tools disagree on its case. */
const spellings = (path) =>
  /^[a-z]:/i.test(path) ? [path[0].toUpperCase() + path.slice(1), path[0].toLowerCase() + path.slice(1)] : [path];

const flags = remaps.flatMap(([from, to]) => spellings(from).map((p) => `--remap-path-prefix=${p}=${to}`));

// CARGO_ENCODED_RUSTFLAGS keeps paths with spaces intact (RUSTFLAGS splits on
// them) and takes precedence over RUSTFLAGS, so any flags set there carry over.
const inherited = process.env.CARGO_ENCODED_RUSTFLAGS
  ? process.env.CARGO_ENCODED_RUSTFLAGS.split("\x1f")
  : (process.env.RUSTFLAGS ?? "").split(/\s+/).filter(Boolean);
const env = { ...process.env, CARGO_ENCODED_RUSTFLAGS: [...inherited, ...flags].join("\x1f") };

const result = spawnSync("npx", ["tauri", "build"], { cwd: root, env, stdio: "inherit", shell: true });
process.exit(result.status ?? 1);
