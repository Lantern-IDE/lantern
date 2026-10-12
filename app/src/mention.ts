// 채팅에서 @로 파일·심볼 집어 넣기: 입력창에서 @를 치면 후보를 보이고, 고른 것은 칩으로 남겨 맥락에 꼭 붙인다.
import { invoke } from "@tauri-apps/api/core";
import { api } from "./api";
import { $, basename, dirname, h } from "./dom";
import { codicon, fileIcon } from "./icons";
import { score } from "./quickopen";

import type { Mention } from "./api";
export type { Mention };
interface Candidate { mention: Mention; label: string; detail: string; icon: HTMLElement; score: number }
interface GraphNode { kind: string; label: string; path?: string; line?: number; detail?: string }

const MAX = 8;
let picked: Mention[] = [];
let files: string[] | null = null;
let items: Candidate[] = [];
let sel = 0;
let open = false;
let token: { start: number; query: string } | null = null;
let seq = 0;
let onChange = () => {};

const key = (m: Mention) => (m.kind === "file" ? `f:${m.path}` : `s:${m.path}:${m.name}:${m.line}`);
const label = (m: Mention) => (m.kind === "file" ? basename(m.path) : m.name);

/** 입력 위치 바로 앞의 '@검색어' */
function currentToken(input: HTMLTextAreaElement): { start: number; query: string } | null {
  const before = input.value.slice(0, input.selectionStart ?? 0);
  const m = before.match(/(?:^|\s)@([^\s@]*)$/);
  return m ? { start: before.length - m[1].length - 1, query: m[1] } : null;
}

async function candidates(query: string): Promise<Candidate[]> {
  files ??= await api.listFiles().catch(() => []);
  const fileHits = files
    .map((p) => ({ p, s: score(p, query, p.lastIndexOf("/") + 1) }))
    .filter((x) => x.s)
    .sort((a, b) => b.s!.score - a.s!.score)
    .slice(0, 5)
    .map(({ p, s }): Candidate => ({ mention: { kind: "file", path: p }, label: basename(p), detail: dirname(p), icon: fileIcon(p), score: s!.score }));
  const nodes = query.length >= 2 ? await invoke<GraphNode[]>("graph_find", { query }).catch(() => []) : [];
  const symHits = nodes
    .filter((n) => n.kind === "symbol" && n.path && n.line)
    .slice(0, 5)
    .map((n): Candidate => ({ mention: { kind: "symbol", name: n.label, path: n.path!, line: n.line! }, label: n.label, detail: `${n.detail ?? ""} · ${n.path}:${n.line}`, icon: codicon("symbol-method"), score: 0 }));
  return [...symHits.filter((c) => c.label.toLowerCase().startsWith(query.toLowerCase())), ...fileHits, ...symHits.filter((c) => !c.label.toLowerCase().startsWith(query.toLowerCase()))].slice(0, MAX);
}

function render() {
  const box = $("#mention-pop");
  box.classList.toggle("hidden", !open || !items.length);
  box.replaceChildren(...items.map((c, i) => {
    const row = h("div", { class: `mention-item${i === sel ? " sel" : ""}`, role: "option", "aria-selected": String(i === sel) }, c.icon, h("span", { class: "mi-label" }, c.label), h("span", { class: "mi-detail" }, c.detail));
    row.addEventListener("mousedown", (e) => {
      e.preventDefault();
      choose(i);
    });
    return row;
  }));
}

function close() {
  open = false;
  token = null;
  render();
}

async function update(input: HTMLTextAreaElement) {
  token = currentToken(input);
  if (!token) return close();
  const mine = ++seq;
  const list = await candidates(token.query);
  if (mine !== seq) return;
  items = list;
  sel = 0;
  open = true;
  render();
}

function choose(i: number) {
  const c = items[i];
  const input = $<HTMLTextAreaElement>("#prompt");
  if (!c || !token) return;
  // '@검색어'를 '@이름 '으로 바꾸고 칩으로 남긴다
  const before = input.value.slice(0, token.start);
  const after = input.value.slice(input.selectionStart ?? input.value.length);
  const text = `@${c.label} `;
  input.value = before + text + after;
  const pos = before.length + text.length;
  input.setSelectionRange(pos, pos);
  if (!picked.some((m) => key(m) === key(c.mention))) picked.push(c.mention);
  close();
  renderChips();
  input.dispatchEvent(new Event("input"));
  input.focus();
}

function renderChips() {
  const box = $("#mentions");
  box.classList.toggle("hidden", !picked.length);
  box.replaceChildren(...picked.map((m, i) => {
    const remove = h("button", { class: "mention-remove", title: "빼기", "aria-label": `${label(m)} 빼기` }, codicon("close"));
    remove.addEventListener("click", () => {
      picked.splice(i, 1);
      renderChips();
      onChange();
    });
    return h("span", { class: "mention-chip", title: m.kind === "file" ? m.path : `${m.path}:${m.line}` },
      m.kind === "file" ? fileIcon(m.path) : codicon("symbol-method"), `@${label(m)}`, remove);
  }));
}

/** 보낼 때: 고른 것을 가져가고 비운다 */
export function take(): Mention[] {
  const out = picked;
  picked = [];
  renderChips();
  return out;
}

/** 대화에 남기는 표시 */
export function chips(list: { label: string; title: string }[]): HTMLElement {
  return h("div", { class: "user-mentions" }, ...list.map((m) => h("span", { class: "mention-chip static", title: m.title }, `@${m.label}`)));
}

export const describe = (list: Mention[]) => list.map((m) => ({ label: label(m), title: m.kind === "file" ? m.path : `${m.path}:${m.line}` }));

/** 파일 목록이 바뀌면 다시 받는다 */
export function invalidate() {
  files = null;
}

export function init(changed: () => void) {
  onChange = changed;
  const input = $<HTMLTextAreaElement>("#prompt");
  input.addEventListener("input", () => void update(input));
  input.addEventListener("click", () => void update(input));
  input.addEventListener("blur", () => setTimeout(close, 150));
  // 후보가 열려 있으면 Enter·Tab·화살표·Esc를 먼저 받는다 (보내기보다 앞서)
  input.addEventListener("keydown", (e) => {
    if (!open || !items.length) return;
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      sel = (sel + (e.key === "ArrowDown" ? 1 : items.length - 1)) % items.length;
      render();
    } else if ((e.key === "Enter" && !e.isComposing) || e.key === "Tab") {
      choose(sel);
    } else if (e.key === "Escape") {
      close();
    } else return;
    e.preventDefault();
    e.stopImmediatePropagation();
  }, { capture: true });
}
