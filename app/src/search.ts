// 검색 뷰 (Ctrl+Shift+F): 프로젝트 전체 텍스트 검색
import { invoke } from "@tauri-apps/api/core";
import { ask } from "./dialog";
import { errorText } from "./api";
import { $, basename, dirname, h } from "./dom";
import { codicon, fileIcon } from "./icons";
import * as editor from "./editor";
import * as toast from "./toast";
import { revertSafely } from "./chat";

interface FileMatches {
  path: string;
  matches: { line: number; col: number; len: number; text: string }[];
}

let timer: number | undefined;
let seq = 0;
let caseSensitive = false;
let openHandler: (path: string, line: number, col: number) => void = () => {};
let changedHandler: (paths: string[]) => void = () => {};
let lastResults: FileMatches[] = [];
let lastQuery = "";
let replaceOpen = false;

export function onChanged(fn: (paths: string[]) => void) {
  changedHandler = fn;
}

interface ReplaceResult { count: number; files: string[]; checkpoint: string | null }

/** 파일들에서 모두 바꾼다. 고치던(저장 안 한) 탭의 파일은 건너뛴다. */
async function replaceIn(paths: string[]) {
  const replacement = $<HTMLInputElement>("#replace-input").value;
  const query = lastQuery;
  if (!query) return;
  const dirty = paths.filter((p) => editor.isDirty(p));
  const targets = paths.filter((p) => !editor.isDirty(p));
  if (!targets.length) {
    toast.warn("저장하지 않은 파일은 바꾸지 않습니다", "먼저 저장하거나 변경을 버리세요.");
    return;
  }
  if (targets.length > 1) {
    const total = lastResults.filter((f) => targets.includes(f.path)).reduce((n, f) => n + f.matches.length, 0);
    const ok = await ask(`파일 ${targets.length}개에서 '${query}'을(를) '${replacement}'(으)로 바꿀까요? (결과 목록 기준 ${total}곳 이상)`, {
      title: "모두 바꾸기", kind: "warning", okLabel: "바꾸기", cancelLabel: "취소",
    });
    if (!ok) return;
  }
  try {
    const r = await invoke<ReplaceResult>("replace_text", { paths: targets, query, replacement, caseSensitive });
    for (const p of r.files) void editor.reloadIfClean(p);
    changedHandler(r.files);
    const cp = r.checkpoint;
    toast.show({
      kind: "success",
      message: `${r.files.length}개 파일에서 ${r.count}곳을 바꿨습니다`,
      detail: dirty.length ? `저장하지 않은 파일 ${dirty.length}개는 건너뛰었습니다.` : undefined,
      actions: cp
        ? [{
            label: "되돌리기",
            run: async () => {
              try {
                if ((await revertSafely(cp)) === null) return;
                for (const p of r.files) void editor.reloadIfClean(p);
                changedHandler(r.files);
                toast.info("바꾸기를 되돌렸습니다");
                void run();
              } catch (e) {
                toast.error("되돌리지 못했습니다", errorText(e));
              }
            },
          }]
        : undefined,
    });
  } catch (e) {
    toast.error("바꾸지 못했습니다", errorText(e));
  }
  void run();
}

export function onOpen(fn: (path: string, line: number, col: number) => void) {
  openHandler = fn;
}

function preview(text: string, col: number, len: number): HTMLElement {
  // 긴 줄은 일치 위치 앞쪽을 잘라 보이게 한다.
  const chars = [...text];
  const start = Math.max(0, col - 30);
  const before = (start > 0 ? "…" : "") + chars.slice(start, col).join("").trimStart();
  return h("span", {}, before, h("mark", {}, chars.slice(col, col + len).join("")), chars.slice(col + len).join(""));
}

