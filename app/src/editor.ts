// 탭과 편집기. 파일 탭은 CodeMirror 편집기 하나씩, 페이지 탭(설정, 시작하기)은 화면 하나씩.
import { basicSetup } from "codemirror";
import { EditorView, keymap } from "@codemirror/view";
import { Compartment, EditorState, type Extension } from "@codemirror/state";
import { indentWithTab } from "@codemirror/commands";
import { forEachDiagnostic } from "@codemirror/lint";
import { javascript } from "@codemirror/lang-javascript";
import { python } from "@codemirror/lang-python";
import { rust } from "@codemirror/lang-rust";
import { json } from "@codemirror/lang-json";
import { markdown } from "@codemirror/lang-markdown";
import { html } from "@codemirror/lang-html";
import { css } from "@codemirror/lang-css";
import { ask } from "./dialog";
import { api, errorText } from "./api";
import { $, basename, h } from "./dom";
import { codicon, fileIcon } from "./icons";
import { clientFor, pathToUri } from "./lsp";
import { inlineCompletion } from "./completion";
import * as prefs from "./prefs";
import { editorAppearance } from "./theme";
import * as toast from "./toast";

interface Lang {
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
  rs: { name: "Rust", cm: () => rust(), lsp: "rust", id: "rust" },
  json: { name: "JSON", cm: () => json() },
  md: { name: "Markdown", cm: () => markdown() },
  html: { name: "HTML", cm: () => html() },
  css: { name: "CSS", cm: () => css() },
};

export function langOf(path: string): (Lang & { ext: string }) | null {
  const ext = path.split(".").pop()?.toLowerCase() ?? "";
  const l = LANGS[ext];
  return l ? { ...l, ext } : null;
}

export function langName(path: string): string {
  return langOf(path)?.name ?? "일반 텍스트";
}

interface Tab {
  /** 파일은 경로, 페이지는 "page:<id>" */
  key: string;
  kind: "file" | "page";
  title: string;
  view: EditorView | null;
  appearance: Compartment | null;
  lspSlot?: Compartment;
  host: HTMLElement;
  tabEl: HTMLElement;
  dirty: boolean;
  onShow?: () => void;
}

export interface Problem {
  path: string;
  line: number;
  col: number;
  severity: "error" | "warning" | "info" | "hint";
  message: string;
  source?: string;
}

const tabs = new Map<string, Tab>();
let order: string[] = [];
let active: string | null = null;
const listeners: ((path: string | null) => void)[] = [];
let posListener: (line: number, col: number) => void = () => {};
let problemListener: (problems: Problem[]) => void = () => {};
let problemTimer: number | undefined;

prefs.onChange(({ dark }) => {
  for (const t of tabs.values()) {
    if (t.view && t.appearance) t.view.dispatch({ effects: t.appearance.reconfigure(editorAppearance(dark)) });
  }
});

export function onActiveChange(fn: (path: string | null) => void) {
  listeners.push(fn);
}

export function onCursor(fn: (line: number, col: number) => void) {
  posListener = fn;
}

export function onProblems(fn: (problems: Problem[]) => void) {
  problemListener = fn;
}

/** 활성 탭이 파일이면 그 경로, 페이지거나 없으면 null */
export function activePath(): string | null {
  const t = active ? tabs.get(active) : null;
  return t?.kind === "file" ? t.key : null;
}

export function activeKey(): string | null {
  return active;
}

export function tabCount(): number {
  return tabs.size;
}

/** AI 질문용 포커스: 현재 파일과 커서 줄 */
export function focusInfo(): { file: string | null; line: number | null } {
  const t = active ? tabs.get(active) : null;
  if (!t?.view) return { file: null, line: null };
  const head = t.view.state.selection.main.head;
  return { file: t.key, line: t.view.state.doc.lineAt(head).number };
}

/** 열린 파일들의 진단 (언어 서버가 보낸 것) */
export function problems(): Problem[] {
  const out: Problem[] = [];
  for (const t of tabs.values()) {
    if (!t.view) continue;
    const doc = t.view.state.doc;
    forEachDiagnostic(t.view.state, (d, from) => {
      const line = doc.lineAt(Math.min(from, doc.length));
      out.push({ path: t.key, line: line.number, col: from - line.from + 1, severity: d.severity, message: d.message, source: d.source });
    });
  }
  return out;
}

function scheduleProblems() {
  clearTimeout(problemTimer);
  problemTimer = window.setTimeout(() => problemListener(problems()), 150);
}

function renderBreadcrumbs() {
  const bc = $("#breadcrumbs");
  bc.replaceChildren();
  const t = active ? tabs.get(active) : null;
  if (!t || t.kind === "page") return;
  const parts = t.key.split("/");
  parts.forEach((p, i) => {
    if (i > 0) bc.append(codicon("chevron-right", "sep"));
    const last = i === parts.length - 1;
    bc.append(h("span", { class: "crumb" }, last ? fileIcon(p) : null, p));
  });
}

