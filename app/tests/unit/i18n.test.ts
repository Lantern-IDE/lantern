// @vitest-environment happy-dom
// 영어 화면 번역: 사전 형식, 템플릿 순서 바꾸기, 복수형
import { beforeAll, describe, expect, it } from "vitest";
import en from "../../src/locales/en.json";

let t: (s: string) => string;

beforeAll(async () => {
  localStorage.setItem("lang", JSON.stringify("en"));
  ({ t } = await import("../../src/i18n"));
});

describe("번역 사전", () => {
  it("템플릿의 자리표시자 수가 원문과 같다", () => {
    for (const [ko, out] of Object.entries(en.templates as Record<string, string>)) {
      const n = ko.split("{}").length - 1;
      const plain = (out.match(/\{\}/g) ?? []).length;
      const numbered = new Set(out.match(/\{\d+\}/g) ?? []).size;
      expect(plain + numbered, `${ko} → ${out}`).toBe(n);
    }
  });

  it("번역문에 한국어가 남아 있지 않다", () => {
    const hangul = /[가-힣]/;
    const allowed = new Set(["한국어"]);
    for (const [ko, out] of Object.entries(en.exact as Record<string, string>)) {
      if (allowed.has(out)) continue;
      expect(hangul.test(out), `${ko} → ${out}`).toBe(false);
    }
  });
});

describe("t()", () => {
  it("그대로 일치", () => {
    expect(t("새 작업")).toBe("New Task");
  });

  it("앞뒤 공백은 유지", () => {
    expect(t("  새 작업 ")).toBe("  New Task ");
  });

  it("사전에 없거나 한국어가 없으면 그대로", () => {
    expect(t("src/app.ts")).toBe("src/app.ts");
    expect(t("사전에 없는 문장")).toBe("사전에 없는 문장");
  });

  it("템플릿은 순서를 바꿀 수 있다", () => {
    expect(t("3개 파일에서 7개 결과")).toBe("7 results in 3 files");
  });

  it("긴 템플릿이 짧은 템플릿보다 먼저 맞는다", () => {
    expect(t("맥락 엔진 인덱스: 파일 12개 (3개 갱신, 2ms). 눌러서 맥락 미리보기")).toBe("Context engine index: 12 files (3 updated, 2ms). Click to open Context Preview");
  });

  it("영어 복수형: 1은 단수", () => {
    expect(t("1개 파일에서 1개 결과")).toBe("1 result in 1 file");
  });

  it("구분자로 이어진 조각은 각각 번역", () => {
    expect(t("src/app.ts · 수정됨")).toBe("src/app.ts · Modified");
  });
});
