// 코드 지도 (지식그래프). 편집기 자리를 번갈아 쓰는 캔버스.
// - 전체 구조: 파일 단위, 모듈(상위 폴더)끼리 모인다
// - 주변: 파일·심볼 하나를 중심으로 호출자·호출하는 것·함께 바뀌던 파일
// - 겹쳐 보기: 에이전트 발자취(읽음·보낸 맥락·수정), 변경 영향 반경, 프로젝트 기억
// 배치는 d3-force, 그리기는 캔버스. 색은 CSS 변수에서 읽어 테마를 따른다.
import { invoke } from "@tauri-apps/api/core";
import { forceCenter, forceCollide, forceLink, forceManyBody, forceSimulation, forceX, forceY, type Simulation, type SimulationLinkDatum, type SimulationNodeDatum } from "d3-force";
import { errorText } from "./api";
import { $, basename, dirname, h } from "./dom";
import { codicon } from "./icons";
import { nodeKeys } from "./lib/nodeKeys";
import * as prefs from "./prefs";
import { t } from "./i18n";

// ── 백엔드 모양 ───────────────────────────────────────────

export type NodeKind = "dir" | "file" | "symbol" | "memory";
export type EdgeKind = "ref" | "calls" | "defines" | "cochange" | "mentions";
export interface GNode { id: string; kind: NodeKind; label: string; path?: string; line?: number; group: string; weight: number; detail?: string; depth?: number; test?: boolean }
export interface GEdge { source: string; target: string; kind: EdgeKind; weight: number; uncertain?: boolean }
export interface Graph { nodes: GNode[]; edges: GEdge[]; truncated: boolean }
export interface Impact {
  path: string; touched: string[]; callers: number; callers2: number; files: number; modules: number;
  cochanged: [string, number][]; tests: string[]; ambiguous: string[]; unique: boolean;
  /** 엔진이 이 언어를 분석함. 아니면 0은 '없음'이 아니라 '모름' */
  supported: boolean;
  /** 프레임워크가 부른다는 표시 (@GetMapping 등). 호출자·테스트에 안 잡힌다 */
  framework: string[];
  /** 테스트 코드까지 센 옛 위험도 (CLI 출력용). 화면에는 code_risk를 보인다 */
  risk: "low" | "medium" | "high";
  /** 테스트 코드를 뺀 호출자·파일·위험도. 테스트는 tests로 따로 보인다 */
  code_callers: number; code_callers2: number; code_files: number; code_risk: "low" | "medium" | "high";
  /** diff를 보면 주석·공백만 바뀜 (승인 카드에서 diff를 넘겼을 때만) */
  comment_only: boolean;
  graph: Graph;
}
export interface MemoryNote { id: string; topic: string; file: string; line: number; text: string; links: string[] }
/** 지도 카드의 수 (프로젝트 전체 기준). callers가 null이면 이름이 흔해 셀 수 없다 */
interface NodeStats {
  callers: number | null; test_callers: number | null; defs: number;
  callees: number; callees_ambiguous: number; callees_common: number;
  refs_in: number; refs_out: number; cochange: number;
}

/** 에이전트가 한 작업에서 남긴 흔적 */
export interface Footprint {
  read: Set<string>;
  context: Map<string, Set<string>>; // 경로 → 보낸 심볼 이름
  edited: Set<string>;
  /** 수정한 심볼 ("경로#이름"). 파일 안 다른 심볼까지 수정으로 칠하지 않으려고 따로 둔다 */
  editedSymbols: Set<string>;
  memory: Set<string>;
  last: string | null;
}

export function emptyFootprint(): Footprint {
  return { read: new Set(), context: new Map(), edited: new Set(), editedSymbols: new Set(), memory: new Set(), last: null };
}

// ── 상태 ─────────────────────────────────────────────────

interface N extends SimulationNodeDatum, GNode { r: number; mark?: "context" | "read" | "edited" | "impact0" | "impact1" | "impact2" | "test"; notes?: string[]; text?: string }
interface L extends SimulationLinkDatum<N> { kind: EdgeKind; weight: number; uncertain?: boolean }

type Mode = "overview" | "focus" | "impact" | "memory";
const overlays = { footprint: true, memory: false };

let mode: Mode = "overview";
let focusLabel = "";
/** 주변 지도의 중심. 폴더를 펼치면 첫 노드가 중심이 아니라서 따로 둔다 (다시 그릴 때 같은 지도를 받으려고) */
let focusCenter = "";
let focusName = "";
let nodes: N[] = [];
let links: L[] = [];
let byId = new Map<string, N>();
let sim: Simulation<N, L> | null = null;
let view = { x: 0, y: 0, k: 1 };
/** 사용자가 지도를 옮기거나 확대했다: 다시 배치해도 화면을 맞추지 않는다 */
let cameraMoved = false;
let fitPending = false;
let selected: N | null = null;
let hovered: N | null = null;
let statsCache = new Map<string, NodeStats | null | undefined>();
let lastStats: { id: string; stats: NodeStats | null } | null = null;
let footprint: Footprint | null = null;
let footprintTitle = "";
let impact: Impact | null = null;
let memoryNotes: MemoryNote[] = [];
let truncated = false;
let visible = false;
let loaded = false;
let raf = 0;
let pulse = 0;
let colors: Record<string, string> = {};
let openFile: (path: string, line?: number) => void = () => {};
let onVisibility: (v: boolean) => void = () => {};

const canvas = () => $<HTMLCanvasElement>("#map-canvas");
/** 운영체제의 움직임 줄이기 설정 */
const calm = () => matchMedia("(prefers-reduced-motion: reduce)").matches;

// 멈춤 진단: localStorage "lantern.trace" = "1"이면 지도가 지금 어느 단계인지 남긴다 (멈춘 뒤 다시 켜서 읽는다)
const readTrace = () => {
  try {
    return localStorage.getItem("lantern.trace") === "1";
  } catch {
    return false;
  }
};
let TRACE = readTrace();
setInterval(() => (TRACE = readTrace()), 1000);
function trace(step: string) {
  if (!TRACE) return;
  // 콘솔로 바로 내보낸다 (디버깅 도구가 실시간으로 받는다)
  console.debug(`[lantern-trace] ${step}`);
}

// ── 색 ──────────────────────────────────────────────────

function readColors() {
  const cs = getComputedStyle(document.documentElement);
  const v = (n: string) => cs.getPropertyValue(n).trim();
  colors = {
    bg: v("--editor-bg"), fg: v("--fg"), strong: v("--fg-strong"), muted: v("--muted"), dim: v("--dim"),
    border: v("--border-strong"), accent: v("--accent"), focus: v("--focus"), context: v("--context"),
    error: v("--error"), warning: v("--warning"), ok: v("--ok"), link: v("--link"),
  };
}

const HUES = [228, 190, 152, 34, 276, 350, 96, 316, 58, 205, 12, 170];
const groupHue = new Map<string, number>();
function groupColor(g: string, alpha = 1): string {
  if (!groupHue.has(g)) groupHue.set(g, HUES[groupHue.size % HUES.length]);
  const hue = groupHue.get(g)!;
  return prefs.isDark() ? `hsla(${hue} 38% 62% / ${alpha})` : `hsla(${hue} 45% 42% / ${alpha})`;
}