function render(results: FileMatches[], query: string) {
  lastResults = results;
  lastQuery = query;
  $<HTMLButtonElement>("#replace-all").disabled = !results.length;
  const box = $("#search-results");
  const total = results.reduce((n, f) => n + f.matches.length, 0);
  $("#search-summary").textContent = query
    ? total ? `${results.length}개 파일에서 ${total}개 결과` : "결과가 없습니다."
    : "";
  box.replaceChildren(
    ...results.map((f) => {
      const lines = h("div");
      let open = true;
      const twisty = h("span", { class: "twisty" }, codicon("chevron-down"));
      const head = h("div", { class: "sr-file", title: f.path },
        twisty, fileIcon(f.path), h("span", {}, basename(f.path)), h("span", { class: "dir" }, dirname(f.path)),
        h("span", { class: "badge" }, String(f.matches.length)));
      if (replaceOpen) {
        const rb = h("button", { class: "icon-btn sr-replace", title: "이 파일에서 모두 바꾸기", "aria-label": "이 파일에서 모두 바꾸기" }, codicon("replace-all"));
        rb.addEventListener("click", (e) => {
          e.stopPropagation();
          void replaceIn([f.path]);
        });
        head.insertBefore(rb, head.lastChild);
      }
      head.addEventListener("click", () => {
        open = !open;
        lines.classList.toggle("hidden", !open);
        twisty.replaceChildren(codicon(open ? "chevron-down" : "chevron-right"));
      });
      for (const m of f.matches) {
        const el = h("div", { class: "sr-line", title: `${m.line}줄` }, preview(m.text, m.col, m.len));
        el.addEventListener("click", () => openHandler(f.path, m.line, m.col + 1));
        lines.append(el);
      }
      return h("div", {}, head, lines);
    }),
  );
}

async function run() {
  const query = $<HTMLInputElement>("#search-input").value;
  const my = ++seq;
  if (!query.trim()) return render([], "");
  $("#search-summary").textContent = "검색 중…";
  try {
    const results = await invoke<FileMatches[]>("search_text", { query, caseSensitive });
    if (my === seq) render(results, query);
  } catch (e) {
    if (my === seq) $("#search-summary").textContent = errorText(e);
  }
}

let openReplace = () => {};

/** Ctrl+Shift+H: 바꾸기까지 연다 */
export function focusReplace() {
  focus();
  openReplace();
}

export function focus() {
  const input = $<HTMLInputElement>("#search-input");
  const sel = window.getSelection()?.toString();
  if (sel && !sel.includes("\n")) {
    input.value = sel;
    void run();
  }
  input.focus();
  input.select();
}

export function clear() {
  $<HTMLInputElement>("#search-input").value = "";
  render([], "");
}

export function init() {
  const input = $<HTMLInputElement>("#search-input");
  input.addEventListener("input", () => {
    clearTimeout(timer);
    timer = window.setTimeout(run, 300);
  });
  input.addEventListener("keydown", (e) => {
    if (e.key === "Enter") {
      clearTimeout(timer);
      void run();
    }
    if (e.altKey && e.key.toLowerCase() === "c") toggleCase();
  });
  const toggleCase = () => {
    caseSensitive = !caseSensitive;
    $("#search-case").classList.toggle("on", caseSensitive);
    void run();
  };
  $("#search-case").addEventListener("click", toggleCase);

  const setReplace = (open: boolean) => {
    replaceOpen = open;
    $("#replace-row").classList.toggle("hidden", !open);
    $("#search-expand").setAttribute("aria-expanded", String(open));
    $("#search-expand").replaceChildren(codicon(open ? "chevron-down" : "chevron-right"));
    render(lastResults, lastQuery);
    if (open) $<HTMLInputElement>("#replace-input").focus();
  };
  $("#search-expand").addEventListener("click", () => setReplace(!replaceOpen));
  $("#replace-all").addEventListener("click", () => void replaceIn(lastResults.map((f) => f.path)));
  $("#replace-input").addEventListener("keydown", (e) => {
    if (e.key === "Enter" && e.ctrlKey && e.altKey) {
      e.preventDefault();
      void replaceIn(lastResults.map((f) => f.path));
    }
  });
  openReplace = () => setReplace(true);
}
