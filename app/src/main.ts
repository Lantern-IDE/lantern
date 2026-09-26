import { getVersion } from "@tauri-apps/api/app";
import "@vscode/codicons/dist/codicon.css";
import "pretendard/dist/web/variable/pretendardvariable-dynamic-subset.css";
import "./styles.css";
import { open as openDialog, ask, message } from "./dialog";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { undo, redo, selectAll } from "@codemirror/commands";
import { openSearchPanel, gotoLine } from "@codemirror/search";
import { api, errorText, on, type IndexEvent, type ProjectInfo } from "./api";
import * as commands from "./commands";
import { $, basename, dirname, h, store } from "./dom";
import { codicon, fileIcon, hex, mountHexes } from "./icons";
import * as editor from "./editor";
import * as tree from "./tree";
import * as lsp from "./lsp";
import * as terminal from "./terminal";
import * as chat from "./chat";
import * as inspector from "./inspector";
import * as quickopen from "./quickopen";
import * as search from "./search";
import * as prefs from "./prefs";
import * as toast from "./toast";
import * as settingsPage from "./settingsPage";
import * as onboarding from "./onboarding";
import * as scm from "./scm";
import * as keybindings from "./keybindings";
import * as report from "./report";
import * as i18n from "./i18n";
import * as update from "./update";
import * as completion from "./completion";
import * as map from "./map";
import * as memory from "./memory";

i18n.init();
prefs.init();
const win = getCurrentWindow();
let project: ProjectInfo | null = null;
let agentRunning = false;
let indexing = false;
let budgetWarned = false;
let indexedFiles = "";
let lastContext = "";

// ── 레이아웃: 기본 사이드바, 패널, 보조 사이드바 ─────────────

type Part = "sidebar" | "panel" | "auxbar";
// 처음 실행할 때 창이 좁으면 채팅 사이드바를 닫고 시작한다 (Ctrl+Alt+B로 연다). 이후에는 사용자가 정한 배치를 따른다.
const layout = store.get<Record<Part, boolean>>("layout.visible", { sidebar: true, panel: false, auxbar: window.innerWidth >= 1100 });

function setPart(part: Part, visible: boolean) {
  layout[part] = visible;
  $(`#${part}`).classList.toggle("collapsed", !visible);
  const btn = { sidebar: "#toggle-sidebar", panel: "#toggle-panel", auxbar: "#toggle-auxbar" }[part];
  $(btn).classList.toggle("on", visible);
  $(btn).setAttribute("aria-pressed", String(visible));
  store.set("layout.visible", layout);
  if (part === "panel" && visible && $("#terminal").classList.contains("active")) void terminal.show();
  if (part === "auxbar") document.querySelector('.ab-item[data-aux="chat"]')?.classList.toggle("active", visible);
  if (part === "sidebar") syncActivity();
}

function togglePart(part: Part) {
  setPart(part, !layout[part]);
}

let activeView = store.get<string>("layout.view", "tasks");

function syncActivity() {
  for (const b of document.querySelectorAll<HTMLElement>(".ab-item[data-view]")) {
    b.classList.toggle("active", layout.sidebar && b.dataset.view === activeView);
  }
}

/** VS Code처럼: 이미 보이는 뷰의 아이콘을 다시 누르면 사이드바를 접는다. */
function showView(view: string, toggle = false) {
  if (toggle && layout.sidebar && activeView === view) return setPart("sidebar", false);
  activeView = view;
  store.set("layout.view", view);
  for (const v of document.querySelectorAll<HTMLElement>("#sidebar .view")) v.classList.toggle("active", v.id === `view-${view}`);
  setPart("sidebar", true);
  if (view === "search") search.focus();
  if (view === "scm") void scm.refresh({ repos: true }); // 터미널에서 원격·브랜치를 바꿨을 수 있다
  if (view === "memory") void memory.refresh();
  if (view === "tasks") {
    chat.renderTaskList();
    setPart("auxbar", true);
  }
}

/** 가운데를 편집기나 지도로 */
function syncCanvasSwitch(mapOn: boolean) {
  for (const b of document.querySelectorAll<HTMLElement>("#canvas-switch [data-canvas]")) {
    const on = (b.dataset.canvas === "map") === mapOn;
    b.classList.toggle("on", on);
    b.setAttribute("aria-selected", String(on));
  }
}

/** 지금 작업의 발자취를 지도에서 */
function showFootprintOnMap() {
  const a = chat.activeFootprint();
  if (a) map.setFootprint(a.fp, a.title);
  map.showFootprint();
}

function showPanel(name: string) {
  for (const b of document.querySelectorAll<HTMLElement>(".panel-tabs button[data-panel]")) {
    b.classList.toggle("active", b.dataset.panel === name);
    b.setAttribute("aria-selected", String(b.dataset.panel === name));
  }
  for (const v of document.querySelectorAll<HTMLElement>("#panel .panel-view")) v.classList.toggle("active", v.id === name);
  setPart("panel", true);
  if (name === "terminal") void terminal.show();
}

