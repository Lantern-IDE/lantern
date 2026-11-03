// @vitest-environment happy-dom
// 편집기 언어 표: 표에 없으면 일반 텍스트로 열린다 (#158)
import { describe, expect, it } from "vitest";
import { langName } from "../../src/lib/langs";

describe("langName", () => {
  it.each([
    ["Makefile", "Makefile"],
    ["sub/GNUmakefile", "Makefile"],
    ["rules.mk", "Makefile"],
    ["build.gradle", "Gradle"],
    ["build.gradle.kts", "Kotlin"],
    ["api/user.proto", "Protocol Buffers"],
    ["src/App.svelte", "Svelte"],
    [".vscode/settings.jsonc", "JSON with Comments"],
    ["Dockerfile.dev", "Dockerfile"],
    ["notes", "일반 텍스트"],
  ])("%s → %s", (path, name) => {
    expect(langName(path)).toBe(name);
  });
});
