// 편집기 언어 표: 확장자·파일 이름 → CodeMirror 문법과 LSP 키. Tauri에 기대지 않아 단위 시험에서 바로 부른다
import type { Extension } from "@codemirror/state";
import { javascript } from "@codemirror/lang-javascript";
import { python } from "@codemirror/lang-python";
import { rust } from "@codemirror/lang-rust";
import { json } from "@codemirror/lang-json";
import { markdown } from "@codemirror/lang-markdown";
import { html } from "@codemirror/lang-html";
import { css } from "@codemirror/lang-css";
import { java } from "@codemirror/lang-java";
import { cpp } from "@codemirror/lang-cpp";
import { go } from "@codemirror/lang-go";
import { php } from "@codemirror/lang-php";
import { sql } from "@codemirror/lang-sql";
import { yaml } from "@codemirror/lang-yaml";
import { xml } from "@codemirror/lang-xml";
import { StreamLanguage, type StreamParser } from "@codemirror/language";
import { csharp, kotlin, scala, dart } from "@codemirror/legacy-modes/mode/clike";
import { shell } from "@codemirror/legacy-modes/mode/shell";
import { powerShell } from "@codemirror/legacy-modes/mode/powershell";
import { toml } from "@codemirror/legacy-modes/mode/toml";
import { ruby } from "@codemirror/legacy-modes/mode/ruby";
import { swift } from "@codemirror/legacy-modes/mode/swift";
import { lua } from "@codemirror/legacy-modes/mode/lua";
import { dockerFile } from "@codemirror/legacy-modes/mode/dockerfile";
import { properties } from "@codemirror/legacy-modes/mode/properties";
import { groovy } from "@codemirror/legacy-modes/mode/groovy";
import { protobuf } from "@codemirror/legacy-modes/mode/protobuf";

export interface Lang {
  name: string;
  cm: () => Extension;
  /** config.toml의 [lsp.<key>] */
  lsp?: string;
  /** LSP languageId */
  id?: string;
}

const LANGS: Record<string, Lang> = {
  ts: { name: "TypeScript", cm: () => javascript({ typescript: true }), lsp: "typescript", id: "typescript" },
  mts: { name: "TypeScript", cm: () => javascript({ typescript: true }), lsp: "typescript", id: "typescript" },
  cts: { name: "TypeScript", cm: () => javascript({ typescript: true }), lsp: "typescript", id: "typescript" },
  tsx: { name: "TypeScript JSX", cm: () => javascript({ typescript: true, jsx: true }), lsp: "typescript", id: "typescriptreact" },
  js: { name: "JavaScript", cm: () => javascript(), lsp: "typescript", id: "javascript" },
  mjs: { name: "JavaScript", cm: () => javascript(), lsp: "typescript", id: "javascript" },
  cjs: { name: "JavaScript", cm: () => javascript(), lsp: "typescript", id: "javascript" },
  jsx: { name: "JavaScript JSX", cm: () => javascript({ jsx: true }), lsp: "typescript", id: "javascriptreact" },
  py: { name: "Python", cm: () => python(), lsp: "python", id: "python" },
  pyi: { name: "Python", cm: () => python(), lsp: "python", id: "python" },
  pyw: { name: "Python", cm: () => python(), lsp: "python", id: "python" },
  rs: { name: "Rust", cm: () => rust(), lsp: "rust", id: "rust" },
  json: { name: "JSON", cm: () => json() },
  md: { name: "Markdown", cm: () => markdown() },
  html: { name: "HTML", cm: () => html() },
  css: { name: "CSS", cm: () => css() },
  scss: { name: "SCSS", cm: () => css() },
  java: { name: "Java", cm: () => java() },
  c: { name: "C", cm: () => cpp() },
  h: { name: "C", cm: () => cpp() },
  cpp: { name: "C++", cm: () => cpp() },
  cc: { name: "C++", cm: () => cpp() },
  cxx: { name: "C++", cm: () => cpp() },
  hpp: { name: "C++", cm: () => cpp() },
  cs: { name: "C#", cm: () => legacy(csharp) },
  kt: { name: "Kotlin", cm: () => legacy(kotlin) },
  kts: { name: "Kotlin", cm: () => legacy(kotlin) },
  scala: { name: "Scala", cm: () => legacy(scala) },
  dart: { name: "Dart", cm: () => legacy(dart) },
  swift: { name: "Swift", cm: () => legacy(swift) },
  go: { name: "Go", cm: () => go() },
  php: { name: "PHP", cm: () => php() },
  rb: { name: "Ruby", cm: () => legacy(ruby) },
  lua: { name: "Lua", cm: () => legacy(lua) },
  sql: { name: "SQL", cm: () => sql() },
  yaml: { name: "YAML", cm: () => yaml() },
  yml: { name: "YAML", cm: () => yaml() },
  toml: { name: "TOML", cm: () => legacy(toml) },
  xml: { name: "XML", cm: () => xml() },
  svg: { name: "XML", cm: () => xml() },
  vue: { name: "Vue", cm: () => html() },
  svelte: { name: "Svelte", cm: () => html() },
  // 주석이 있는 JSON: JSON 문법은 주석을 칠하지 못한다
  jsonc: { name: "JSON with Comments", cm: () => javascript() },
  gradle: { name: "Gradle", cm: () => legacy(groovy) },
  groovy: { name: "Groovy", cm: () => legacy(groovy) },
  proto: { name: "Protocol Buffers", cm: () => legacy(protobuf) },
  // Makefile 전용 문법이 없어 레시피가 셸인 점을 따라 셸로 칠한다
  makefile: { name: "Makefile", cm: () => legacy(shell) },
  mk: { name: "Makefile", cm: () => legacy(shell) },
  sh: { name: "Shell", cm: () => legacy(shell) },
  bash: { name: "Shell", cm: () => legacy(shell) },
  zsh: { name: "Shell", cm: () => legacy(shell) },
  ps1: { name: "PowerShell", cm: () => legacy(powerShell) },
  properties: { name: "Properties", cm: () => legacy(properties) },
  ini: { name: "INI", cm: () => legacy(properties) },
  env: { name: "Properties", cm: () => legacy(properties) },
  dockerfile: { name: "Dockerfile", cm: () => legacy(dockerFile) },
};

function legacy<T>(parser: StreamParser<T>): Extension {
  return StreamLanguage.define(parser);
}

export function langOf(path: string): (Lang & { ext: string }) | null {
  // 확장자가 없는 Dockerfile·Makefile은 파일 이름으로 찾는다 (Dockerfile.dev 같은 이름 포함)
  const name = (path.split(/[\\/]/).pop() ?? path).toLowerCase();
  const dot = name.lastIndexOf(".");
  const ext = name.startsWith("dockerfile")
    ? "dockerfile"
    : name === "makefile" || name === "gnumakefile"
      ? "makefile"
      : dot >= 0
        ? name.slice(dot + 1)
        : "";
  const l = LANGS[ext];
  return l ? { ...l, ext } : null;
}

export function langName(path: string): string {
  return langOf(path)?.name ?? "일반 텍스트";
}
