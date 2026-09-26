// 작업 (에이전트 대화). Lantern은 파일이 아니라 작업 단위로 일한다:
// 작업마다 대화, AI에 보낸 맥락, 에이전트가 읽고 고친 곳(발자취), 되돌리기가 묶인다.
// 왼쪽 "작업" 목록에서 고르고, 오른쪽에서 대화하며, 지도에서 발자취를 본다.
// 답변마다 맥락 엔진이 붙인 코드("Lantern이 본 코드")를 먼저 보여주고,
// 파일 수정 승인 카드에는 변경 영향 반경(호출자·테스트·함께 바뀌던 파일)을 붙인다.
import { marked } from "marked";
import { invoke } from "@tauri-apps/api/core";
import { ask } from "./dialog";
import DOMPurify from "dompurify";
import { api, errorText, on, type AgentDef, type AgentEvent } from "./api";
import { $, basename, dirname, h, renderDiff, store } from "./dom";
import { codicon, fileIcon, hex, setWorking } from "./icons";
import * as editor from "./editor";
import { emptyFootprint, type Footprint, type GNode, type Impact } from "./map";
import { parseContext } from "./lib/context";
import { diffTarget } from "./lib/diff";
export { parseContext } from "./lib/context";
import * as lsp from "./lsp";
import * as toast from "./toast";

export type TaskStatus = "idle" | "running" | "approval" | "done" | "error" | "stopped";

/** 저장·복원용 기록 한 줄 */
type Entry =
  | { t: "user"; text: string }
  | { t: "ev"; ev: AgentEvent }
  | { t: "touched"; id: string; names: string[] }
  | { t: "iso-done" };

interface Isolation { path: string; branch: string; state: "active" | "discarded" }

interface Task {
  id: string;
  title: string;
  status: TaskStatus;
  created: number;
  changed: string[];
  footprint: Footprint;
  el: HTMLElement;
  body: HTMLElement | null;
  botMark: HTMLElement | null;
  current: { el: HTMLElement; text: string } | null;
  pendingTool: HTMLElement | null;
  toolEls: Map<string, HTMLElement>;
  approvalEls: Map<string, HTMLElement>;
  approvalPaths: Map<string, string>;
  /** 승인 카드의 영향 반경에서 찾은, 이 수정이 바꾸는 심볼 */
  approvalTouched: Map<string, string[]>;
  toolInputs: Map<string, { name: string; input: Record<string, unknown> }>;
  renderQueued: boolean;
  lastPrompt: string;
  pendingApprovals: number;
  /** 다시 켰을 때 되살리는 기록 */
  log: Entry[];
  isolated: Isolation | null;
  updated: number;
}

const tasks: Task[] = [];
let active: Task | null = null;
let agents: AgentDef[] = [];
/** 저장된 기록을 되살리는 중이면 부수 효과(파일 새로 읽기, 알림, 초점 이동)를 막는다 */
let replaying = false;

type Hooks = {
  onContext: (usedTokens: number, budgetTokens: number) => void;
  onFilesChanged: (paths: string[]) => void;
  onUsage: () => void;
  showInspector: (requestId: number) => void;
  openSettings: (section?: string) => void;
  onRunning: (running: boolean) => void;
  /** 작업 목록이나 상태가 바뀜 */
  onTasks: () => void;
  /** 지금 작업의 발자취가 바뀜 */
  onFootprint: (fp: Footprint, title: string, running: boolean) => void;
  showOnMap: () => void;
  showImpact: (i: Impact) => void;
};
let hooks: Hooks = {
  onContext: () => {}, onFilesChanged: () => {}, onUsage: () => {}, showInspector: () => {}, openSettings: () => {}, onRunning: () => {},
  onTasks: () => {}, onFootprint: () => {}, showOnMap: () => {}, showImpact: () => {},
};

const messages = () => $("#messages");

function nearBottom(): boolean {
  const m = messages();
  return m.scrollHeight - m.scrollTop - m.clientHeight < 80;
}

function stick<T>(task: Task, fn: () => T): T {
  const s = task === active && nearBottom();
  const r = fn();
  if (s) messages().scrollTop = messages().scrollHeight;
  return r;
}

function turn(task: Task, who: "user" | "bot"): HTMLElement {
  const b = h("div", { class: "turn-body" });
  let avatar: HTMLElement;
  if (who === "user") avatar = h("span", { class: "avatar" }, codicon("account"));
  else {
    task.botMark = hex("brand", isRunning(task));
    avatar = h("span", { class: "avatar bot" }, task.botMark);
  }
  const el = h("div", { class: "turn" }, h("div", { class: "turn-head" }, avatar, who === "user" ? "나" : "Lantern"), b);
  stick(task, () => {
    task.el.querySelector(".empty-chat")?.remove();
    task.el.append(el);
  });
  return b;
}

function append(task: Task, el: HTMLElement) {
  if (!task.body) task.body = turn(task, "bot");
  stick(task, () => task.body!.append(el));
}

/** 모델 출력 Markdown → 정화된 HTML. `경로:줄` 코드는 클릭하면 파일을 연다. */
function renderMarkdown(el: HTMLElement, text: string, streaming = false) {
  el.innerHTML = DOMPurify.sanitize(marked.parse(text, { async: false, gfm: true, breaks: false }) as string);
  for (const a of el.querySelectorAll("a")) {
    a.addEventListener("click", (e) => e.preventDefault());
    a.title = a.getAttribute("href") ?? "";
  }
  for (const code of el.querySelectorAll<HTMLElement>(":not(pre) > code")) {
    const m = code.textContent?.match(/^([\w@./\\-]+\.[A-Za-z0-9]+)(?::(\d+))?(?:-\d+)?$/);
    if (!m) continue;
    code.classList.add("file-link");
    code.title = "클릭하여 열기";
    code.addEventListener("click", () => void editor.openFile(m[1].replace(/\\/g, "/"), m[2] ? Number(m[2]) : undefined));
  }
  if (streaming) {
    const last = el.lastElementChild ?? el;
    (last.tagName === "PRE" || last.tagName === "UL" || last.tagName === "OL" ? el : last).append(h("span", { class: "cursor" }));
  }
}

