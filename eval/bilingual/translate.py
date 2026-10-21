"""한국어 질문(q_ko)을 영어로 기계 번역해 q_ko_mt에 넣는다 ("번역한 뒤 검색" 비교용).

    python eval/bilingual/translate.py [--set eval/bilingual] [--model facebook/nllb-200-distilled-600M]

로컬 CPU에서 도는 공개 번역 모델을 써서 비용 없이 누구나 같은 결과를 낸다. 쓴 모델 이름은 각 JSON의 mt_model에 남긴다.
기본은 NLLB-200 600M(CC-BY-NC 4.0, 연구용). Helsinki-NLP/opus-mt-ko-en도 쓸 수 있지만 품질이 낮아 비교 상대로 약하다.
이미 번역한 문항은 한국어 질문과 모델이 같으면 그대로 둔다 (q_ko_mt_src로 확인).
transformers, sentencepiece가 필요하다.
"""

import argparse
import glob
import json
import os

from baselines import REVISION

HERE = os.path.dirname(os.path.abspath(__file__))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--set", default=HERE)
    ap.add_argument("--model", default="facebook/nllb-200-distilled-600M")
    a = ap.parse_args()
    nllb = "nllb" in a.model.lower()
    tok = model = extra = None
    total = 0
    for f in sorted(glob.glob(os.path.join(a.set, "*.json"))):
        with open(f, encoding="utf-8") as fh:
            s = json.load(fh)
        todo = [q for q in s["questions"] if q.get("q_ko") and (q.get("q_ko_mt_src") != q["q_ko"] or s.get("mt_model") != a.model)]
        if todo and model is None:
            # 번역할 문항이 있을 때만 모델을 받는다 (저장된 번역으로 재현할 때는 3GB 모델이 필요 없음)
            from transformers import AutoModelForSeq2SeqLM, AutoTokenizer

            tok = AutoTokenizer.from_pretrained(a.model, revision=REVISION.get(a.model), **({"src_lang": "kor_Hang"} if nllb else {}))
            model = AutoModelForSeq2SeqLM.from_pretrained(a.model, revision=REVISION.get(a.model))
            extra = {"forced_bos_token_id": tok.convert_tokens_to_ids("eng_Latn")} if nllb else {}
        for i in range(0, len(todo), 16):
            batch = todo[i : i + 16]
            enc = tok([q["q_ko"] for q in batch], return_tensors="pt", padding=True, truncation=True)
            out = model.generate(**enc, **extra, num_beams=4, max_new_tokens=128)
            for q, text in zip(batch, tok.batch_decode(out, skip_special_tokens=True)):
                q["q_ko_mt"] = text.strip()
                q["q_ko_mt_src"] = q["q_ko"]
        if todo:
            s["mt_model"] = a.model
            with open(f, "w", encoding="utf-8", newline="\n") as fh:
                fh.write(json.dumps(s, ensure_ascii=False, indent=2) + "\n")
        total += len(todo)
        print(f"{s['project']}: {len(todo)}개 번역")
    print(f"모두 {total}개")


if __name__ == "__main__":
    main()