function showAux(tab: "chat" | "inspector") {
  setPart("auxbar", true);
  for (const b of document.querySelectorAll<HTMLElement>(".aux-tabs button")) {
    b.classList.toggle("active", b.dataset.ai === tab);
    b.setAttribute("aria-selected", String(b.dataset.ai === tab));
  }
  for (const p of document.querySelectorAll<HTMLElement>("#auxbar .ai-panel")) p.classList.toggle("active", p.id === tab);
  if (tab === "chat") chat.focusPrompt();
}

/** 토스트가 채팅(승인 카드)을 가리지 않도록 보조 사이드바 폭을 CSS 변수로 알려준다. */
function trackAuxWidth() {
  const aux = $("#auxbar");
  const sync = () => document.documentElement.style.setProperty("--aux-open-w", aux.classList.contains("collapsed") ? "0px" : `${aux.offsetWidth}px`);
  new ResizeObserver(sync).observe(aux);
  new MutationObserver(sync).observe(aux, { attributes: true, attributeFilter: ["class"] });
  sync();
}

function initSashes() {
  const root = document.documentElement;
  const saved = store.get<Record<string, string>>("layout.sizes", {});
  for (const [k, v] of Object.entries(saved)) root.style.setProperty(k, v);
  for (const s of document.querySelectorAll<HTMLElement>(".sash, .sash-h")) {
    s.addEventListener("mousedown", (down) => {
      down.preventDefault();
      const target = $(`#${s.dataset.target}`);
      const vertical = s.classList.contains("sash-h");
      const start = vertical ? down.clientY : down.clientX;
      const size = vertical ? target.offsetHeight : target.offsetWidth;
      const dir = vertical ? -1 : Number(s.dataset.dir ?? 1);
      const cssVar = s.dataset.var!;
      s.classList.add("dragging");
      document.body.style.cursor = vertical ? "ns-resize" : "ew-resize";
      const move = (e: MouseEvent) => {
        const delta = (vertical ? e.clientY : e.clientX) - start;
        root.style.setProperty(cssVar, `${Math.max(120, size + dir * delta)}px`);
      };
      const up = () => {
        s.classList.remove("dragging");
        document.body.style.cursor = "";
        window.removeEventListener("mousemove", move);
        window.removeEventListener("mouseup", up);
        store.set("layout.sizes", { ...store.get("layout.sizes", {}), [cssVar]: root.style.getPropertyValue(cssVar) });
      };
      window.addEventListener("mousemove", move);
      window.addEventListener("mouseup", up);
    });
  }
}

/** 폴더가 없으면 시작 화면, 폴더는 있고 탭이 없으면 워터마크 */
function updateBackdrop() {
  $("#welcome").classList.toggle("hidden", !!project || editor.tabCount() > 0);
  $("#watermark").classList.toggle("show", !!project && editor.tabCount() === 0);
  $("#tree").classList.toggle("hidden", !project);
  $("#no-folder").classList.toggle("hidden", !!project);
  $("#project-header").classList.toggle("hidden", !project);
}

// ── 프로젝트 ───────────────────────────────────────────

function recent(): string[] {
  return store.get<string[]>("recent", []);
}

function renderRecent() {
  const list = recent().slice(0, 6);
  $("#recent").replaceChildren(
    ...(list.length
      ? list.map((p) => {
          const norm = p.replace(/\\/g, "/");
          const a = h("a", { class: "wl", title: p }, codicon("folder"), h("span", {}, basename(norm)), h("span", { class: "rdir" }, dirname(norm)));
          a.addEventListener("click", () => void openProject(p));
          return a;
        })
      : [h("span", { class: "muted" }, "최근 연 폴더가 없습니다.")]),
  );
}

// ── 언어 서버 설치 제안 ─────────────────────────────────
const LSP_DISMISSED = "lantern.lsp.dismissed";

function lspDismissed(): string[] {
  try {
    return JSON.parse(localStorage.getItem(LSP_DISMISSED) ?? "[]");
  } catch {
    return [];
  }
}

async function offerLspInstall(lang: string, ignoreFound: boolean) {
  if (lspDismissed().includes(lang)) return;
  const plan = await api.lspInstallPlan(lang, ignoreFound).catch(() => null);
  if (!plan) return;
  const never = {
    label: "다시 묻지 않기",
    run: () => {
      try {
        localStorage.setItem(LSP_DISMISSED, JSON.stringify([...lspDismissed(), lang]));
      } catch { /* 저장 못 해도 이번 세션에는 다시 묻지 않는다 */ }
    },
  };
  if (plan.missing) {
    toast.show({ kind: "info", message: `${lang} 자동 완성·정의 이동을 쓰려면 언어 서버가 필요합니다`, detail: plan.missing, actions: [never] });
    return;
  }
  toast.show({
    kind: "info",
    message: `${lang} 언어 서버를 설치할까요?`,
    detail: `자동 완성, 정의로 이동, 오류 표시가 켜집니다. ${plan.summary}.`,
    timeout: 0,
    actions: [{ label: "설치", primary: true, run: () => void installLsp(lang) }, never],
  });
}

