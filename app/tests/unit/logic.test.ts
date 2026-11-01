// 화면과 무관한 순수 로직 (Diff 해석, 맥락 해석, 단축키)
import { describe, expect, it } from "vitest";
import { changedSpans, diffTarget } from "../../src/lib/diff";
import { parseContext } from "../../src/lib/context";
import { displayKey, fromEvent } from "../../src/lib/keys";
import { isTestPath, testPrompt } from "../../src/lib/testPrompt";

describe("diffTarget: 바뀌는 원래 줄 범위", () => {
  it("가운데 한 줄 수정", () => {
    const d = ["--- a/src/a.ts", "+++ b/src/a.ts", "@@ -3,7 +3,7 @@", " a", " b", " c", "-old", "+new", " d", " e"].join("\n");
    expect(diffTarget(d)).toEqual({ path: "src/a.ts", ranges: [[6, 6]] });
  });

  it("끼워 넣기만 하면 그 앞 줄", () => {
    const d = ["--- a/x.rs", "+++ b/x.rs", "@@ -10,2 +10,3 @@", " keep", "+added", " keep2"].join("\n");
    expect(diffTarget(d)).toEqual({ path: "x.rs", ranges: [[10, 10]] });
  });

  it("여러 곳은 붙어 있으면 합치고 떨어져 있으면 나눈다", () => {
    const d = ["--- a/m.py", "+++ b/m.py", "@@ -1,3 +1,3 @@", "-a", "-b", "+ab", " c", "@@ -20,1 +20,1 @@", "-x", "+y"].join("\n");
    expect(diffTarget(d)?.ranges).toEqual([[1, 2], [20, 20]]);
  });

  it("새 파일은 영향 반경이 없다", () => {
    const d = ["--- a/new.ts", "+++ b/new.ts", "@@ -0,0 +1,2 @@", "+one", "+two"].join("\n");
    expect(diffTarget(d)).toBeNull();
  });
});

describe("changedSpans: 짝지은 -/+ 줄의 달라진 곳", () => {
  /** 줄마다 칠해지는 글자 */
  const marked = (...lines: string[]) => {
    const m = changedSpans(lines);
    return lines.map((l, i) => (m.has(i) ? l.slice(...m.get(i)!) : null));
  };

  it("가운데만 다르면 공통 앞뒤를 뺀다", () => {
    expect(marked("-  return v + '.sig';", "+  return v + '.signed';")).toEqual([null, "ned"]);
    expect(marked("-  a(1, 2);", "+  a(1, 3);")).toEqual(["2", "3"]);
  });

  it("줄 끝에 덧붙이면 + 줄만 칠한다", () => {
    expect(marked("-for x {", "+for x { // note")).toEqual([null, " // note"]);
  });

  it("공백 말고 같은 데가 없으면 줄 전체가 바뀐 것이라 칠하지 않는다", () => {
    expect(marked("-  one", "+  two")).toEqual([null, null]);
    expect(marked("-one\r", "+two\r")).toEqual([null, null]);
  });

  it("줄 수가 같은 덩어리만 차례로 짝짓는다", () => {
    expect(marked("-a1", "-b1", "+a2", "+b2")).toEqual(["1", "1", "2", "2"]);
    expect(marked("-a1", "-b1", "+a2")).toEqual([null, null, null]);
    expect(marked("-a1", " x", "+a2")).toEqual([null, null, null]);
  });

  it("머리 줄은 짝이 아니다", () => {
    expect(marked("--- a/x.ts", "+++ b/x.ts", "@@ -1 +1 @@", "-a1", "+a2")).toEqual([null, null, null, "1", "2"]);
  });

  it("아주 긴 줄은 건너뛴다", () => {
    const long = "x".repeat(2001);
    expect(marked(`-${long}a`, `+${long}b`)).toEqual([null, null]);
  });

  it("이모지를 가르지 않는다", () => {
    expect(marked("-ok 😀", "+ok 😁")).toEqual(["😀", "😁"]);
  });
});

describe("parseContext: 맥락 엔진 Markdown", () => {
  const md = [
    "# 프로젝트 맥락",
    "## 프로젝트 메모리",
    "### .lantern/memory/conventions.md",
    "- 세션은 issueSession으로",
    "## src/auth/session.ts",
    "### `issueSession` (function, L1-3) — 'session' 일치",
    "```ts",
    "## 코드 블록 안의 제목은 무시",
    "```",
    "### `signCookie` (function, L5) · 시그니처만 — `issueSession`가 사용",
  ].join("\n");

  it("파일, 심볼, 포함 이유, 메모리를 뽑는다", () => {
    const r = parseContext(md);
    expect(r.memory).toEqual([".lantern/memory/conventions.md"]);
    expect(r.files.map((f) => f.path)).toEqual(["src/auth/session.ts"]);
    const [a, b] = r.files[0].symbols;
    expect(a).toMatchObject({ name: "issueSession", kind: "function", line: 1, end: 3, signatureOnly: false, why: "'session' 일치" });
    expect(b).toMatchObject({ name: "signCookie", line: 5, end: 5, signatureOnly: true });
  });
});

describe("단축키", () => {
  const ev = (o: Partial<{ key: string; code: string; ctrlKey: boolean; metaKey: boolean; shiftKey: boolean; altKey: boolean }>) =>
    ({ key: "", code: "", ctrlKey: false, metaKey: false, shiftKey: false, altKey: false, ...o });

  it("키 이벤트 → 문자열 (⌘는 Ctrl로)", () => {
    expect(fromEvent(ev({ key: "g", code: "KeyG", ctrlKey: true, altKey: true }))).toBe("Ctrl+Alt+G");
    expect(fromEvent(ev({ key: "p", code: "KeyP", metaKey: true, shiftKey: true }))).toBe("Ctrl+Shift+P");
    expect(fromEvent(ev({ key: "F5", code: "F5" }))).toBe("F5");
    expect(fromEvent(ev({ key: "a", code: "KeyA" }))).toBeNull();
    expect(fromEvent(ev({ key: "Shift", code: "ShiftLeft", shiftKey: true }))).toBeNull();
  });

  it("macOS 표시", () => {
    expect(displayKey("Ctrl+Shift+P", true)).toBe("⇧⌘P");
    expect(displayKey("Ctrl+Alt+M", true)).toBe("⌥⌘M");
    expect(displayKey("Ctrl+,", true)).toBe("⌘,");
    expect(displayKey("Ctrl+Shift+P", false)).toBe("Ctrl+Shift+P");
  });
});

describe("테스트 만들기 요청", () => {
  it("엔진과 같은 기준으로 테스트 파일을 알아본다", () => {
    for (const p of ["tests/a.ts", "src/test/java/A.java", "a/__tests__/b.js", "x.test.ts", "y.spec.js", "pkg/z_test.go", "app/test_views.py"]) expect(isTestPath(p)).toBe(true);
    for (const p of ["src/auth/session.ts", "src/latest/a.ts", "contest.py"]) expect(isTestPath(p)).toBe(false);
  });

  it("대상과 예시 테스트를 담는다", () => {
    const p = testPrompt([{ path: "src/a.ts", symbols: ["f", "g"] }], ["tests/a.test.ts"], "ko");
    expect(p).toContain("- src/a.ts: f, g");
    expect(p).toContain("tests/a.test.ts");
    expect(testPrompt([{ path: "a.py", symbols: [] }], [], "en")).toContain("no tests in this language");
  });
});
