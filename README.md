# Lantern IDE

**An AI code editor that shows you what the agent sees, what it changes, and how far the change reaches.**

[한국어](README.ko.md) · Windows beta · Apache-2.0 · Bring your own model (Claude, OpenAI-compatible, Ollama, LM Studio)

![Approval card with impact radius](docs/images/impact-en.png)

> **Beta.** Lantern is young software built by one developer. Windows is the tested platform; macOS and Linux build in CI but have not been used day to day. Expect rough edges and please [report them](https://github.com/Lantern-IDE/lantern/issues/new/choose).

## Why another AI editor?

Most AI editors are organised around files and a chat box. Once you let an agent edit code, the questions change: *what did it look at, what did it touch, and what else will break?* Lantern is organised around those questions.

- **Code map.** The centre of the window switches between the editor and a live map of your code (folders → files → symbols, calls, and files that usually change together).
- **Impact radius on every edit.** Before you approve an agent's edit, the card shows the symbols it touches, direct and indirect callers, the tests that cover them (or that there are none), and a risk level. One click shows it on the map.
- **Agent footprint.** Each task records what was sent as context, what the agent read, and what it edited, and paints it on the map.
- **Project memory map.** Team conventions live in `.lantern/memory/*.md`; Lantern links each rule to the code it talks about.
- **Tasks, not tabs.** A task keeps its conversation, context, changed files, and undo together. Tasks survive restarts, and can run in an isolated git worktree so nothing touches your working tree until you apply it.
- **Nothing hidden.** No hidden system prompt; the inspector shows every file, token, and cent sent to the model. No telemetry.

## Context engine

The core is a local context engine (`crates/lantern-context`, Rust + tree-sitter + SQLite FTS5). For every question it assembles relevant code from the symbol graph, full-text search, git co-change history, and project memory within a token budget.

Languages the engine understands (symbols, calls, map, impact radius): Rust, Python, TypeScript/JavaScript, Java, Go, C#. Other files still open with syntax colors, but have no symbols.

**Public benchmark.** 60 real commits from [Flask](https://github.com/pallets/flask) (Python), [Hono](https://github.com/honojs/hono) (TypeScript), and [Apache Commons Lang](https://github.com/apache/commons-lang) (Java). The question is the commit message, the answer files are the code files that commit changed, and each question is measured at the commit's parent. Nobody hand-picks the answers. Same 8,000-token budget, no model calls:

| | Keyword search (grep, read whole files) | Keyword search (grep, snippets around matches) | Lantern |
|---|---|---|---|
| Answer files retrieved (mean recall) | 13% | 41% | **88%** |
| An answer file among the first 3 files | 15% | 30% | **65%** |
| Same questions in Korean: recall / first 3 | 13% / 13% | 38% / 27% | **76% / 43%** |

Picking as many files at random as Lantern returns would hit an answer file 10% of the time. Korean questions are human translations of the same commit messages. Reproduce with `node eval/bench/run.mjs` ([results](eval/bench/결과.md), [method](eval/README.md)). Bring your own project and questions with `eval/retrieval.mjs`.

The engine also runs on its own as a CLI and an MCP server, so you can use it from other agents:

```bash
cargo build --release -p lantern-context
./target/release/lantern -C <project> context "where is the session cookie issued?"
./target/release/lantern -C <project> graph impact src/auth/session.ts --lines 40-60   # blast radius as JSON
claude mcp add lantern -- <path-to-lantern> mcp <project>
```

## Install

Download from [Releases](https://github.com/Lantern-IDE/lantern/releases). Builds are not code-signed yet:

- **Windows** (`*-setup.exe`): when SmartScreen warns you, choose **More info → Run anyway**.
- **macOS** (`*.dmg`, Apple Silicon and Intel, untested): drag Lantern to Applications, then run `xattr -cr /Applications/Lantern.app` or allow it in **System Settings → Privacy & Security → Open Anyway**.
- **Linux** (`*.AppImage`, `*.deb`, untested).

On first run, **Connect an AI Model** walks you through it (later: **Settings → Models**): an Anthropic or OpenAI-compatible key, or a local model through Ollama / LM Studio (free, and your code never leaves the machine). Keys go to the OS credential store or environment variables, never to the config file.

## Build from source

Requirements: Rust (stable), Node.js 22, and on Windows WebView2 (preinstalled on Windows 11). Linux needs `libwebkit2gtk-4.1-dev` and friends (see [CI](.github/workflows/ci.yml)).

```bash
cd app
npm install
npx tauri dev            # development
npx tauri build          # installer in target/release/bundle
```

## Tests

```bash
cargo test --workspace                     # context engine + app backend
cargo clippy --workspace --all-targets -- -D warnings
cd app && npm run typecheck && npm test    # frontend
cd app && npx tauri build --no-bundle && npm run e2e   # drives the real app (Windows)
python scripts/i18n_check.py               # missing English strings
```

## Project layout

| Path | What |
|---|---|
| `crates/lantern-context` | Context engine library, `lantern` CLI, MCP server |
| `app/` | The IDE: Tauri v2 + CodeMirror 6 frontend, Rust backend in `app/src-tauri` |
| `eval/` | Retrieval evaluation and model A/B harness |
| `docs/` | Design notes (Korean): context engine, IDE, the task-and-map design, release guide |

## Language

The UI ships in Korean and English. Source comments and design docs are mostly Korean; issues and pull requests in English or Korean are both welcome.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Security issues: [SECURITY.md](SECURITY.md).

## License

[Apache-2.0](LICENSE). Third-party notices: [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
