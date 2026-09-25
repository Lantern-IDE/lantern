// 화면과 무관한 순수 로직 (Diff 해석, 맥락 해석, 단축키)
import { describe, expect, it } from "vitest";
import { diffTarget } from "../../src/lib/diff";
import { parseContext } from "../../src/lib/context";
import { displayKey, fromEvent } from "../../src/lib/keys";

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
