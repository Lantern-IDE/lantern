# lantern check on public commits: results

Written by `node eval/check/run.mjs`. Method and limits: [README.md](README.md). Lantern ea865cf, 2026-11-02, win32, node v24.16.0; cap 600 s per repository.

71 commits with 2 or more gold files, 152 applications (one gold file applied to the parent), 182 (applied file, other gold file) pairs.

## Does a warning point to the other gold files?

| Method | Commits with a hit | Applications with a hit | Pairs found | Files flagged per application (mean / median) | Files flagged per commit (mean / median) | Flagged files outside gold (upper bound on noise) | Of those, other files the commit changed |
|---|---|---|---|---|---|---|---|
| `lantern check` | 6 / 71 (8.5%) | 7 / 152 (4.6%) | 8 / 182 (4.4%) | 0.16 / 0 | 0.35 / 0 | 17 / 25 (68.0%) | 4 |
| check, Dangling only | 4 / 71 (5.6%) | 4 / 152 (2.6%) | 4 / 182 (2.2%) | 0.03 / 0 | 0.07 / 0 | 1 / 5 (20.0%) | 0 |
| check, Stale caller only | 3 / 71 (4.2%) | 4 / 152 (2.6%) | 5 / 182 (2.7%) | 0.14 / 0 | 0.30 / 0 | 16 / 21 (76.2%) | 4 |
| Impact radius: co-change + callers, 2 hops (`graph impact`) | 57 / 71 (80.3%) | 107 / 152 (70.4%) | 130 / 182 (71.4%) | 19.60 / 13 | 41.96 / 38 | 2849 / 2979 (95.6%) | 125 |
| Impact radius, co-change only | 40 / 71 (56.3%) | 74 / 152 (48.7%) | 89 / 182 (48.9%) | 6.28 / 8 | 13.45 / 16 | 866 / 955 (90.7%) | 90 |
| Impact radius, direct callers only | 48 / 71 (67.6%) | 70 / 152 (46.1%) | 81 / 182 (44.5%) | 11.57 / 7 | 24.77 / 22 | 1678 / 1759 (95.4%) | 49 |
| Every symbol, every reference, every co-change pair | 58 / 71 (81.7%) | 113 / 152 (74.3%) | 138 / 182 (75.8%) | 30.14 / 25 | 64.52 / 55 | 4443 / 4581 (97.0%) | 153 |
| Every symbol, references only | 51 / 71 (71.8%) | 74 / 152 (48.7%) | 86 / 182 (47.3%) | 17.39 / 10 | 37.23 / 30 | 2557 / 2643 (96.7%) | 64 |
| Every co-change pair only | 43 / 71 (60.6%) | 93 / 152 (61.2%) | 112 / 182 (61.5%) | 17.69 / 14 | 37.87 / 35 | 2577 / 2689 (95.8%) | 126 |

## Warnings from `lantern check`

| | Count |
|---|---|
| Warnings, all applications | 63 |
| Dangling | 5 |
| Stale caller | 58 |
| In test files | 48 |
| Warnings per commit, mean / median / max | 0.89 / 0 / 35 |
| Dangling per commit, mean / median / max | 0.07 / 0 / 2 |
| Stale caller per commit, mean / median / max | 0.82 / 0 / 35 |
| Commits with at least one warning | 9 / 71 |
| Applications with at least one warning | 12 / 152 |
| Applications with a removed symbol or a changed signature | 28 / 152 |
| Commits with a removed symbol or a changed signature in some gold file | 21 / 71 |
| Commits with a hit, among those | 6 / 21 |
| Applications with more warnings than the 20 shown without `--all` | 1 |
| Gold hits only past those 20 | 0 |
| Applications where check saw anything but the one applied file | 0 |
| Applied files outside the languages Lantern parses | 0 |

## By repository

| Repository | Commits measured | Applications | With removed or changed signature | Commits hit: check | Commits hit: impact | Commits hit: every | Dangling | Stale caller | Files per application: check / impact / every |
|---|---|---|---|---|---|---|---|---|---|
| axios | 8 of 8 | 16 | 4 | 0 | 6 | 6 | 0 | 0 | 0 / 17.19 / 45.31 |
| axum | 4 of 4 | 10 | 3 | 0 | 4 | 4 | 0 | 0 | 0 / 38.50 / 53.20 |
| commons-lang | 3 of 3 | 7 | 0 | 0 | 2 | 2 | 0 | 0 | 0 / 10.57 / 9.43 |
| flask | 11 of 11 | 23 | 7 | 1 | 11 | 11 | 0 | 20 | 0.65 / 31.13 / 49.96 |
| gin | 7 of 7 | 14 | 3 | 1 | 5 | 5 | 1 | 35 | 0.29 / 21.64 / 20.43 |
| hono | 5 of 5 | 10 | 2 | 0 | 3 | 3 | 0 | 0 | 0 / 5.40 / 8.20 |
| httpx | 13 of 13 | 26 | 3 | 2 | 11 | 12 | 3 | 0 | 0.12 / 15.35 / 31.38 |
| jq | 9 of 9 | 21 | 3 | 0 | 7 | 7 | 0 | 0 | 0 / 13.86 / 17.33 |
| okhttp | 8 of 8 | 17 | 3 | 2 | 6 | 6 | 1 | 3 | 0.18 / 16.53 / 23.76 |
| vite | 3 of 3 | 8 | 0 | 0 | 2 | 2 | 0 | 0 | 0 / 25.13 / 19.63 |

## Run time

`lantern check`, one process per application, one file changed, index already at the parent: median 474.5 ms, 90th percentile 736 ms, max 3270 ms; 151 of 152 within 3 s. `graph impact` on a ready index: median 121 ms, max 1966 ms. The first `graph impact` of each commit also builds the index from empty: median 959 ms, max 7968 ms.

| Repository | Wall time (s) | Of which setup (s) | Cap reached | Gold blobs fetched into the temp clone | `check` median / max (ms) |
|---|---|---|---|---|---|
| axios | 24.5 | 1.8 | no | 4 of 16 | 464.5 / 691 |
| axum | 14.1 | 1.3 | no | 5 of 10 | 464.5 / 536 |
| commons-lang | 15.8 | 1.0 | no | 5 of 7 | 466 / 785 |
| flask | 22.2 | 1.1 | no | 13 of 23 | 375 / 670 |
| gin | 15.1 | 1.0 | no | 2 of 14 | 405 / 604 |
| hono | 17.9 | 1.3 | no | 2 of 10 | 395.5 / 466 |
| httpx | 45.0 | 1.2 | no | 3 of 26 | 581 / 2193 |
| jq | 40.0 | 2.0 | no | 1 of 21 | 506 / 3270 |
| okhttp | 44.9 | 0.9 | no | 0 of 17 | 659 / 1345 |
| vite | 31.5 | 0.6 | no | 0 of 8 | 657.5 / 1325 |
| **Total** | 271.3 | 12.1 | 0 repositories | 35 of 152 | |