function withAlpha(color: string, a: number): string {
  // #rrggbb → rgba
  const m = color.match(/^#([0-9a-f]{6})$/i);
  if (!m) return color;
  const n = parseInt(m[1], 16);
  return `rgba(${(n >> 16) & 255}, ${(n >> 8) & 255}, ${n & 255}, ${a})`;
}

// ── 그래프 적재 ──────────────────────────────────────────

function radius(n: GNode): number {
  if (n.kind === "memory") return 7;
  if (n.kind === "dir") return Math.min(30, 9 + Math.sqrt(n.weight) * 2.2);
  if (n.kind === "file") return Math.min(18, 4 + Math.sqrt(n.weight) * 1.5);
  return Math.min(14, 5 + Math.sqrt(n.weight) * 1.6);
}

function setGraph(g: Graph, keepPositions = false) {
  trace(`setGraph n=${g.nodes.length} e=${g.edges.length} keep=${keepPositions}`);
  statsCache = new Map();
  const oldKeys = keepPositions ? nodeKeys(nodes) : [];
  const old = new Map<string, N>(oldKeys.map((k, i) => [k, nodes[i]]));
  // 노드 객체가 새로 만들어진다: 고른 노드도 새 것으로 잇는다 (아니면 고리와 이름표가 사라진다)
  const was = selected ? oldKeys[nodes.indexOf(selected)] : undefined;
  const keys = nodeKeys(g.nodes);
  nodes = g.nodes.map((n, i) => {
    const prev = old.get(keys[i]);
    return { ...n, r: radius(n), x: prev?.x, y: prev?.y };
  });
  byId = new Map(nodes.map((n) => [n.id, n]));
  // 같은 이름의 파일은 폴더까지 보인다 (binding/json.go, render/json.go)
  const sameName = new Map<string, number>();
  for (const n of nodes) if (n.kind === "file") sameName.set(n.label, (sameName.get(n.label) ?? 0) + 1);
  for (const n of nodes) if (n.kind === "file" && sameName.get(n.label)! > 1) n.text = n.path?.split("/").slice(-2).join("/");
  links = g.edges.filter((e) => byId.has(e.source) && byId.has(e.target)).map((e) => ({ source: e.source, target: e.target, kind: e.kind, weight: e.weight, uncertain: e.uncertain }) as unknown as L);
  truncated = g.truncated;
  selected = was ? nodes[keys.indexOf(was)] ?? null : null;
  hovered = null;
  applyMarks();
  trace("setGraph: layout");
  layout(keepPositions);
  trace("setGraph: chrome");
  renderChrome();
  trace("setGraph: done");
}

/** 메모 노드를 지금 그래프에 합친다 (전체 구조에서는 심볼 연결을 파일로 올린다) */
function mergeMemory(g: Graph, memory: Graph): Graph {
  const ids = new Set(g.nodes.map((n) => n.id));
  const byMem = new Map(memory.nodes.map((n) => [n.id, n]));
  const nodesOut = [...g.nodes];
  const edgesOut = [...g.edges];
  for (const n of memory.nodes) if (n.kind === "memory") nodesOut.push(n);
  for (const e of memory.edges) {
    let target = e.target;
    if (!ids.has(target)) {
      const path = byMem.get(target)?.path ?? "";
      const dir = g.nodes.filter((n) => n.kind === "dir" && (n.path === "" ? !path.includes("/") : path.startsWith(`${n.path}/`))).sort((a, b) => (b.path?.length ?? 0) - (a.path?.length ?? 0))[0];
      if (path && ids.has(`f:${path}`)) target = `f:${path}`;
      else if (dir) target = dir.id;
      else continue;
    }
    edgesOut.push({ ...e, target });
  }
  return { nodes: nodesOut, edges: edgesOut, truncated: g.truncated };
}

let baseGraph: Graph | null = null;
/** 지도를 바꿀 때마다 늘린다. 늦게 끝난 불러오기가 그 사이 바뀐 화면을 덮지 않게 한다 */
let loadSeq = 0;

async function load(fetcher: () => Promise<Graph>, nextMode: Mode, label = "") {
  const my = ++loadSeq;
  // 미리 보기 중에 사용자가 직접 다른 지도로 갔다: 결정해도 되돌리지 않는다
  if (preview) {
    preview.saved = null;
    preview.opened = true;
  }
  setStatus(t("지도를 그리는 중…"));
  try {
    let g = await fetcher();
    if (my !== loadSeq) return;
    baseGraph = g;
    if (overlays.memory && nextMode !== "memory") g = await withMemory(g);
    if (my !== loadSeq) return;
    mode = nextMode;
    focusLabel = label;
    view = { x: 0, y: 0, k: 1 };
    cameraMoved = false;
    setGraph(g);
    loaded = true;
    setStatus(g.nodes.length ? "" : t("표시할 코드가 없습니다. 인덱싱이 끝났는지 확인하세요."));
  } catch (e) {
    if (my === loadSeq) setStatus(errorText(e));
  }
}

async function withMemory(g: Graph): Promise<Graph> {
  const [notes, mg] = await invoke<[MemoryNote[], Graph]>("graph_memory");
  memoryNotes = notes;
  return mergeMemory(g, mg);
}

export function showOverview() {
  impact = null;
  return load(() => invoke<Graph>("graph_overview", { maxNodes: 400 }), "overview");
}

/** 주변 지도를 받는다. 다시 인덱싱해 심볼 id가 바뀌었으면 같은 이름을 찾아 잇는다 (같은 파일의 것을 먼저) */
async function fetchFocus(): Promise<Graph> {
  const g = await invoke<Graph>("graph_neighborhood", { center: focusCenter, limit: 30 });
  if (g.nodes.length || !focusCenter.startsWith("s:")) return g;
  const path = byId.get(focusCenter)?.path;
  const same = (await invoke<GNode[]>("graph_find", { query: focusName })).filter((n) => n.kind === "symbol" && n.label === focusName);
  const next = same.find((n) => n.path === path) ?? same[0];
  if (!next) return g;
  focusCenter = next.id;
  return invoke<Graph>("graph_neighborhood", { center: next.id, limit: 30 });
}

/** `f:경로`, `s:id`, 또는 이름 */
export function focus(center: string, label?: string) {
  impact = null;
  show();
  focusCenter = center;
  focusName = label ?? "";
  return load(fetchFocus, "focus", label ?? center.replace(/^[fs]:/, ""));
}

function applyImpact(i: Impact) {
  loadSeq++;
  impact = i;
  mode = "impact";
  focusLabel = `${basename(i.path)}${i.touched.length ? ` · ${i.touched.join(", ")}` : ""}`;
  view = { x: 0, y: 0, k: 1 };
  cameraMoved = false;
  baseGraph = i.graph;
  setGraph(i.graph);
  loaded = true;
  setStatus(i.graph.nodes.length > 1 ? "" : t("이 변경에 닿는 다른 코드를 찾지 못했습니다."));
}

export function showImpact(i: Impact) {
  // 승인 카드의 '지도에서 보기': 기다리는 동안의 미리 보기와 같아서, 결정하면 돌아온다
  if (preview?.impact === i && (preview.saved || !preview.opened)) return show();
  if (preview) {
    preview.saved = null;
    preview.opened = true;
  }
  show();
  applyImpact(i);
}

// ── 승인 대기 미리 보기 ─────────────────────────────────────
// 수정 승인을 기다리는 동안 지도는 그 수정의 영향 반경을 보여 준다. 결정하면 보던 화면으로,
// 배치와 카메라까지 그대로 돌아간다 (다시 불러오면 사용자가 옮겨 둔 곳을 잃는다).

interface Saved {
  mode: Mode; focusLabel: string; impact: Impact | null; baseGraph: Graph | null;
  nodes: N[]; links: L[]; byId: Map<string, N>; sim: Simulation<N, L> | null;
  view: typeof view; cameraMoved: boolean; selected: N | null; truncated: boolean; loaded: boolean;
  /** 미리 보는 사이 파일이 바뀌었다: 돌아간 뒤 새로 그린다 */
  stale: boolean;
}

/** opened: 한 번 띄웠다 (사용자가 닫으면 다시 띄우지 않는다). saved: 돌아갈 화면 (사용자가 직접 옮겼으면 null) */
let preview: { impact: Impact; opened: boolean; saved: Saved | null } | null = null;

function openPreview() {
  if (!preview) return;
  preview.saved ??= { mode, focusLabel, impact, baseGraph, nodes, links, byId, sim, view, cameraMoved, selected, truncated, loaded, stale: false };
  preview.opened = true;
  applyImpact(preview.impact);
}

function restore(s: Saved) {
  loadSeq++;
  sim?.stop();
  ({ mode, focusLabel, impact, baseGraph, nodes, links, byId, sim, view, cameraMoved, selected, truncated, loaded } = s);
  // 노드를 끄는 중에 미리 보기가 끼어들었으면 놓는 동작이 이 sim에 닿지 않았다: 끝없이 다시 그리지 않게
  sim?.alphaTarget(0);
  hovered = null;
  applyMarks();
  renderChrome();
  setStatus("");
  // 배치가 끝나기 전에 미리 보기가 끼어들었으면 마저 한다
  if (sim && sim.alpha() > sim.alphaMin()) sim.restart();
  kick();
  if (visible && !nodes.length) void showOverview();
  else if (s.stale) refreshSoon();
}

/** 수정 승인을 기다린다. 지도가 숨어 있으면 띄울 때 보여 준다 */
export function previewImpact(i: Impact) {
  if (preview?.impact === i) return;
  preview = { impact: i, opened: false, saved: preview?.saved ?? null };
  if (visible) openPreview();
}

/** 결정했다 (적용·거절·중단) */
export function endPreview() {
  const s = preview?.saved;
  preview = null;
  if (s) restore(s);
}

/** 사용자가 지도에서 직접 돌아왔다. 승인은 아직 기다리는 중이어도 다시 띄우지 않는다 */
function leavePreview() {
  const s = preview?.saved;
  if (!s) return;
  preview!.saved = null;
  restore(s);
}

export async function showMemory() {
  show();
  impact = null;
  await load(async () => {
    const [notes, g] = await invoke<[MemoryNote[], Graph]>("graph_memory");
    memoryNotes = notes;
    return g;
  }, "memory");
  if (!memoryNotes.length) setStatus(t("아직 프로젝트 기억이 없습니다. 에이전트가 규칙·결정을 기억하거나 '기억' 보기에서 직접 추가할 수 있습니다."));
}

// ── 겹쳐 보기 표시 ─────────────────────────────────────────

function applyMarks() {
  const tests = new Set(impact?.tests ?? []);
  const notesBy = new Map<string, string[]>();
  for (const m of memoryNotes) for (const l of m.links) notesBy.set(l, [...(notesBy.get(l) ?? []), m.text]);
  for (const n of nodes) {
    n.mark = undefined;
    n.notes = notesBy.get(n.id);
    if (mode === "impact") {
      if (n.depth === 0) n.mark = "impact0";
      else if (n.depth === 1) n.mark = "impact1";
      else if (n.depth === 2) n.mark = "impact2";
      if (n.path && tests.has(n.path) && n.depth !== 0) n.mark = "test";
      continue;
    }
    if (mode === "memory" || !overlays.footprint || !footprint || n.path === undefined) continue;
    if (n.kind === "dir") {
      const inside = (p: string) => (n.path === "" ? !p.includes("/") : p.startsWith(`${n.path}/`));
      if ([...footprint.edited].some(inside)) n.mark = "edited";
      else if ([...footprint.context.keys()].some(inside)) n.mark = "context";
      else if ([...footprint.read].some(inside)) n.mark = "read";
      continue;
    }
    const ctx = footprint.context.get(n.path);
    if (n.kind === "symbol") {
      // 심볼은 실제로 바뀐 것과 맥락으로 보낸 것만
      if (footprint.editedSymbols.has(`${n.path}#${n.label}`)) n.mark = "edited";
      else if (ctx?.has(n.label)) n.mark = "context";
      continue;
    }
    if (footprint.edited.has(n.path)) n.mark = "edited";
    else if (ctx) n.mark = "context";
    else if (footprint.read.has(n.path)) n.mark = "read";
  }
}

function footprintSize(): number {
  if (!footprint) return 0;
  return footprint.read.size + footprint.context.size + footprint.edited.size;
}

/** 작업을 바꾸거나 에이전트가 움직일 때 */
export function setFootprint(fp: Footprint | null, title = "") {
  footprint = fp;
  footprintTitle = title;
  if (!loaded) return;
  applyMarks();
  renderChrome();
  kick();
}

// ── 배치 ─────────────────────────────────────────────────

/** keep: 파일이 바뀌어 다시 그린다. 있던 노드는 제자리에 두고 새 노드만 자리를 찾는다 (보던 곳이 흔들리지 않게) */
function layout(keep = false) {
  const unfinished = sim && sim.alpha() > sim.alphaMin() ? sim.alpha() : 0;
  sim?.stop();
  const el = canvas();
  const w = el.clientWidth || 800;
  const hgt = el.clientHeight || 600;
  const fresh = new Set(nodes.filter((n) => n.x === undefined));
  const groups = [...new Set(nodes.map((n) => n.group))];
  const ring = Math.min(w, hgt) * (nodes.length < 40 ? 0.18 : 0.3);
  // 연결이 없는 노드는 멀리 밀려나지 않게 가운데 쪽으로 더 당긴다
  const degree = new Map<string, number>();
  for (const l of links) {
    const a = typeof l.source === "string" ? l.source : (l.source as N).id;
    const b = typeof l.target === "string" ? l.target : (l.target as N).id;
    degree.set(a, (degree.get(a) ?? 0) + 1);
    degree.set(b, (degree.get(b) ?? 0) + 1);
  }
  const pull = (d: N, base: number) => (degree.get(d.id) ? base : Math.max(base, 0.12));
  const anchor = new Map(groups.map((g, i) => {
    const a = (i / Math.max(1, groups.length)) * Math.PI * 2;
    return [g, groups.length > 1 ? { x: Math.cos(a) * ring, y: Math.sin(a) * ring } : { x: 0, y: 0 }];
  }));
  // 대부분 새 노드면(인덱싱 중에 그린 지도) 처음부터 배치한다
  const gentle = keep && fresh.size <= nodes.length / 2;
  if (gentle) {
    // 새 노드는 연결된 노드 곁에서 시작한다
    for (const n of fresh) {
      const near = links.flatMap((l) => (l.source === n.id ? [byId.get(l.target as string)] : l.target === n.id ? [byId.get(l.source as string)] : [])).find((o) => o && o.x !== undefined);
      const from = near ?? nodes.find((o) => o.group === n.group && o.x !== undefined);
      const a = Math.random() * Math.PI * 2;
      n.x = (from?.x ?? anchor.get(n.group)!.x) + Math.cos(a) * 40;
      n.y = (from?.y ?? anchor.get(n.group)!.y) + Math.sin(a) * 40;
    }
  }
  const dist: Record<EdgeKind, number> = { defines: 36, calls: 70, ref: 80, cochange: 110, mentions: 70 };
  // 영향 지도: 테스트는 바깥쪽으로 밀어 실제 호출자가 바뀌는 곳 가까이에 모이게 한다
  const reach = (l: L) => dist[l.kind] * ((l.source as N).mark === "test" || (l.target as N).mark === "test" ? 1.8 : 1);
  const clustered = mode === "overview";
  sim = forceSimulation<N, L>(nodes)
    .force("link", forceLink<N, L>(links).id((d) => d.id).distance(reach).strength((l) => (l.kind === "cochange" ? 0.05 : Math.min(0.6, 0.15 + Math.log1p(l.weight) * 0.08))))
    .force("charge", forceManyBody<N>().strength(nodes.length > 200 ? -60 : -160).distanceMax(400))
    .force("collide", forceCollide<N>().radius((d) => (d.kind === "dir" ? d.r * 1.15 + 8 : d.r + 3)).strength(0.9))
    .force("center", forceCenter(0, 0))
    .force("x", forceX<N>((d) => (clustered ? anchor.get(d.group)!.x : 0)).strength((d) => pull(d, clustered ? 0.06 : 0.02)))
    .force("y", forceY<N>((d) => (clustered ? anchor.get(d.group)!.y : 0)).strength((d) => pull(d, clustered ? 0.06 : 0.02)))
    .alphaDecay(0.035)
    .on("tick", kick)
    .on("end", () => {
      if (!cameraMoved) fit(false);
    });
  // 중심 노드는 가운데에 고정
  if (mode !== "overview" && nodes[0] && mode !== "memory") {
    nodes[0].fx = 0;
    nodes[0].fy = 0;
  }
  trace("layout: tick");
  if (gentle && unfinished) sim.alpha(unfinished); // 아직 배치 중이었다: 하던 만큼만 마저 한다
  else if (gentle) {
    sim.stop();
    const pinned = nodes.filter((n) => !fresh.has(n) && n.fx == null);
    for (const n of pinned) {
      n.fx = n.x;
      n.fy = n.y;
    }
    if (fresh.size) sim.tick(60);
    for (const n of pinned) {
      n.fx = null;
      n.fy = null;
    }
    // 다 놓였다: 끌어 움직일 때 처음부터 다시 흔들지 않게
    sim.alpha(sim.alphaMin());
  } else {
    sim.tick(Math.min(120, 30 + nodes.length / 4)); // 처음 몇 번은 그리지 않고 계산만 (첫 화면이 덜 흔들리게)
    if (calm()) {
      // 움직임 줄이기: 노드가 떠다니는 모습 없이 끝난 배치를 바로 보인다
      sim.stop();
      while (sim.alpha() > sim.alphaMin()) sim.tick();
    }
  }
  trace(`layout: fit ${nodes.map((n) => `${Math.round(n.x ?? NaN)},${Math.round(n.y ?? NaN)}`).join(" ")}`);
  if (cameraMoved) kick();
  else fit(false);
}

function bounds() {
  let x0 = Infinity, y0 = Infinity, x1 = -Infinity, y1 = -Infinity;
  for (const n of nodes) {
    x0 = Math.min(x0, n.x! - n.r); y0 = Math.min(y0, n.y! - n.r);
    x1 = Math.max(x1, n.x! + n.r); y1 = Math.max(y1, n.y! + n.r);
  }
  return { x0, y0, x1, y1 };
}

export function fit(animate = true) {
  if (!nodes.length) return kick(); // 비었으면 지난 지도를 지운다
  const el = canvas();
  const { x0, y0, x1, y1 } = bounds();
  const w = el.clientWidth, hgt = el.clientHeight;
  // 숨어 있는 동안 배치가 끝났다: 크기를 모르니 다시 보일 때 맞춘다
  fitPending = !w || !hgt;
  if (fitPending) return;
  const k = Math.max(0.2, Math.min(2.2, Math.min(w / (x1 - x0 + 80), hgt / (y1 - y0 + 80))));
  trace(`fit w=${w} h=${hgt} k=${k} box=${x0},${y0},${x1},${y1}`);
  const target = { k, x: w / 2 - ((x0 + x1) / 2) * k, y: hgt / 2 - ((y0 + y1) / 2) * k };
  if (!animate || calm()) {
    view = target;
    kick();
    return;
  }
  const from = { ...view };
  const start = performance.now();
  const step = (now: number) => {
    const p = Math.min(1, (now - start) / 260);
    const e = 1 - Math.pow(1 - p, 3);
    view = { k: from.k + (target.k - from.k) * e, x: from.x + (target.x - from.x) * e, y: from.y + (target.y - from.y) * e };
    draw();
    if (p < 1) requestAnimationFrame(step);
  };
  requestAnimationFrame(step);
}

// ── 그리기 ───────────────────────────────────────────────

function kick() {
  if (!visible || raf) return;
  raf = requestAnimationFrame(() => {
    raf = 0;
    draw();
  });
}

function neighborsOf(n: N): Set<string> {
  const s = new Set<string>([n.id]);
  for (const l of links) {
    const a = (l.source as N).id, b = (l.target as N).id;
    if (a === n.id) s.add(b);
    if (b === n.id) s.add(a);
  }
  return s;
}

function markColor(m: N["mark"]): string | null {
  switch (m) {
    case "edited": return colors.accent;
    case "context": return colors.context;
    case "read": return colors.context;
    case "impact0": return colors.error;
    case "impact1": return colors.warning;
    case "impact2": return colors.warning;
    case "test": return colors.ok;
    default: return null;
  }
}

function drawNodeShape(ctx: CanvasRenderingContext2D, n: N) {
  const { x, y, r } = { x: n.x!, y: n.y!, r: n.r };
  ctx.beginPath();
  if (n.kind === "dir") ctx.roundRect(x - r, y - r * 0.8, r * 2, r * 1.6, Math.min(8, r / 2.5));
  else if (n.kind === "file") ctx.roundRect(x - r, y - r, r * 2, r * 2, Math.min(4, r / 2));
  else if (n.kind === "memory") {
    ctx.moveTo(x, y - r); ctx.lineTo(x + r, y); ctx.lineTo(x, y + r); ctx.lineTo(x - r, y); ctx.closePath();
  } else ctx.arc(x, y, r, 0, Math.PI * 2);
}

/** 전체 구조의 덩어리와 이름 자리. 노드가 셋 이상인 모듈만 (한두 개면 자기 이름표로 충분하다) */
function clusters() {
  const by = new Map<string, N[]>();
  for (const n of nodes) {
    if (n.kind === "memory" || !n.group || n.x === undefined) continue;
    if (!by.has(n.group)) by.set(n.group, []);
    by.get(n.group)!.push(n);
  }
  if (by.size < 2) return [];
  return [...by].filter(([, list]) => list.length >= 3).sort((a, b) => b[1].length - a[1].length).map(([group, list]) => {
    // 멀리 떨어진 노드 하나 때문에 이름이 덩어리에서 멀어지지 않게 맨 위가 아니라 위쪽 10% 자리에 둔다
    const tops = list.map((n) => n.y! - n.r).sort((a, b) => a - b);
    return { group, x: list.reduce((sum, n) => sum + n.x!, 0) / list.length, top: tops[Math.floor(tops.length * 0.1)] };
  });
}

function draw() {
  const el = canvas();
  const dpr = window.devicePixelRatio || 1;
  const w = el.clientWidth, hgt = el.clientHeight;
  if (el.width !== Math.round(w * dpr) || el.height !== Math.round(hgt * dpr)) {
    el.width = Math.round(w * dpr);
    el.height = Math.round(hgt * dpr);
  }
  // GPU 캔버스를 쓰지 않는다: 휠로 확대·축소만 해도 GPU 프로세스가 450MB(화면 배율 2에서 760MB)까지 오른다. 소프트웨어는 110MB이고 그리기가 1~3ms 늘어난다
  const ctx = el.getContext("2d", { willReadFrequently: true })!;
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, w, hgt);
  ctx.translate(view.x, view.y);
  ctx.scale(view.k, view.k);

  const focusSet = selected ? neighborsOf(selected) : hovered ? neighborsOf(hovered) : null;
  const fpActive = mode !== "impact" && mode !== "memory" && overlays.footprint && footprintSize() > 0;
  const dimmed = (n: N) => (focusSet ? !focusSet.has(n.id) : fpActive ? !n.mark && n.kind !== "memory" : false);
  // 영향 지도의 테스트는 물러나 있다: 실제 호출자가 먼저 읽히도록. 고르거나 올리면 또렷해진다
  const receded = (n: N) => (n.mark === "test" || (n.test && !n.mark)) && !focusSet?.has(n.id);

  trace(`draw k=${view.k} x=${view.x} y=${view.y} w=${w} h=${hgt}`);
  // 간선
  for (const l of links) {
    const a = l.source as N, b = l.target as N;
    if (a.x === undefined || b.x === undefined) continue;
    const lit = focusSet ? focusSet.has(a.id) && focusSet.has(b.id) : !(dimmed(a) && dimmed(b));
    ctx.beginPath();
    ctx.moveTo(a.x, a.y!);
    ctx.lineTo(b.x, b.y!);
    ctx.setLineDash(l.kind === "cochange" ? [5, 4] : l.kind === "mentions" ? [2, 3] : l.uncertain ? [1, 3] : []);
    // 함께 바뀜은 무채색: 청록은 맥락 엔진이 보낸 것만 뜻한다
    const base = l.kind === "cochange" ? colors.dim : l.kind === "mentions" ? colors.link : colors.border;
    ctx.strokeStyle = withAlpha(base, lit ? (l.kind === "defines" ? 0.35 : receded(a) || receded(b) ? 0.3 : 0.7) : 0.12);
    ctx.lineWidth = (l.kind === "defines" ? 0.8 : Math.min(3, 0.8 + Math.log1p(l.weight) * 0.5)) / Math.sqrt(view.k);
    ctx.stroke();
    // 호출 방향 화살표
    if ((l.kind === "calls" || l.kind === "ref") && lit && view.k > 0.6) {
      const dx = b.x - a.x, dy = b.y! - a.y!;
      const len = Math.hypot(dx, dy) || 1;
      const ux = dx / len, uy = dy / len;
      const tx = b.x - ux * (b.r + 3), ty = b.y! - uy * (b.r + 3);
      ctx.setLineDash([]);
      ctx.beginPath();
      ctx.moveTo(tx, ty);
      ctx.lineTo(tx - ux * 6 - uy * 3, ty - uy * 6 + ux * 3);
      ctx.lineTo(tx - ux * 6 + uy * 3, ty - uy * 6 - ux * 3);
      ctx.closePath();
      ctx.fillStyle = withAlpha(base, 0.7);
      ctx.fill();
    }
  }
  ctx.setLineDash([]);

  trace("draw: nodes");
  // 노드
  for (const n of nodes) {
    if (n.x === undefined) continue;
    const dim = dimmed(n);
    const mc = markColor(n.mark);
    // 바탕은 물러난 노드도 불투명하게: 간선이 노드 안으로 비쳐 보이지 않게
    ctx.globalAlpha = dim ? 0.22 : 1;
    drawNodeShape(ctx, n);
    ctx.fillStyle = colors.bg;
    ctx.fill();
    ctx.globalAlpha = dim ? 0.22 : receded(n) ? 0.5 : 1;
    const fillBase = n.kind === "memory" ? colors.link : groupColor(n.group);
    ctx.fillStyle = mc && (n.mark === "edited" || n.mark === "context" || n.mark === "impact0") ? withAlpha(mc, 0.85) : n.kind === "memory" ? withAlpha(fillBase, 0.8) : groupColor(n.group, n.kind === "dir" ? 0.4 : 0.28);
    ctx.fill();
    ctx.lineWidth = (mc ? 2.2 : 1.2) / Math.sqrt(view.k);
    ctx.strokeStyle = mc ?? (n.kind === "memory" ? colors.link : groupColor(n.group, 0.9));
    if (n.mark === "read" || n.mark === "impact2") ctx.setLineDash([3, 2]);
    ctx.stroke();
    ctx.setLineDash([]);
    if (n.notes?.length && n.kind !== "memory") {
      // 기억이 연결된 코드: 오른쪽 위 작은 마름모
      const s = 3.5;
      ctx.beginPath();
      ctx.moveTo(n.x + n.r, n.y! - n.r - s); ctx.lineTo(n.x + n.r + s, n.y! - n.r); ctx.lineTo(n.x + n.r, n.y! - n.r + s); ctx.lineTo(n.x + n.r - s, n.y! - n.r); ctx.closePath();
      ctx.fillStyle = colors.link;
      ctx.fill();
    }
    if (n === selected) {
      ctx.beginPath();
      ctx.arc(n.x, n.y!, n.r + 5, 0, Math.PI * 2);
      ctx.strokeStyle = colors.focus;
      ctx.lineWidth = 2 / Math.sqrt(view.k);
      ctx.stroke();
    }
    // 에이전트가 방금 본 곳: 번지는 고리 (작업 중일 때만)
    if (footprint?.last && n.path === footprint.last && overlays.footprint && mode !== "impact" && pulseOn) {
      const p = calm() ? 0.4 : (pulse % 1200) / 1200;
      ctx.beginPath();
      ctx.arc(n.x, n.y!, n.r + 4 + p * 14, 0, Math.PI * 2);
      ctx.strokeStyle = withAlpha(colors.context, 0.6 * (1 - p));
      ctx.lineWidth = 2 / Math.sqrt(view.k);
      ctx.stroke();
    }
    ctx.globalAlpha = 1;
  }

  trace("draw: labels");
  // 이름표: 중요한 것부터 놓고, 이미 놓은 이름표와 겹치면 건너뛴다
  const fs = 11 / view.k;
  ctx.font = `${fs}px Pretendard Variable, Pretendard, sans-serif`;
  ctx.textAlign = "center";
  ctx.textBaseline = "top";
  ctx.lineJoin = "round";
  // 영향 지도: 바뀌는 곳, 직접 호출, 간접 호출 순으로 자리를 얻고 테스트는 남는 자리에
  const markScore = (n: N) => (n.mark === "impact0" ? 3e5 : n.mark === "impact1" ? 2e5 : n.mark === "test" ? 0 : n.mark ? 1e5 : 0);
  const score = (n: N) => (n === selected ? 1e9 : n === hovered ? 1e8 : 0) + (focusSet?.has(n.id) ? 1e6 : 0) + markScore(n) + (n.kind === "dir" ? 1e4 : 0) + n.r * 100 - (n.test ? 2e3 : 0);
  // 흐리게 처리된 노드도 이름은 옅게 남긴다 (어디쯤인지 알 수 있게). 대신 가장 나중에 자리를 얻는다
  const rank = (n: N) => score(n) - (dimmed(n) && n !== hovered ? 5e5 : 0);
  const candidates = nodes.filter((n) => n.x !== undefined).sort((a, b) => rank(b) - rank(a));
  const placed: { x0: number; y0: number; x1: number; y1: number }[] = [];
  const pad = 2 / view.k;
  const taken = (box: (typeof placed)[number]) => placed.some((b) => box.x0 < b.x1 && b.x0 < box.x1 && box.y0 < b.y1 && b.y0 < box.y1);
  // 덩어리 이름: 전체 구조에서만. 노드 이름표보다 먼저 자리를 잡는다
  if (mode === "overview") {
    const size = 12 / view.k;
    ctx.font = `600 ${size}px Pretendard Variable, Pretendard, sans-serif`;
    ctx.textBaseline = "bottom";
    ctx.globalAlpha = focusSet || fpActive ? 0.45 : 1;
    for (const c of clusters()) {
      const half = ctx.measureText(c.group).width / 2;
      const y = c.top - 5 / view.k;
      const box = { x0: c.x - half - pad, y0: y - size - pad, x1: c.x + half + pad, y1: y + pad };
      if (taken(box)) continue;
      placed.push(box);
      ctx.lineWidth = 3 / view.k;
      ctx.strokeStyle = colors.bg;
      ctx.strokeText(c.group, c.x, y);
      ctx.fillStyle = groupColor(c.group);
      ctx.fillText(c.group, c.x, y);
    }
    ctx.globalAlpha = 1;
    ctx.textBaseline = "top";
    ctx.font = `${fs}px Pretendard Variable, Pretendard, sans-serif`;
  }
  for (const n of candidates) {
    const must = n === selected || n === hovered;
    if (!must && n.kind !== "dir" && n.r * view.k < 6 && !n.mark && !focusSet?.has(n.id)) continue;
    const text = n.kind === "memory" ? (n.detail ?? n.label).slice(0, 28) + ((n.detail?.length ?? 0) > 28 ? "…" : "") : n.text ?? n.label;
    const w = ctx.measureText(text).width;
    const y = n.y! + (n.kind === "dir" ? n.r * 0.8 : n.r) + 3 / view.k;
    const box = { x0: n.x! - w / 2 - pad, y0: y - pad, x1: n.x! + w / 2 + pad, y1: y + fs + pad };
    if (!must && taken(box)) continue;
    placed.push(box);
    ctx.globalAlpha = dimmed(n) && n !== hovered ? 0.45 : 1;
    ctx.lineWidth = 3 / view.k;
    ctx.strokeStyle = colors.bg;
    ctx.strokeText(text, n.x!, y);
    ctx.fillStyle = n === selected ? colors.strong : n.kind === "dir" ? colors.strong : receded(n) ? colors.muted : colors.fg;
    if (n.kind === "dir") ctx.font = `600 ${fs}px Pretendard Variable, Pretendard, sans-serif`;
    ctx.fillText(text, n.x!, y);
    if (n.kind === "dir") ctx.font = `${fs}px Pretendard Variable, Pretendard, sans-serif`;
  }
  ctx.globalAlpha = 1;
  trace("draw: end");
}