function endText(task: Task) {
  if (task.current) renderMarkdown(task.current.el, task.current.text);
  task.current = null;
}

function toolSummary(input: Record<string, unknown>): string {
  const v = input.path ?? input.query ?? input.name ?? input.command ?? input.topic ?? "";
  return typeof v === "string" ? v : JSON.stringify(v);
}

function isRunning(task: Task | null): boolean {
  return !!task && (task.status === "running" || task.status === "approval");
}

function updateSendState() {
  const empty = !$<HTMLTextAreaElement>("#prompt").value.trim();
  ($("#btn-send") as HTMLButtonElement).disabled = isRunning(active) || empty;
}

function syncComposer() {
  const r = isRunning(active);
  $("#btn-stop").classList.toggle("hidden", !r);
  $("#btn-send").classList.toggle("hidden", r);
  updateSendState();
  hooks.onRunning(tasks.some(isRunning));
}

function setStatus(task: Task, status: TaskStatus) {
  task.status = status;
  setWorking(task.botMark, isRunning(task));
  if (!isRunning(task)) {
    task.pendingTool?.remove();
    task.pendingTool = null;
    endText(task);
  }
  syncComposer();
  hooks.onTasks();
  if (task === active) emitFootprint(task);
}

// ── 발자취 ──────────────────────────────────────────────

let fpQueued = false;
function emitFootprint(task: Task) {
  if (task !== active || fpQueued) return;
  fpQueued = true;
  requestAnimationFrame(() => {
    fpQueued = false;
    if (active) {
      hooks.onFootprint(active.footprint, active.title, isRunning(active));
      renderHead();
    }
  });
}

function addContext(task: Task, md: string) {
  const { files, memory } = parseContext(md);
  for (const f of files) {
    const set = task.footprint.context.get(f.path) ?? new Set<string>();
    for (const s of f.symbols) set.add(s.name);
    task.footprint.context.set(f.path, set);
  }
  for (const m of memory) task.footprint.memory.add(m);
  if (files[0]) task.footprint.last = files[0].path;
  emitFootprint(task);
}

// ── Lantern이 본 코드 ────────────────────────────────────

function contextCard(md: string, tokens: number, requestId: number): HTMLElement {
  const { files, memory } = parseContext(md);
  const symbols = files.reduce((n, f) => n + f.symbols.length, 0);
  const list = h("div", { class: "ctx-files" });
  for (const m of memory) {
    list.append(h("div", { class: "ctx-file" }, codicon("book"), h("span", {}, basename(m)), h("span", { class: "dir" }, "프로젝트 메모리")));
  }
  for (const f of files) {
    list.append(h("div", { class: "ctx-file" }, fileIcon(f.path), h("span", {}, basename(f.path)), h("span", { class: "dir" }, dirname(f.path))));
    for (const s of f.symbols) {
      const row = h("div", { class: "ctx-sym", title: `${f.path}:${s.line} · ${s.why}` },
        h("span", { class: "name" }, s.name, s.signatureOnly ? h("span", { class: "sig" }, " 시그니처") : null),
        h("span", { class: "why" }, s.why));
      row.addEventListener("click", () => void editor.openFile(f.path, s.line));
      list.append(row);
    }
  }
  const more = h("span", { class: "link small" }, "인스펙터에서 전문 보기");
  more.addEventListener("click", () => hooks.showInspector(requestId));
  const map = h("span", { class: "link small" }, "지도에서 보기");
  map.addEventListener("click", () => hooks.showOnMap());
  list.append(h("div", { class: "ctx-foot" }, map, more));
  return h("details", { class: "ctx-card" },
    h("summary", {},
      hex("plain"),
      h("span", { class: "title" }, "Lantern이 본 코드"),
      h("span", { class: "meta" }, `파일 ${files.length} · 심볼 ${symbols}${memory.length ? ` · 메모리 ${memory.length}` : ""}`),
      h("span", { class: "tokens" }, `약 ${tokens.toLocaleString()} 토큰`),
      codicon("chevron-right", "chev")),
    list);
}

// ── 변경 영향 반경 ─────────────────────────────────────────

const RISK_LABEL: Record<string, string> = { low: "낮음", medium: "보통", high: "높음" };

/**
 * 언어 서버가 켜져 있으면 바뀌는 심볼의 참조를 다시 센다.
 * - 이름이 여러 곳에 정의되어 있으면 언어 서버 결과를 쓴다 (같은 이름의 다른 심볼을 구분)
 * - 한 곳에만 정의되어 있으면 이름 기준과 합친다. 언어 서버는 열린 파일·설정된 프로젝트만 보고
 *   (예: tsconfig 없는 TypeScript) 일부를 놓치기도 하므로, 결과가 줄어들게 두지 않는다.
 */
async function lspCallers(i: Impact): Promise<{ callers: number; files: number; lspOnly: number } | null> {
  const lang = editor.langOf(i.path)?.lsp;
  if (!lang) return null;
  const touched = i.graph.nodes.filter((n) => n.depth === 0 && n.line).slice(0, 3);
  if (!touched.length) return null;
  const text = await api.readFile(i.path).catch(() => "");
  const lines = text.split("\n");
  const found = new Map<string, GNode>();
  const touchedIds = new Set(touched.map((n) => n.id));
  for (const n of touched) {
    // 정의 줄(또는 그 아래 몇 줄)에서 이름이 시작하는 열
    let pos: { line: number; col: number } | null = null;
    for (let l = n.line! - 1; l < Math.min(lines.length, n.line! + 3); l++) {
      const col = lines[l]?.search(new RegExp(`\\b${n.label.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}\\b`));
      if (col !== undefined && col >= 0) {
        pos = { line: l, col };
        break;
      }
    }
    if (!pos) return null;
    const refs = await lsp.references(lang, i.path, pos.line, pos.col);
    if (!refs) return null;
    const enclosing = await invoke<(GNode | null)[]>("graph_enclosing", { locations: refs.map((r) => [r.path, r.line]) }).catch(() => null);
    if (!enclosing) return null;
    for (const e of enclosing) if (e && !touchedIds.has(e.id)) found.set(e.id, e);
  }
  const lspOnly = [...found.keys()].filter((id) => !i.graph.nodes.some((n) => n.id === id && n.depth === 1)).length;
  if (i.unique) for (const n of i.graph.nodes) if (n.depth === 1) found.set(n.id, n);
  return { callers: found.size, files: new Set([...found.values()].map((n) => n.path).filter((p) => p !== i.path)).size, lspOnly };
}

