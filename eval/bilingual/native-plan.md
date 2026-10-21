# Analysis plan for Korean questions written by developers

Written on 2026-10-21, before any question was collected. Changes to this plan after data collection will be listed at the end with their reasons.

## Why

All Korean questions in the benchmark were drafted and checked by an LLM. This study asks whether the main conclusions hold for questions written by Korean developers: Lantern ties the BM25+dense hybrid in Korean, Korean questions lose accuracy against English, and Lantern loses less than the hybrid.

## Questions

- 42 questions, 6 per repository from the 7 repositories never used during Lantern's development (axios, axum, gin, httpx, jq, okhttp, vite), chosen at random among questions not used during development: `python eval/bilingual/native_sheet.py export <sheet>.xlsx --dir <clones>` (seed 7).
- Writers see the code change of the commit with file names removed, not the commit subject, and write how they would ask a coding agent for that change, before the change. No translation tools or LLMs. They may use names that exist in the code before the change, but not file names or names that only appear in added lines.
- 5–10 Korean developers. Each question gets one writer (A) and, where possible, a second writer (B) who does not see A's question. Writers are recorded only as codes (W1, W2, …); no personal data is collected. Writers agree that their questions are published in this repository under Apache-2.0.

## Frozen before measuring

- Engine and dictionary: the evaluated version (tag `icpc2027-eval-v2`). No change to the engine, its dictionary, the baselines or these scripts before the measurement.
- Exclusions, decided now: empty questions; questions that name a gold file or path (counted and reported). Nothing else is excluded.

## Measurement

`bash eval/bilingual/run_all.sh <clones>` measures the questions as `ko-native` (writer A) and `ko-native-b` (writer B) with the same methods, budget and metrics as the paper. `report.py` writes the comparisons below; nothing else is used as a primary result.

## Pre-specified comparisons (per writer, same questions, paired bootstrap 95% CI over questions, 10,000 resamples, seed 7)

1. Lantern − hybrid on the developers' Korean questions (A@3, MRR).
2. Developers' Korean − LLM Korean, for Lantern and for the hybrid (A@3, MRR).
3. Difference in drop from English: (Lantern on developers' Korean − Lantern on English) − (same for the hybrid).
4. Dictionary hit rate (questions where Lantern's dictionary adds an English word) on developers' vs LLM questions.

## Reading the results

With about 40 questions, intervals will be wide; we will report the direction and the intervals, not significance alone. The conclusions are supported if, on developers' questions, Lantern and the hybrid stay within a few points of each other and Lantern's drop from English is not larger than the hybrid's. If the results go the other way, for example because developers use more Hangul loanwords that the dictionary does not cover, we will report that as it is.