// 번지는 고리 애니메이션 (작업 중에만 켠다: 화면에 움직임은 하나만)
let pulseOn = false;
let pulseTimer = 0;
export function setWorking(on: boolean) {
  pulseOn = on;
  cancelAnimationFrame(pulseTimer);
  if (!on || calm()) return kick();
  const loop = (ts: number) => {
    pulse = ts;
    if (visible) draw();
    if (pulseOn) pulseTimer = requestAnimationFrame(loop);
  };
  pulseTimer = requestAnimationFrame(loop);
}

// ── 화면 틀 (도구 막대, 정보 카드, 범례) ─────────────────────

function setStatus(text: string) {
  const s = $("#map-status");
  s.textContent = text;
  s.classList.toggle("hidden", !text);
}

function modeTitle(): string {
  switch (mode) {
    case "overview": return t("전체 구조");
    case "focus": return focusLabel;
    case "impact": return `${t("영향 반경")} · ${focusLabel}`;
    case "memory": return t("프로젝트 기억");
  }
}

const RISK: Record<string, string> = { low: "낮음", medium: "보통", high: "높음" };

function renderChrome() {
  $("#map-crumb").replaceChildren(
    ...(mode === "overview" ? [h("span", { class: "cur" }, modeTitle())] : [
      (() => {
        const b = h("button", { class: "crumb-link", title: t("전체 구조") }, codicon("type-hierarchy"), h("span", {}, t("전체 구조")));
        b.addEventListener("click", () => {
          leavePreview();
          if (mode !== "overview" || !nodes.length) void showOverview();
        });
        return b;
      })(),
      codicon("chevron-right"),
      h("span", { class: "cur" }, modeTitle()),
    ]),
    h("span", { class: "map-count" }, `${nodes.length}${truncated ? "+" : ""}`),
  );
  for (const b of document.querySelectorAll<HTMLElement>("#map-overlays [data-overlay]")) {
    const key = b.dataset.overlay as keyof typeof overlays;
    b.classList.toggle("on", overlays[key]);
    b.setAttribute("aria-pressed", String(overlays[key]));
  }
  renderLegend();
  renderCard();
}

