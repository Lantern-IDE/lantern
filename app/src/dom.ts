// 작은 DOM 헬퍼. 프레임워크 없이 화면을 만든다.

type Attrs = Record<string, string | number | boolean | EventListener | undefined>;
type Child = Node | string | null | undefined | false;

export function h<K extends keyof HTMLElementTagNameMap>(tag: K, attrs: Attrs = {}, ...children: Child[]): HTMLElementTagNameMap[K] {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (v === undefined || v === false) continue;
    if (k.startsWith("on") && typeof v === "function") el.addEventListener(k.slice(2), v);
    else if (k === "class") el.className = String(v);
    else el.setAttribute(k, v === true ? "" : String(v));
  }
  for (const c of children) {
    if (c === null || c === undefined || c === false) continue;
    el.append(typeof c === "string" ? document.createTextNode(c) : c);
  }
  return el;
}

export function $<T extends HTMLElement = HTMLElement>(sel: string): T {
  const el = document.querySelector<T>(sel);
  if (!el) throw new Error(`요소 없음: ${sel}`);
  return el;
}

export function basename(path: string): string {
  return path.split("/").pop() ?? path;
}

export function dirname(path: string): string {
  const i = path.lastIndexOf("/");
  return i < 0 ? "" : path.slice(0, i);
}

/** 설정·편의용 localStorage. 막혀 있어도 앱은 동작해야 한다. */
export const store = {
  get<T>(key: string, fallback: T): T {
    try {
      const v = localStorage.getItem(key);
      return v === null ? fallback : (JSON.parse(v) as T);
    } catch {
      return fallback;
    }
  },
  set(key: string, value: unknown) {
    try {
      localStorage.setItem(key, JSON.stringify(value));
    } catch {
      /* 저장 실패는 무시 */
    }
  },
};

/** unified diff 텍스트를 색이 있는 요소로 */
export function renderDiff(diff: string): HTMLElement {
  const box = h("div", { class: "diff" });
  for (const line of diff.split("\n")) {
    const cls = line.startsWith("+++") || line.startsWith("---") ? "meta"
      : line.startsWith("@@") ? "hunk"
      : line.startsWith("+") ? "add"
      : line.startsWith("-") ? "del" : "";
    box.append(h("div", { class: cls }, line || " "));
  }
  return box;
}