function emitActive() {
  renderBreadcrumbs();
  const path = activePath();
  listeners.forEach((fn) => fn(path));
}

function renderTabState(t: Tab) {
  t.tabEl.classList.toggle("dirty", t.dirty);
  t.tabEl.classList.toggle("active", t.key === active);
  t.tabEl.setAttribute("aria-selected", String(t.key === active));
}

function activate(key: string) {
  active = key;
  for (const t of tabs.values()) {
    t.host.classList.toggle("active", t.key === key);
    renderTabState(t);
  }
  const t = tabs.get(key);
  if (t) {
    t.tabEl.scrollIntoView({ block: "nearest", inline: "nearest" });
    if (t.view) {
      t.view.focus();
      const head = t.view.state.selection.main.head;
      const line = t.view.state.doc.lineAt(head);
      posListener(line.number, head - line.from + 1);
    }
    t.onShow?.();
  }
  emitActive();
}

function gotoLine(view: EditorView, line: number, col = 1) {
  const l = view.state.doc.line(Math.min(Math.max(line, 1), view.state.doc.lines));
  const pos = Math.min(l.from + Math.max(col - 1, 0), l.to);
  view.dispatch({ selection: { anchor: pos }, effects: EditorView.scrollIntoView(pos, { y: "center" }) });
  view.focus();
}

function makeTab(key: string, kind: Tab["kind"], title: string, icon: HTMLElement): Tab {
  const host = h("div", { class: `editor-host${kind === "page" ? " page-host" : ""}` });
  $("#editors").append(host);
  const close = h("button", {
    class: "close",
    title: "닫기 (Ctrl+W)",
    "aria-label": `${title} 닫기`,
    onclick: (e: Event) => {
      e.stopPropagation();
      void closeTab(key);
    },
  }, codicon("close"), codicon("circle-filled"));
  const tabEl = h("div", { class: `tab${kind === "page" ? " is-page" : ""}`, title: kind === "file" ? key : title, role: "tab", onclick: () => activate(key) },
    icon, h("span", {}, title), close);
  tabEl.addEventListener("auxclick", (e) => {
    if ((e as MouseEvent).button === 1) void closeTab(key);
  });
  $("#tabs").append(tabEl);
  const tab: Tab = { key, kind, title, view: null, appearance: null, host, tabEl, dirty: false };
  tabs.set(key, tab);
  order.push(key);
  return tab;
}

/** 설정·시작하기 같은 화면을 탭으로 연다. 이미 열려 있으면 그 탭으로 간다. */
export function openPage(id: string, title: string, icon: string, render: (host: HTMLElement) => void, onShow?: () => void): HTMLElement {
  const key = `page:${id}`;
  let t = tabs.get(key);
  if (!t) {
    t = makeTab(key, "page", title, h("span", { class: "ficon" }, codicon(icon)));
    t.onShow = onShow;
    render(t.host);
  }
  activate(key);
  return t.host;
}

export async function openFile(path: string, line?: number, col?: number): Promise<EditorView | null> {
  const existing = tabs.get(path);
  if (existing?.view) {
    activate(path);
    if (line) gotoLine(existing.view, line, col);
    return existing.view;
  }
  let text: string;
  try {
    text = await api.readFile(path);
  } catch (e) {
    const msg = errorText(e);
    toast.show({ kind: msg.includes("바이너리") ? "info" : "error", message: `${basename(path)}을(를) 편집기에서 열 수 없습니다`, detail: msg });
    return null;
  }
  if (tabs.has(path)) return openFile(path, line, col); // 기다리는 사이 다른 곳에서 열었을 때

  const lang = langOf(path);
  const lspSlot = new Compartment();
  const appearance = new Compartment();
  const tab = makeTab(path, "file", basename(path), fileIcon(path));
  tab.appearance = appearance;
  tab.lspSlot = lspSlot;
  tab.view = new EditorView({
    parent: tab.host,
    state: EditorState.create({
      doc: text,
      extensions: [
        basicSetup,
        keymap.of([indentWithTab]),
        appearance.of(editorAppearance(prefs.isDark())),
        lang ? lang.cm() : [],
        lspSlot.of([]),
        inlineCompletion(path),
        EditorView.updateListener.of((u) => {
          if (u.docChanged && !tab.dirty) {
            tab.dirty = true;
            renderTabState(tab);
          }
          if (u.selectionSet || u.docChanged) {
            const head = u.state.selection.main.head;
            const ln = u.state.doc.lineAt(head);
            if (tab.key === active) posListener(ln.number, head - ln.from + 1);
          }
          if (u.transactions.length) scheduleProblems();
        }),
      ],
    }),
  });
  const view = tab.view;
  activate(path);
  if (line) gotoLine(view, line, col);

  if (lang?.lsp && lang.id) {
    const client = await clientFor(lang.lsp);
    if (client && tabs.get(path) === tab) {
      view.dispatch({ effects: lspSlot.reconfigure(client.plugin(pathToUri(path), lang.id)) });
    }
  }
  return view;
}