function renderLegend() {
  const item = (swatch: HTMLElement, label: string) => h("span", { class: "lg" }, swatch, label);
  const dot = (cls: string) => h("i", { class: `sw ${cls}` });
  const items: HTMLElement[] = [];
  if (mode === "impact" && impact) {
    items.push(item(dot("impact0"), "바뀌는 곳"), item(dot("impact1"), "직접 호출"), item(dot("impact2"), "간접 호출"), item(dot("test"), "테스트"), item(dot("cochange"), "함께 바뀌던 파일"));
  } else {
    if (nodes.some((n) => n.kind === "dir")) items.push(item(dot("dir"), "폴더 (두 번 눌러 펼치기)"));
    items.push(item(dot("file"), "파일"), item(dot("symbol"), "심볼"), item(dot("cochange"), "함께 바뀜"));
    if (nodes.some((n) => n.test)) items.push(item(dot("file faded"), "테스트"));
    if (links.some((l) => l.uncertain)) items.push(item(dot("uncertain"), "이름만 일치"));
    if (overlays.footprint && footprintSize()) items.push(item(dot("context"), "AI에 보낸 맥락"), item(dot("read"), "읽음"), item(dot("edited"), "수정"));
    if (overlays.memory || mode === "memory") items.push(item(dot("memory"), "기억"));
  }
  $("#map-legend").replaceChildren(...items);

  const fp = $("#map-footprint");
  if (mode !== "impact" && overlays.footprint && footprint && footprintSize()) {
    fp.replaceChildren(
      h("span", { class: "fp-title" }, footprintTitle || t("현재 작업")),
      h("span", {}, h("b", {}, String(footprint.context.size)), t("보낸 맥락")),
      h("span", {}, h("b", {}, String(footprint.read.size)), t("읽음")),
      h("span", {}, h("b", {}, String(footprint.edited.size)), t("수정")));
    fp.classList.remove("hidden");
  } else if (mode === "impact" && impact) {
    fp.replaceChildren(
      impact.comment_only ? h("span", { class: "risk risk-none" }, t("코드 변경 없음")) : h("span", { class: `risk risk-${impact.code_risk}` }, `${t("위험도")} ${t(RISK[impact.code_risk])}`),
      h("span", {}, h("b", {}, String(impact.code_callers)), t("직접 호출")),
      h("span", {}, h("b", {}, String(impact.code_callers2)), t("간접 호출")),
      h("span", {}, h("b", {}, String(impact.tests.length)), t("테스트")));
    fp.classList.remove("hidden");
  } else fp.classList.add("hidden");
}