async function installLsp(lang: string) {
  const close = toast.show({ kind: "info", message: `${lang} 언어 서버를 설치하는 중…`, detail: "네트워크 속도에 따라 1~2분 걸릴 수 있습니다.", timeout: 0 });
  try {
    await api.lspInstall(lang);
    close();
    toast.success(`${lang} 언어 서버를 설치했습니다`);
    lsp.retry(lang);
    await editor.reattachLsp(lang);
  } catch (e) {
    close();
    appendOutput(`lsp:${lang}`, errorText(e));
    toast.show({ kind: "error", message: `${lang} 언어 서버를 설치하지 못했습니다`, detail: errorText(e).split("\n").at(-1), actions: [{ label: "출력 보기", run: () => showPanel("output") }] });
  }
}

async function openProject(path: string) {
  if (editor.hasDirty()) {
    const ok = await ask("저장하지 않은 파일이 있습니다. 그래도 다른 폴더를 열까요?", { title: "Lantern", kind: "warning", okLabel: "열기", cancelLabel: "취소" });
    if (!ok) return;
  }
  try {
    project = await api.openProject(path);
  } catch (e) {
    store.set("recent", recent().filter((p) => p !== path));
    renderRecent();
    toast.error("폴더를 열 수 없습니다", errorText(e));
    return;
  }
  if (!project.trusted) {
    // VS Code의 작업 영역 신뢰와 같다. 모르는 저장소의 설정이 모델 주소를 바꾸거나 명령을 실행하지 못하게 한다.
    const ok = await ask(
      `${project.name} 폴더의 작성자를 신뢰하나요?\n\n신뢰하면 이 폴더의 .lantern 설정(모델 주소, 훅, 언어 서버)과 에이전트를 쓰고, AI가 파일을 수정하거나 명령을 실행할 수 있습니다(매번 승인 필요).\n\n신뢰하지 않으면 제한 모드로 엽니다. 코드를 읽고 질문하는 것은 그대로 됩니다.`,
      { title: "폴더 신뢰", kind: "warning", okLabel: "신뢰함", cancelLabel: "제한 모드로 열기" },
    );
    if (ok) {
      await api.setTrust(true).catch((e) => toast.error("신뢰 설정을 저장하지 못했습니다", errorText(e)));
      project.trusted = true;
    }
  }
  renderTrust();
  if (project.migrated) {
    toast.info("설정 폴더 이름을 .lantern으로 바꿨습니다", "제품 이름이 Lantern으로 바뀌어 예전 .khala 폴더를 옮겼습니다. git에 올려 두었다면 이름 바뀜으로 보입니다.");
  }
  store.set("recent", [project.root, ...recent().filter((p) => p !== project!.root)].slice(0, 10));
  document.title = `${project.name} — Lantern`;
  $("#cc-label").textContent = project.name;
  $("#project-name").textContent = project.name;

  editor.closeAllFiles();
  lsp.configure({
    rootUri: project.root_uri,
    display: (p) => editor.openFile(p),
    onStatus: (t) => ($("#st-lsp").textContent = t ? `LSP ${t}` : ""),
    onMissing: (lang) => void offerLspInstall(lang, false),
    onExit: (lang, reason) => {
      // 설치가 덜 됐거나(rustup 껍데기), 함께 설치한 TypeScript를 못 찾으면 (재)설치를 제안한다
      if (/unknown binary|not installed|valid TypeScript installation/i.test(reason)) {
        void offerLspInstall(lang, true);
        return;
      }
      appendOutput(`lsp:${lang}`, `언어 서버가 종료되었습니다.${reason ? `\n${reason}` : ""}`);
      toast.show({ kind: "warn", message: `${lang} 언어 서버를 쓸 수 없습니다`, detail: reason.split("\n").at(-1) || undefined, actions: [{ label: "출력 보기", run: () => showPanel("output") }, { label: "설정", run: () => settingsPage.open() }] });
    },
  });
  tree.reset();
  await tree.refresh();
  quickopen.invalidate();
  search.clear();
  await terminal.restart();
  if (layout.panel && $("#terminal").classList.contains("active")) void terminal.show();
  await chat.loadAgents();
  await chat.resetAll();
  map.reset();
  void memory.refresh();
  updateBackdrop();
  // 프로젝트를 열면 코드 지도부터 (열린 탭이 없을 때)
  if (editor.tabCount() === 0) map.show();
  settingsPage.refreshIfOpen();
  void refreshModelInfo();
  scm.reset();
  void scm.refresh({ repos: true });
}

function renderTrust() {
  $("#sb-restricted").classList.toggle("hidden", !project || project.trusted);
}

async function toggleTrust() {
  if (!project) return;
  const next = !project.trusted;
  const ok = await ask(
    next
      ? `${project.name} 폴더를 신뢰할까요? 이 폴더의 .lantern 설정과 에이전트를 쓰고, AI가 승인을 받아 파일을 수정하거나 명령을 실행할 수 있게 됩니다.`
      : `${project.name} 폴더의 신뢰를 거둘까요? 제한 모드로 바뀌어 프로젝트 설정과 에이전트를 쓰지 않고, AI는 읽기만 합니다.`,
    { title: "폴더 신뢰", kind: "warning", okLabel: next ? "신뢰함" : "신뢰 해제", cancelLabel: "취소" },
  );
  if (!ok) return;
  try {
    await api.setTrust(next);
    project.trusted = next;
    renderTrust();
    await chat.loadAgents();
    void refreshModelInfo();
    settingsPage.refreshIfOpen();
    toast.info(next ? "이 폴더를 신뢰합니다" : "제한 모드로 바꿨습니다");
  } catch (e) {
    toast.error("신뢰 설정을 바꾸지 못했습니다", errorText(e));
  }
}