function impactRow(diff: string, onTouched: (names: string[]) => void): HTMLElement | null {
  const target = diffTarget(diff);
  if (!target) return null;
  const row = h("div", { class: "impact loading" }, codicon("loading", "codicon-modifier-spin"), h("span", {}, "영향 반경 계산 중…"));
  void invoke<Impact>("graph_impact", { path: target.path, ranges: target.ranges }).then((i) => {
    onTouched(i.touched);
    row.classList.remove("loading");
    const nothing = !i.touched.length && !i.callers && !i.cochanged.length;
    const stats: HTMLElement[] = [];
    const stat = (n: number, label: string) => stats.push(h("span", { class: "istat" }, h("b", {}, String(n)), label));
    if (i.touched.length) stats.push(h("span", { class: "itouch", title: i.touched.join(", ") }, codicon("symbol-method"), i.touched.slice(0, 2).join(", ") + (i.touched.length > 2 ? ` 외 ${i.touched.length - 2}` : "")));
    stat(i.callers, "직접 호출");
    if (i.callers2) stat(i.callers2, "간접 호출");
    if (i.files) stat(i.files, "파일에 걸침");
    if (i.cochanged.length) stat(i.cochanged.length, "함께 바뀌던 파일");
    stats.push(h("span", { class: `istat ${i.tests.length ? "tests" : "no-tests"}` }, codicon(i.tests.length ? "beaker" : "warning"), i.tests.length ? `테스트 ${i.tests.length}` : "테스트 없음"));
    const view = h("button", { class: "btn btn-ghost", title: "영향 범위를 지도에서 보기" }, codicon("type-hierarchy"), "지도에서 보기");
    view.addEventListener("click", () => hooks.showImpact(i));
    // 숫자를 사실로 믿지 않게: 무엇을 기준으로 셌고 무엇을 놓치는지
    const basis = h("span", { class: "ibasis", title: "이름으로 찾은 호출 관계(정적 분석)입니다. DI·리플렉션·이벤트·라우팅처럼 실행 중에 정해지는 호출과, URL로 부르는 테스트는 세지 않습니다." }, codicon("info"));
    row.replaceChildren(...([
      h("div", { class: "ihead" },
        codicon("pulse"),
        h("span", { class: "ititle" }, "영향 반경"),
        basis,
        i.supported ? h("span", { class: `risk risk-${i.risk}` }, `위험도 ${RISK_LABEL[i.risk]}`) : null,
        i.graph.nodes.length > 1 ? view : null),
      !i.supported
        ? h("div", { class: "inote" }, "이 언어는 호출 관계를 분석하지 않아 영향 범위를 알 수 없습니다. 바뀌는 곳을 직접 확인하세요.")
        : nothing
          ? h("div", { class: "inote" }, "이 변경을 호출하는 다른 코드를 찾지 못했습니다.")
          : h("div", { class: "istats" }, ...stats),
      i.framework.length ? h("div", { class: "inote framework" }, codicon("warning"), `${i.framework.join(", ")}: 프레임워크가 부르는 코드라 호출자·테스트 수에 잡히지 않을 수 있습니다`) : null,
      i.ambiguous.length ? h("div", { class: "inote ambiguous" }, `이름이 흔해 호출자를 셀 수 없음: ${i.ambiguous.join(", ")}`) : null,
    ].filter(Boolean) as Node[]));
    if (i.supported) row.classList.add(`risk-${i.risk}`);
    // 언어 서버로 다시 확인 (켜져 있을 때만). 이름 기준 결과와 다르면 고쳐 보여준다.
    void lspCallers(i).then((r) => {
      if (!r) return;
      const direct = row.querySelector(".istat b");
      if (direct) direct.textContent = String(r.callers);
      row.querySelector(".inote.ambiguous")?.remove();
      const note = r.callers === i.callers ? "언어 서버로 확인함" : `언어 서버로 확인함 (이름 기준 ${i.callers})`;
      if (r.lspOnly && i.unique) row.querySelector(".istats")?.append(h("span", { class: "istat" }, h("b", {}, `+${r.lspOnly}`), "언어 서버가 더 찾음"));
      row.querySelector(".ihead")!.insertBefore(h("span", { class: "lsp-ok", title: "언어 서버의 참조 찾기로 다시 센 직접 호출 수입니다" }, codicon("verified"), note), row.querySelector(".ihead .btn"));
    });
  }).catch(() => row.remove());
  return row;
}

/** 되돌리기. 그 뒤로 다른 곳에서 바뀐 파일이 있으면 한 번 더 묻는다. 취소하면 null */
export async function revertSafely(cp: string): Promise<number | null> {
  try {
    return await api.revertCheckpoint(cp);
  } catch (e) {
    const msg = errorText(e);
    if (!msg.startsWith("CONFLICT:")) throw e;
    const files = msg.slice("CONFLICT:".length).split("\n");
    const ok = await ask(`이 작업 뒤로 다른 곳에서 바뀐 파일이 있습니다:\n\n${files.join("\n")}\n\n되돌리면 그 변경도 사라집니다. 그래도 되돌릴까요?`, { title: "되돌리기", kind: "warning", okLabel: "그래도 되돌리기", cancelLabel: "취소" });
    if (!ok) return null;
    return api.revertCheckpoint(cp, true);
  }
}

// ── 이벤트 ──────────────────────────────────────────────

function showError(task: Task, message: string) {
  const actions = h("div", { class: "err-actions" });
  if (task.lastPrompt) {
    actions.append(h("button", { class: "btn btn-secondary", onclick: () => void send(task.lastPrompt) }, codicon("debug-restart"), "다시 시도"));
  }
  if (/API 키|api key|401|403|설정/i.test(message)) {
    actions.append(h("button", { class: "btn btn-secondary", onclick: () => hooks.openSettings("models") }, codicon("key"), "모델 설정 열기"));
  }
  append(task, h("div", { class: "err-box", role: "alert" },
    h("div", { class: "err-msg" }, codicon("error"), h("span", {}, message)),
    actions.childElementCount ? actions : null));
}