/** 노드의 전체 수. 값이 undefined면 받는 중, null이면 말할 수 없다 (노드가 이제 없다) */
function statsOf(n: N): NodeStats | null | undefined {
  const cache = statsCache;
  if (!cache.has(n.id)) {
    cache.set(n.id, undefined);
    void invoke<NodeStats | null>("graph_node_stats", { id: n.id, label: n.label }).catch(() => null).then((s) => {
      cache.set(n.id, s);
      if (cache === statsCache && selected?.id === n.id) renderCard();
    });
  }
  const got = cache.get(n.id);
  // 다시 세는 동안은 보던 수를 둔다 (에이전트가 일할 때 카드가 깜박이지 않게)
  if (got === undefined) return lastStats?.id === n.id ? lastStats.stats : undefined;
  lastStats = { id: n.id, stats: got };
  return got;
}

/** 수를 어떻게 셌는지와 얼마나 믿을 수 있는지 */
function statsBasis(n: N, s: NodeStats): HTMLElement {
  if (n.kind !== "symbol") return h("div", { class: "mc-basis" }, t(n.kind === "dir" ? "참조는 이름으로 찾은 바깥 파일 수입니다" : "참조는 이름으로 찾은 파일 수입니다"));
  const lines = [t("이름으로 찾은 수입니다")];
  if (s.callers === null) lines.push(t(`이름이 흔해(${s.defs}곳 정의) 부르는 곳을 셀 수 없습니다`));
  else if (s.defs > 1) lines.push(t(`같은 이름이 ${s.defs}곳에 정의되어 다른 쪽을 부르는 곳이 섞일 수 있습니다`));
  if (s.callees_ambiguous) lines.push(t(`부르는 것 중 ${s.callees_ambiguous}개는 같은 이름이 여러 곳에 있어 확실하지 않습니다`));
  if (s.callees_common) lines.push(t(`이름이 흔해 세지 않은 것 ${s.callees_common}개`));
  return h("div", { class: "mc-basis", title: t("이름으로 찾은 호출 관계(정적 분석)입니다. 테스트 코드는 호출자로 세지 않고 따로 보입니다. DI·리플렉션·이벤트·라우팅처럼 실행 중에 정해지는 호출과, URL로 부르는 테스트는 세지 않습니다.") }, lines.join(" · "));
}