async function pickProject() {
  const dir = await openDialog({ directory: true, multiple: false, title: "폴더 열기" });
  if (typeof dir === "string") await openProject(dir);
}

async function openConfigFile() {
  if (!project) return toast.info("먼저 프로젝트 폴더를 여세요", "프로젝트 설정 파일은 <프로젝트>/.lantern/config.toml 입니다.");
  try {
    const path = await api.ensureConfig();
    await editor.openFile(path);
    await tree.refresh();
    await chat.loadAgents();
  } catch (e) {
    toast.error("설정 파일을 열 수 없습니다", errorText(e));
  }
}

// ── 상태 표시줄 ────────────────────────────────────────

function renderContextStatus(text: string, title: string, cls = "") {
  const el = $("#ctx-status");
  el.className = `sb-item clickable sb-context${cls ? " " + cls : ""}`;
  if (!text) return el.replaceChildren();
  el.replaceChildren(hex("plain", indexing || agentRunning), text);
  el.title = title;
}

const indexedOnce = new Set<string>();
const dur = (ms: number) => (ms < 1000 ? `${ms}ms` : `${(ms / 1000).toFixed(1)}초`);
on<IndexEvent>("index", (ev) => {
  indexing = ev.status === "running";
  if (ev.status === "running") renderContextStatus("인덱싱 중", "맥락 엔진이 코드를 인덱싱하고 있습니다");
  else if (ev.status === "done" && ev.stats) {
    const s = ev.stats;
    const files = (s.indexed + s.unchanged).toLocaleString();
    indexedFiles = files;
    renderContextStatus(`맥락 ${files}${lastContext}`, `맥락 엔진 인덱스: 파일 ${files}개 (${s.indexed}개 갱신, ${dur(s.elapsed_ms)}). 눌러서 맥락 미리보기`);
    // 인덱싱 중에 그린 지도는 비었거나 모자랄 수 있다
    if (s.indexed > 0 || map.isEmpty()) map.refreshSoon();
    chat.setIndexInfo(s.indexed > 0
      ? `이 프로젝트의 파일 ${files}개를 인덱싱했습니다 · ${dur(s.elapsed_ms)}`
      : `이 프로젝트의 파일 ${files}개가 인덱스에 있습니다 · 최신 상태`);
    if (project && !indexedOnce.has(project.root)) {
      indexedOnce.add(project.root);
      if (s.indexed > 0) toast.info(`맥락 인덱스를 만들었습니다 · 파일 ${files}개`, `${dur(s.elapsed_ms)}. 이제 질문하면 관련 코드가 함께 전달됩니다.`);
    }
  } else {
    renderContextStatus("인덱스 오류", ev.message ?? "", "err");
    toast.error("맥락 인덱스를 만들지 못했습니다", ev.message);
  }
});

async function refreshModelInfo() {
  const model = $("#st-model");
  const meter = $("#usage-meter");
  try {
    const info = await api.modelInfo();
    const m = info.models.find((x) => x.key === info.default);
    const missingKey = !!m && !m.has_key;
    model.className = `sb-item clickable${missingKey ? " warn" : ""}`;
    model.replaceChildren(codicon(missingKey ? "warning" : "sparkle"), m ? `${m.model}${missingKey ? " · API 키 없음" : ""}` : `모델 '${info.default}' 없음`);
    model.title = m ? `기본 모델 '${m.key}' (${m.provider}). 눌러서 설정` : "모델 설정";
    chat.setModelLabel(m ? m.model : "모델 없음", missingKey || !m);
    const cost = info.usage.cost_usd;
    const limit = info.limit_usd;
    const pct = limit > 0 ? (cost / limit) * 100 : 0;
    meter.className = `sb-item clickable${pct >= 100 ? " err" : pct >= info.warn_at_percent ? " warn" : ""}`;
    meter.textContent = limit > 0 ? `$${cost.toFixed(2)} / $${limit.toFixed(0)}` : `$${cost.toFixed(2)}`;
    meter.title = `${info.month} AI 사용: 요청 ${info.usage.requests}회 · 입력 ${info.usage.input_tokens.toLocaleString()} / 출력 ${info.usage.output_tokens.toLocaleString()} 토큰`;
    if (limit > 0 && pct >= info.warn_at_percent && !budgetWarned) {
      budgetWarned = true;
      toast.show({ kind: pct >= 100 ? "error" : "warn", message: pct >= 100 ? "이번 달 비용 한도에 도달했습니다" : `이번 달 비용이 한도의 ${Math.floor(pct)}%입니다`, detail: `$${cost.toFixed(2)} / $${limit.toFixed(0)}`, actions: [{ label: "한도 설정", run: () => settingsPage.open("cost") }] });
    }
  } catch (e) {
    model.className = "sb-item clickable err";
    model.replaceChildren(codicon("error"), "설정 오류");
    model.title = errorText(e);
    toast.error("설정 파일에 오류가 있습니다", errorText(e));
  }
}

