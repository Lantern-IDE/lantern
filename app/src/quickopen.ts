// 빠른 입력: Ctrl+P 파일로 이동, Ctrl+Shift+P(또는 '>' 입력) 명령 팔레트
import { api } from "./api";
import { displayKey, isMac } from "./lib/keys";
import * as commands from "./commands";
import { $, basename, dirname, h } from "./dom";
import { fileIcon } from "./icons";

type Item = { label: string; desc?: string; key?: string; path?: string; commandId?: string; score: number; hits: number[] };

let files: string[] | null = null;
let results: Item[] = [];
let sel = 0;
let openHandler: (path: string) => void = () => {};

export function onOpen(fn: (path: string) => void) {
  openHandler = fn;
}

export function invalidate() {
  files = null;
}

/** 부분 수열 일치. 연속 일치, 이름 부분 일치, 단어 시작 일치에 가산점. */
export function score(text: string, q: string, nameStart = 0): { score: number; hits: number[] } | null {
  if (!q) return { score: 0, hits: [] };
  const lower = text.toLowerCase();
  let s = 0;
  let pi = 0;
  let prev = -2;
  const hits: number[] = [];
  for (const ch of q.toLowerCase()) {
    if (ch === " ") continue;
    const i = lower.indexOf(ch, pi);
    if (i < 0) return null;
    s += 1;
    if (i === prev + 1) s += 3;
    if (i >= nameStart) s += 2;
    if (i === 0 || "/_-. ".includes(text[i - 1]) || (text[i] !== lower[i] && text[i - 1] === lower[i - 1])) s += 2;
    hits.push(i);
    prev = i;
    pi = i + 1;
  }
  return { score: s - text.length * 0.01, hits };
}

function highlight(text: string, offset: number, hits: number[]): HTMLElement {
  const span = h("span", { class: "label" });
  const set = new Set(hits.map((i) => i - offset));
  let buf = "";
  for (let i = 0; i < text.length; i++) {
    if (set.has(i)) {
      if (buf) span.append(buf);
      buf = "";
      span.append(h("b", {}, text[i]));
    } else buf += text[i];
  }
  if (buf) span.append(buf);
  return span;
}

function render() {
  const list = $("#qo-list");
  if (!results.length) {
    list.replaceChildren(h("li", { class: "empty" }, isCommandMode() ? "일치하는 명령이 없습니다" : "일치하는 파일이 없습니다"));
    return;
  }
  list.replaceChildren(
    ...results.map((r, i) => {
      const li = r.path
        ? h("li", { class: i === sel ? "sel" : "" },
            fileIcon(r.path),
            highlight(basename(r.path), r.path.length - basename(r.path).length, r.hits),
            h("span", { class: "desc" }, dirname(r.path)))
        : h("li", { class: i === sel ? "sel" : "" },
            highlight(r.label, 0, r.hits),
            r.key ? h("span", { class: "key" }, ...(isMac ? [h("kbd", {}, displayKey(r.key))] : r.key.split("+").flatMap((k, j) => (j ? ["+", h("kbd", {}, k)] : [h("kbd", {}, k)])))) : null);
      li.addEventListener("mousedown", (e) => {
        e.preventDefault();
        choose(i);
      });
      return li;
    }),
  );
  list.querySelector(".sel")?.scrollIntoView({ block: "nearest" });
}

function isCommandMode() {
  return $<HTMLInputElement>("#qo-input").value.startsWith(">");
}

function update() {
  const raw = $<HTMLInputElement>("#qo-input").value;
  if (raw.startsWith(">")) {
    const q = raw.slice(1).trim();
    results = commands
      .all()
      .map((c) => ({ label: c.label, key: c.key, commandId: c.id, ...(score(c.label, q) ?? { score: -Infinity, hits: [] }) }))
      .filter((r) => r.score > -Infinity)
      .sort((a, b) => (q ? b.score - a.score : a.label.localeCompare(b.label, "ko")));
  } else {
    const q = raw.trim();
    results = (files ?? [])
      .map((path) => ({ label: path, path, ...(score(path, q, path.lastIndexOf("/") + 1) ?? { score: -Infinity, hits: [] }) }))
      .filter((r) => r.score > -Infinity)
      .sort((a, b) => b.score - a.score)
      .slice(0, 50);
  }
  sel = 0;
  $<HTMLInputElement>("#qo-input").placeholder = raw.startsWith(">") ? "" : "이름으로 파일 검색 ('>'를 입력하면 명령)";
  render();
}

function choose(i: number) {
  const r = results[i];
  close();
  if (!r) return;
  if (r.path) openHandler(r.path);
  else if (r.commandId) commands.run(r.commandId);
}

export function close() {
  $("#quickopen").classList.add("hidden");
}

/** prefix가 ">"면 명령 팔레트로 연다 */
export async function open(prefix = "") {
  $("#quickopen").classList.remove("hidden");
  const input = $<HTMLInputElement>("#qo-input");
  input.value = prefix;
  input.focus();
  if (!prefix && !files) {
    $("#qo-list").replaceChildren(h("li", { class: "empty" }, "파일 목록을 읽는 중…"));
    files = await api.listFiles().catch(() => []);
  }
  update();
}

export function init() {
  const input = $<HTMLInputElement>("#qo-input");
  input.addEventListener("input", async () => {
    if (!isCommandMode() && !files) files = await api.listFiles().catch(() => []);
    update();
  });
  input.addEventListener("keydown", (e) => {
    if (e.key === "ArrowDown") sel = Math.min(sel + 1, results.length - 1);
    else if (e.key === "ArrowUp") sel = Math.max(sel - 1, 0);
    else if (e.key === "Enter") return choose(sel);
    else if (e.key === "Escape") return close();
    else return;
    e.preventDefault();
    render();
  });
  input.addEventListener("blur", () => setTimeout(close, 100));
}
