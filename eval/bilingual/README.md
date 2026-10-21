# English–Korean parallel benchmark (eval/bilingual)

[한국어](README.ko.md)

Each question is paired in English and Korean with the same gold files, so the effect of **changing only the language of the question** on repository-level context retrieval can be measured. It extends [eval/bench](../bench) (3 repositories, 60 questions).

| | |
|---|---|
| Repositories | 10, in 8 programming languages (table below) |
| Questions | 320 (the 32 most recent eligible commits per repository); 212 are from 2026 |
| Questions text | English: the commit message subject. Korean: an LLM draft, checked by a second, independent LLM pass (4 of 320 edited) |
| Gold files | Code files changed by that commit (tests, build scripts, examples and docs excluded). 1 file: 250 questions, 2 files: 60, 3 files: 10 |
| Snapshot | The parent of that commit (retrieval runs on the code before the fix) |

| Repository | Language | Snapshot ref |
|---|---|---|
| [flask](https://github.com/pallets/flask) | Python | `d73fa1cdcb` |
| [httpx](https://github.com/encode/httpx) | Python | `b5addb64f0` |
| [hono](https://github.com/honojs/hono) | TypeScript | `ee0622e144` |
| [vite](https://github.com/vitejs/vite) | TypeScript | `74ae7099f9` |
| [commons-lang](https://github.com/apache/commons-lang) | Java | `29624cdb50` |
| [gin](https://github.com/gin-gonic/gin) | Go | `0f09c3a9b4` |
| [axum](https://github.com/tokio-rs/axum) | Rust | `fc78d5bcdb` |
| [okhttp](https://github.com/square/okhttp) | Kotlin | `5a4b597d11` |
| [axios](https://github.com/axios/axios) | JavaScript | `326843a390` |
| [jq](https://github.com/jqlang/jq) | C | `6b53f80311` |

## Data format

One JSON file per repository: `project`, `url`, `ref` (snapshot), and `questions`, each with

| Field | Meaning |
|---|---|
| `q` | English question (commit subject, conventional-commit prefixes and PR numbers removed) |
| `q_ko_draft` | Korean draft written by an LLM agent that did not know this engine |
| `q_ko` | Korean question after review (`null` until reviewed); `q_ko_reviewed` says who reviewed (`"llm"`, or `true` for a person via the review sheet) |
| `q_ko_mt` | `q_ko` machine-translated back to English (NLLB-200 600M), for the translate-then-retrieve baseline |
| `golden` | Gold files, repository-relative |
| `commit`, `parent`, `date` | The commit, its parent (where retrieval runs), and the commit date |

## How the questions were built

Questions come from commit history automatically ([make.mjs](../bench/make.mjs)); no person picks questions or gold files.

```bash
node eval/bench/make.mjs --repo <clone> --name <name> --url <repo url> --count 32 --ref <snapshot> --out eval/bilingual
```

Commits are skipped if they are merges; version, docs, build, lint or dependency chores; add, delete or rename files; touch more than 3 code files; or mention a gold file name in the message. At most two questions share the same set of gold files. Tests (`test/`, `jvmTest/`, `*_test.go`, `…Test.kt`, test-support modules), Gradle build logic, and `examples/`, `docs/`, `playground/`, `scripts/`, `benchmarks/` are never gold files.

## Korean questions

Two steps:

1. **Draft** (`q_ko_draft`): an LLM agent that knew nothing about this engine or its Korean dictionary received only the English question and the writing guidelines. It did not see the repository, the commit or any code. A writer who knows the engine might lean toward the words its dictionary covers.
2. **Review** (`q_ko`): a second LLM agent, also unaware of the engine, checked every draft against the same guidelines (meaning, negations and conditions, no added information, natural phrasing) and edited 4 of 320. No person has reviewed them yet. The review sheet below lets a person review them; only questions marked as reviewed get a `q_ko`, and Korean results use reviewed questions only.

```bash
python eval/bilingual/ko_sheet.py export review.xlsx   # questions and drafts → review sheet
python eval/bilingual/ko_sheet.py import review.xlsx   # review sheet → q_ko (also reports the share of edited drafts)
```

Guidelines: write as a Korean developer would actually ask a colleague or an AI assistant; add no information that is not in the English question; code identifiers may stay in English; never look at the diff. The review sheet does not show gold files, so file names cannot leak into the questions.

For the **translate-then-retrieve** baseline, `q_ko` is translated back to English with a local model (NLLB-200 distilled 600M, CC-BY-NC 4.0) into `q_ko_mt` ([translate.py](translate.py)).

## Reproducing

```bash
cargo build --release -p lantern-context
bash eval/bilingual/run_all.sh <folder for the clones> [python with aider-chat installed]
```

It translates the Korean questions, then runs Lantern, BM25 / dense / RRF, Aider RepoMap, Lantern with dense retrieval, and the scale measurement, and writes a comparison report (`결과-비교.md` for all questions, `결과-비교-test.md` for the test split). There are no model API calls and nothing costs money, but building embeddings on a CPU takes a few hours the first time (vectors are cached).

| Script | What it does |
|---|---|
| [../bench/run.mjs](../bench/run.mjs) `--set eval/bilingual` | Keyword search (two variants) and Lantern |
| [baselines.py](baselines.py) | BM25 (file, 40-line chunk), dense retrieval (multilingual-e5-small), BM25 + dense with RRF |
| [repomap_baseline.py](repomap_baseline.py) | Aider RepoMap (aider-chat 0.86.2; question words passed as mentioned identifiers) |
| [embed_server.py](embed_server.py) | Local `/embeddings` server so Lantern's semantic search uses the same embedding model |
| [translate.py](translate.py) | Korean → English machine translation |
| [scale.py](scale.py) | Indexing time and peak memory, index size, per-question assembly time |
| [report.py](report.py) | Recall@8k, R@10f, Acc@1/3/5, MRR, paired bootstrap 95% CIs, McNemar tests, LaTeX tables (`--tex`) |

All methods get the same 8,000-token budget and the same file filter; tokens are estimated as 4 characters per token. The main results use the 261 questions that do not overlap with [eval/bench](../bench) (`report.py --subset unseen`, written to `결과-비교-unseen.md`), because eval/bench was used to check ranking and dictionary changes while developing the engine. Dev/test split: a question is dev if the first 8 hex digits of its commit hash, as an integer, modulo 10 are 0–2 (about 30%), otherwise test. Engine dictionaries and weights are tuned on dev only; results are reported on test.

## Limitations

- Commit messages are written after the fact and contain more identifiers than real user questions (only commits that mention a gold file name are skipped).
- Gold files are the files that were changed; files one had to read but did not change are not gold.
- Korean questions are LLM translations checked by an LLM, not questions written by Korean developers, and some LLM phrasing may remain.
- Because the drafts came from English, translating them back can land close to the original wording, which may favor translate-then-retrieve (hence the translation model differs from the drafting LLM).
- These are popular repositories, so older commits may be in model training data. Commit dates are kept so 2026-only results can be reported.

## License

The question files reuse commit subjects and file paths from the listed open-source repositories (MIT, Apache-2.0, BSD, and jq's license); see each repository. The Korean questions, scripts and annotations are released with Lantern under Apache-2.0.