function renderProblems(list: editor.Problem[]) {
  const errors = list.filter((p) => p.severity === "error").length;
  const warnings = list.filter((p) => p.severity === "warning").length;
  $("#sb-errors").textContent = String(errors);
  $("#sb-warnings").textContent = String(warnings);
  const badge = $("#problems-badge");
  badge.textContent = String(list.length);
  badge.classList.toggle("hidden", list.length === 0);

  const box = $("#problems");
  if (!list.length) {
    box.replaceChildren(h("div", { class: "pb-empty" }, "열린 파일에서 발견된 문제가 없습니다. 언어 서버가 알려준 오류와 경고가 여기에 표시됩니다."));
    return;
  }
  const byFile = new Map<string, editor.Problem[]>();
  for (const p of list) byFile.set(p.path, [...(byFile.get(p.path) ?? []), p]);
  box.replaceChildren(
    ...[...byFile].flatMap(([path, items]) => [
      h("div", { class: "pb-file" }, codicon("chevron-down"), fileIcon(path), basename(path), h("span", { class: "dir" }, dirname(path)), h("span", { class: "badge" }, String(items.length))),
      ...items.map((p) => {
        const icon = p.severity === "error" ? "error" : p.severity === "warning" ? "warning" : "info";
        const el = h("div", { class: "pb-item" }, codicon(icon), h("span", {}, p.message),
          h("span", { class: "where" }, `${p.source ? p.source + " " : ""}[${p.line}, ${p.col}]`));
        el.addEventListener("click", () => void editor.openFile(path, p.line, p.col));
        return el;
      }),
    ]),
  );
}

// ── 출력 패널 ──────────────────────────────────────────

on<{ source: string; text: string }>("output", ({ source, text }) => appendOutput(source, text));

// 다른 프로그램이 바꾼 파일: 트리와 탭에 반영 (저장하지 않은 탭은 건드리지 않는다)
let treeTimer: number | undefined;
on<{ paths: string[] }>("fs-change", ({ paths }) => {
  for (const p of paths) void editor.reloadIfClean(p);
  quickopen.invalidate();
  clearTimeout(treeTimer);
  treeTimer = window.setTimeout(() => {
    void tree.refresh();
    void scm.refresh();
  }, 150);
  if (paths.some((p) => p.startsWith(".lantern/memory/"))) void memory.refresh();
  if (paths.some((p) => !p.startsWith(".lantern/"))) map.refreshSoon();
  if (paths.some((p) => p.startsWith(".lantern/"))) {
    completion.settingsChanged();
    void chat.loadAgents();
    void refreshModelInfo();
    settingsPage.refreshIfOpen();
  }
});

function appendOutput(source: string, text: string) {
  const out = $("#output");
  const stickBottom = out.scrollHeight - out.scrollTop - out.clientHeight < 40;
  out.append(h("span", { class: "src" }, `[${source}] `), text.endsWith("\n") ? text : text + "\n");
  if (stickBottom) out.scrollTop = out.scrollHeight;
}

// ── 명령과 메뉴 ────────────────────────────────────────

function withView(fn: (v: NonNullable<ReturnType<typeof editor.activeView>>) => unknown) {
  return () => {
    const v = editor.activeView();
    if (v) {
      v.focus();
      fn(v);
    }
  };
}

function setTheme(theme: prefs.ThemePref) {
  prefs.set({ theme });
  settingsPage.refreshIfOpen();
}

