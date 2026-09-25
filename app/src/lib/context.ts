// 맥락 엔진이 만든 Markdown 해석 (순수 함수: 단위 테스트 대상)

export interface CtxSymbol { name: string; kind: string; line: number; end: number; signatureOnly: boolean; why: string }
export interface CtxFile { path: string; symbols: CtxSymbol[] }

/** 맥락 엔진이 만든 Markdown에서 파일·심볼·포함 이유를 뽑는다 (assemble.rs의 to_markdown 형식). */
export function parseContext(md: string): { files: CtxFile[]; memory: string[] } {
  const files: CtxFile[] = [];
  const memory: string[] = [];
  let inFence = false;
  let section: "memory" | "file" | null = null;
  for (const line of md.split("\n")) {
    if (line.startsWith("```")) {
      inFence = !inFence;
      continue;
    }
    if (inFence) continue;
    if (line.startsWith("## ")) {
      const title = line.slice(3).trim();
      if (title === "프로젝트 메모리") section = "memory";
      else {
        section = "file";
        files.push({ path: title, symbols: [] });
      }
      continue;
    }
    if (!line.startsWith("### ")) continue;
    if (section === "memory") memory.push(line.slice(4).trim());
    else if (section === "file") {
      const m = line.match(/^### `(.+?)` \((.+?), L(\d+)(?:-(\d+))?\)( · 시그니처만)? — (.*)$/);
      if (m) files.at(-1)!.symbols.push({ name: m[1], kind: m[2], line: Number(m[3]), end: Number(m[4] ?? m[3]), signatureOnly: !!m[5], why: m[6] });
    }
  }
  return { files, memory };
}
