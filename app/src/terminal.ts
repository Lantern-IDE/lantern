// 통합 터미널 (xterm.js ↔ Rust PTY)
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import { api, errorText, on } from "./api";
import { $ } from "./dom";
import * as prefs from "./prefs";

const DARK = {
  background: "#16171c", foreground: "#d5d8e0", cursor: "#9aa3ff", cursorAccent: "#16171c", selectionBackground: "rgba(108,115,245,0.35)",
  black: "#1b1c22", red: "#f2656a", green: "#45bd86", yellow: "#dcae3a", blue: "#6aa8f8", magenta: "#c586c0", cyan: "#2cc6e0", white: "#d5d8e0",
  brightBlack: "#6f7585", brightRed: "#ff8a8e", brightGreen: "#6fd6a4", brightYellow: "#f0c65a", brightBlue: "#8fbfff", brightMagenta: "#dba6d6", brightCyan: "#6fdcee", brightWhite: "#f0f1f5",
};
const LIGHT = {
  background: "#f2f3f7", foreground: "#2a2d37", cursor: "#4f55d8", cursorAccent: "#f2f3f7", selectionBackground: "rgba(79,85,216,0.22)",
  black: "#2a2d37", red: "#c9353b", green: "#1d8a57", yellow: "#8a6100", blue: "#2a6fd1", magenta: "#a0309a", cyan: "#0b7f94", white: "#5d6272",
  brightBlack: "#6f7585", brightRed: "#d94a50", brightGreen: "#25a067", brightYellow: "#9a6b00", brightBlue: "#3d7fe0", brightMagenta: "#b045a8", brightCyan: "#1592a8", brightWhite: "#14161c",
};
prefs.onChange(({ dark }) => {
  if (term) term.options.theme = dark ? DARK : LIGHT;
});

let term: Terminal | null = null;
let fit: FitAddon | null = null;
let id: number | null = null;

on<{ id: number; data: string }>("term-data", (p) => {
  if (p.id === id) term?.write(p.data);
});
on<{ id: number }>("term-exit", (p) => {
  if (p.id === id) {
    term?.write("\r\n\x1b[90m[셸이 종료되었습니다. 패널을 다시 열면 새로 시작합니다]\x1b[0m\r\n");
    id = null;
  }
});

function ensureTerminal() {
  if (term) return;
  term = new Terminal({
    fontFamily: '"Cascadia Mono", Consolas, "Courier New", monospace',
    fontSize: 13,
    lineHeight: 1.2,
    cursorBlink: true,
    theme: prefs.isDark() ? DARK : LIGHT,
    allowProposedApi: false,
  });
  fit = new FitAddon();
  term.loadAddon(fit);
  term.open($("#terminal"));
  term.onData((d) => {
    if (id !== null) void api.termWrite(id, d);
  });
  new ResizeObserver(() => resize()).observe($("#terminal"));
}

function resize() {
  if (!term || !fit || !$("#terminal").offsetParent) return;
  try {
    fit.fit();
  } catch {
    return;
  }
  if (id !== null) void api.termResize(id, term.cols, term.rows);
}

/** 터미널 패널이 보일 때 호출. 셸이 없으면 새로 띄운다 (프로젝트 폴더에서). */
export async function show() {
  ensureTerminal();
  resize();
  if (id === null && term) {
    try {
      id = await api.termSpawn(term.cols || 80, term.rows || 24);
    } catch (e) {
      term.write(`터미널을 시작하지 못했습니다: ${errorText(e)}\r\n`);
    }
  }
  term?.focus();
}

/** 프로젝트가 바뀌면 셸을 새 폴더에서 다시 시작한다. */
export async function restart() {
  if (id !== null) await api.termKill(id).catch(() => {});
  id = null;
  term?.reset();
}