function str(v: unknown): string {
  return typeof v === "string" ? v : "";
}

function record(task: Task, ev: AgentEvent) {
  if (replaying) return;
  const last = task.log.at(-1);
  if (ev.kind === "text" && last?.t === "ev" && last.ev.kind === "text") {
    last.ev = { ...last.ev, text: last.ev.text + ev.text };
  } else if (ev.kind === "tool_result") {
    task.log.push({ t: "ev", ev: { ...ev, content: ev.content.length > 20000 ? `${ev.content.slice(0, 20000)}\n…(기록에서 생략)` : ev.content } });
  } else if (ev.kind === "request") {
    // 시스템 프롬프트는 매번 같아 기록하지 않는다
    task.log.push({ t: "ev", ev: { ...ev, system: "" } });
  } else if (ev.kind !== "tool_start" && ev.kind !== "usage") {
    task.log.push({ t: "ev", ev });
  }
  persist(task);
}

function handle(ev: AgentEvent) {
  const task = tasks.find((t) => t.id === ev.session);
  if (!task) return;
  record(task, ev);
  switch (ev.kind) {
    case "request":
      if (ev.context) {
        append(task, contextCard(ev.context, ev.context_tokens, ev.request_id));
        addContext(task, ev.context);
        const m = ev.context.match(/토큰 약 (\d+)\/(\d+)/);
        if (m && task === active && !replaying) hooks.onContext(Number(m[1]), Number(m[2]));
      }
      break;
    case "text":
      if (!task.current) {
        const el = h("div", { class: "md" });
        append(task, el);
        task.current = { el, text: "" };
      }
      task.current.text += ev.text;
      if (!task.renderQueued) {
        task.renderQueued = true;
        requestAnimationFrame(() => {
          task.renderQueued = false;
          if (task.current) stick(task, () => renderMarkdown(task.current!.el, task.current!.text, true));
        });
      }
      break;
    case "tool_start":
      endText(task);
      task.pendingTool?.remove();
      task.pendingTool = h("div", { class: "note" }, codicon("loading", "codicon-modifier-spin"), `${ev.name} 준비 중`);
      append(task, task.pendingTool);
      break;
    case "tool_call": {
      endText(task);
      task.pendingTool?.remove();
      task.pendingTool = null;
      const el = h("details", { class: "tool" },
        h("summary", {},
          h("span", { class: "tstate run" }, codicon("loading", "codicon-modifier-spin")),
          h("span", { class: "tname" }, ev.name),
          h("span", { class: "targ" }, toolSummary(ev.input))),
        h("pre", {}, JSON.stringify(ev.input, null, 2)));
      task.toolEls.set(ev.id, el);
      task.toolInputs.set(ev.id, { name: ev.name, input: ev.input });
      append(task, el);
      const path = str(ev.input.path);
      if (path) {
        if (ev.name === "read_file") task.footprint.read.add(path);
        task.footprint.last = path;
        emitFootprint(task);
      }
      break;
    }
    case "tool_result": {
      const el = task.toolEls.get(ev.id);
      if (ev.name === "get_context" && !ev.is_error) addContext(task, ev.content);
      if (!el) break;
      el.querySelector(".tstate")!.replaceWith(
        h("span", { class: `tstate ${ev.is_error ? "err" : "ok"}`, title: ev.is_error ? "실패" : "완료" }, codicon(ev.is_error ? "error" : "check")));
      el.querySelector("pre")!.textContent = ev.content;
      break;
    }
    case "approval": {
      endText(task);
      const isEdit = ev.approval_kind === "edit";
      const decide = (ok: boolean) => void api.resolveApproval(task.id, ev.id, ok);
      const apply = h("button", { class: "btn btn-primary", onclick: () => decide(true) }, codicon("check"), isEdit ? "적용" : "실행");
      const target = isEdit ? diffTarget(ev.detail) : null;
      const newPath = isEdit ? ev.detail.match(/^\+\+\+ (?:b\/)?(.+)$/m)?.[1]?.trim() : undefined;
      if (newPath) task.approvalPaths.set(ev.id, newPath);
      const el = h("div", { class: "approval", role: "group", "aria-label": ev.title },
        h("div", { class: "ahead" }, codicon(isEdit ? "edit" : "terminal"), h("span", { class: "atitle" }, ev.title), h("span", { class: "verdict" })),
        isEdit ? renderDiff(ev.detail) : h("pre", { class: "diff" }, h("div", {}, ev.detail)),
        target && !replaying ? impactRow(ev.detail, (names) => {
          task.approvalTouched.set(ev.id, names);
          task.log.push({ t: "touched", id: ev.id, names });
          persist(task);
        }) : null,
        h("div", { class: "actions" },
          apply,
          h("button", { class: "btn btn-secondary", onclick: () => decide(false) }, "거절"),
          h("span", { class: "hint" }, isEdit ? "적용하기 전에는 파일이 바뀌지 않습니다" : "허용 목록 밖의 명령입니다")));
      task.approvalEls.set(ev.id, el);
      append(task, el);
      task.pendingApprovals++;
      setStatus(task, "approval");
      if (task === active && !replaying) {
        // 승인 버튼 줄까지 보이게 한다.
        el.querySelector(".actions")!.scrollIntoView({ block: "nearest" });
        apply.focus({ preventScroll: true });
      }
      break;
    }
    case "approval_resolved": {
      const el = task.approvalEls.get(ev.id);
      if (!el) break;
      el.classList.add("resolved");
      const v = el.querySelector(".verdict")!;
      v.textContent = ev.approved ? "적용됨" : "거절됨";
      v.classList.toggle("yes", ev.approved);
      const p = task.approvalPaths.get(ev.id);
      if (ev.approved && p) {
        task.footprint.edited.add(p);
        for (const name of task.approvalTouched.get(ev.id) ?? []) task.footprint.editedSymbols.add(`${p}#${name}`);
      }
      task.pendingApprovals = Math.max(0, task.pendingApprovals - 1);
      if (task.status === "approval" && !task.pendingApprovals) setStatus(task, "running");
      else emitFootprint(task);
      break;
    }
    case "usage":
      if (!replaying) hooks.onUsage();
      break;
    case "done":
      if (ev.changed.length) {
        const unique = [...new Set(ev.changed)];
        for (const p of unique) task.footprint.edited.add(p);
        task.changed = [...new Set([...task.changed, ...unique])];
        const head = h("div", { class: "ch-head" }, codicon("pass"), `파일 ${unique.length}개를 바꿨습니다`);
        const box = h("div", { class: "changed" }, head);
        if (ev.checkpoint) {
          const cp = ev.checkpoint;
          const undo = h("button", { class: "btn btn-secondary ch-undo" }, codicon("discard"), "이 작업 되돌리기");
          undo.addEventListener("click", async () => {
            const ok = await ask(`이 작업으로 바뀐 파일 ${unique.length}개를 작업 전으로 되돌릴까요? 새로 만든 파일은 휴지통으로 옮깁니다.`, { title: "되돌리기", kind: "warning", okLabel: "되돌리기", cancelLabel: "취소" });
            if (!ok) return;
            try {
              const n = await revertSafely(cp);
              if (n === null) return;
              undo.remove();
              head.replaceChildren(codicon("discard"), `되돌렸습니다 (파일 ${n}개)`);
              box.classList.add("reverted");
              if (!task.isolated || task.isolated.state !== "active") {
                for (const p of unique) void editor.reloadIfClean(p);
                hooks.onFilesChanged(unique);
              }
            } catch (e) {
              showError(task, errorText(e));
            }
          });
          head.append(undo);
        }
        for (const p of unique) {
          const a = h("a", { title: p }, fileIcon(p), p);
          a.addEventListener("click", () => void editor.openFile(p));
          box.append(a);
        }
        append(task, box);
        // 격리된 작업의 변경은 작업 공간에 있다. 프로젝트에 적용할 때 반영한다.
        if (task.isolated?.state === "active") {
          if (!replaying) task.log.push({ t: "iso-done" });
          append(task, isolationBox(task));
        } else if (!replaying) hooks.onFilesChanged(ev.changed);
      }
      task.body = null;
      task.pendingApprovals = 0;
      setStatus(task, "done");
      break;
    case "error":
      showError(task, ev.message);
      task.body = null;
      task.pendingApprovals = 0;
      setStatus(task, "error");
      break;
  }
}

