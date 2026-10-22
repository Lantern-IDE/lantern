# Analysis plan for Korean questions translated by a different model (KO-NLLB)

Written on 2026-10-22, after the freeze at tag `icpc2027-eval-v6` and before any of these questions was generated. Changes to this plan after generation will be listed at the end with their reasons.

## Why

A reviewer pointed out that one LLM (Claude Opus 5.5) wrote the Korean questions, Lantern's dictionary and the rewrites. If the dictionary works because it shares the question writer's vocabulary, the paper's main finding (Lantern's smaller top-3 drop comes from its dictionary, and chunk BM25 given the same terms matches Lantern) may not hold for Korean written by someone else. This control gives every method Korean that no Claude model wrote.

## Questions

- Source: the English commit subject (`q`) of all 320 questions.
- Translator: NLLB-200 distilled 600M (Meta), `eng_Latn` → `kor_Hang`, the same pinned revision as KO→MT, beam search with 4 beams, at most 128 new tokens: `python eval/bilingual/translate.py --to-korean`. The output goes to `q_ko_nllb` as it comes out. No question is edited, filtered or regenerated.
- These are machine translations of the subjects, not developers' questions: they are literal and may translate, transliterate or drop identifiers. They test the vocabulary concern only.

## Frozen before measuring

- Engine, dictionary, baselines, budget, metrics and scripts: as in `icpc2027-eval-v6`. Nothing is changed after seeing these questions; only the new question form and its plumbing are added.
- Exclusions, decided now: empty translations (counted and reported). Nothing else is excluded.

## Measurement

Appended at the end of the `run_all.sh` order, so every existing result stays as it was (checked against the v6 results): Lantern, Lantern without Korean term expansion, Lantern + dense, BM25 (file, chunk), dense, hybrid on `ko-nllb`; BM25 and the hybrid given Lantern's dictionary terms at half weight, as in Table I (`ko-nllb-dictw`). Dictionary terms come from `expand.py` (`q_ko_nllb_dict_terms`).

## Pre-specified comparisons (261 questions not used during development; paired bootstrap 95% CI over questions, 10,000 resamples, seed 7; cluster bootstrap over repositories as a further check)

1. Dictionary hit rate (questions where the dictionary adds an English word) on KO-NLLB vs KO (191 of 261).
2. Difference in drop from English: (Lantern on KO-NLLB − Lantern on EN) − (same for the hybrid), A@3 and MRR.
3. Lantern − hybrid on KO-NLLB (A@3, MRR).
4. Lantern − Lantern without Korean term expansion on KO-NLLB (A@3, MRR).
5. Lantern on KO-NLLB − chunk BM25 on KO-NLLB with the dictionary terms at half weight (A@3, MRR).

## Reporting

All five, whatever they show, in `results-unseen.md` and in the paper's threats paragraph, labeled as added after the freeze. A few translations are quoted as examples. If the dictionary fires much less often, or Lantern's smaller drop disappears, the paper says so and narrows its claim to the LLM-written questions.
