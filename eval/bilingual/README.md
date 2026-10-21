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
| `q_ko_dict`, `q_ko_dict_terms` | `q_ko` plus the English words Lantern's Korean dictionary adds to it (`q_ko_dict_terms`, empty if the dictionary does not fire), so the baselines can get the same dictionary |
| `q_ko_llm` | `q_ko` rewritten into an English code-search query by an LLM (see below), for the rewrite-then-retrieve baseline |
| `q_ko_native`, `q_ko_native_by` | Optional: a Korean question written from scratch by a person ([native_sheet.py](native_sheet.py)) |
| `q_para` | `q` reworded in English by an LLM (control for rewording, see below) |
| `q_ko_loan` | `q_ko` rewritten by an LLM with more Hangul loanwords (stress test, see below) |
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

## Other forms of the Korean question

Each form gives every method the same input, so the effect of each step can be separated.

- **Korean + dictionary** (`q_ko_dict`, [expand.py](expand.py)): the English words Lantern's dictionary adds (the ASCII `query_terms` of `lantern context --json` that do not come from the question's own identifiers) are appended to the Korean question. BM25 and the embedding baselines then get the same dictionary as Lantern. The dictionary fires on 236 of 320 questions (191 of the 261 unseen ones). Lantern is not run on this form, since it applies the same dictionary itself.
- **Korean → LLM rewrite** (`q_ko_llm`): what a coding agent could do with one extra model call. An LLM agent (Claude, as a Claude Code subagent) received only the Korean questions and repository names, with this instruction, and wrote all 320 in one pass:

  > You are the query-rewriting step of a coding agent. A developer asked a question in Korean about an open-source repository; before searching the code, you rewrite it as an English code-search query. (1) Write a short English query (one line) that a coding agent would use to search this repository for the code to read or change: say what the code does or what should change, in the words developers use in English code and commit messages. (2) Keep every code identifier, option, flag, header, function and type name from the Korean exactly as written. (3) Do not add information that is not in the Korean question (no guessed file names, function names or causes), and do not look anything up: do not open any repository, website, commit or source code. Work only from the Korean text and the repository name. (4) One query per item; no explanations.

  The model was Claude Opus 5.5 (`claude-opus-5-5`), the same model that drafted and checked the Korean questions. [leak.py](leak.py) checks the rewrites: on the 261 unseen questions, no rewrite adds a code identifier or gold file name that is not in the Korean question, but 47 (18%) reproduce the English commit subject verbatim, and rewrites share more words with it than the NLLB back-translations (Jaccard 0.71 vs 0.42). Without those 47, the hybrid still reaches A@3 66.4 (English 68.7, Korean 56.5). Verbatim reconstructions are about as frequent for commits after the model's training-data cutoff (June 2026: 18/116 = 15.5% vs 29/145 = 20.0% before, Fisher p = 0.42), so they point to transparent translation rather than memorized commit history. Treat this baseline as a strong reference point that may be favored by the shared model, not as an upper bound. Cost ([rewrite_latency.py](rewrite_latency.py), one call per question through Claude Code, 30 questions): median 2.6 s of API time (90th percentile 4.8 s), about 29 output tokens and $0.01 at list price per question.
