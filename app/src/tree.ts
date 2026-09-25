// 파일 트리 (VS Code 탐색기). 폴더는 펼칠 때 불러온다.
// 선택, 오른쪽 클릭 메뉴, 트리 안에서 이름 입력(새 파일·새 폴더·이름 바꾸기), 키보드(F2, Delete, Enter, 방향키).
import { ask } from "./dialog";
import { invoke } from "@tauri-apps/api/core";
import { api, errorText, type Entry } from "./api";
import { $, basename, dirname, h } from "./dom";
import { codicon, fileIcon } from "./icons";
import { showContextMenu } from "./commands";
import * as toast from "./toast";

const INDENT = 8;
const expanded = new Set<string>();
let openHandler: (path: string) => void = () => {};
let changeHandler: (ev: { kind: "rename" | "delete"; from: string; to?: string }) => void = () => {};
let activePath: string | null = null;
let selected: { path: string; isDir: boolean } | null = null;

export function onOpen(fn: (path: string) => void) {
  openHandler = fn;
}

/** 이름 바꾸기·삭제를 편집기 탭에 알린다 */
export function onPathChange(fn: typeof changeHandler) {
  changeHandler = fn;
}

function select(path: string, isDir: boolean) {
  selected = { path, isDir };
  for (const el of document.querySelectorAll<HTMLElement>("#tree .node")) el.classList.toggle("selected", el.dataset.path === path);
}

function row(e: Entry, depth: number, isOpen: boolean): HTMLElement {
  const r = h(
    "div",
    {
      class: `node${e.name.startsWith(".") ? " dim" : ""}${e.path === activePath ? " active" : ""}${selected?.path === e.path ? " selected" : ""}`,
      style: `padding-left:${depth * INDENT + 8}px`,
      title: e.path,
      "data-path": e.path,
      "data-dir": e.is_dir ? "1" : "",
      role: "treeitem",
      "aria-expanded": e.is_dir ? String(isOpen) : undefined,
    },
    h("span", { class: "twisty" }, e.is_dir ? codicon(isOpen ? "chevron-down" : "chevron-right") : null),
    e.is_dir ? null : fileIcon(e.name),
    h("span", { class: "fname" }, e.name),
  );
  for (let i = 0; i < depth; i++) r.append(h("span", { class: "indent-guide", style: `left:${i * INDENT + 15}px` }));
  return r;
}

function node(e: Entry, depth: number): HTMLElement {
  const wrap = h("div", { "data-wrap": e.path });
  const r = row(e, depth, expanded.has(e.path));
  const children = h("div", { class: "children", "data-depth": String(depth + 1) });
  wrap.append(r, children);

  const load = async () => {
    children.replaceChildren();
    try {
      const list = await api.listDir(e.path);
      if (!list.length) children.append(h("div", { class: "node empty", style: `padding-left:${(depth + 1) * INDENT + 24}px` }, "비어 있음"));
      for (const c of list) children.append(node(c, depth + 1));
    } catch (err) {
      children.append(h("div", { class: "node dim", style: `padding-left:${(depth + 1) * INDENT + 24}px` }, errorText(err)));
    }
  };
  if (e.is_dir && expanded.has(e.path)) void load();

  r.addEventListener("click", () => {
    select(e.path, e.is_dir);
    if (!e.is_dir) return openHandler(e.path);
    if (expanded.has(e.path)) {
      expanded.delete(e.path);
      children.replaceChildren();
    } else {
      expanded.add(e.path);
      void load();
    }
    r.setAttribute("aria-expanded", String(expanded.has(e.path)));
    r.querySelector(".twisty")!.replaceChildren(codicon(expanded.has(e.path) ? "chevron-down" : "chevron-right"));
  });
  r.addEventListener("contextmenu", (ev) => {
    ev.preventDefault();
    select(e.path, e.is_dir);
    menuFor(e.path, e.is_dir, ev.clientX, ev.clientY);
  });
  return wrap;
}

function menuFor(path: string, isDir: boolean, x: number, y: number) {
  const base = isDir ? path : dirname(path);
  showContextMenu(x, y, [
    { label: "새 파일…", run: () => startCreate(base, false) },
    { label: "새 폴더…", run: () => startCreate(base, true) },
    "-",
    { label: "파일 탐색기에서 보기", run: () => invoke("reveal_path", { path }).catch((e) => toast.error("열 수 없습니다", errorText(e))) },
    { label: "상대 경로 복사", run: () => navigator.clipboard.writeText(path).then(() => toast.info("경로를 복사했습니다", path)) },
    "-",
    { label: "이름 바꾸기…", key: "F2", run: () => startRename(path) },
    { label: "삭제", key: "Delete", run: () => remove(path, isDir) },
  ]);
}

/** 트리 안에 입력칸을 띄운다. Enter면 onCommit, Esc나 포커스를 잃으면 취소. */
function inlineInput(host: HTMLElement, depth: number, initial: string, icon: HTMLElement | null, onCommit: (name: string) => Promise<boolean>) {
  const input = h("input", { class: "rename-input", value: initial, spellcheck: "false", "aria-label": "이름" }) as HTMLInputElement;
  const r = h("div", { class: "node", style: `padding-left:${depth * INDENT + 8}px` }, h("span", { class: "twisty" }), icon, input);
  host.replaceWith(r);
  let done = false;
  const finish = async (commit: boolean) => {
    if (done) return;
    done = true;
    const name = input.value.trim();
    if (commit && name && name !== initial) {
      if (!(await onCommit(name))) {
        done = false;
        input.focus();
        return;
      }
    }
    r.replaceWith(host);
    void refresh();
  };
  input.addEventListener("keydown", (ev) => {
    ev.stopPropagation();
    if (ev.key === "Enter") void finish(true);
    else if (ev.key === "Escape") void finish(false);
  });
  input.addEventListener("blur", () => void finish(true));
  input.focus();
  // 확장자 앞까지만 선택 (VS Code와 같게)
  const dot = initial.lastIndexOf(".");
  input.setSelectionRange(0, dot > 0 ? dot : initial.length);
}

