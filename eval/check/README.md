# `lantern check` on public commits (eval/check)

Does `lantern check` point to the other files a commit had to change, and how many warnings does it give? No model, no API key, no cost.

The commits are the multi-file commits of the [English–Korean benchmark](../bilingual) (10 repositories, 8 languages). No person picks them; [make.mjs](../bench/make.mjs) takes them from history.

## Command

```bash
cargo build --release -p lantern-context
node eval/check/run.mjs --dir <folder with the clones> [--max-seconds 180] [--repos flask,gin] [--lantern <binary>] [--keep-index] [--out <folder>]
node eval/check/run.mjs --summarize   # rewrites results.md from results.json without measuring
```

`--dir` is the folder `node eval/bench/run.mjs --set eval/bilingual --dir <folder>` clones into. `--lantern` defaults to `release/lantern` under `CARGO_TARGET_DIR` or `target`. Requires Node.js 24 (`node:sqlite`) and git. Output: [results.md](results.md) (tables) and [results.json](results.json) (one row per application, with every warning). The published run was started at below-normal priority, with `--max-seconds 600` because other jobs were running on the PC.

## Method

For each commit in `eval/bilingual/*.json` with 2 or more gold files (the code files it changed; tests, build scripts, examples and docs are never gold):

1. A temporary clone (`git clone --shared --no-checkout --no-tags`, `core.autocrlf=false`, hooks off, its own index and data folders) is checked out at the commit's parent, and the index starts empty, as in the other benchmarks. The clones in `--dir` are not changed: the script compares their config, HEAD, index, object folder and pack list before and after each repository and stops if anything differs. Repositories run one after another.
2. **Naive radius, at the parent state.** For each gold file, before anything is applied:
   - *Impact radius*: `lantern graph impact <file> --lines <whole file>`, the same calculation as the IDE's approval card: up to 8 co-changed files, plus direct callers and callers of callers of the innermost symbols (names defined more than 3 times are skipped; at most 60 callers per symbol).
   - *Every symbol*: the same index read directly, for every symbol in the file including classes and other containers: all references in the same language family, and all co-change pairs. Names defined more than 3 times are skipped here too. No other cap.

   Co-change comes from the last 500 commits up to the parent, so the commit itself cannot leak in. Neither radius looks at what the commit changed.
3. **One file at a time.** Only that file's bytes from the commit are written over the parent's, and `lantern check --base <parent> --json --all` runs. Then the file is restored and the next one is applied. `--all` keeps warnings past the 20 that are shown by default, so kinds can be counted exactly. The JSON holds every warning; the text report hides Stale caller unless `--details` is given, because that kind did not reach its precision bar.

**Network.** `lantern` uses none. The clones from `eval/bench/run.mjs` are blob-less, so they hold every parent's files but not always the commit's version of a gold file (35 of 152 were missing). The script fetches exactly those blobs from the public remote into the temporary clone, before measuring. While measuring, `GIT_NO_LAZY_FETCH=1` keeps git off the network.

**Cap.** At most 180 s per repository by default (`--max-seconds`), checked between commits. Commits past the cap are reported as not measured. In the published run no repository reached its cap.

## What counts

- **Application**: one gold file applied alone. A commit with *k* gold files gives *k* applications and *k*(*k*−1) pairs.
- **Hit**: a warning, or a file in the naive radius, whose path is one of the commit's *other* gold files. Reported per commit (at least one of its applications hits), per application and per pair. A commit-level hit is the fairest of the three: if file A changes a signature and file B calls it, applying A can hit but applying B alone has nothing to warn about, so application and pair rates cannot reach 100%.
- **Warnings per commit**: summed over the commit's applications, split by kind (Dangling, Stale caller). One warning is one (changed symbol, referring file).
- **Outside gold**: flagged files that are not gold. A warning there is not necessarily wrong (the file may really need a change that the commit left for later, or a test may have changed), so this is an **upper bound on noise**, not a false-positive rate. results.md also counts those the commit did change anyway, tests for example: 4 of the 17 for `lantern check`.
- **Run time**: wall time of the `lantern check` process, index refresh included, with the index already at the parent state and one file changed. Measured on a Windows 11 PC (Intel Core i5-11400) at below-normal priority.

## Results

Lantern `ea865cf`, 2026-11-02. 71 commits, 152 applications, 182 pairs. All tables are in [results.md](results.md).

| Method | Commits with a hit | Applications with a hit | Pairs found | Files flagged per commit (mean / median) | Flagged files outside gold (upper bound on noise) |
|---|---|---|---|---|---|
| `lantern check` | 6 / 71 (8.5%) | 7 / 152 (4.6%) | 8 / 182 (4.4%) | 0.35 / 0 | 17 / 25 (68.0%) |
| check, Dangling only | 4 / 71 (5.6%) | 4 / 152 (2.6%) | 4 / 182 (2.2%) | 0.07 / 0 | 1 / 5 (20.0%) |
| check, Stale caller only | 3 / 71 (4.2%) | 4 / 152 (2.6%) | 5 / 182 (2.7%) | 0.30 / 0 | 16 / 21 (76.2%) |
| Impact radius | 57 / 71 (80.3%) | 107 / 152 (70.4%) | 130 / 182 (71.4%) | 41.96 / 38 | 2849 / 2979 (95.6%) |
| Every symbol, every reference, every co-change pair | 58 / 71 (81.7%) | 113 / 152 (74.3%) | 138 / 182 (75.8%) | 64.52 / 55 | 4443 / 4581 (97.0%) |

