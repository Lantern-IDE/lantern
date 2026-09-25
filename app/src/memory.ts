// 프로젝트 기억 보기. `.lantern/memory/<주제>.md`의 목록 줄 하나가 기억 하나다.
// 에이전트가 remember 도구로 쌓은 규칙·결정을 여기서 읽고, 고치고, 지우고, 직접 더한다.
// 기억마다 언급한 코드(심볼·파일)를 칩으로 보여주고 지도에서 연결을 볼 수 있다.
import { invoke } from "@tauri-apps/api/core";
import { api, errorText } from "./api";
import { ask } from "./dialog";
import { $, basename, h } from "./dom";
import { codicon } from "./icons";
import type { Graph, MemoryNote } from "./map";
import * as toast from "./toast";

let hooks: { openFile: (path: string, line?: number) => void; focusNode: (id: string, label: string) => void; showMap: () => void; readOnly: () => boolean } = {
  openFile: () => {}, focusNode: () => {}, showMap: () => {}, readOnly: () => false,
};
let busy = false;

async function rewrite(file: string, fn: (lines: string[]) => string[]) {
  const text = await api.readFile(file).catch(() => "");
  const next = fn(text.split("\n")).join("\n");
  await api.writeFile(file, next.endsWith("\n") ? next : `${next}\n`);
}

function topicFile(topic: string): string {
  const clean = topic.trim().replace(/[^\p{L}\p{N}_-]+/gu, "-").replace(/^-+|-+$/g, "") || "notes";
  return `.lantern/memory/${clean}.md`;
}

function noteRow(n: MemoryNote, labels: Map<string, string>): HTMLElement {
  const text = h("div", { class: "mem-text" }, n.text);
  const chips = h("div", { class: "mem-links" }, ...n.links.map((id) => {
    const c = h("button", { class: `mem-chip ${id.startsWith("f:") ? "file" : "sym"}`, title: id.startsWith("f:") ? id.slice(2) : labels.get(id) ?? id },
      codicon(id.startsWith("f:") ? "file" : "symbol-method"),
      id.startsWith("f:") ? basename(id.slice(2)) : labels.get(id) ?? id);
    c.addEventListener("click", () => hooks.focusNode(id, labels.get(id) ?? id.slice(2)));
    return c;
  }));
  const edit = h("button", { class: "icon-btn", title: "고치기", "aria-label": "기억 고치기" }, codicon("edit"));
  const del = h("button", { class: "icon-btn", title: "지우기", "aria-label": "기억 지우기" }, codicon("trash"));
  const open = h("button", { class: "icon-btn", title: "파일에서 보기", "aria-label": "기억 파일에서 보기" }, codicon("go-to-file"));
  const row = h("div", { class: "mem-note" }, text, n.links.length ? chips : null, h("div", { class: "mem-actions" }, open, edit, del));
  open.addEventListener("click", () => hooks.openFile(n.file, n.line));
  edit.addEventListener("click", () => {
    if (hooks.readOnly()) return toast.warn("제한 모드에서는 기억을 고칠 수 없습니다", "파일 메뉴에서 이 폴더를 신뢰하세요.");
    const area = h("textarea", { class: "mem-edit", rows: "3", "aria-label": "기억 내용" }) as HTMLTextAreaElement;
    area.value = n.text;
    const save = async () => {
      const v = area.value.replace(/\s*\n\s*/g, " ").trim();
      if (!v || v === n.text) return void refresh();
      try {
        await rewrite(n.file, (lines) => lines.map((l, i) => (i === n.line - 1 ? l.replace(/^(\s*[-*]\s+).*$/, `$1${v}`) : l)));
        await refresh();
      } catch (e) {
        toast.error("기억을 고치지 못했습니다", errorText(e));
      }
    };
    area.addEventListener("keydown", (e) => {
      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        void save();
      } else if (e.key === "Escape") void refresh();
    });
    area.addEventListener("blur", () => void save());
    text.replaceWith(area);
    area.focus();
  });
  del.addEventListener("click", async () => {
    if (hooks.readOnly()) return toast.warn("제한 모드에서는 기억을 지울 수 없습니다", "파일 메뉴에서 이 폴더를 신뢰하세요.");
    const ok = await ask(`이 기억을 지울까요?\n\n${n.text}`, { title: "기억 지우기", kind: "warning", okLabel: "지우기", cancelLabel: "취소" });
    if (!ok) return;
    try {
      await rewrite(n.file, (lines) => lines.filter((_, i) => i !== n.line - 1));
      await refresh();
    } catch (e) {
      toast.error("기억을 지우지 못했습니다", errorText(e));
    }
  });
  return row;
}

export async function refresh() {
  if (busy) return;
  busy = true;
  const body = $("#memory-body");
  try {
    const [notes, g] = await invoke<[MemoryNote[], Graph]>("graph_memory");
    const labels = new Map(g.nodes.map((n) => [n.id, n.label]));
    if (!notes.length) {
      body.replaceChildren(h("div", { class: "empty-view" },
        h("p", {}, "프로젝트 기억은 AI가 매번 알아야 할 규칙과 결정입니다. 요청할 때마다 맥락에 함께 들어갑니다."),
        h("p", { class: "muted" }, "에이전트가 기억하자고 제안하거나, 아래에서 직접 더할 수 있습니다.")));
    } else {
      const byTopic = new Map<string, MemoryNote[]>();
      for (const n of notes) byTopic.set(n.topic, [...(byTopic.get(n.topic) ?? []), n]);
      body.replaceChildren(...[...byTopic].map(([topic, list]) =>
        h("div", { class: "mem-topic" },
          h("div", { class: "mem-topic-head" }, codicon("book"), h("span", {}, topic), h("span", { class: "badge" }, String(list.length))),
          ...list.map((n) => noteRow(n, labels)))));
    }
    $("#memory-count").textContent = notes.length ? String(notes.length) : "";
  } catch (e) {
    body.replaceChildren(h("div", { class: "empty-view" }, errorText(e)));
  } finally {
    busy = false;
  }
}

async function add() {
  if (hooks.readOnly()) return toast.warn("제한 모드에서는 기억을 더할 수 없습니다", "파일 메뉴에서 이 폴더를 신뢰하세요.");
  const topic = $<HTMLInputElement>("#memory-topic").value.trim() || "conventions";
  const input = $<HTMLTextAreaElement>("#memory-input");
  const note = input.value.replace(/\s*\n\s*/g, " ").trim();
  if (!note) return input.focus();
  const file = topicFile(topic);
  try {
    const old = await api.readFile(file).catch(() => "");
    const name = file.slice(".lantern/memory/".length, -3);
    await api.writeFile(file, old.trim() ? `${old.trimEnd()}\n- ${note}\n` : `# ${name}\n\n- ${note}\n`);
    input.value = "";
    await refresh();
  } catch (e) {
    toast.error("기억을 더하지 못했습니다", errorText(e));
  }
}

export function init(h_: typeof hooks) {
  hooks = h_;
  $("#memory-add").addEventListener("click", () => void add());
  $("#memory-input").addEventListener("keydown", (e) => {
    if ((e as KeyboardEvent).key === "Enter" && ((e as KeyboardEvent).ctrlKey || (e as KeyboardEvent).metaKey)) {
      e.preventDefault();
      void add();
    }
  });
  $("#btn-memory-map").addEventListener("click", () => hooks.showMap());
  $("#btn-memory-refresh").addEventListener("click", () => void refresh());
}
