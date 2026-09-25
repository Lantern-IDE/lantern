// 명령 목록. 메뉴, 명령 팔레트(Ctrl+Shift+P), 단축키가 모두 여기를 쓴다.
import { $, h } from "./dom";
import { displayKey } from "./lib/keys";

export interface Command {
  id: string;
  label: string;
  /** 표시용 단축키. 예: "Ctrl+Shift+P" */
  key?: string;
  run: () => unknown;
}

const commands = new Map<string, Command>();
/** 코드에 적힌 기본 단축키 (사용자가 바꾼 뒤에도 되돌릴 수 있게) */
const defaults = new Map<string, string | undefined>();
/** 사용자가 바꾼 명령 id — 키가 겹치면 이쪽이 먼저 */
let userIds = new Set<string>();

export function register(list: Command[]) {
  for (const c of list) {
    commands.set(c.id, c);
    defaults.set(c.id, c.key);
  }
}

/** keybindings.json 적용. 빈 문자열은 단축키 끄기 */
export function applyOverrides(map: Map<string, string>) {
  userIds = new Set(map.keys());
  for (const c of commands.values()) {
    c.key = map.has(c.id) ? map.get(c.id) || undefined : defaults.get(c.id);
  }
}

export function all(): Command[] {
  return [...commands.values()];
}

export function run(id: string) {
  const c = commands.get(id);
  if (c) void c.run();
}

/** 단축키 문자열이 키 이벤트와 맞는지 */
function matches(key: string, e: KeyboardEvent): boolean {
  const parts = key.split("+");
  const main = parts.pop()!.toLowerCase();
  const want = { ctrl: parts.includes("Ctrl"), shift: parts.includes("Shift"), alt: parts.includes("Alt") };
  const pressed = e.key.length === 1 ? e.key.toLowerCase() : e.key.toLowerCase();
  const code = e.code.startsWith("Key") ? e.code.slice(3).toLowerCase() : e.code === "Backquote" ? "`" : pressed;
  return (e.ctrlKey || e.metaKey) === want.ctrl && e.shiftKey === want.shift && e.altKey === want.alt && (code === main || pressed === main);
}

export function installKeybindings() {
  window.addEventListener(
    "keydown",
    (e) => {
      if (!(e.ctrlKey || e.metaKey || e.altKey) && !e.key.startsWith("F")) return;
      const ordered = [...commands.values()].sort((a, b) => Number(userIds.has(b.id)) - Number(userIds.has(a.id)));
      for (const c of ordered) {
        if (c.key && matches(c.key, e)) {
          e.preventDefault();
          e.stopPropagation();
          closeMenu();
          void c.run();
          return;
        }
      }
    },
    true,
  );
}

// ── 메뉴 표시줄 ────────────────────────────────────────

type MenuItem = string | "-";
let openMenu: HTMLElement | null = null;

function closeMenu() {
  $("#menu-popup").classList.add("hidden");
  openMenu?.classList.remove("open");
  openMenu = null;
}

function showMenu(btn: HTMLElement, items: MenuItem[]) {
  const pop = $("#menu-popup");
  pop.replaceChildren(
    ...items.map((id) => {
      if (id === "-") return h("div", { class: "menu-sep" });
      if (id.startsWith("#")) return h("div", { class: "menu-head" }, id.slice(1));
      const c = commands.get(id);
      if (!c) return h("div");
      const el = h("div", { class: "menu-item" }, c.label, c.key ? h("span", { class: "key" }, displayKey(c.key)) : null);
      el.addEventListener("mousedown", (e) => {
        e.preventDefault();
        closeMenu();
        void c.run();
      });
      return el;
    }),
  );
  const r = btn.getBoundingClientRect();
  pop.style.left = `${r.left}px`;
  pop.style.top = `${r.bottom}px`;
  pop.classList.remove("hidden");
  openMenu?.classList.remove("open");
  openMenu = btn;
  btn.classList.add("open");
}

export interface MenuEntry {
  label: string;
  key?: string;
  run: () => unknown;
  disabled?: boolean;
}

/** 오른쪽 클릭 메뉴. 메뉴 표시줄과 같은 팝업을 쓴다. */
export function showContextMenu(x: number, y: number, items: (MenuEntry | "-")[]) {
  const pop = $("#menu-popup");
  pop.replaceChildren(
    ...items.map((it) => {
      if (it === "-") return h("div", { class: "menu-sep" });
      const el = h("div", { class: `menu-item${it.disabled ? " disabled" : ""}`, role: "menuitem" }, it.label, it.key ? h("span", { class: "key" }, displayKey(it.key)) : null);
      if (!it.disabled) {
        el.addEventListener("mousedown", (e) => {
          e.preventDefault();
          closeMenu();
          void it.run();
        });
      }
      return el;
    }),
  );
  openMenu?.classList.remove("open");
  openMenu = pop; // 바깥 클릭·Esc로 닫히게 같은 상태를 쓴다
  pop.classList.remove("hidden");
  const r = pop.getBoundingClientRect();
  pop.style.left = `${Math.min(x, innerWidth - r.width - 4)}px`;
  pop.style.top = `${Math.min(y, innerHeight - r.height - 4)}px`;
}

export function installMenubar(menus: { title: string; items: MenuItem[] }[]) {
  const bar = $("#menubar");
  // 좁은 창에서는 메뉴를 버튼 하나로 접는다 (VS Code와 같은 방식).
  const compact = h("button", { class: "menu-compact", title: "메뉴", "aria-label": "메뉴" }, h("i", { class: "codicon codicon-menu" }));
  const all: MenuItem[] = menus.flatMap((m, i) => [...(i ? ["-"] : []), `#${m.title}`, ...m.items.filter((x) => x !== "-")]);
  compact.addEventListener("mousedown", (e) => {
    e.preventDefault();
    if (openMenu === compact) closeMenu();
    else showMenu(compact, all);
  });
  bar.append(compact);
  for (const m of menus) {
    const btn = h("button", {}, m.title);
    btn.addEventListener("mousedown", (e) => {
      e.preventDefault();
      if (openMenu === btn) closeMenu();
      else showMenu(btn, m.items);
    });
    btn.addEventListener("mouseenter", () => {
      if (openMenu && openMenu !== btn) showMenu(btn, m.items);
    });
    bar.append(btn);
  }
  // 메뉴가 들어갈 자리가 없으면 접는다. 글자 폭(언어·글꼴)에 따라 달라서 화면 폭 기준이 아니라 실제로 잰다.
  const fit = () => {
    bar.classList.remove("compact");
    const left = bar.parentElement!;
    const room = left.clientWidth - (left.firstElementChild as HTMLElement).offsetWidth - 8;
    bar.classList.toggle("compact", bar.scrollWidth > room);
  };
  new ResizeObserver(fit).observe($("#titlebar"));
  new MutationObserver(fit).observe(bar, { subtree: true, characterData: true, childList: true });
  void document.fonts.ready.then(fit);
  window.addEventListener("mousedown", (e) => {
    const t = e.target;
    if (!openMenu) return;
    if (!(t instanceof Node) || (!$("#menu-popup").contains(t) && !bar.contains(t))) closeMenu();
  });
  window.addEventListener("contextmenu", (e) => {
    // 앱 안에서는 브라우저 기본 메뉴(새로 고침, 검사 등)를 띄우지 않는다. 입력창은 예외.
    const t = e.target as HTMLElement;
    if (!t.closest("input, textarea, .cm-content")) e.preventDefault();
  });
  window.addEventListener("keydown", (e) => {
    if (e.key === "Escape" && openMenu) closeMenu();
  });
  window.addEventListener("blur", closeMenu);
}
