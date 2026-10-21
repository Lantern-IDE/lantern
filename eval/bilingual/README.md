# English–Korean parallel benchmark (eval/bilingual)

[한국어](README.ko.md)

Each question is paired in English and Korean with the same gold files, so the effect of **changing only the language of the question** on repository-level context retrieval can be measured. It extends [eval/bench](../bench) (3 repositories, 60 questions).

| | |
|---|---|
| Repositories | 10, in 8 programming languages (table below) |
| Questions | 320 (the 32 most recent eligible commits per repository); 212 are from 2026 |
| Questions text | English: the commit message subject. Korean: an LLM draft, checked by a second, independent LLM pass (4 of 320 edited) |
| Gold files | Code files changed by that commit (tests, build scripts, examples and docs excluded). 1 file: 249 questions, 2 files: 61, 3 files: 10 |
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
| `q_ko_llm`, `q_ko_llm_src` | `q_ko` rewritten into an English code-search query by an LLM, one call per question (see below), for the rewrite-then-retrieve baseline; `q_ko_llm_src` is the Korean text it was made from |
| `q_ko_native`, `q_ko_native_by` | Optional: a Korean question written from scratch by a person ([native_sheet.py](native_sheet.py)) |
| `q_para` | `q` reworded in English by an LLM (control for rewording, see below) |
| `q_ko_loan`, `q_ko_loan_dict_terms` | `q_ko` rewritten by an LLM with more Hangul loanwords (stress test, see below), and the English words the dictionary adds to it |
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
- **Korean → LLM rewrite** (`q_ko_llm`, [rewrite.py](rewrite.py)): what a coding agent could do with one extra model call. Claude Opus 5.5 (`claude-opus-5-5`), the same model that drafted and checked the Korean questions, was called once per question through Claude Code's headless mode without tools, given only the repository name and the Korean question, with prompt 3 in [prompts.md](prompts.md) (keep every identifier, add nothing that is not in the question, reply with the query only).
  [leak.py](leak.py) checks the rewrites ([leak-check.md](leak-check.md)). On the 261 unseen questions, 44 (17%) add a code identifier that is not in the Korean question and 24 (9%) name a gold file, often with a plain word such as `router` or `logger`; together 61, which suggests the model knows these popular repositories. Only 9 (3%) reproduce the English commit subject verbatim, equally often for commits before and after the model's training-data cutoff (June 2026: 5/145 vs 4/116), so the rewrites do not recall the commits themselves. The rewrites are longer than the subjects (10 words vs 7 on average) and make the hybrid better than in English (A@3 70.9 vs 68.2); without the 61 suspect rewrites it still reaches 68.0 (English 65.0, Korean 57.5). Treat this baseline as a strong reference point that profits from the model's knowledge of the repositories and from sharing a model with the Korean questions, not as an upper bound.
  Cost ([measurements/rewrite.json](measurements/rewrite.json), 261 calls): median 3.4 s of API time (90th percentile 5.0 s), 25 output tokens and $0.01 at list price per question. Up to tag `icpc2027-eval-v4`, `q_ko_llm` came from one batched pass over all questions instead (see prompts.md).
- **English reworded** (`q_para`, `en-para`): a control for rewording alone. The same model rewrote each English subject in its own words, with the same guidelines as the Korean drafts (natural phrasing, mixed requests/questions/issue titles, identifiers kept, nothing added), but staying in English. The drop from `en` to `en-para` is what rewording costs; the rest of the drop to `ko` is the language.
- **Korean with more loanwords** (`q_ko_loan`, `ko-loan`): a stress test, not native questions. The same model rewrote each Korean question the way developers who use many Hangul loanwords type (요청 → 리퀘스트, 기본값 → 디폴트, progress → 프로그레스), keeping identifiers and meaning; 51 of 320 had nothing to change. On the 261 unseen questions the dictionary then fires on 147 (56%, vs 191) and 56 questions have no English word (vs 27). Chunk-level BM25, the hybrid and Lantern then lose another 8.0, 7.3 and 6.1 A@3 points; Lantern (49.8) still ties the hybrid (49.0).
- **Korean + dictionary, half weight** (`ko-dictw`, baselines only): the same dictionary words, but scored separately at half weight as Lantern does (BM25 score of the question words + 0.5 × BM25 score of the added words), so common verbs do not outweigh the question's identifiers. The dense side gets the same text as `ko-dict`. Sensitivity (`--dict-weight`, unseen 261): chunk-level BM25 A@3 is 51.3 without the dictionary and 55.2 / 57.1 / 57.1 / 55.9 at weight 0.25 / 0.5 / 0.75 / 1.0 (1.0 = appending, `ko-dict`), so the dictionary matters more than the exact weight.
- **Written by developers** (`q_ko_native`, `q_ko_native_b`): [native_sheet.py](native_sheet.py) picks 6 questions per repository from the 7 repositories never used during development (42) and makes a sheet plus an HTML page that shows each commit's code change with file names removed, not the commit subject (showing the subject would make people translate it). A developer writes how they would ask a coding agent for that change, without translation tools; a second developer can write the same question independently (B). Writers are recorded as codes only, and agree that the questions are published here. The analysis is fixed in advance in [native-plan.md](native-plan.md); `import` also drops questions that name a gold file, as the plan says.