| `lantern check` warnings | |
|---|---|
| Total | 63: 5 Dangling, 58 Stale caller |
| Per commit, mean / median / max | 0.89 / 0 / 35 |
| Dangling per commit, mean / median / max | 0.07 / 0 / 2 |
| Stale caller per commit, mean / median / max | 0.82 / 0 / 35 |
| Commits with at least one warning | 9 / 71 |
| In test files | 48 / 63 |
| Commits with a removed symbol or a changed signature in some gold file | 21 / 71 |
| Commits with a hit, among those | 6 / 21 |

How to read this:

- `lantern check` is quiet and misses most of these commits. In 50 of 71 commits no gold file removes a symbol or changes a signature, so there is nothing for it to report by design. Among the other 21, 6 hit.
- One commit gives 35 of the 63 warnings, all in test files (gin `41d8591eb1`, `context.go`).
- The wide nets reach about 80% of the commits but flag 42 to 65 files per commit, more than 95% of them outside gold. A commit's other files are usually near it in the graph; the graph alone does not say which ones need a change.
- Dangling flagged 5 files and 4 are gold. That is too few to call a precision.
- One hit read by hand: applying only `fs.go` of gin `8791c96960` gives a HIGH warning at `routergroup.go:221` for `onlyFilesFS`, and the commit changes exactly that line to `OnlyFilesFS`.

Run time: `lantern check` median 474.5 ms, 90th percentile 736 ms, max 3270 ms; 151 of 152 within 3 s. The whole run took 271.3 s, of which setup 12.1 s; the longest repository was httpx at 45.0 s. Builds were running on the PC at low priority, so read these as rough.

Index check, measured before tuning at `639b2e1`: `--keep-index` reuses one index across a repository's commits. Compared with the empty-index run, all warnings and both "every symbol" and co-change numbers are identical on all 152 applications. Only the impact radius changes, in 26 applications, most likely because `graph impact` gets callers from an unordered query with a limit. Commits hit stay at 57 for the impact radius and go from 48 to 47 for direct callers alone.

## What tuning changed

Rules came from reading warnings on real sessions (see [docs/에이전트_부검.md](../../docs/에이전트_부검.md)): methods and functions must be linked by the caller naming the file or type, an import, or the package; `private` definitions and framework entry points are out; Java boxing, TypeScript parameter widening and Kotlin inserted defaults count as compatible.

Before, at `639b2e1`: 65 warnings (5 Dangling, 60 Stale caller), 27 flagged files. After: 63 warnings, 25 flagged files. **All 8 pairs that hit stay.** The 2 warnings that left are in test files outside gold and call a different `complete` (`promise.complete(...)`, `completableFuture.complete(...)`). This benchmark was not fully held out: the first version of the link rule also dropped 3 real calls written `flask.redirect(...)` after `import flask`, so package-qualified calls were added and the benchmark measured again.

## Limits

- **No judgement of the warnings.** A hit is a path match, not proof that the warning is about the same symbol, and a warning outside gold is not proof of noise. Whether warnings are right was measured separately by reading a sample on the author's own sessions: 0 of 30 Stale caller warnings were real, before the tuning above, and no Dangling warning survived the history check. That is one person's sessions, so it says little about these repositories.
- **Small sample.** 6 commits hit out of 71, and 5 Dangling warnings in total. Small changes in the commit list would move these percentages.
- **Small commits only.** The benchmark skips commits that add, delete or rename files or touch more than 3 code files. Large refactors, where missed callers would be expected, are not here.
- **Not an agent.** Applying one file of a finished human commit imitates an agent that stopped early. The other files of a commit may be linked by behaviour rather than by a name or a signature, which check does not look at by design, and a person's commit may keep a call site valid in ways a partial edit does not.
- **Reads are ignored.** Here check has no session, so it cannot drop callers the agent already read. With a session log, Stale caller warnings can only get fewer.
- **Different units.** One warning is one (symbol, file); the comparison between methods uses distinct files. Per-commit counts add up the commit's applications, so a file flagged from two applications counts twice.
- **Name based.** Dynamic dispatch, dependency injection and name collisions are not resolved. Names defined more than 5 times (check) or 3 times (both radii) are skipped, and check also skips short and generic names.
- **Radius caps.** The impact radius keeps its product caps (8 co-changed files, 60 callers per symbol, the second hop stops at 60 callers), and it looks only at innermost symbols; "every symbol" has neither limit. Co-change only counts pairs seen at least twice in the last 500 commits, ignoring merges and commits with more than 30 files.
- **Timing.** Windows only, below-normal priority, one process per application, a shared PC. Parsing the first index may use up to 8 threads for a moment. The time does not include building the index, which the `graph impact` call before it does.
