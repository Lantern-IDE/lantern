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
import * as prefs from "./prefs";
import { t } from "./i18n";

// ── 백엔드 모양 ───────────────────────────────────────────

export type NodeKind = "dir" | "file" | "symbol" | "memory";
export type EdgeKind = "ref" | "calls" | "defines" | "cochange" | "mentions";
export interface GNode { id: string; kind: NodeKind; label: string; path?: string; line?: number; group: string; weight: number; detail?: string; depth?: number }
export interface GEdge { source: string; target: string; kind: EdgeKind; weight: number }
export interface Graph { nodes: GNode[]; edges: GEdge[]; truncated: boolean }
export interface Impact {
  path: string; touched: string[]; callers: number; callers2: number; files: number; modules: number;
  cochanged: [string, number][]; tests: string[]; ambiguous: string[]; unique: boolean; risk: "low" | "medium" | "high"; graph: Graph;
}
export interface MemoryNote { id: string; topic: string; file: string; line: number; text: string; links: string[] }

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

interface N extends SimulationNodeDatum, GNode { r: number; mark?: "context" | "read" | "edited" | "impact0" | "impact1" | "impact2" | "test"; notes?: string[] }
interface L extends SimulationLinkDatum<N> { kind: EdgeKind; weight: number }

type Mode = "overview" | "focus" | "impact" | "memory";
const overlays = { footprint: true, memory: false };

let mode: Mode = "overview";
let focusLabel = "";
let nodes: N[] = [];
let links: L[] = [];
let byId = new Map<string, N>();
let sim: Simulation<N, L> | null = null;
let view = { x: 0, y: 0, k: 1 };
let selected: N | null = null;
let hovered: N | null = null;
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
  const old = keepPositions ? byId : new Map<string, N>();
  nodes = g.nodes.map((n) => {
    const prev = old.get(n.id);
    return { ...n, r: radius(n), x: prev?.x, y: prev?.y };
  });
  byId = new Map(nodes.map((n) => [n.id, n]));
  links = g.edges.filter((e) => byId.has(e.source) && byId.has(e.target)).map((e) => ({ source: e.source, target: e.target, kind: e.kind, weight: e.weight }) as unknown as L);
  truncated = g.truncated;
  if (!keepPositions || (selected && !byId.has(selected.id))) selected = null;
  hovered = null;
  applyMarks();
  trace("setGraph: layout");
  layout();
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