```bash
python eval/bilingual/native_sheet.py export native.xlsx --dir <clones>   # also writes native.html
python eval/bilingual/native_sheet.py import native.xlsx
```

## Reproducing

Requirements (versions used): Rust 1.97, Node.js 24, Python 3.12 with `pip install -r eval/bilingual/requirements.txt` (model revisions are pinned in `baselines.py`), git, and bash (Git Bash on Windows). Aider RepoMap is optional and needs its own virtual environment, since aider-chat 0.86.2 installs only on Python 3.10–3.12. The first run needs the network: `eval/bench/run.mjs` clones the 10 repositories (blob-less, so blobs are fetched at each checkout; about 250 MB in total), and the embedding model (about 470 MB) and NLLB (about 2.5 GB, only for the translation cost) are downloaded once.

```bash
cargo build --release -p lantern-context
python -m venv .venv-aider && .venv-aider/bin/pip install aider-chat==0.86.2      # optional (Windows: .venv-aider\Scripts\pip)
git clone https://github.com/microsoft/vscode <vscode> && git -C <vscode> checkout 76e9dff43b237f3fcbb8a289ed2f8bdf28105baf   # optional, for the vscode row of the scale table
VSCODE=<vscode> bash eval/bilingual/run_all.sh <folder for the clones> [.venv-aider/bin/python]
```

The script checks that every method sees the same files (`check_files.py`), dictionary-expands the Korean questions, runs Lantern, the ablations, BM25 / dense / RRF, Aider RepoMap and Lantern with dense retrieval on every question form, measures scale and costs, and writes the reports (`results-unseen.md` for the main results, `results-fresh-repos.md`, `results-dev-repos.md`, `results-all.md`, `results-test.md`), the leak check (`leak-check.md`), the raw timings (`measurements/`) and the paper's two tables (`<clones>/results/bilingual/tables/`). It makes no model API calls and costs nothing: the LLM-written forms (`q_ko_draft`, `q_ko`, `q_ko_llm`, `q_para`, `q_ko_loan`) and the NLLB translations (`q_ko_mt`) are stored in the JSON files. `rewrite.py` regenerates `q_ko_llm` with real model calls (about $4 at list price), and deleting `q_ko_mt_src` makes `translate.py` translate again. Building embeddings on a CPU takes a few hours the first time (vectors are cached by chunk text).