function registerCommands() {
  commands.register([
    { id: "file.openFolder", label: "폴더 열기…", key: "Ctrl+O", run: pickProject },
    { id: "file.save", label: "저장", key: "Ctrl+S", run: () => editor.save() },
    { id: "file.saveAll", label: "모두 저장", run: () => editor.saveAll() },
    { id: "file.newFile", label: "새 파일…", key: "Ctrl+Alt+N", run: () => { showView("explorer"); tree.newFile(false); } },
    { id: "file.newFolder", label: "새 폴더…", run: () => { showView("explorer"); tree.newFile(true); } },
    { id: "file.close", label: "편집기 닫기", key: "Ctrl+W", run: () => editor.closeTab() },
    { id: "file.settings", label: "설정", key: "Ctrl+,", run: () => settingsPage.open() },
    { id: "file.settingsFile", label: "설정 파일 열기 (config.toml)", run: openConfigFile },
    { id: "file.keybindings", label: "단축키", run: () => void keybindings.load().then(keybindings.open) },
    { id: "file.exit", label: "끝내기", run: () => win.close() },
    { id: "file.trust", label: "작업 영역 신뢰 관리", run: toggleTrust },
    { id: "edit.undo", label: "실행 취소", run: withView(undo) },
    { id: "edit.redo", label: "다시 실행", run: withView(redo) },
    { id: "edit.find", label: "찾기", run: withView(openSearchPanel) },
    { id: "edit.findInFiles", label: "파일에서 찾기", key: "Ctrl+Shift+F", run: () => showView("search") },
    { id: "edit.replaceInFiles", label: "파일에서 바꾸기", key: "Ctrl+Shift+H", run: () => { showView("search"); search.focusReplace(); } },
    { id: "view.scm", label: "소스 제어", key: "Ctrl+Shift+G", run: () => showView("scm") },
    { id: "selection.all", label: "모두 선택", run: withView(selectAll) },
    { id: "view.commandPalette", label: "명령 팔레트…", key: "Ctrl+Shift+P", run: () => quickopen.open(">") },
    { id: "view.tasks", label: "작업", key: "Ctrl+Alt+T", run: () => showView("tasks") },
    { id: "view.map", label: "코드 지도", key: "Ctrl+Alt+M", run: () => map.toggle() },
    { id: "view.memory", label: "프로젝트 기억", run: () => showView("memory") },
    { id: "view.explorer", label: "탐색기", key: "Ctrl+Shift+E", run: () => showView("explorer") },
    { id: "view.search", label: "검색", run: () => showView("search") },
    { id: "view.chat", label: "Lantern 채팅", key: "Ctrl+Alt+I", run: () => showAux("chat") },
    { id: "view.inspector", label: "인스펙터", run: () => showAux("inspector") },
    { id: "view.problems", label: "문제", key: "Ctrl+Shift+M", run: () => showPanel("problems") },
    { id: "view.output", label: "출력", key: "Ctrl+Shift+U", run: () => showPanel("output") },
    { id: "view.notifications", label: "알림 보기", run: () => toast.toggleCenter(true) },
    { id: "view.toggleSidebar", label: "기본 사이드바 전환", key: "Ctrl+B", run: () => togglePart("sidebar") },
    { id: "view.togglePanel", label: "패널 전환", key: "Ctrl+J", run: () => togglePart("panel") },
    { id: "view.toggleAuxbar", label: "채팅 사이드바 전환", key: "Ctrl+Alt+B", run: () => togglePart("auxbar") },
    { id: "view.themeDark", label: "테마: 다크", run: () => setTheme("dark") },
    { id: "view.themeLight", label: "테마: 라이트", run: () => setTheme("light") },
    { id: "view.themeSystem", label: "테마: 시스템 설정 따르기", run: () => setTheme("system") },
    { id: "view.zoomIn", label: "편집기 글꼴 키우기", key: "Ctrl+=", run: () => prefs.set({ fontSize: Math.min(24, prefs.get().fontSize + 1) }) },
    { id: "view.zoomOut", label: "편집기 글꼴 줄이기", key: "Ctrl+-", run: () => prefs.set({ fontSize: Math.max(10, prefs.get().fontSize - 1) }) },
    { id: "go.file", label: "파일로 이동…", key: "Ctrl+P", run: () => (project ? quickopen.open() : toast.info("먼저 폴더를 여세요")) },
    { id: "go.line", label: "줄/열로 이동…", key: "Ctrl+G", run: withView(gotoLine) },
    { id: "terminal.toggle", label: "터미널 전환", key: "Ctrl+`", run: () => (layout.panel && $("#terminal").classList.contains("active") ? setPart("panel", false) : showPanel("terminal")) },
    { id: "terminal.new", label: "새 터미널", run: async () => { await terminal.restart(); showPanel("terminal"); } },
    { id: "chat.new", label: "새 작업", run: () => { showAux("chat"); void chat.reset(); } },
    { id: "map.footprint", label: "지금 작업의 발자취를 지도에서 보기", run: showFootprintOnMap },
    { id: "map.memory", label: "기억 지도 보기", run: () => void map.showMemory() },
    { id: "chat.preview", label: "맥락 미리보기", run: () => { showAux("inspector"); $("#preview-query").focus(); } },
    { id: "chat.models", label: "모델 설정", run: () => settingsPage.open("models") },
    { id: "help.onboarding", label: "Lantern 시작하기", run: () => onboarding.open() },
    { id: "help.shortcuts", label: "모든 명령 보기", run: () => quickopen.open(">") },
    { id: "help.checkUpdate", label: "업데이트 확인…", run: () => void update.check(true) },
    { id: "help.reportIssue", label: "문제 보고…", run: () => report.open() },
    { id: "help.openLogs", label: "로그 폴더 열기", run: () => report.openLogDir() },
    { id: "help.about", label: "Lantern 정보", run: async () => message(`Lantern ${await getVersion()}\n내 프로젝트를 이해하는, 가볍고 자유로운 AI IDE\n\n아이콘: VS Code Codicons (CC BY 4.0)\n글꼴: Pretendard (OFL 1.1)`, { title: "Lantern 정보" }) },
  ]);
  commands.installMenubar([
    { title: "파일", items: ["file.newFile", "file.newFolder", "file.openFolder", "-", "file.save", "file.saveAll", "-", "file.settings", "file.keybindings", "file.settingsFile", "file.trust", "-", "file.close", "file.exit"] },
    { title: "편집", items: ["edit.undo", "edit.redo", "-", "edit.find", "edit.findInFiles", "edit.replaceInFiles"] },
    { title: "선택", items: ["selection.all"] },
    { title: "보기", items: ["view.commandPalette", "-", "view.tasks", "view.map", "view.memory", "-", "view.explorer", "view.search", "view.scm", "view.chat", "view.inspector", "-", "view.problems", "view.output", "terminal.toggle", "-", "view.toggleSidebar", "view.togglePanel", "view.toggleAuxbar", "-", "view.themeDark", "view.themeLight", "view.themeSystem"] },
    { title: "이동", items: ["go.file", "go.line"] },
    { title: "터미널", items: ["terminal.new", "terminal.toggle"] },
    { title: "AI", items: ["view.chat", "chat.new", "-", "map.footprint", "map.memory", "-", "chat.preview", "view.inspector", "-", "chat.models"] },
    { title: "도움말", items: ["help.onboarding", "help.shortcuts", "-", "help.reportIssue", "help.openLogs", "-", "help.checkUpdate", "help.about"] },
  ]);
  commands.installKeybindings();
  void keybindings.load();
  report.init();
  update.schedule(5000);
}

