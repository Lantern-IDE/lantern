# Rating plan for the LLM-written Korean questions

Written on 2026-10-21, before any rating. The author, a native Korean speaker and developer, rates a random sample of the LLM-written Korean questions. This is a check of the existing questions, not a rewrite: no question is changed, because the author has already seen the results.

## Sample

30 questions drawn at random (seed 11) from the 261 questions not used during development: `python eval/bilingual/ko_rating.py export <sheet>.xlsx`. The rater sees the English commit subject and the Korean question only, not the gold files or any retrieval result.

## Criteria (fixed now)

- Naturalness: 3 = a Korean developer would ask it this way; 2 = understandable but translated or awkward in places; 1 = a Korean developer would not ask it this way.
- Meaning against the English subject: same; partly (a condition, negation or object is missing or changed); different.
- A rating, once written, is not revised.

## Reporting

`ko_rating.py import` writes `ratings/ko-ratings.json` and the counts per category. We report the counts as they are, with the limitation that there is one rater, who knows the study. Any question rated as "different" in meaning is listed in the report; it stays in the benchmark (removing it after seeing results would be a post hoc change), but we also report the main comparisons without those questions.