function renderCard() {
  const card = $("#map-card");
  const n = selected;
  if (!n) {
    card.classList.add("hidden");
    return;
  }
  const kindLabel = n.kind === "dir" ? t("폴더") : n.kind === "file" ? t("파일") : n.kind === "memory" ? t("기억") : (n.detail ?? t("심볼"));
  // 수는 지금 보이는 그래프가 아니라 프로젝트 전체에서 센 것만 보인다. 받는 중에는 '–'
  const s = n.kind === "memory" ? null : statsOf(n);
  const stat = (label: string, value?: number | string | null) => h("div", { class: "mc-stat" }, h("span", {}, t(label)), h("b", {}, value === undefined ? "–" : String(value ?? "?")));
  const rows: HTMLElement[] = [];
  if (s !== null && n.kind === "symbol") rows.push(stat("부르는 곳", s?.callers), stat("테스트", s?.test_callers), stat("부르는 것", s?.callees));
  else if (s !== null && n.kind === "dir") rows.push(stat("파일", n.detail), stat("참조받음", s?.refs_in), stat("참조함", s?.refs_out));
  else if (s !== null && n.kind === "file") rows.push(stat("참조받음", s?.refs_in), stat("참조함", s?.refs_out), stat("함께 바뀜", s?.cochange));
  const markText: Record<string, string> = { edited: "이 작업에서 수정함", context: "AI에 맥락으로 보냄", read: "에이전트가 읽음", impact0: "이 변경이 바꾸는 곳", impact1: "바뀌는 코드를 직접 호출", impact2: "간접적으로 영향", test: "영향 범위의 테스트" };
  const actions = h("div", { class: "mc-actions" });
  const btn = (icon: string, label: string, fn: () => void, primary = false) => {
    const b = h("button", { class: `btn ${primary ? "btn-primary" : "btn-secondary"}` }, codicon(icon), t(label));
    b.addEventListener("click", fn);
    actions.append(b);
  };
  if (n.kind === "dir") btn("unfold", "펼치기", () => void focus(n.id, n.path || n.label), true);
  else if (n.path) btn("go-to-file", n.kind === "memory" ? "기억 파일 열기" : "코드 열기", () => openFile(n.path!, n.line), true);
  if (n.kind === "file" || n.kind === "symbol") btn("type-hierarchy-sub", "주변 보기", () => void focus(n.id, n.label));
  if (n.kind === "symbol" && n.path && n.line) {
    btn("pulse", "영향 보기", async () => {
      try {
        const i = await invoke<Impact>("graph_impact", { path: n.path, ranges: [[n.line!, n.line!]] });
        showImpact(i);
      } catch (e) {
        setStatus(errorText(e));
      }
    });
  }
  card.replaceChildren(...([
    h("div", { class: "mc-head" },
      h("span", { class: `mc-kind k-${n.kind}` }, kindLabel),
      h("button", { class: "icon-btn", title: t("닫기"), "aria-label": t("닫기"), onclick: () => select(null) }, codicon("close"))),
    h("div", { class: "mc-title" }, n.kind === "memory" ? (n.detail ?? "") : n.label),
    n.path && n.kind !== "dir" ? h("div", { class: "mc-path", title: n.path }, `${dirname(n.path) ? dirname(n.path) + "/" : ""}${basename(n.path)}${n.line ? `:${n.line}` : ""}`) : null,
    n.kind === "dir" ? h("div", { class: "mc-path", title: n.path }, `${n.path || "(root)"}/`) : null,
    n.mark ? h("div", { class: `mc-mark m-${n.mark}` }, t(markText[n.mark])) : null,
    rows.length ? h("div", { class: "mc-stats" }, ...rows) : null,
    s ? statsBasis(n, s) : null,
    n.notes?.length ? h("div", { class: "mc-notes" }, h("div", { class: "mc-sub" }, codicon("book"), t("연결된 기억")), ...n.notes.slice(0, 4).map((x) => h("div", { class: "mc-note" }, x))) : null,
    actions,
  ].filter(Boolean) as Node[]));
  card.classList.remove("hidden");
}

