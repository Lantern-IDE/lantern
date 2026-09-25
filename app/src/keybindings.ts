// 단축키 바꾸기. 사용자 설정은 데이터 폴더의 keybindings.json (VS Code와 같은 모양)에 둔다:
//   [{ "key": "Ctrl+K", "command": "view.chat" }, { "key": "", "command": "view.zoomIn" }]
// key가 빈 문자열이면 그 명령의 단축키를 끈다.
import { invoke } from "@tauri-apps/api/core";
import { errorText } from "./api";
import * as commands from "./commands";
import { h } from "./dom";
import { codicon } from "./icons";
import * as editor from "./editor";
import * as toast from "./toast";
import { displayKey, fromEvent } from "./lib/keys";

interface Binding { key: string; command: string }

let overrides = new Map<string, string>();
let filePath = "";
let host: HTMLElement | null = null;
let filter = "";

export { fromEvent } from "./lib/keys";

function apply() {
  commands.applyOverrides(overrides);
}

export async function load() {
  try {
    const [path, text] = await invoke<[string, string]>("get_keybindings");
    filePath = path;
    const list = JSON.parse(text || "[]") as Binding[];
    overrides = new Map(list.filter((b) => b && typeof b.command === "string" && typeof b.key === "string").map((b) => [b.command, b.key]));
  } catch (e) {
    overrides = new Map();
    toast.warn("keybindings.json을 읽지 못해 기본 단축키를 씁니다", errorText(e));
  }
  apply();
}

async function save() {
  const list: Binding[] = [...overrides].map(([command, key]) => ({ key, command }));
  try {
    await invoke("set_keybindings", { json: JSON.stringify(list, null, 2) });
  } catch (e) {
    toast.error("단축키를 저장하지 못했습니다", errorText(e));
  }
  apply();
  render();
}

function conflictOf(key: string, id: string): commands.Command | undefined {
  const k = key.toLowerCase();
  return commands.all().find((c) => c.id !== id && c.key?.toLowerCase() === k);
}

function record(c: commands.Command, cell: HTMLElement) {
  const box = h("span", { class: "kb-recording", tabindex: "0", role: "textbox", "aria-label": `${c.label} 새 단축키` }, "새 키를 누르세요 (Esc 취소)");
  cell.replaceChildren(box);
  const done = () => {
    box.removeEventListener("keydown", onKey, true);
    render();
  };
  const onKey = (e: KeyboardEvent) => {
    e.preventDefault();
    e.stopPropagation();
    if (e.key === "Escape" && !e.ctrlKey && !e.shiftKey && !e.altKey) return done();
    const key = fromEvent(e);
    if (!key) return;
    const other = conflictOf(key, c.id);
    if (other) toast.warn(`${key}는 '${other.label}'에도 쓰이고 있습니다`, "바꾼 단축키가 우선합니다. 필요하면 다른 명령의 키를 바꾸세요.");
    overrides.set(c.id, key);
    box.removeEventListener("keydown", onKey, true);
    void save();
  };
  box.addEventListener("keydown", onKey, true);
  box.addEventListener("blur", done, { once: true });
  box.focus();
}

function render() {
  if (!host) return;
  const list = host.querySelector(".kb-list");
  if (!list) return;
  const q = filter.toLowerCase();
  const rows = commands.all()
    .filter((c) => !q || c.label.toLowerCase().includes(q) || c.id.includes(q) || (c.key ?? "").toLowerCase().includes(q))
    .sort((a, b) => a.label.localeCompare(b.label, "ko"));
  list.replaceChildren(
    ...rows.map((c) => {
      const changed = overrides.has(c.id);
      const keyCell = h("span", { class: "kb-key" }, c.key ? h("kbd", {}, displayKey(c.key)) : h("span", { class: "muted" }, "—"));
      const edit = h("button", { class: "icon-btn", title: "단축키 바꾸기", "aria-label": `${c.label} 단축키 바꾸기` }, codicon("edit"));
      edit.addEventListener("click", () => record(c, keyCell));
      const actions = h("span", { class: "kb-actions" }, edit);
      if (c.key) {
        const off = h("button", { class: "icon-btn", title: "단축키 끄기", "aria-label": `${c.label} 단축키 끄기` }, codicon("circle-slash"));
        off.addEventListener("click", () => {
          overrides.set(c.id, "");
          void save();
        });
        actions.append(off);
      }
      if (changed) {
        const reset = h("button", { class: "icon-btn", title: "기본값으로", "aria-label": `${c.label} 기본값으로` }, codicon("discard"));
        reset.addEventListener("click", () => {
          overrides.delete(c.id);
          void save();
        });
        actions.append(reset);
      }
      const row = h("div", { class: `kb-row${changed ? " changed" : ""}` },
        h("span", { class: "kb-label" }, c.label, changed ? h("span", { class: "kb-tag" }, "사용자") : null),
        keyCell,
        h("span", { class: "kb-id" }, c.id),
        actions);
      row.addEventListener("dblclick", () => record(c, keyCell));
      return row;
    }),
  );
}

export function open() {
  editor.openPage("keybindings", "단축키", "keyboard", (el) => {
    host = el;
    const search = h("input", { placeholder: "명령, 단축키로 찾기", spellcheck: "false", "aria-label": "단축키 찾기" }) as HTMLInputElement;
    search.addEventListener("input", () => {
      filter = search.value;
      render();
    });
    const resetAll = h("button", { class: "btn btn-secondary" }, "모두 기본값으로");
    resetAll.addEventListener("click", () => {
      overrides.clear();
      void save();
    });
    el.append(
      h("div", { class: "page-content kb-page scroll" },
        h("h1", {}, "단축키"),
        h("p", { class: "lead" }, "행을 두 번 클릭하거나 연필 아이콘을 눌러 새 키를 지정합니다. 바꾼 내용은 ", h("code", {}, filePath || "keybindings.json"), "에 저장되고, 파일을 직접 고쳐도 됩니다."),
        h("div", { class: "kb-toolbar" }, search, resetAll),
        h("div", { class: "kb-head" }, h("span", {}, "명령"), h("span", {}, "단축키"), h("span", {}, "ID"), h("span")),
        h("div", { class: "kb-list" })),
    );
    render();
    search.focus();
  });
}