- **English reworded** (`q_para`, `en-para`): a control for rewording alone. The same model rewrote each English subject in its own words, with the same guidelines as the Korean drafts (natural phrasing, mixed requests/questions/issue titles, identifiers kept, nothing added), but staying in English. The drop from `en` to `en-para` is what rewording costs; the rest of the drop to `ko` is the language.
- **Korean with more loanwords** (`q_ko_loan`, `ko-loan`): a stress test, not native questions. The same model rewrote each Korean question the way developers who use many Hangul loanwords type (요청 → 리퀘스트, 기본값 → 디폴트, progress → 프로그레스), keeping identifiers and meaning; 51 of 320 had nothing to change. On the 261 unseen questions the dictionary then fires on 147 (56%, vs 191) and 56 questions have no English word (vs 27). Chunk-level BM25, the hybrid and Lantern then lose another 8.0, 7.3 and 6.1 A@3 points; Lantern (49.8) still ties the hybrid (48.7).
- **Korean + dictionary, half weight** (`ko-dictw`, baselines only): the same dictionary words, but scored separately at half weight as Lantern does (BM25 score of the question words + 0.5 × BM25 score of the added words), so common verbs do not outweigh the question's identifiers. The dense side gets the same text as `ko-dict`. Sensitivity (`--dict-weight`, unseen 261): chunk-level BM25 A@3 is 51.3 without the dictionary and 54.0 / 55.9 / 55.6 / 54.4 at weight 0.25 / 0.5 / 0.75 / 1.0 (1.0 = appending, `ko-dict`), so the dictionary matters more than the exact weight.
- **Written by developers** (`q_ko_native`, `q_ko_native_b`): [native_sheet.py](native_sheet.py) picks 6 questions per repository from the 7 repositories never used during development (42) and makes a sheet plus an HTML page that shows each commit's code change with file names removed, not the commit subject (showing the subject would make people translate it). A developer writes how they would ask a coding agent for that change, without translation tools; a second developer can write the same question independently (B). Writers are recorded as codes only, and agree that the questions are published here. The analysis is fixed in advance in [native-plan.md](native-plan.md); `import` also drops questions that name a gold file, as the plan says.

```bash
python eval/bilingual/native_sheet.py export native.xlsx --dir <clones>   # also writes native.html
python eval/bilingual/native_sheet.py import native.xlsx
```

## Reproducing

```bash
cargo build --release -p lantern-context
bash eval/bilingual/run_all.sh <folder for the clones> [python with aider-chat installed]
```

It translates and dictionary-expands the Korean questions, then runs Lantern, BM25 / dense / RRF, Aider RepoMap, Lantern with dense retrieval, and the scale measurement for every question form, and writes a comparison report (`results-all.md` for all questions, `results-test.md` for the test split, `results-unseen.md` for the main results). There are no model API calls and nothing costs money (`q_ko_llm` is stored, not regenerated), but building embeddings on a CPU takes several hours the first time (vectors are cached).