// ── 창 ─────────────────────────────────────────────────

async function initWindow() {
  $("#win-min").addEventListener("click", () => void win.minimize());
  $("#win-max").addEventListener("click", () => void win.toggleMaximize());
  $("#win-close").addEventListener("click", () => void win.close());
  const syncMax = async () => {
    const max = await win.isMaximized();
    $("#win-max").replaceChildren(codicon(max ? "chrome-restore" : "chrome-maximize"));
    $("#win-max").title = max ? "이전 크기로 복원" : "최대화";
  };
  await win.onResized(() => void syncMax());
  void syncMax();
  await win.onCloseRequested(async (e) => {
    if (editor.hasDirty() && !(await ask("저장하지 않은 파일이 있습니다. 그래도 닫을까요?", { title: "Lantern", kind: "warning", okLabel: "닫기", cancelLabel: "취소" }))) {
      e.preventDefault();
      return;
    }
    // 작업 기록의 밀린 저장을 끝내고 닫는다
    await chat.flushAll();
  });
}

// ── 시작 ───────────────────────────────────────────────

function init() {
  mountHexes();
  toast.init();
  registerCommands();
  initSashes();
  trackAuxWidth();
  void initWindow();
  quickopen.init();
  search.init();
  tree.init();
  scm.init({
    onCount: (n) => {
      const b = $("#scm-badge");
      b.textContent = n > 99 ? "99+" : String(n);
      b.classList.toggle("hidden", n === 0);
    },
    // 상태 표시줄 브랜치: 소스 제어에서 고른 저장소 기준 (여러 저장소면 "이름: 브랜치")
    onBranch: (text, title) => {
      const el = $("#sb-branch");
      el.classList.toggle("hidden", !text);
      el.querySelector("span")!.textContent = text ?? "";
      el.title = title ? `${title} (눌러서 소스 제어 열기)` : "현재 git 브랜치 (눌러서 소스 제어 열기)";
    },
  });
  tree.onPathChange((ev) => {
    if (ev.kind === "rename") void editor.pathRenamed(ev.from, ev.to!);
    else void editor.pathDeleted(ev.from);
    quickopen.invalidate();
  });
  inspector.init();
  settingsPage.init({
    // 제한 모드에서는 프로젝트 설정을 읽지 않으므로 프로젝트 범위도 쓰지 않는다.
    hasProject: () => !!project && project.trusted,
    openProjectConfig: () => void openConfigFile(),
    onChanged: () => {
      completion.settingsChanged();
      void refreshModelInfo();
      void chat.loadAgents();
    },
    openPreview: () => commands.run("chat.preview"),
  });
  onboarding.init({
    pickProject,
    hasProject: () => !!project,
    showChat: () => showAux("chat"),
    onModelChanged: () => {
      void refreshModelInfo();
      settingsPage.refreshIfOpen();
    },
  });

  quickopen.onOpen((p) => void editor.openFile(p));
  tree.onOpen((p) => void editor.openFile(p));
  search.onOpen((p, line, col) => void editor.openFile(p, line, col));
  search.onChanged(() => {
    void scm.refresh();
    quickopen.invalidate();
  });
  editor.onActiveChange((p) => {
    // 파일을 열면 가운데는 편집기로
    if (p && map.isVisible()) map.hide();
    void tree.reveal(p);
    $("#st-lang").textContent = p ? editor.langName(p) : "";
    $("#st-enc").classList.toggle("hidden", !p);
    $("#focus-name").textContent = p ? basename(p) : "현재 파일 없음";
    if (!p) $("#st-pos").textContent = "";
    updateBackdrop();
  });
  editor.onCursor((line, col) => ($("#st-pos").textContent = `줄 ${line}, 열 ${col}`));
  editor.onProblems(renderProblems);
  chat.init({
    onContext: (used, budget) => {
      const k = (n: number) => (n >= 1000 ? `${(n / 1000).toFixed(1)}k` : String(n));
      lastContext = ` · ${k(used)}/${k(budget)}`;
      renderContextStatus(`맥락 ${indexedFiles}${lastContext}`, `마지막 질문에 맥락 예산 ${budget.toLocaleString()} 토큰 중 약 ${used.toLocaleString()} 토큰을 썼습니다. 눌러서 맥락 미리보기`);
    },
    onFilesChanged: (paths) => {
      for (const p of paths) void editor.reloadIfClean(p);
      void tree.refresh();
      quickopen.invalidate();
      if (!layout.auxbar) toast.show({ kind: "success", message: `Lantern이 파일 ${new Set(paths).size}개를 바꿨습니다`, actions: [{ label: "작업 보기", run: () => showAux("chat") }] });
      map.refreshSoon();
    },
    onUsage: () => void refreshModelInfo(),
    showInspector: (id) => {
      showAux("inspector");
      inspector.show(id);
    },
    openSettings: (sec) => settingsPage.open(sec),
    onRunning: (v) => {
      agentRunning = v;
      document.querySelector("#ctx-status .hex")?.classList.toggle("working", v || indexing);
    },
    onTasks: () => {
      chat.renderTaskList();
      const { approval } = chat.taskCount();
      const b = $("#task-badge");
      b.textContent = String(approval);
      b.classList.toggle("hidden", approval === 0);
      b.title = approval ? `승인을 기다리는 작업 ${approval}개` : "";
    },
    onFootprint: (fp, title, running) => {
      map.setFootprint(fp, title);
      map.setWorking(running);
    },
    showOnMap: showFootprintOnMap,
    showImpact: (i) => map.showImpact(i),
  });
  map.init({
    openFile: (p, line) => {
      map.hide();
      void editor.openFile(p, line);
    },
    onVisibility: (v) => {
      syncCanvasSwitch(v);
      updateBackdrop();
    },
  });
  memory.init({
    openFile: (p, line) => void editor.openFile(p, line),
    focusNode: (id, label) => void map.focus(id, label),
    showMap: () => void map.showMemory(),
    readOnly: () => !project?.trusted,
  });
  for (const b of document.querySelectorAll<HTMLElement>("#canvas-switch [data-canvas]")) {
    b.addEventListener("click", () => (b.dataset.canvas === "map" ? map.show() : map.hide()));
  }
  $("#btn-new-task").addEventListener("click", () => commands.run("chat.new"));

  $("#toggle-sidebar").addEventListener("click", () => togglePart("sidebar"));
  $("#toggle-panel").addEventListener("click", () => togglePart("panel"));
  $("#toggle-auxbar").addEventListener("click", () => togglePart("auxbar"));
  for (const b of document.querySelectorAll<HTMLElement>(".ab-item[data-view]")) b.addEventListener("click", () => showView(b.dataset.view!, true));
  $("#ab-settings").addEventListener("click", () => settingsPage.open());
  for (const b of document.querySelectorAll<HTMLElement>(".panel-tabs button[data-panel]")) b.addEventListener("click", () => showPanel(b.dataset.panel!));
  $("#btn-hide-panel").addEventListener("click", () => setPart("panel", false));
  $("#btn-clear-output").addEventListener("click", () => $("#output").replaceChildren());
  for (const b of document.querySelectorAll<HTMLElement>(".aux-tabs button")) b.addEventListener("click", () => showAux(b.dataset.ai as "chat" | "inspector"));
  $("#btn-new-chat").addEventListener("click", () => commands.run("chat.new"));
  $("#btn-close-aux").addEventListener("click", () => setPart("auxbar", false));
  $("#command-center").addEventListener("click", () => (project ? quickopen.open() : quickopen.open(">")));

  $("#btn-refresh-tree").addEventListener("click", () => {
    quickopen.invalidate();
    void tree.refresh();
  });
  $("#btn-collapse-tree").addEventListener("click", () => tree.collapseAll());
  $("#btn-new-file").addEventListener("click", () => tree.newFile(false));
  $("#btn-new-folder").addEventListener("click", () => tree.newFile(true));
  const toggleProjectSection = () => {
    const hidden = $("#tree").classList.toggle("hidden");
    $("#project-header").querySelector(".codicon")!.className = `codicon codicon-${hidden ? "chevron-right" : "chevron-down"}`;
  };
  $("#project-header").addEventListener("click", toggleProjectSection);
  $("#project-header").addEventListener("keydown", (e) => (e.key === "Enter" || e.key === " ") && toggleProjectSection());
  $("#btn-open-side").addEventListener("click", () => void pickProject());

  $("#wl-open").addEventListener("click", () => void pickProject());
  $("#wl-onboard").addEventListener("click", () => onboarding.open());
  $("#wl-config").addEventListener("click", () => settingsPage.open());

  $("#ctx-status").addEventListener("click", () => commands.run("chat.preview"));
  $("#sb-restricted").addEventListener("click", () => void toggleTrust());
  $("#sb-problems").addEventListener("click", () => showPanel("problems"));
  $("#sb-branch").addEventListener("click", () => showView("scm"));
  $("#st-model").addEventListener("click", () => settingsPage.open("models"));
  $("#usage-meter").addEventListener("click", () => settingsPage.open("cost"));

  window.addEventListener("keyup", (e) => {
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "s" && editor.activePath()?.startsWith(".lantern/")) {
      void chat.loadAgents();
      void refreshModelInfo();
      settingsPage.refreshIfOpen();
    }
  });
  window.addEventListener("focus", () => {
    if (project) {
      void refreshModelInfo();
      // 터미널이나 다른 도구에서 커밋·브랜치 전환을 했을 수 있다
      void scm.refresh({ repos: true });
    }
  });

  const saved = { ...layout };
  showView(activeView);
  (["sidebar", "panel", "auxbar"] as Part[]).forEach((p) => setPart(p, saved[p]));
  renderContextStatus("", "");
  renderProblems([]);
  renderRecent();
  updateBackdrop();
  void chat.loadAgents();
  void refreshModelInfo();
  void api.startupPath().then(async (arg) => {
    const target = arg ?? recent()[0];
    if (target) await openProject(target);
    if (!onboarding.done()) onboarding.open();
  });
}

init();