async function send(text?: string) {
  const input = $<HTMLTextAreaElement>("#prompt");
  const msg = (text ?? input.value).trim();
  if (!msg) return;
  if (!active) activate(newTask());
  const task = active!;
  if (isRunning(task)) return;
  // 첫 요청에서 격리를 골랐으면 작업 공간부터 만든다
  const first = !task.log.some((e) => e.t === "user");
  if (first && $<HTMLInputElement>("#isolate-task").checked && !task.isolated) {
    try {
      const w = await invoke<{ path: string; branch: string }>("worktree_create", { session: task.id });
      task.isolated = { ...w, state: "active" };
    } catch (e) {
      toast.error("격리 작업 공간을 만들지 못했습니다", errorText(e));
      return;
    }
  }
  task.lastPrompt = msg;
  if (task.title === NEW_TITLE) task.title = msg.length > 48 ? `${msg.slice(0, 46)}…` : msg;
  const agent = $<HTMLSelectElement>("#agent-select").value;
  const useFocus = $<HTMLInputElement>("#focus-file").checked;
  const f = useFocus ? editor.focusInfo() : { file: null, line: null };
  if (!text) input.value = "";
  const u = turn(task, "user");
  u.append(h("div", { class: "user-text" }, msg));
  if (first && task.isolated) u.append(isolationNote(task.isolated));
  task.log.push({ t: "user", text: msg });
  persist(task);
  syncIsolateToggle();
  task.footprint.last = null;
  setStatus(task, "running");
  task.body = turn(task, "bot");
  setWorking(task.botMark, true);
  try {
    await api.agentSend(task.id, agent, msg, f.file, f.line);
  } catch (e) {
    showError(task, errorText(e));
    task.body = null;
    setStatus(task, "error");
  }
}

// ── 작업 목록 ─────────────────────────────────────────────

const NEW_TITLE = "새 작업";

function newTask(): Task {
  const task: Task = {
    id: crypto.randomUUID(),
    title: NEW_TITLE,
    status: "idle",
    created: Date.now(),
    changed: [],
    footprint: emptyFootprint(),
    el: h("div", { class: "task-log" }),
    body: null, botMark: null, current: null, pendingTool: null,
    toolEls: new Map(), approvalEls: new Map(), approvalPaths: new Map(), approvalTouched: new Map(), toolInputs: new Map(),
    renderQueued: false, lastPrompt: "", pendingApprovals: 0,
    log: [], isolated: null, updated: Date.now(),
  };
  task.el.append(emptyState());
  tasks.unshift(task);
  return task;
}

function activate(task: Task) {
  active = task;
  const box = messages();
  for (const t of tasks) t.el.classList.toggle("hidden", t !== task);
  if (!task.el.isConnected) box.append(task.el);
  box.scrollTop = box.scrollHeight;
  syncComposer();
  syncIsolateToggle();
  hooks.onTasks();
  hooks.onFootprint(task.footprint, task.title, isRunning(task));
  renderHead();
}

/** 오른쪽 위: 지금 작업의 이름과 발자취 요약 */
function renderHead() {
  const head = $("#task-head");
  const t = active;
  if (!t || t.title === NEW_TITLE) {
    head.classList.add("hidden");
    return;
  }
  const fp = t.footprint;
  const chip = (icon: string, n: number, label: string, cls: string) => h("span", { class: `fp-chip ${cls}`, title: label }, codicon(icon), String(n));
  const mapBtn = h("button", { class: "icon-btn", title: "발자취를 지도에서 보기", "aria-label": "발자취를 지도에서 보기" }, codicon("type-hierarchy"));
  mapBtn.addEventListener("click", () => hooks.showOnMap());
  head.replaceChildren(
    h("span", { class: `th-status s-${t.status}` }, statusIcon(t)),
    h("span", { class: "th-title", title: t.title }, t.title),
    chip("eye", fp.context.size, "AI에 보낸 맥락 파일", "c"),
    chip("book", fp.read.size, "읽은 파일", "r"),
    chip("edit", fp.edited.size, "수정한 파일", "e"),
    mapBtn);
  head.classList.remove("hidden");
}