function select(n: N | null) {
  selected = n;
  renderCard();
  kick();
}

// ── 조작 ─────────────────────────────────────────────────

function toWorld(ev: MouseEvent) {
  const r = canvas().getBoundingClientRect();
  return { x: (ev.clientX - r.left - view.x) / view.k, y: (ev.clientY - r.top - view.y) / view.k };
}

function hit(ev: MouseEvent): N | null {
  const p = toWorld(ev);
  let best: N | null = null;
  let bestD = Infinity;
  for (const n of nodes) {
    const d = Math.hypot(n.x! - p.x, n.y! - p.y);
    if (d < n.r + 4 / view.k && d < bestD) {
      best = n;
      bestD = d;
    }
  }
  return best;
}

function bindCanvas() {
  const el = canvas();
  let drag: { node: N | null; sx: number; sy: number; vx: number; vy: number; moved: boolean } | null = null;
  el.addEventListener("mousedown", (e) => {
    if (e.button !== 0) return;
    const n = hit(e);
    drag = { node: n, sx: e.clientX, sy: e.clientY, vx: view.x, vy: view.y, moved: false };
    if (n) {
      n.fx = n.x;
      n.fy = n.y;
    }
  });
  window.addEventListener("mousemove", (e) => {
    if (!visible) return;
    if (drag) {
      const dx = e.clientX - drag.sx, dy = e.clientY - drag.sy;
      if (Math.abs(dx) + Math.abs(dy) > 3) {
        drag.moved = true;
        cameraMoved = true;
      }
      if (drag.node) {
        const p = toWorld(e);
        drag.node.fx = p.x;
        drag.node.fy = p.y;
        sim?.alphaTarget(0.2).restart();
      } else {
        view.x = drag.vx + dx;
        view.y = drag.vy + dy;
        kick();
      }
      el.style.cursor = "grabbing";
      return;
    }
    if (e.target !== el) return;
    const n = hit(e);
    if (n !== hovered) {
      hovered = n;
      el.style.cursor = n ? "pointer" : "grab";
      el.title = n ? (n.kind === "memory" ? n.detail ?? "" : `${n.label}${n.path ? ` — ${n.path}${n.line ? `:${n.line}` : ""}` : ""}`) : "";
      kick();
    }
  });
  window.addEventListener("mouseup", () => {
    if (!drag) return;
    const d = drag;
    drag = null;
    el.style.cursor = hovered ? "pointer" : "grab";
    sim?.alphaTarget(0);
    // 중심 노드가 아니면 놓은 자리를 풀어 준다
    if (d.node && d.node !== nodes[0]) {
      d.node.fx = null;
      d.node.fy = null;
    }
    if (!d.moved) select(d.node);
  });
  el.addEventListener("dblclick", (e) => {
    const n = hit(e);
    if (n?.kind === "dir") void focus(n.id, n.path || n.label);
    else if (n?.path) openFile(n.path, n.line);
  });
  el.addEventListener("wheel", (e) => {
    e.preventDefault();
    const r = el.getBoundingClientRect();
    const mx = e.clientX - r.left, my = e.clientY - r.top;
    const k = Math.max(0.15, Math.min(4, view.k * Math.exp(-e.deltaY * 0.0015)));
    view.x = mx - ((mx - view.x) / view.k) * k;
    view.y = my - ((my - view.y) / view.k) * k;
    view.k = k;
    cameraMoved = true;
    kick();
  }, { passive: false });
  el.addEventListener("keydown", (e) => {
    if (e.key === "Escape") select(null);
    if (e.key === "Enter" && selected?.kind === "dir") void focus(selected.id, selected.path || selected.label);
    else if (e.key === "Enter" && selected?.path) openFile(selected.path, selected.line);
  });
  new ResizeObserver(() => kick()).observe(el);
}