export async function save(key = active): Promise<boolean> {
  const t = key ? tabs.get(key) : null;
  if (!t?.view) return false;
  try {
    await api.writeFile(t.key, t.view.state.doc.toString());
    t.dirty = false;
    renderTabState(t);
    return true;
  } catch (e) {
    toast.error(`${t.title}을(를) 저장하지 못했습니다`, errorText(e));
    return false;
  }
}

export async function saveAll() {
  for (const t of tabs.values()) if (t.dirty) await save(t.key);
}

export async function closeTab(key = active): Promise<void> {
  const t = key ? tabs.get(key) : null;
  if (!t) return;
  if (t.dirty) {
    const ok = await ask(`${t.key}의 변경 사항을 저장하지 않고 닫을까요?`, { title: "저장하지 않은 변경", kind: "warning", okLabel: "저장 안 함", cancelLabel: "취소" });
    if (!ok) return;
  }
  t.view?.destroy();
  t.host.remove();
  t.tabEl.remove();
  tabs.delete(t.key);
  const idx = order.indexOf(t.key);
  order = order.filter((p) => p !== t.key);
  if (active === t.key) {
    active = null;
    const next = order[Math.min(idx, order.length - 1)];
    if (next) activate(next);
    else emitActive();
  }
  scheduleProblems();
}

/** 파일 탭을 모두 닫는다 (프로젝트를 바꿀 때). 페이지 탭은 둔다. */
export function closeAllFiles() {
  for (const t of [...tabs.values()]) {
    if (t.kind !== "file") continue;
    t.view?.destroy();
    t.host.remove();
    t.tabEl.remove();
    tabs.delete(t.key);
    order = order.filter((k) => k !== t.key);
  }
  if (active && !tabs.has(active)) {
    active = null;
    const next = order.at(-1);
    if (next) activate(next);
  }
  emitActive();
  scheduleProblems();
}

/** 언어 서버를 새로 설치했을 때, 이미 열린 그 언어 탭들에 붙인다 */
export async function reattachLsp(lspKey: string) {
  for (const t of tabs.values()) {
    if (t.kind !== "file" || !t.view || !t.lspSlot) continue;
    const lang = langOf(t.key);
    if (lang?.lsp !== lspKey || !lang.id) continue;
    const client = await clientFor(lspKey);
    if (!client) return;
    t.view.dispatch({ effects: t.lspSlot.reconfigure(client.plugin(pathToUri(t.key), lang.id)) });
  }
}

export function isDirty(path: string): boolean {
  return !!tabs.get(path)?.dirty;
}

export function hasDirty(): boolean {
  return [...tabs.values()].some((t) => t.dirty);
}

/** 에이전트가 바꾼 파일을 다시 읽는다. 사용자가 고치던 탭은 건드리지 않는다. */
export async function reloadIfClean(path: string) {
  const t = tabs.get(path);
  if (!t?.view || t.dirty) return;
  try {
    const text = await api.readFile(path);
    const doc = t.view.state.doc.toString();
    if (text === doc) return;
    const head = Math.min(t.view.state.selection.main.head, text.length);
    t.view.dispatch({ changes: { from: 0, to: doc.length, insert: text }, selection: { anchor: head } });
    t.dirty = false;
    renderTabState(t);
  } catch {
    /* 파일이 지워졌으면 그대로 둔다 */
  }
}

/** 탐색기에서 이름을 바꾸면 열린 탭을 새 경로로 옮긴다 (폴더면 그 안의 파일 모두). */
export async function pathRenamed(from: string, to: string) {
  for (const t of [...tabs.values()]) {
    if (t.kind !== "file" || !(t.key === from || t.key.startsWith(from + "/"))) continue;
    const next = to + t.key.slice(from.length);
    if (t.dirty) {
      toast.warn(`${t.title}에 저장하지 않은 변경이 있어 탭을 옮기지 않았습니다`, `새 경로: ${next}`);
      continue;
    }
    const wasActive = active === t.key;
    const head = t.view?.state.selection.main.head ?? 0;
    const line = t.view ? t.view.state.doc.lineAt(head).number : undefined;
    await closeTab(t.key);
    const v = await openFile(next, line);
    if (!wasActive && v) activate(order.find((k) => k !== next) ?? next);
  }
}

/** 파일이 지워지면 저장하지 않은 변경이 없는 탭은 닫고, 있으면 남겨 두고 알린다. */
export async function pathDeleted(path: string) {
  for (const t of [...tabs.values()]) {
    if (t.kind !== "file" || !(t.key === path || t.key.startsWith(path + "/"))) continue;
    if (t.dirty) {
      t.tabEl.classList.add("deleted");
      t.tabEl.title = `${t.key} (삭제됨, 저장하면 다시 만듭니다)`;
    } else await closeTab(t.key);
  }
}

/** 활성 편집기 (메뉴의 실행 취소, 찾기 등에 쓴다) */
export function activeView(): EditorView | null {
  return active ? tabs.get(active)?.view ?? null : null;
}
