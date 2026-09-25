// 파일 종류별 아이콘 (Codicons) 과 Lantern 육각형 표시
import { h } from "./dom";

const BY_EXT: Record<string, [string, string]> = {
  ts: ["file-code", "#3178c6"],
  mts: ["file-code", "#3178c6"],
  cts: ["file-code", "#3178c6"],
  tsx: ["file-code", "#3178c6"],
  js: ["file-code", "#c9b83b"],
  mjs: ["file-code", "#c9b83b"],
  cjs: ["file-code", "#c9b83b"],
  jsx: ["file-code", "#c9b83b"],
  py: ["file-code", "#4b8bbe"],
  rs: ["file-code", "#d9844a"],
  go: ["file-code", "#3fa7c9"],
  java: ["file-code", "#c9474d"],
  html: ["file-code", "#e0703a"],
  css: ["file-code", "#4b8bbe"],
  scss: ["file-code", "#d9558a"],
  json: ["json", "#c9b83b"],
  md: ["markdown", "#4b8bbe"],
  toml: ["settings", "#7fad4a"],
  yaml: ["settings", "#9a72c4"],
  yml: ["settings", "#9a72c4"],
  lock: ["lock", "#8a8f9e"],
  svg: ["file-media", "#9a72c4"],
  png: ["file-media", "#9a72c4"],
  jpg: ["file-media", "#9a72c4"],
  ico: ["file-media", "#9a72c4"],
  pdf: ["file-pdf", "#c9474d"],
  zip: ["file-zip", "#8a8f9e"],
  sql: ["file-code", "#d9558a"],
  sh: ["terminal", "#7fad4a"],
  ps1: ["terminal", "#4b8bbe"],
};

const BY_NAME: Record<string, [string, string]> = {
  ".gitignore": ["git-commit", "#8a8f9e"],
  "Cargo.toml": ["settings", "#d9844a"],
  "package.json": ["json", "#7fad4a"],
  "tsconfig.json": ["json", "#3178c6"],
  "config.toml": ["settings-gear", "#2cc6e0"],
};

export function fileIcon(name: string): HTMLElement {
  const base = name.split("/").pop() ?? name;
  const ext = base.includes(".") ? base.split(".").pop()!.toLowerCase() : "";
  const [icon, color] = BY_NAME[base] ?? BY_EXT[ext] ?? ["file", "#8a8f9e"];
  return h("span", { class: "ficon" }, h("i", { class: `codicon codicon-${icon}`, style: `color:${color}` }));
}

export function codicon(name: string, extra = ""): HTMLElement {
  return h("i", { class: `codicon codicon-${name}${extra ? " " + extra : ""}`, "aria-hidden": "true" });
}

const SVG = "http://www.w3.org/2000/svg";

/**
 * Lantern 육각형 표시. brand = 브랜드 그라데이션, plain = 현재 글자색.
 * working이면 윤곽을 따라 선이 돈다 (AI가 일하는 중에만 쓰는 유일한 반복 애니메이션).
 */
export function hex(variant: "brand" | "plain" = "plain", working = false): HTMLElement {
  const span = h("span", { class: `hex${variant === "brand" ? " brand" : ""}${working ? " working" : ""}`, "aria-hidden": "true" });
  const svg = document.createElementNS(SVG, "svg");
  svg.setAttribute("viewBox", "0 0 16 16");
  const poly = document.createElementNS(SVG, "polygon");
  poly.setAttribute("class", "hex-edge");
  poly.setAttribute("points", "8,1.3 13.8,4.65 13.8,11.35 8,14.7 2.2,11.35 2.2,4.65");
  poly.setAttribute("pathLength", "66");
  const core = document.createElementNS(SVG, "circle");
  core.setAttribute("class", "hex-core");
  core.setAttribute("cx", "8");
  core.setAttribute("cy", "8");
  core.setAttribute("r", "2.1");
  svg.append(poly, core);
  span.append(svg);
  return span;
}

export function setWorking(el: Element | null, working: boolean) {
  el?.classList.toggle("working", working);
}

/** HTML의 data-hex 자리에 육각형 표시를 채운다 */
export function mountHexes(root: ParentNode = document) {
  for (const el of root.querySelectorAll<HTMLElement>("[data-hex]")) {
    el.append(hex(el.dataset.hex === "brand" ? "brand" : "plain"));
    el.removeAttribute("data-hex");
  }
}