function statusIcon(t: Task): HTMLElement {
  switch (t.status) {
    case "running": return hex("plain", true);
    case "approval": return codicon("bell-dot");
    case "done": return codicon("pass");
    case "error": return codicon("error");
    case "stopped": return codicon("debug-pause");
    default: return codicon("circle-large-outline");
  }
}

const STATUS_TEXT: Record<TaskStatus, string> = { idle: "대기", running: "진행 중", approval: "승인 대기", done: "완료", error: "오류", stopped: "중단됨" };

function ago(ms: number): string {
  const s = Math.round((Date.now() - ms) / 1000);
  if (s < 60) return "방금";
  if (s < 3600) return `${Math.floor(s / 60)}분 전`;
  if (s < 86400) return `${Math.floor(s / 3600)}시간 전`;
  return `${Math.floor(s / 86400)}일 전`;
}

/** 격리되지 않은 작업끼리 같은 파일을 고쳤으면 (서로 덮어썼을 수 있다) */
function overlaps(t: Task): string[] {
  if (t.isolated?.state === "active") return [];
  const mine = t.footprint.edited;
  if (!mine.size) return [];
  const out = new Set<string>();
  for (const o of tasks) {
    if (o === t || o.isolated?.state === "active") continue;
    for (const p of o.footprint.edited) if (mine.has(p)) out.add(p);
  }
  return [...out];
}

/** 왼쪽 "작업" 보기 */
export function renderTaskList() {
  const list = $("#task-list");
  const shown = tasks.filter((t) => t.title !== NEW_TITLE || t === active);
  if (!shown.length || (shown.length === 1 && shown[0].title === NEW_TITLE)) {
    const b = h("button", { class: "btn btn-primary block" }, codicon("add"), "새 작업");
    b.addEventListener("click", () => void reset());
    list.replaceChildren(h("div", { class: "empty-view" },
      h("p", {}, "작업은 AI에게 맡긴 일 하나입니다. 대화, 보낸 맥락, 읽고 고친 파일, 되돌리기가 작업마다 묶입니다."),
      h("p", { class: "muted" }, "오른쪽에 요청을 적으면 작업이 시작됩니다."),
      b));
    return;
  }
  list.replaceChildren(...shown.map((t) => {
    const fp = t.footprint;
    const meta: string[] = [];
    const clash = overlaps(t);
    if (fp.edited.size) meta.push(`수정 ${fp.edited.size}`);
    if (fp.read.size) meta.push(`읽음 ${fp.read.size}`);
    if (fp.context.size) meta.push(`맥락 ${fp.context.size}`);
    const close = h("button", { class: "icon-btn", title: "작업 닫기", "aria-label": "작업 닫기" }, codicon("close"));
    close.addEventListener("click", (e) => {
      e.stopPropagation();
      void closeTask(t);
    });
    const map = h("button", { class: "icon-btn", title: "발자취를 지도에서 보기", "aria-label": "발자취를 지도에서 보기" }, codicon("type-hierarchy"));
    map.addEventListener("click", (e) => {
      e.stopPropagation();
      activate(t);
      hooks.showOnMap();
    });
    const row = h("div", { class: `task${t === active ? " active" : ""} s-${t.status}`, role: "option", tabindex: "0", "aria-selected": String(t === active) },
      h("span", { class: "task-status", title: STATUS_TEXT[t.status] }, statusIcon(t)),
      h("div", { class: "task-main" },
        h("div", { class: "task-title" }, t.title),
        h("div", { class: "task-meta" },
          t.isolated?.state === "active" ? h("span", { class: "iso", title: `격리된 작업 공간: ${t.isolated.branch}` }, codicon("git-branch")) : null,
          t.status === "approval" ? h("span", { class: "need" }, "승인 대기") : null,
          clash.length ? h("span", { class: "clash", title: `다른 작업도 고친 파일:\n${clash.join("\n")}` }, codicon("warning"), `겹침 ${clash.length}`) : null,
          meta.join(" · ") || STATUS_TEXT[t.status], h("span", { class: "ago" }, ago(t.updated)))),
      h("span", { class: "task-actions" }, map, close));
    row.addEventListener("click", () => activate(t));
    row.addEventListener("keydown", (e) => (e.key === "Enter" || e.key === " ") && activate(t));
    return row;
  }));
}

async function closeTask(t: Task) {
  if (isRunning(t)) {
    const ok = await ask("진행 중인 작업입니다. 중지하고 닫을까요?", { title: "작업 닫기", kind: "warning", okLabel: "중지하고 닫기", cancelLabel: "취소" });
    if (!ok) return;
    await api.agentCancel(t.id).catch(() => {});
  } else if (t.isolated?.state === "active") {
    const ok = await ask("이 작업의 격리된 작업 공간도 지웁니다. 프로젝트에 적용하지 않은 변경은 사라집니다. 닫을까요?", { title: "작업 닫기", kind: "warning", okLabel: "닫기", cancelLabel: "취소" });
    if (!ok) return;
  }
  clearTimeout(saveTimers.get(t.id));
  saveTimers.delete(t.id);
  await invoke("task_delete", { id: t.id }).catch((e) => toast.error("작업을 지우지 못했습니다", errorText(e)));
  await api.agentReset(t.id).catch(() => {});
  t.el.remove();
  tasks.splice(tasks.indexOf(t), 1);
  if (active === t) activate(tasks[0] ?? newTask());
  else hooks.onTasks();
}

export function taskCount(): { running: number; approval: number } {
  return { running: tasks.filter((t) => t.status === "running").length, approval: tasks.filter((t) => t.status === "approval").length };
}

let indexInfo = "";