// ── 검색 ─────────────────────────────────────────────────

function bindSearch() {
  const input = $<HTMLInputElement>("#map-search");
  const list = $("#map-results");
  let timer = 0;
  let seq = 0;
  const run = async () => {
    const q = input.value.trim();
    const my = ++seq;
    if (!q) return list.classList.add("hidden");
    const found = await invoke<GNode[]>("graph_find", { query: q }).catch(() => []);
    if (my !== seq) return;
    list.replaceChildren(...found.map((n, i) => {
      const row = h("button", { class: `mr-row${i === 0 ? " active" : ""}`, role: "option" },
        codicon(n.kind === "file" ? "file" : "symbol-method"),
        h("span", { class: "mr-name" }, n.label),
        h("span", { class: "mr-path" }, n.path ?? ""));
      row.addEventListener("mousedown", (e) => {
        e.preventDefault();
        pick(n);
      });
      return row;
    }));
    if (!found.length) list.replaceChildren(h("div", { class: "mr-empty" }, t("일치하는 코드가 없습니다")));
    list.classList.remove("hidden");
  };
  const pick = (n: GNode) => {
    list.classList.add("hidden");
    input.value = "";
    input.blur();
    void focus(n.id, n.label);
  };
  input.addEventListener("input", () => {
    clearTimeout(timer);
    timer = window.setTimeout(run, 180);
  });
  input.addEventListener("keydown", (e) => {
    const rows = [...list.querySelectorAll<HTMLElement>(".mr-row")];
    const i = rows.findIndex((r) => r.classList.contains("active"));
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      rows[i]?.classList.remove("active");
      rows[(i + (e.key === "ArrowDown" ? 1 : rows.length - 1)) % rows.length]?.classList.add("active");
    } else if (e.key === "Enter") {
      e.preventDefault();
      rows[Math.max(0, i)]?.dispatchEvent(new MouseEvent("mousedown"));
    } else if (e.key === "Escape") {
      list.classList.add("hidden");
      input.blur();
    }
  });
  input.addEventListener("blur", () => setTimeout(() => list.classList.add("hidden"), 120));
}

// ── 보이기 ───────────────────────────────────────────────

/** 발자취를 보려면 전체 구조나 주변이어야 한다 */
export function showFootprint() {
  overlays.footprint = true;
  show();
  leavePreview();
  if (!loaded || mode === "impact" || mode === "memory") void showOverview();
  else {
    applyMarks();
    renderChrome();
    kick();
  }
}

/** 지도를 그렸는데 노드가 하나도 없음 (인덱스가 비어 있을 때 그렸음) */
export function isEmpty(): boolean {
  return loaded && nodes.length === 0;
}

export function isVisible(): boolean {
  return visible;
}

export function show() {
  if (visible) return;
  visible = true;
  $("#map-view").classList.remove("hidden");
  $("#editor-group").classList.add("hidden");
  onVisibility(true);
  readColors();
  if (preview && !preview.opened) openPreview();
  else if (!loaded || nodes.length === 0) void showOverview();
  else if (fitPending && !cameraMoved) fit(false);
  else kick();
  if (staleWhileHidden) {
    staleWhileHidden = false;
    refreshSoon();
  }
}

export function hide() {
  if (!visible) return;
  visible = false;
  $("#map-view").classList.add("hidden");
  $("#editor-group").classList.remove("hidden");
  onVisibility(false);
}

export function toggle() {
  if (visible) hide();
  else show();
}

/** 프로젝트를 바꾸면 처음부터 */
export function reset() {
  sim?.stop();
  nodes = [];
  links = [];
  byId = new Map();
  selected = null;
  lastStats = null;
  impact = null;
  preview = null;
  footprint = null;
  memoryNotes = [];
  loaded = false;
  mode = "overview";
  cameraMoved = false;
  groupHue.clear();
  if (visible) void showOverview();
}

/** 파일이 바뀌면 (인덱스 갱신 후) 지금 보는 지도를 다시 그린다 */
export function refreshSoon() {
  // 미리 보기 중이면 돌아간 뒤에 새로 그린다
  const held = () => {
    if (mode !== "impact") return false;
    if (preview?.saved) preview.saved.stale = true;
    return true;
  };
  if (!loaded || held()) return;
  clearTimeout(refreshTimer);
  refreshTimer = window.setTimeout(async () => {
    if (held()) return;
    // 심볼 id는 다시 인덱싱하면 바뀐다: 노드를 고른 채여도 그래프를 다시 받아 카드가 새 id로 센다
    const fetcher = mode === "overview" ? () => invoke<Graph>("graph_overview", { maxNodes: 400 }) : mode === "focus" ? fetchFocus : null;
    // 그래프를 다시 받지 않는 화면이어도 카드의 수는 새로 센다
    statsCache = new Map();
    if (selected && visible && !fetcher) renderCard();
    if (!visible) staleWhileHidden = true;
    if (!fetcher || !visible) return;
    const my = loadSeq;
    try {
      let g = await fetcher();
      if (overlays.memory) g = await withMemory(g);
      if (my !== loadSeq) {
        held();
        return;
      }
      setGraph(g, true);
      setStatus(g.nodes.length ? "" : t("표시할 코드가 없습니다. 인덱싱이 끝났는지 확인하세요."));
    } catch { /* 다음 변경 때 다시 */ }
  }, 1200);
}
let refreshTimer = 0;
/** 숨어 있는 동안 파일이 바뀌었다: 다시 띄울 때 새로 그린다 */
let staleWhileHidden = false;

export function init(opts: { openFile: (path: string, line?: number) => void; onVisibility: (v: boolean) => void }) {
  openFile = opts.openFile;
  onVisibility = opts.onVisibility;
  bindCanvas();
  bindSearch();
  $("#map-fit").addEventListener("click", () => {
    cameraMoved = false;
    fit();
  });
  $("#map-close").addEventListener("click", hide);
  for (const b of document.querySelectorAll<HTMLElement>("#map-overlays [data-overlay]")) {
    b.addEventListener("click", async () => {
      const key = b.dataset.overlay as keyof typeof overlays;
      overlays[key] = !overlays[key];
      if (key === "memory" && baseGraph && mode !== "impact" && mode !== "memory") {
        const g = overlays.memory ? await withMemory(baseGraph).catch(() => baseGraph!) : baseGraph;
        if (!overlays.memory) memoryNotes = [];
        setGraph(g, true);
      } else {
        if (key === "memory" && preview?.saved) preview.saved.stale = true;
        applyMarks();
        renderChrome();
        kick();
      }
    });
  }
  prefs.onChange(() => {
    readColors();
    kick();
  });
}
