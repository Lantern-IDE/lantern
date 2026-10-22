# Rating plan for the LLM-written Korean questions

Written on 2026-10-21, before any rating; revised on 2026-10-22, still before any rating (see Changes). Native Korean developers rate a random sample of the LLM-written Korean questions. This is a check of the existing questions, not a rewrite: no question is changed, because the questions have already been measured.

## Sample

60 questions drawn at random (seed 11) from the 261 questions not used during development: `python eval/bilingual/ko_rating.py export <sheet>.xlsx --rater A` (and `--rater B`). The first 30 are the original 30-question sample. Raters see the English commit subject and the Korean question only, not the gold files or any retrieval result.

## Raters

- Rater A: the author, a native Korean speaker and developer, who knows the study.
- Rater B: another native Korean developer who has not seen the study's results. B rates independently, on a separate copy of the sheet, without seeing A's ratings.
- Raters are recorded only as A and B; no personal data is collected.

## Criteria (fixed now)

- Naturalness: 3 = a Korean developer would ask it this way; 2 = understandable but translated or awkward in places; 1 = a Korean developer would not ask it this way. Level 3 is the "would a developer write this" judgment.
- Meaning against the English subject: same; partly (a condition, negation or object is missing or changed); different.
- A rating, once written, is not revised.

## Reporting

`ko_rating.py import <sheet> --rater A|B` writes `ratings/ko-ratings-<rater>.json`; `ko_rating.py agree` prints each rater's counts, percent agreement, Cohen's kappa for meaning and quadratic-weighted kappa for naturalness on the questions both rated. We report these as they are. If only one rater finishes, we report that rater's counts with the limitation that there is one rater. Any question rated as "different" in meaning by either rater is listed in the report; it stays in the benchmark (removing it after seeing results would be a post hoc change), but we also report the main comparisons without those questions.

## Changes

- 2026-10-22, before any rating: following a review that asked for at least two native-speaker raters, 50–100 questions and inter-rater agreement, the sample grows from 30 to 60 (the first 30 unchanged), rater B and the agreement statistics are added. Criteria are unchanged.