/** 빈 채팅 화면에 보여줄 인덱스 정보 (맥락이 먼저다) */
export function setIndexInfo(text: string) {
  indexInfo = text;
  for (const note of document.querySelectorAll(".empty-chat .index-note span")) note.textContent = text;
  if (text && active && active.el.querySelector(".empty-chat") && !active.el.querySelector(".index-note")) active.el.replaceChildren(emptyState());
}

const EXAMPLES = [
  { icon: "symbol-structure", text: "이 프로젝트의 구조와 주요 흐름을 설명해줘" },
  { icon: "bug", text: "지금 열린 파일에서 버그가 될 만한 부분을 찾아줘" },
  { icon: "beaker", text: "현재 함수에 대한 테스트를 작성해줘" },
];

function emptyState(): HTMLElement {
  const box = h("div", { class: "empty-chat" },
    hex("brand"),
    h("h3", {}, "무엇을 맡길까요?"),
    h("p", {}, "요청마다 Lantern이 프로젝트에서 관련 코드를 골라 함께 보냅니다. 무엇을 보냈는지, 에이전트가 어디를 읽고 고쳤는지는 지도에서 볼 수 있습니다."),
    indexInfo ? h("div", { class: "index-note" }, hex("plain"), h("span", {}, indexInfo)) : null);
  const ex = h("div", { class: "examples" });
  for (const e of EXAMPLES) {
    const b = h("button", { class: "example" }, codicon(e.icon), h("span", {}, e.text));
    b.addEventListener("click", () => {
      const p = $<HTMLTextAreaElement>("#prompt");
      p.value = e.text;
      updateSendState();
      p.focus();
    });
    ex.append(b);
  }
  box.append(ex);
  return box;
}

export async function loadAgents() {
  agents = await api.listAgents().catch(() => []);
  const select = $<HTMLSelectElement>("#agent-select");
  const saved = store.get<string>("agent", "ask");
  select.replaceChildren(...agents.map((a) => h("option", { value: a.id, title: a.description }, a.name)));
  select.value = agents.some((a) => a.id === saved) ? saved : agents[0]?.id ?? "";
}

/** 새 작업. 지금 작업이 비어 있으면 그대로 쓴다. */
export async function reset() {
  if (active && active.title === NEW_TITLE && !isRunning(active)) {
    activate(active);
    focusPrompt();
    return;
  }
  activate(newTask());
  focusPrompt();
}

/** 프로젝트를 바꾸면 지금 작업들을 저장해 닫고, 새 프로젝트의 저장된 작업을 되살린다 */
export async function resetAll() {
  await flushAll();
  for (const t of [...tasks]) {
    if (isRunning(t)) await api.agentCancel(t.id).catch(() => {});
    await api.agentReset(t.id).catch(() => {});
    t.el.remove();
  }
  tasks.length = 0;
  active = null;
  const saved = await invoke<SavedTask[]>("task_list").catch(() => []);
  // 목록은 최근 것이 위: 오래된 것부터 되살려 앞에 끼운다
  for (const data of [...saved].reverse()) {
    try {
      restore(data);
    } catch (e) {
      console.warn("작업 복원 실패", data.id, e);
    }
  }
  activate(newTask());
}

// ── 저장과 복원 ─────────────────────────────────────────

interface SavedTask { id: string; title: string; created: number; updated: number; status: TaskStatus; isolated: Isolation | null; log: Entry[] }

const saveTimers = new Map<string, number>();

function snapshot(t: Task): SavedTask {
  return { id: t.id, title: t.title, created: t.created, updated: t.updated, status: t.status, isolated: t.isolated, log: t.log };
}

/** 잦은 이벤트를 묶어 0.8초 뒤에 저장한다 */
function persist(task: Task) {
  if (replaying || !task.log.length) return;
  task.updated = Date.now();
  clearTimeout(saveTimers.get(task.id));
  saveTimers.set(task.id, window.setTimeout(() => {
    saveTimers.delete(task.id);
    void invoke("task_save", { id: task.id, data: snapshot(task) }).catch((e) => console.warn("작업 저장 실패", e));
  }, 800));
}

/** 창을 닫거나 프로젝트를 바꾸기 전에 밀린 저장을 끝낸다 */
export async function flushAll() {
  const pending = [...saveTimers.keys()];
  for (const id of pending) {
    clearTimeout(saveTimers.get(id));
    saveTimers.delete(id);
    const t = tasks.find((x) => x.id === id);
    if (t) await invoke("task_save", { id, data: snapshot(t) }).catch(() => {});
  }
}

function restore(data: SavedTask) {
  const task = newTask();
  task.id = data.id;
  task.title = data.title;
  task.created = data.created;
  task.updated = data.updated;
  task.isolated = data.isolated;
  task.log = data.log ?? [];
  task.el.classList.add("hidden");
  messages().append(task.el);
  replaying = true;
  try {
    for (const e of task.log) {
      if (e.t === "user") {
        const u = turn(task, "user");
        u.append(h("div", { class: "user-text" }, e.text));
        if (task.isolated && e === task.log.find((x) => x.t === "user")) u.append(isolationNote(task.isolated));
        task.lastPrompt = e.text;
        task.body = turn(task, "bot");
      } else if (e.t === "touched") {
        task.approvalTouched.set(e.id, e.names);
      } else if (e.t === "ev") {
        handle({ ...e.ev, session: task.id } as AgentEvent);
        if (e.ev.kind === "approval_resolved") {
          const p = task.approvalPaths.get(e.ev.id);
          if (e.ev.approved && p) for (const n of task.approvalTouched.get(e.ev.id) ?? []) task.footprint.editedSymbols.add(`${p}#${n}`);
        }
      }
    }
    // 끝나지 않은 채 저장된 작업: 승인 카드는 중단으로
    for (const [, el] of task.approvalEls) {
      if (el.classList.contains("resolved")) continue;
      el.classList.add("resolved");
      el.querySelector(".verdict")!.textContent = "중단됨";
      el.querySelector(".actions")?.remove();
    }
  } finally {
    replaying = false;
  }
  task.pendingApprovals = 0;
  task.status = data.status === "running" || data.status === "approval" ? "stopped" : data.status;
  task.body = null;
  setWorking(task.botMark, false);
  if (task.status === "stopped") task.el.append(h("div", { class: "note stopped-note" }, codicon("debug-pause"), "앱이 닫혀 이 작업이 중간에 멈췄습니다. 이어서 하려면 메시지를 보내세요."));
}

