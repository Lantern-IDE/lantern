// 화면 환경설정 (테마, 편집기 글꼴 크기). 이 컴퓨터에만 저장되는 개인 설정이라 localStorage에 둔다.
import { store } from "./dom";

export type ThemePref = "system" | "dark" | "light";

interface Prefs {
  theme: ThemePref;
  fontSize: number;
}

const prefs: Prefs = { theme: "system", fontSize: 14, ...store.get<Partial<Prefs>>("prefs", {}) };
const listeners: ((p: { dark: boolean; fontSize: number }) => void)[] = [];
const media = window.matchMedia("(prefers-color-scheme: dark)");

export function get(): Prefs {
  return { ...prefs };
}

export function isDark(): boolean {
  return prefs.theme === "dark" || (prefs.theme === "system" && media.matches);
}

function apply() {
  const dark = isDark();
  document.documentElement.dataset.theme = dark ? "dark" : "light";
  document.documentElement.style.setProperty("--editor-font-size", `${prefs.fontSize}px`);
  listeners.forEach((fn) => fn({ dark, fontSize: prefs.fontSize }));
}

export function set(patch: Partial<Prefs>) {
  Object.assign(prefs, patch);
  store.set("prefs", prefs);
  apply();
}

export function onChange(fn: (p: { dark: boolean; fontSize: number }) => void) {
  listeners.push(fn);
}

export function init() {
  media.addEventListener("change", () => prefs.theme === "system" && apply());
  apply();
}