async function load(fetcher: () => Promise<Graph>, nextMode: Mode, label = "") {
  setStatus(t("지도를 그리는 중…"));
  try {
    let g = await fetcher();
    baseGraph = g;
    if (overlays.memory && nextMode !== "memory") g = await withMemory(g);
    mode = nextMode;
    focusLabel = label;
    view = { x: 0, y: 0, k: 1 };
    setGraph(g);
    loaded = true;
    setStatus(g.nodes.length ? "" : t("표시할 코드가 없습니다. 인덱싱이 끝났는지 확인하세요."));
  } catch (e) {
    setStatus(errorText(e));
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

/** `f:경로`, `s:id`, 또는 이름 */
export function focus(center: string, label?: string) {
  impact = null;
  show();
  return load(() => invoke<Graph>("graph_neighborhood", { center, limit: 30 }), "focus", label ?? center.replace(/^[fs]:/, ""));
}

export function showImpact(i: Impact) {
  impact = i;
  show();
  mode = "impact";
  focusLabel = `${basename(i.path)}${i.touched.length ? ` · ${i.touched.join(", ")}` : ""}`;
  view = { x: 0, y: 0, k: 1 };
  baseGraph = i.graph;
  setGraph(i.graph);
  loaded = true;
  setStatus(i.graph.nodes.length > 1 ? "" : t("이 변경에 닿는 다른 코드를 찾지 못했습니다."));
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

function layout() {
  sim?.stop();
  const el = canvas();
  const w = el.clientWidth || 800;
  const hgt = el.clientHeight || 600;
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
  const dist: Record<EdgeKind, number> = { defines: 36, calls: 70, ref: 80, cochange: 110, mentions: 70 };
  const clustered = mode === "overview";
  sim = forceSimulation<N, L>(nodes)
    .force("link", forceLink<N, L>(links).id((d) => d.id).distance((l) => dist[l.kind]).strength((l) => (l.kind === "cochange" ? 0.05 : Math.min(0.6, 0.15 + Math.log1p(l.weight) * 0.08))))
    .force("charge", forceManyBody<N>().strength(nodes.length > 200 ? -60 : -160).distanceMax(400))
    .force("collide", forceCollide<N>().radius((d) => (d.kind === "dir" ? d.r * 1.15 + 8 : d.r + 3)).strength(0.9))
    .force("center", forceCenter(0, 0))
    .force("x", forceX<N>((d) => (clustered ? anchor.get(d.group)!.x : 0)).strength((d) => pull(d, clustered ? 0.06 : 0.02)))
    .force("y", forceY<N>((d) => (clustered ? anchor.get(d.group)!.y : 0)).strength((d) => pull(d, clustered ? 0.06 : 0.02)))
    .alphaDecay(0.035)
    .on("tick", kick)
    .on("end", () => fit(false));
  // 중심 노드는 가운데에 고정
  if (mode !== "overview" && nodes[0] && mode !== "memory") {
    nodes[0].fx = 0;
    nodes[0].fy = 0;
  }
  // 처음 몇 번은 그리지 않고 계산만 (첫 화면이 덜 흔들리게)
  trace("layout: tick");
  sim.tick(Math.min(120, 30 + nodes.length / 4));
  trace(`layout: fit ${nodes.map((n) => `${Math.round(n.x ?? NaN)},${Math.round(n.y ?? NaN)}`).join(" ")}`);
  fit(false);
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
  if (!nodes.length) return;
  const el = canvas();
  const { x0, y0, x1, y1 } = bounds();
  const w = el.clientWidth, hgt = el.clientHeight;
  const k = Math.max(0.2, Math.min(2.2, Math.min(w / (x1 - x0 + 80), hgt / (y1 - y0 + 80))));
  trace(`fit w=${w} h=${hgt} k=${k} box=${x0},${y0},${x1},${y1}`);
  const target = { k, x: w / 2 - ((x0 + x1) / 2) * k, y: hgt / 2 - ((y0 + y1) / 2) * k };
  if (!animate) {
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

function draw() {
  const el = canvas();
  const dpr = window.devicePixelRatio || 1;
  const w = el.clientWidth, hgt = el.clientHeight;
  if (el.width !== Math.round(w * dpr) || el.height !== Math.round(hgt * dpr)) {
    el.width = Math.round(w * dpr);
    el.height = Math.round(hgt * dpr);
  }
  const ctx = el.getContext("2d")!;
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, w, hgt);
  ctx.translate(view.x, view.y);
  ctx.scale(view.k, view.k);

  const focusSet = selected ? neighborsOf(selected) : hovered ? neighborsOf(hovered) : null;
  const fpActive = mode !== "impact" && mode !== "memory" && overlays.footprint && footprintSize() > 0;
  const dimmed = (n: N) => (focusSet ? !focusSet.has(n.id) : fpActive ? !n.mark && n.kind !== "memory" : false);

  trace(`draw k=${view.k} w=${w} h=${hgt}`);
  // 간선
  for (const l of links) {
    const a = l.source as N, b = l.target as N;
    if (a.x === undefined || b.x === undefined) continue;
    const lit = focusSet ? focusSet.has(a.id) && focusSet.has(b.id) : !(dimmed(a) && dimmed(b));
    ctx.beginPath();
    ctx.moveTo(a.x, a.y!);
    ctx.lineTo(b.x, b.y!);
    ctx.setLineDash(l.kind === "cochange" ? [5, 4] : l.kind === "mentions" ? [2, 3] : []);
    const base = l.kind === "cochange" ? colors.context : l.kind === "mentions" ? colors.link : colors.border;
    ctx.strokeStyle = withAlpha(base, lit ? (l.kind === "defines" ? 0.35 : 0.7) : 0.12);
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
    ctx.globalAlpha = dim ? 0.22 : 1;
    drawNodeShape(ctx, n);
    ctx.fillStyle = colors.bg;
    ctx.fill();
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
      const p = (pulse % 1200) / 1200;
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
  const score = (n: N) => (n === selected ? 1e9 : n === hovered ? 1e8 : 0) + (focusSet?.has(n.id) ? 1e6 : 0) + (n.mark ? 1e5 : 0) + (n.kind === "dir" ? 1e4 : 0) + n.r * 100;
  // 흐리게 처리된 노드도 이름은 옅게 남긴다 (어디쯤인지 알 수 있게). 대신 가장 나중에 자리를 얻는다
  const rank = (n: N) => score(n) - (dimmed(n) && n !== hovered ? 5e5 : 0);
  const candidates = nodes.filter((n) => n.x !== undefined).sort((a, b) => rank(b) - rank(a));
  const placed: { x0: number; y0: number; x1: number; y1: number }[] = [];
  const pad = 2 / view.k;
  for (const n of candidates) {
    const must = n === selected || n === hovered;
    if (!must && n.kind !== "dir" && n.r * view.k < 6 && !n.mark && !focusSet?.has(n.id)) continue;
    const text = n.kind === "memory" ? (n.detail ?? n.label).slice(0, 28) + ((n.detail?.length ?? 0) > 28 ? "…" : "") : n.label;
    const w = ctx.measureText(text).width;
    const y = n.y! + (n.kind === "dir" ? n.r * 0.8 : n.r) + 3 / view.k;
    const box = { x0: n.x! - w / 2 - pad, y0: y - pad, x1: n.x! + w / 2 + pad, y1: y + fs + pad };
    if (!must && placed.some((b) => box.x0 < b.x1 && b.x0 < box.x1 && box.y0 < b.y1 && b.y0 < box.y1)) continue;
    placed.push(box);
    ctx.globalAlpha = dimmed(n) && n !== hovered ? 0.45 : 1;
    ctx.lineWidth = 3 / view.k;
    ctx.strokeStyle = colors.bg;
    ctx.strokeText(text, n.x!, y);
    ctx.fillStyle = n === selected ? colors.strong : n.kind === "dir" ? colors.strong : colors.fg;
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
  if (!on) return kick();
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
        const b = h("button", { class: "crumb-link" }, codicon("type-hierarchy"), t("전체 구조"));
        b.addEventListener("click", () => void showOverview());
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
      h("span", { class: `risk risk-${impact.risk}` }, `${t("위험도")} ${t(RISK[impact.risk])}`),
      h("span", {}, h("b", {}, String(impact.callers)), t("직접 호출")),
      h("span", {}, h("b", {}, String(impact.callers2)), t("간접 호출")),
      h("span", {}, h("b", {}, String(impact.tests.length)), t("테스트")));
    fp.classList.remove("hidden");
  } else fp.classList.add("hidden");
}

function renderCard() {
  const card = $("#map-card");
  const n = selected;
  if (!n) {
    card.classList.add("hidden");
    return;
  }
  const inE = links.filter((l) => (l.target as N).id === n.id);
  const outE = links.filter((l) => (l.source as N).id === n.id);
  const count = (list: L[], kinds: EdgeKind[]) => list.filter((l) => kinds.includes(l.kind)).length;
  const kindLabel = n.kind === "dir" ? t("폴더") : n.kind === "file" ? t("파일") : n.kind === "memory" ? t("기억") : (n.detail ?? t("심볼"));
  const rows: HTMLElement[] = [];
  if (n.kind === "symbol") {
    rows.push(h("div", { class: "mc-stat" }, h("span", {}, t("부르는 곳")), h("b", {}, String(count(inE, ["calls"])))));
    rows.push(h("div", { class: "mc-stat" }, h("span", {}, t("부르는 것")), h("b", {}, String(count(outE, ["calls"])))));
  } else if (n.kind === "dir") {
    rows.push(h("div", { class: "mc-stat" }, h("span", {}, t("파일")), h("b", {}, n.detail ?? "0")));
    rows.push(h("div", { class: "mc-stat" }, h("span", {}, t("참조받음")), h("b", {}, String(count(inE, ["ref"])))));
    rows.push(h("div", { class: "mc-stat" }, h("span", {}, t("참조함")), h("b", {}, String(count(outE, ["ref"])))));
  } else if (n.kind === "file") {
    rows.push(h("div", { class: "mc-stat" }, h("span", {}, t("참조받음")), h("b", {}, String(count(inE, ["ref"])))));
    rows.push(h("div", { class: "mc-stat" }, h("span", {}, t("참조함")), h("b", {}, String(count(outE, ["ref"])))));
    rows.push(h("div", { class: "mc-stat" }, h("span", {}, t("함께 바뀜")), h("b", {}, String(count([...inE, ...outE], ["cochange"])))));
  }
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
      if (Math.abs(dx) + Math.abs(dy) > 3) drag.moved = true;
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
  if (!loaded || nodes.length === 0) void showOverview();
  else kick();
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
  impact = null;
  footprint = null;
  memoryNotes = [];
  loaded = false;
  mode = "overview";
  groupHue.clear();
  if (visible) void showOverview();
}

/** 파일이 바뀌면 (인덱스 갱신 후) 지금 보는 지도를 다시 그린다 */
export function refreshSoon() {
  if (!loaded || mode === "impact") return;
  clearTimeout(refreshTimer);
  refreshTimer = window.setTimeout(async () => {
    const fetcher = mode === "overview" ? () => invoke<Graph>("graph_overview", { maxNodes: 400 }) : mode === "focus" && selected === null && nodes[0] ? () => invoke<Graph>("graph_neighborhood", { center: nodes[0].id, limit: 30 }) : null;
    if (!fetcher || !visible) return;
    try {
      let g = await fetcher();
      if (overlays.memory) g = await withMemory(g);
      setGraph(g, true);
      setStatus(g.nodes.length ? "" : t("표시할 코드가 없습니다. 인덱싱이 끝났는지 확인하세요."));
    } catch { /* 다음 변경 때 다시 */ }
  }, 1200);
}
let refreshTimer = 0;

export function init(opts: { openFile: (path: string, line?: number) => void; onVisibility: (v: boolean) => void }) {
  openFile = opts.openFile;
  onVisibility = opts.onVisibility;
  bindCanvas();
  bindSearch();
  $("#map-fit").addEventListener("click", () => fit());
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
