// 편집기 안 즉시 수정 (Ctrl+K). 코드를 선택하고 지시하면, 고친 코드를 diff와 영향 반경으로 보여 주고 적용한다.
// 선택이 없으면 커서 자리에 넣을 코드를 만든다. 적용은 편집기 변경이라 Ctrl+Z로 되돌릴 수 있다.
import { invoke } from "@tauri-apps/api/core";
import { StateEffect, StateField, type Extension } from "@codemirror/state";
import { Decoration, EditorView, type DecorationSet } from "@codemirror/view";
import { errorText } from "./api";
import { h, renderDiff } from "./dom";
import { codicon } from "./icons";
import * as toast from "./toast";

interface Result { replacement: string; diff: string }

const setRange = StateEffect.define<{ from: number; to: number } | null>();
const mark = Decoration.mark({ class: "cm-inline-edit-range" });

/** 고치는 범위를 옅게 칠한다 */
const rangeField = StateField.define<DecorationSet>({
  create: () => Decoration.none,
  update(deco, tr) {
    deco = deco.map(tr.changes);
    for (const e of tr.effects) {
      if (e.is(setRange)) deco = e.value && e.value.to > e.value.from ? Decoration.set([mark.range(e.value.from, e.value.to)]) : Decoration.none;
    }
    return deco;
  },
  provide: (f) => EditorView.decorations.from(f),
});

export const inlineEditExtension: Extension = [rangeField];

let impactRow: (diff: string) => HTMLElement | null = () => null;
let open: { close: () => void } | null = null;

/** 영향 반경 표시는 채팅의 승인 카드와 같은 것을 쓴다 (main.ts가 넘겨준다) */
export function init(opts: { impactRow: (diff: string) => HTMLElement | null }) {
  impactRow = opts.impactRow;
}

/** 패널을 선택 끝 줄 바로 아래에 둔다 (편집기와 함께 스크롤된다) */
function place(view: EditorView, panel: HTMLElement, pos: number) {
  const scroller = view.scrollDOM;
  const block = view.lineBlockAt(pos);
  const top = view.documentTop - scroller.getBoundingClientRect().top + scroller.scrollTop + block.bottom + 4;
  panel.style.top = `${top}px`;
  panel.style.left = `${view.contentDOM.offsetLeft + 8}px`;
}

export function start(view: EditorView, path: string) {
  open?.close();
  const sel = view.state.selection.main;
  const from = sel.from;
  const to = sel.to;
  const doc = view.state.doc;
  const original = doc.sliceString(from, to);
  const line = doc.lineAt(from).number;
  view.dispatch({ effects: setRange.of({ from, to }) });

  const panel = h("div", { class: "inline-edit", role: "dialog", "aria-label": "AI로 바로 고치기" });
  const input = h("input", { class: "ie-input", placeholder: original ? "선택한 코드를 어떻게 바꿀까요? (Enter)" : "여기에 넣을 코드를 설명하세요 (Enter)", spellcheck: "false", "aria-label": "고칠 내용" }) as HTMLInputElement;
  const body = h("div", { class: "ie-body" });
  const head = h("div", { class: "ie-head" }, codicon("sparkle"), h("span", {}, original ? `선택한 ${to - from}자를 고칩니다` : "커서 자리에 넣습니다"), h("span", { class: "ie-keys" }, "Esc 닫기"));
  panel.append(head, input, body);
  view.scrollDOM.append(panel);
  place(view, panel, to);
  const onScroll = () => place(view, panel, to);
  view.scrollDOM.addEventListener("scroll", onScroll);
  input.focus();

  let result: Result | null = null;
  let busy = false;
  const close = () => {
    view.scrollDOM.removeEventListener("scroll", onScroll);
    panel.remove();
    view.dispatch({ effects: setRange.of(null) });
    if (open?.close === close) open = null;
    view.focus();
  };
  open = { close };

  const apply = () => {
    if (!result) return;
    // 그사이 사용자가 그 자리를 고쳤으면 덮어쓰지 않는다
    if (view.state.doc.sliceString(from, to) !== original) {
      toast.warn("그사이 이 부분이 바뀌어 적용하지 않았습니다", "다시 선택해서 Ctrl+K를 누르세요.");
      return close();
    }
    view.dispatch({ changes: { from, to, insert: result.replacement }, selection: { anchor: from, head: from + result.replacement.length }, userEvent: "input.ai" });
    toast.info("적용했습니다", "Ctrl+Z로 되돌릴 수 있습니다.");
    close();
  };

  const generate = async () => {
    const instruction = input.value.trim();
    if (!instruction || busy) return;
    busy = true;
    input.disabled = true;
    body.replaceChildren(h("div", { class: "ie-status" }, codicon("loading", "codicon-modifier-spin"), "만드는 중…"));
    try {
      result = await invoke<Result>("inline_edit", {
        path,
        before: doc.sliceString(0, from),
        selection: original,
        after: doc.sliceString(to),
        instruction,
        line,
      });
      if (result.replacement === original) {
        body.replaceChildren(h("div", { class: "ie-status" }, codicon("info"), "바꿀 것이 없다고 답했습니다. 지시를 바꿔 보세요."));
      } else {
        const ok = h("button", { class: "btn btn-primary" }, codicon("check"), "적용");
        const again = h("button", { class: "btn btn-secondary" }, codicon("refresh"), "다시");
        const cancel = h("button", { class: "btn btn-secondary" }, "취소");
        ok.addEventListener("click", apply);
        again.addEventListener("click", () => { input.disabled = false; input.focus(); input.select(); });
        cancel.addEventListener("click", close);
        body.replaceChildren(
          renderDiff(result.diff),
          ...[impactRow(result.diff)].filter((x): x is HTMLElement => !!x),
          h("div", { class: "ie-actions" }, ok, again, cancel, h("span", { class: "ie-keys" }, "Ctrl+Enter 적용")));
        ok.focus();
      }
      place(view, panel, to);
    } catch (e) {
      body.replaceChildren(h("div", { class: "ie-status bad" }, codicon("error"), errorText(e)));
    } finally {
      busy = false;
      input.disabled = false;
    }
  };

  input.addEventListener("keydown", (e) => {
    if (e.key === "Enter" && !e.isComposing) {
      e.preventDefault();
      void generate();
    }
  });
  panel.addEventListener("keydown", (e) => {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      close();
    } else if (e.key === "Enter" && (e.ctrlKey || e.metaKey) && result) {
      e.preventDefault();
      apply();
    }
  });
}

/** 열린 패널 닫기 (탭을 바꿀 때 등) */
export function closeOpen() {
  open?.close();
}
