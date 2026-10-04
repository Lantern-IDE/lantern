// 커밋 전 영향 검토 → AI 리뷰 요청 글. 모델에게 보내는 내용이라 화면 번역(i18n) 대상이 아니고,
// 화면 언어에 맞춰 한국어·영어로 만든다 (모델은 질문한 언어로 답한다).
import type { Impact } from "../map";

export interface FileReview { path: string; status: "modified" | "added" | "deleted"; added: number; removed: number; impact: Impact | null }
export interface Review { files: FileReview[]; risk: "low" | "medium" | "high"; untested: string[]; framework: string[]; diff: string }

const RISK = {
  ko: { low: "낮음", medium: "보통", high: "높음" },
  en: { low: "low", medium: "medium", high: "high" },
} as const;

export function reviewPrompt(r: Review, lang: "ko" | "en"): string {
  const risk = RISK[lang];
  const ko = lang === "ko";
  const lines = r.files.map((f) => {
    const i = f.impact;
    if (!i) return `- ${f.path} (${f.status === "deleted" ? (ko ? "삭제" : "deleted") : ko ? "분석 안 함" : "not analyzed"})`;
    if (!i.supported) return `- ${f.path}: ${ko ? "이 언어는 호출 관계를 분석하지 않음" : "calls not analyzed for this language"}`;
    const tests = i.tests.length ? (ko ? `테스트 ${i.tests.length}개` : `${i.tests.length} tests`) : ko ? "영향 범위에 테스트 없음" : "no tests in range";
    const fw = i.framework.length ? (ko ? `, 프레임워크가 부름(${i.framework.join(", ")})` : `, called by a framework (${i.framework.join(", ")})`) : "";
    const touched = i.touched.join(", ") || (ko ? "없음" : "none");
    return ko
      ? `- ${f.path}: 위험도 ${risk[i.risk]}, 바뀐 심볼 ${touched}, 직접 호출 ${i.callers} · 간접 ${i.callers2}, ${tests}${fw}`
      : `- ${f.path}: risk ${risk[i.risk]}, changed symbols ${touched}, ${i.callers} direct · ${i.callers2} indirect callers, ${tests}${fw}`;
  });
  return [
    ko
      ? "아직 커밋하지 않은 변경을 리뷰해줘. 버그 가능성, 호출하는 쪽에 미치는 영향, 빠진 테스트를 중심으로, 고쳐야 할 것부터 짧게."
      : "Review my uncommitted changes. Focus on likely bugs, effects on callers, and missing tests; list what to fix first, briefly.",
    "",
    ko ? `Lantern 영향 검토 (이름 기준 정적 분석, 전체 위험도 ${risk[r.risk]}):` : `Lantern impact review (name-based static analysis, overall risk ${risk[r.risk]}):`,
    ...lines,
    "",
    "```diff",
    r.diff,
    "```",
  ].join("\n");
}
