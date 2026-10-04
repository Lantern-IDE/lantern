// '테스트 없음' → 테스트 만들기 요청 글. 모델에게 보내는 내용이라 화면 번역 대상이 아니고 화면 언어로 만든다.

export interface TestTarget { path: string; symbols: string[] }

/** 엔진 graph::is_test_path와 같은 기준 */
export function isTestPath(path: string): boolean {
  const p = path.toLowerCase().replace(/\\/g, "/");
  const file = p.split("/").pop() ?? "";
  return /(^|\/)tests?\//.test(p) || p.includes("__tests__") || /\.(test|spec)\./.test(p) || p.includes("_test.") || file.startsWith("test_");
}

export function testPrompt(targets: TestTarget[], examples: string[], lang: "ko" | "en"): string {
  const ko = lang === "ko";
  const line = (t: TestTarget) => `- ${t.path}${t.symbols.length ? `: ${t.symbols.join(", ")}` : ""}`;
  return [
    ko
      ? "다음 코드에 대한 테스트를 만들어줘. Lantern 영향 반경에서 이 코드를 확인하는 테스트가 없었다."
      : "Write tests for the following code. Lantern's impact radius found no tests covering it.",
    ...targets.map(line),
    examples.length
      ? ko
        ? `이 프로젝트의 기존 테스트 파일 위치와 형식을 따라라: ${examples.join(", ")}`
        : `Follow the location and style of this project's existing tests: ${examples.join(", ")}`
      : ko
        ? "이 프로젝트에는 같은 언어의 테스트 파일이 없다. 언어의 일반적인 테스트 위치와 형식을 쓰고, 어떤 테스트 도구를 썼는지 알려줘."
        : "This project has no tests in this language yet. Use the language's usual test location and style, and say which test tool you used.",
    ko
      ? "정상 경우와 경계 경우를 다루고, 이 코드를 부르는 쪽이 기대하는 동작도 확인해라. 만든 뒤 테스트를 돌리는 명령을 알려줘."
      : "Cover the normal and edge cases, and check what the callers expect. When done, tell me the command to run the tests.",
  ].join("\n");
}