async function startCreate(dir: string, isDir: boolean) {
  const target = dir === "" || dir === "." ? "" : dir;
  if (target && !expanded.has(target)) {
    expanded.add(target);
    await refresh();
  }
  const container = target ? document.querySelector<HTMLElement>(`[data-wrap="${CSS.escape(target)}"] > .children`) : $("#tree");
  if (!container) return;
  const depth = target ? Number(container.dataset.depth ?? 1) : 0;
  const placeholder = h("div");
  container.prepend(placeholder);
  inlineInput(placeholder, depth, "", isDir ? null : h("span", { class: "ficon" }, codicon("file")), async (name) => {
    const path = target ? `${target}/${name}` : name;
    try {
      await invoke("create_path", { path, dir: isDir });
      if (!isDir) openHandler(path);
      return true;
    } catch (e) {
      toast.error(`${name}을(를) 만들 수 없습니다`, errorText(e));
      return false;
    }
  });
}

function startRename(path: string) {
  const r = document.querySelector<HTMLElement>(`#tree .node[data-path="${CSS.escape(path)}"]`);
  if (!r) return;
  const depth = Math.round((parseInt(r.style.paddingLeft) - 8) / INDENT);
  const isDir = r.dataset.dir === "1";
  inlineInput(r, depth, basename(path), isDir ? null : fileIcon(path), async (name) => {
    const dir = dirname(path);
    const to = dir ? `${dir}/${name}` : name;
    try {
      await invoke("rename_path", { from: path, to });
      if (expanded.delete(path)) expanded.add(to);
      changeHandler({ kind: "rename", from: path, to });
      return true;
    } catch (e) {
      toast.error("이름을 바꿀 수 없습니다", errorText(e));
      return false;
    }
  });
}

async function remove(path: string, isDir: boolean) {
  const ok = await ask(`'${basename(path)}'${isDir ? " 폴더와 그 안의 모든 파일" : ""}을(를) 휴지통으로 옮길까요?`, {
    title: "삭제",
    kind: "warning",
    okLabel: "휴지통으로 이동",
    cancelLabel: "취소",
  });
  if (!ok) return;
  try {
    await invoke("delete_path", { path });
    expanded.delete(path);
    changeHandler({ kind: "delete", from: path });
    if (selected?.path === path) selected = null;
    await refresh();
  } catch (e) {
    toast.error("삭제하지 못했습니다", errorText(e));
  }
}

/** 루트부터 다시 그린다. 펼친 폴더는 유지한다. */
export async function refresh() {
  const tree = $("#tree");
  const scroll = tree.scrollTop;
  try {
    const entries = await api.listDir(".");
    tree.replaceChildren(...entries.map((e) => node(e, 0)));
    tree.scrollTop = scroll;
  } catch (e) {
    tree.replaceChildren(h("div", { class: "node dim" }, errorText(e)));
  }
}

export function collapseAll() {
  expanded.clear();
  void refresh();
}

export function reset() {
  expanded.clear();
  activePath = null;
  selected = null;
}

/** 새 파일·새 폴더 (탐색기 제목 줄 버튼과 명령) */
export function newFile(isDir = false) {
  const base = selected ? (selected.isDir ? selected.path : dirname(selected.path)) : "";
  void startCreate(base, isDir);
}

/** 현재 파일을 트리에서 표시한다. 조상 폴더를 펼쳐 보이게 한다. */
export async function reveal(path: string | null) {
  activePath = path;
  if (path) {
    const parts = path.split("/");
    let changed = false;
    for (let i = 1; i < parts.length; i++) {
      const dir = parts.slice(0, i).join("/");
      if (!expanded.has(dir)) {
        expanded.add(dir);
        changed = true;
      }
    }
    if (changed) await refresh();
  }
  for (const el of document.querySelectorAll<HTMLElement>("#tree .node")) {
    const on = el.dataset.path === path;
    el.classList.toggle("active", on);
    if (on) el.scrollIntoView({ block: "nearest" });
  }
}

export function init() {
  const tree = $("#tree");
  tree.addEventListener("keydown", (e) => {
    if ((e.target as HTMLElement).tagName === "INPUT") return;
    const rows = [...tree.querySelectorAll<HTMLElement>(".node[data-path]")];
    const idx = rows.findIndex((r) => r.dataset.path === selected?.path);
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const next = rows[Math.max(0, Math.min(rows.length - 1, idx + (e.key === "ArrowDown" ? 1 : -1)))];
      if (next) {
        select(next.dataset.path!, next.dataset.dir === "1");
        next.scrollIntoView({ block: "nearest" });
      }
    } else if (!selected) {
      return;
    } else if (e.key === "Enter") {
      e.preventDefault();
      rows[idx]?.click();
    } else if (e.key === "F2") {
      e.preventDefault();
      startRename(selected.path);
    } else if (e.key === "Delete") {
      e.preventDefault();
      void remove(selected.path, selected.isDir);
    }
  });
  tree.addEventListener("contextmenu", (e) => {
    if ((e.target as HTMLElement).closest(".node[data-path]")) return;
    e.preventDefault();
    showContextMenu(e.clientX, e.clientY, [
      { label: "새 파일…", run: () => startCreate("", false) },
      { label: "새 폴더…", run: () => startCreate("", true) },
    ]);
  });
}