| Script | What it does |
|---|---|
| [../bench/run.mjs](../bench/run.mjs) `--set eval/bilingual` | Keyword search (two variants) and Lantern |
| [baselines.py](baselines.py) | BM25 (file, 40-line chunk), dense retrieval (multilingual-e5-small; another model with e.g. `--embed-model BAAI/bge-m3 --tag bgem3`, not part of `run_all.sh` because it takes about 19 hours on 4 CPU cores), BM25 + dense with RRF. Tokens: identifiers whole and split into words, as in Lantern; Hangul runs are kept as whole tokens (they match only Korean comments or strings) |
| [expand.py](expand.py) | Korean question + the English words Lantern's dictionary adds (`q_ko_dict`) and the dictionary hit rate |
| [native_sheet.py](native_sheet.py) | Sheet for Korean questions written from scratch by a person (`q_ko_native`) |
| [leak.py](leak.py) | Leak check of the LLM rewrites (new identifiers, gold file names, overlap with the English original, verbatim rate before/after the model's training-data cutoff) |
| [hybrid_cost.py](hybrid_cost.py) | Cost of the BM25 + dense hybrid on the same CPU: embedding every chunk from scratch, and time per question |
| [scale_questions.txt](scale_questions.txt) | Questions used only to time context assembly on a repository outside the benchmark (`scale.py --extra vscode=<clone>`) |
| [rewrite_latency.py](rewrite_latency.py) | Latency, tokens and price of the LLM rewrite, one real model call per question (uses your account) |
| [errors.py](errors.py) | Questions Lantern answers in English but not in Korean; causes are classified by hand in [error-analysis.md](error-analysis.md) (16 of 26 miss a dictionary word, 11 of them Hangul transliterations of English terms) |
| [repomap_baseline.py](repomap_baseline.py) | Aider RepoMap (aider-chat 0.86.2; question words passed as mentioned identifiers) |
| [embed_server.py](embed_server.py) | Local `/embeddings` server so Lantern's semantic search uses the same embedding model |
| [translate.py](translate.py) | Korean → English machine translation |
| [scale.py](scale.py) | Indexing time and peak memory, index size, per-question assembly time |
| [report.py](report.py) | Recall@8k, R@10f, Acc@1/3/5, MRR, paired bootstrap 95% CIs (per question, and per repository for the key comparisons), McNemar tests with Holm correction, per-repository and per-subgroup tables, LaTeX tables (`--tex`) |

All methods get the same 8,000-token budget and the same file filter; tokens are estimated as 4 characters per token. The main results use the 261 questions that do not overlap with [eval/bench](../bench) (`report.py --subset unseen`, written to `results-unseen.md`), because eval/bench was used to check ranking and dictionary changes while developing the engine. Dev/test split: a question is dev if the first 8 hex digits of its commit hash, as an integer, modulo 10 are 0–2 (about 30%), otherwise test. Engine dictionaries and weights are tuned on dev only; results are reported on test.

## Repositories never used during development

The 59 development questions come from flask, hono and commons-lang. On the other seven repositories (224 questions, `report.py --subset fresh-repos`, [results-fresh-repos.md](results-fresh-repos.md)) the conclusions are the same: Lantern trails the hybrid in English (A@3 −8.9), ties it in Korean (+1.3), its smaller drop from English is significant even by repository (+10.3, [1.8, 19.2]), removing the dictionary costs 4.9, and chunk-level BM25 with the dictionary at half weight matches it.

## Notes on Lantern's cost measurements

`scale.py` measures full indexing from an empty index three times and reports the median, with a warm file cache (the first run on a fresh vscode clone took 92 s; with a warm cache the median of three runs is 42 s); Lantern parses with up to 8 threads. vscode is measured at commit `76e9dff` with the ten questions in [scale_questions.txt](scale_questions.txt); an earlier draft reported 136 ms from two ad-hoc questions. The engine is unchanged across all measurements (last engine change d108680); a rebuilt binary gives identical file lists on gin and flask.

## Cost of the hybrid baseline

[hybrid_cost.py](hybrid_cost.py) on the same CPU as Lantern's scale table (Intel Core i5-11400), multilingual-e5-small with 4 threads, every chunk embedded from scratch at each repository's snapshot; per question = BM25 + query embedding + similarity + RRF + packing, median of 10 Korean questions. Lantern indexes the same repositories in 2–5 s and assembles a context in 5–12 ms.

| Repository | Chunks | BM25 index (s) | Embedding (s) | Per question (ms) |
|---|---|---|---|---|
| axios | 1,393 | 1.7 | 137 | 16 |
| axum | 1,357 | 1.2 | 139 | 17 |
| commons-lang | 5,003 | 4.2 | 588 | 29 |
| flask | 502 | 0.4 | 54 | 17 |
| gin | 661 | 0.7 | 78 | 20 |
| hono | 2,380 | 2.1 | 262 | 18 |
| httpx | 473 | 0.5 | 54 | 19 |
| jq | 1,251 | 0.4 | 147 | 18 |
| okhttp | 4,025 | 3.4 | 433 | 25 |
| vite | 3,877 | 2.5 | 334 | 33 |

## Limitations

- Commit messages are written after the fact and contain more identifiers than real user questions (only commits that mention a gold file name are skipped).
- Gold files are the files that were changed; files one had to read but did not change are not gold.
- Korean questions are LLM translations checked by an LLM, not questions written by Korean developers, and some LLM phrasing may remain.
- Because the drafts came from English, translating them back can land close to the original wording, which may favor translate-then-retrieve (hence the translation model differs from the drafting LLM) and even more the LLM rewrite (same model family as the drafting LLM).
- Lantern's Korean dictionary was written with an AI coding assistant, and the Korean questions by an LLM; a model of the same family may share vocabulary with the dictionary even without seeing it. The person-written subset (`q_ko_native`) is meant to measure this.
- Dense retrieval uses only the small multilingual-e5-small (also for Lantern + dense, so those comparisons are matched); larger multilingual models such as BGE-M3 were not evaluated.
- Ten repositories: repository-level bootstrap intervals are wide, and results vary by repository (see the per-repository table in the report).
- These are popular repositories, so older commits may be in model training data. Commit dates are kept so 2026-only results can be reported.

## License

The question files reuse commit subjects and file paths from the listed open-source repositories (MIT, Apache-2.0, BSD, and jq's license); see each repository. The Korean questions, scripts and annotations are released with Lantern under Apache-2.0.