The results were measured on Windows 11 (Intel Core i5-11400), where Git for Windows checks files out with CRLF line endings by default (`core.autocrlf=true`). Every method reads the same bytes, so the token estimates include the carriage returns; with LF checkouts on other platforms the estimates are slightly smaller, so packed contexts, and the last digits of some results, can differ. Lantern breaks score ties by symbol id, which depends on the order in which files were indexed ([#139](https://github.com/Lantern-IDE/lantern/issues/139)), so `run_all.sh` starts from an empty index.

| Script | What it does |
|---|---|
| [run_all.sh](run_all.sh) | Everything below in order |
| [../bench/run.mjs](../bench/run.mjs) `--set eval/bilingual` | Keyword search (two variants) and Lantern |
| [check_files.py](check_files.py) | Checks that the baselines, RepoMap and keyword search see exactly the files Lantern indexes (`.gitignore`, hidden paths, `vendor/` and other skipped folders, 1 MiB, binary or minified files), at each repository's snapshot and three question commits |
| [baselines.py](baselines.py) | BM25 (file, 40-line chunk), dense retrieval (multilingual-e5-small; another model with e.g. `--embed-model BAAI/bge-m3 --tag bgem3`, not part of `run_all.sh` because it takes about 19 hours on 4 CPU cores), BM25 + dense with RRF. Tokens: identifiers whole and split into words, as in Lantern; Hangul runs are kept as whole tokens (they match only Korean comments or strings) |
| [expand.py](expand.py) | Korean question + the English words Lantern's dictionary adds (`q_ko_dict`) and the dictionary hit rate, also for the loanword form |
| [rewrite.py](rewrite.py) | The KO→LLM rewrite: one real model call per question through Claude Code (uses your account); stores each call's output, time, tokens and price in [measurements/rewrite.json](measurements/rewrite.json) |
| [prompts.md](prompts.md) | The exact prompts of every LLM step: Korean drafts, their review, the KO→LLM rewrite, the English rewording control and the loanword stress test |
| [ko_rating.py](ko_rating.py) | Sheet for the author to rate (not edit) 30 random LLM-written Korean questions for naturalness and meaning; criteria fixed in advance in [ko-rating-plan.md](ko-rating-plan.md) |
| [native_sheet.py](native_sheet.py) | Sheet for Korean questions written from scratch by a person (`q_ko_native`) |
| [leak.py](leak.py) | Leak check of the LLM rewrites, written to [leak-check.md](leak-check.md): new identifiers, gold file names, overlap with the English original, verbatim rate before and after the model's training-data cutoff (Fisher's exact test), A@3 without the suspect rewrites |
| [hybrid_cost.py](hybrid_cost.py) | Cost of the BM25 + dense hybrid on the same CPU: embedding every chunk from scratch, and time per question ([measurements/hybrid_cost.json](measurements/hybrid_cost.json)) |
| [nllb_cost.py](nllb_cost.py) | Cost of translate-then-retrieve: NLLB-200 time per question, one question at a time, and peak memory ([measurements/nllb.json](measurements/nllb.json)) |
| [scale_questions.txt](scale_questions.txt) | Questions used only to time context assembly on a repository outside the benchmark (`scale.py --extra vscode=<clone>`) |
| [errors.py](errors.py) | Questions Lantern answers in English but not in Korean; causes are classified in [error-analysis.md](error-analysis.md) (16 of 26 miss a dictionary word, 11 of them Hangul transliterations of English terms) |
| [repomap_baseline.py](repomap_baseline.py) | Aider RepoMap (aider-chat 0.86.2; question words passed as mentioned identifiers) |
| [embed_server.py](embed_server.py) | Local `/embeddings` server so Lantern's semantic search uses the same embedding model |
| [translate.py](translate.py) | Korean → English machine translation |
| [scale.py](scale.py) | Indexing time and peak memory, index size, per-question assembly time; every run is kept in [measurements/scale.json](measurements/scale.json), and `--from` rebuilds the table from it without measuring again |
| [report.py](report.py) | Recall@8k, R@10f, A@1/3/5, MRR, paired bootstrap 95% intervals (per question, and per repository for the key comparisons), McNemar tests with Holm correction, per-repository and per-subgroup tables, LaTeX tables (`--tex`); the reports are in English |

All methods get the same 8,000-token budget and the same files (the engine's file rules, checked by `check_files.py`); tokens are estimated as 4 characters per token. The main results use the 261 questions that do not overlap with [eval/bench](../bench) (`report.py --subset unseen`, written to `results-unseen.md`), because eval/bench was used to check ranking and dictionary changes while developing the engine. The engine, its dictionary and its weights were not changed after this benchmark was built: the dictionary last changed in ee4c6dd (2026-10-14), the benchmark was built on 2026-10-16, and the only later engine commits are a CLI fix for questions starting with `-` (5206286) and the ablation switch (d108680, a no-op when unset). A hash-based dev/test split (a question is dev if the first 8 hex digits of its commit hash, as an integer, modulo 10 are 0–2, about 30%) is reported in `results-test.md` for reference and kept for future tuning.

### Where the paper's numbers come from

| In the paper | Command | Output |
|---|---|---|
| Table I, results and robustness paragraphs | `report.py --subset unseen --tex …` | `results-unseen.md`, `tables/results.tex` |
| Repositories never used during development; development repositories | `report.py --subset fresh-repos`, `--subset dev-repos` | `results-fresh-repos.md`, `results-dev-repos.md` |
| Table II and Lantern's cost | `scale.py --json … --tex …` | `measurements/scale.json`, `tables/scale.tex` |
| Cost of the hybrid, of NLLB and of the LLM rewrite | `hybrid_cost.py`, `nllb_cost.py`, `rewrite.py` | `measurements/hybrid_cost.json`, `measurements/nllb.json`, `measurements/rewrite.json` |
| Dictionary hit rates (KO and loanword form) | `expand.py` | printed |
| Leak check of the rewrites, identifiers lost by NLLB | `leak.py` | `leak-check.md` |
| Error analysis | `errors.py` | `error-analysis.md` (classified by reading) |

## Repositories never used during development

The 59 development questions come from flask, hono and commons-lang. On the other seven repositories (224 questions, `report.py --subset fresh-repos`, [results-fresh-repos.md](results-fresh-repos.md)) the conclusions are the same: Lantern trails the hybrid in English (A@3 −9.4), ties it in Korean (+0.9), its smaller drop from English is significant even by repository (+10.3, [1.8, 19.2]), removing the dictionary costs 4.9, and chunk-level BM25 with the dictionary at half weight does as well (Lantern −1.3, not significant). With only 7–10 repositories, repository-level intervals are approximate. On the 37 unseen questions of the three development repositories (`--subset dev-repos`, [results-dev-repos.md](results-dev-repos.md)) Lantern does not fare better: English +2.7 against the hybrid, Korean −8.1, and its Korean drop is 4 questions larger than the hybrid's, so there is no sign of overfitting to those repositories.

## Notes on Lantern's cost measurements

`scale.py` measures full indexing from an empty index three times and reports the median, so the file cache is warm after the first run (vscode: 70.9, 39.5 and 39.4 s in [measurements/scale.json](measurements/scale.json); an earlier first run on a freshly cloned vscode took 92 s); Lantern parses with up to 8 threads. vscode is measured at commit `76e9dff` with the ten questions in [scale_questions.txt](scale_questions.txt); an earlier draft reported 136 ms from two ad-hoc questions. The engine is unchanged across all measurements (last engine change d108680); a rebuilt binary gives identical file lists on gin and flask.

## Cost of the hybrid baseline

[hybrid_cost.py](hybrid_cost.py) on the same CPU as Lantern's scale table (Intel Core i5-11400), multilingual-e5-small with 4 threads, every chunk embedded from scratch at each repository's snapshot; per question = BM25 + query embedding + similarity + RRF + packing, median of 10 Korean questions ([measurements/hybrid_cost.json](measurements/hybrid_cost.json)). Lantern indexes the same repositories in 2–5 s and assembles a context in 5–14 ms. Translate-then-retrieve adds NLLB-200: a median of 2.0 s per question (90th percentile 2.8 s) when questions are translated one at a time with 4 threads, 3.6 s to load the model and about 3 GB of peak memory ([nllb_cost.py](nllb_cost.py), [measurements/nllb.json](measurements/nllb.json)).

| Repository | Chunks | BM25 index (s) | Embedding (s) | Per question (ms) |
|---|---|---|---|---|
| axios | 1,375 | 0.3 | 117 | 16 |
| axum | 1,357 | 0.3 | 119 | 18 |
| commons-lang | 5,484 | 1.5 | 552 | 27 |
| flask | 502 | 0.2 | 45 | 17 |
| gin | 661 | 0.2 | 62 | 16 |
| hono | 2,380 | 0.5 | 225 | 19 |
| httpx | 473 | 0.1 | 46 | 14 |
| jq | 733 | 0.2 | 69 | 16 |
| okhttp | 4,025 | 0.9 | 360 | 21 |
| vite | 3,847 | 0.9 | 270 | 23 |

## Limitations

- Commit messages are written after the fact and contain more identifiers than real user questions (only commits that mention a gold file name are skipped).
- Gold files are the files that were changed; files one had to read but did not change are not gold.
- Korean questions are LLM translations checked by an LLM, not questions written by Korean developers, and some LLM phrasing may remain.
- Because the drafts came from English, translating them back can land close to the original wording, which may favor translate-then-retrieve (hence the translation model differs from the drafting LLM) and even more the LLM rewrite (same model family as the drafting LLM).
- Lantern's Korean dictionary was written with the same model that wrote the Korean questions (Claude Opus 5.5, through Claude Code), so the two may share vocabulary even though the question writer never saw the dictionary. The person-written subset (`q_ko_native`) is meant to measure this.
- The LLM rewrite adds what the model knows about these popular repositories (17% of rewrites add an identifier absent from the question); for private or less-known code it may help less.
- Dense retrieval uses only the small multilingual-e5-small (also for Lantern + dense, so those comparisons are matched); larger multilingual models such as BGE-M3 were not evaluated.
- Ten repositories: repository-level bootstrap intervals are wide, and results vary by repository (see the per-repository table in the report).
- These are popular repositories, so older commits may be in model training data. Commit dates are kept so 2026-only results can be reported.

## License

The question files reuse commit subjects and file paths from the listed open-source repositories (MIT, Apache-2.0, BSD, and jq's license); see each repository. The Korean questions, scripts and annotations are released with Lantern under Apache-2.0.