// ── 격리 ────────────────────────────────────────────────

function isolationNote(w: Isolation): HTMLElement {
  return h("div", { class: "iso-note", title: w.path }, codicon("git-branch"), `격리된 작업 공간에서 합니다 · ${w.branch}`);
}

/** 작업 공간의 변경을 보고, 프로젝트에 적용하거나 버린다 */
function isolationBox(task: Task): HTMLElement {
  const box = h("div", { class: "iso-box" });
  const render = async () => {
    const w = task.isolated;
    if (!w || w.state === "discarded") {
      box.replaceChildren(h("div", { class: "iso-head" }, codicon("trash"), "격리된 작업 공간을 버렸습니다"));
      box.classList.add("done");
      return;
    }
    const ch = await invoke<{ files: string[]; diff: string }>("worktree_changes", { session: task.id }).catch((e) => ({ files: [] as string[], diff: "", error: errorText(e) }));
    if ("error" in ch) {
      box.replaceChildren(h("div", { class: "iso-head" }, codicon("warning"), ch.error as string));
      return;
    }
    if (!ch.files.length) {
      box.replaceChildren(h("div", { class: "iso-head" }, codicon("pass"), "작업 공간의 변경이 모두 프로젝트에 반영되었습니다"));
      box.classList.add("done");
      return;
    }
    const view = h("button", { class: "btn btn-secondary" }, codicon("diff"), "변경 보기");
    view.addEventListener("click", () => {
      editor.openPage(`wt:${task.id}`, `변경: ${task.title}`, "git-compare", (host) => {
        host.classList.add("diff-page");
        host.append(h("div", { class: "diff-head" }, codicon("git-branch"), h("b", {}, w.branch), h("span", { class: "muted" }, `파일 ${ch.files.length}개 · 아직 프로젝트에 적용하지 않음`)), renderDiff(ch.diff));
      });
    });
    const apply = h("button", { class: "btn btn-primary" }, codicon("check"), "프로젝트에 적용");
    apply.addEventListener("click", async () => {
      apply.setAttribute("disabled", "");
      try {
        const files = await invoke<string[]>("worktree_apply", { session: task.id });
        hooks.onFilesChanged(files);
        toast.success(`격리된 변경 ${files.length}개 파일을 프로젝트에 적용했습니다`);
        await render();
      } catch (e) {
        apply.removeAttribute("disabled");
        toast.show({ kind: "error", message: "프로젝트에 적용하지 못했습니다", detail: errorText(e), timeout: 0 });
      }
    });
    const discard = h("button", { class: "btn btn-secondary" }, "버리기");
    discard.addEventListener("click", async () => {
      const ok = await ask(`격리된 작업 공간의 변경 ${ch.files.length}개 파일을 버릴까요? 되돌릴 수 없습니다.`, { title: "변경 버리기", kind: "warning", okLabel: "버리기", cancelLabel: "취소" });
      if (!ok) return;
      await invoke("worktree_discard", { session: task.id }).catch((e) => toast.error("작업 공간을 지우지 못했습니다", errorText(e)));
      task.isolated = { ...w, state: "discarded" };
      persist(task);
      hooks.onTasks();
      await render();
    });
    box.replaceChildren(
      h("div", { class: "iso-head" }, codicon("git-branch"), `격리된 작업 공간에 파일 ${ch.files.length}개 변경`, h("span", { class: "muted" }, "프로젝트는 아직 그대로입니다")),
      h("div", { class: "iso-files" }, ...ch.files.slice(0, 8).map((f) => h("span", { title: f }, fileIcon(f), basename(f))), ch.files.length > 8 ? h("span", { class: "muted" }, `외 ${ch.files.length - 8}`) : null),
      h("div", { class: "iso-actions" }, apply, view, discard));
  };
  box.append(h("div", { class: "iso-head" }, codicon("loading", "codicon-modifier-spin"), "변경을 확인하는 중…"));
  void render();
  return box;
}

/** 격리 체크는 아직 시작하지 않은 작업에서만 바꿀 수 있다 */
function syncIsolateToggle() {
  const box = document.querySelector<HTMLInputElement>("#isolate-task");
  if (!box || !active) return;
  const started = active.log.some((e) => e.t === "user");
  if (started) box.checked = active.isolated?.state === "active";
  box.disabled = started;
  box.closest("label")?.classList.toggle("disabled", started);
}

export function focusPrompt() {
  $("#prompt").focus();
}

export function activeFootprint(): { fp: Footprint; title: string } | null {
  return active ? { fp: active.footprint, title: active.title === NEW_TITLE ? "" : active.title } : null;
}

export function setModelLabel(text: string, warn: boolean) {
  $("#composer-model-name").textContent = text;
  $("#composer-model").classList.toggle("warn", warn);
  $("#composer-model").title = warn ? "API 키가 없습니다. 눌러서 설정하세요" : "모델 설정";
}

export function init(h_: Hooks) {
  hooks = h_;
  on<AgentEvent>("agent", handle);
  activate(newTask());
  $("#btn-send").addEventListener("click", () => void send());
  $("#btn-stop").addEventListener("click", () => active && void api.agentCancel(active.id));
  $("#composer-model").addEventListener("click", () => hooks.openSettings("models"));
  $("#agent-select").addEventListener("change", (e) => store.set("agent", (e.target as HTMLSelectElement).value));
  const prompt = $<HTMLTextAreaElement>("#prompt");
  prompt.addEventListener("input", () => {
    updateSendState();
    prompt.style.height = "auto";
    prompt.style.height = `${Math.min(prompt.scrollHeight, 220)}px`;
  });
  prompt.addEventListener("keydown", (e) => {
    if (e.key === "Enter" && !e.shiftKey && !e.isComposing) {
      e.preventDefault();
      void send();
    } else if (e.key === "Escape" && isRunning(active)) {
      void api.agentCancel(active!.id);
    }
  });
  // "n분 전"을 가끔 새로 그린다
  setInterval(() => hooks.onTasks(), 60_000);
}
