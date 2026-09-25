// 인라인 자동 완성 (고스트 텍스트). 타이핑이 멈추면 모델에 커서 앞뒤를 보내고, 제안을 흐리게 보여준다.
// Tab 수락, Esc 무시. 설정에서 자동 완성 모델을 고르기 전에는 요청하지 않는다.
import { invoke } from "@tauri-apps/api/core";
import { EditorView, Decoration, WidgetType, ViewPlugin, keymap, type DecorationSet, type ViewUpdate } from "@codemirror/view";
import { Prec, StateEffect, StateField, type Extension } from "@codemirror/state";

const DEBOUNCE_MS = 450;
/** 커서 뒤가 이런 글자뿐이면 제안한다 (줄 가운데서 끼어들지 않게) */
const TAIL_OK = /^[\s)\]}>;,'"`]*$/;

/** 꺼져 있으면(모델 미설정) 설정이 바뀔 때까지 묻지 않는다 */
let disabled = false;
export function settingsChanged() {
  disabled = false;
}

interface Ghost { pos: number; text: string }
const setGhost = StateEffect.define<Ghost | null>();

class GhostWidget extends WidgetType {
  constructor(readonly text: string) {
    super();
  }
  eq(other: GhostWidget) {
    return other.text === this.text;
  }
  toDOM() {
    const span = document.createElement("span");
    span.className = "cm-ghost";
    span.textContent = this.text;
    span.setAttribute("aria-hidden", "true");
    return span;
  }
  ignoreEvent() {
    return false;
  }
}

const ghostField = StateField.define<Ghost | null>({
  create: () => null,
  update(value, tr) {
    for (const e of tr.effects) if (e.is(setGhost)) return e.value;
    // 문서가 바뀌거나 커서가 움직이면 제안은 버린다
    if (tr.docChanged || tr.selection) return null;
    return value;
  },
  provide: (f) =>
    EditorView.decorations.from(f, (g): DecorationSet =>
      g ? Decoration.set([Decoration.widget({ widget: new GhostWidget(g.text), side: 1 }).range(g.pos)]) : Decoration.none),
});

function accept(view: EditorView): boolean {
  const g = view.state.field(ghostField);
  if (!g) return false;
  view.dispatch({
    changes: { from: g.pos, insert: g.text },
    selection: { anchor: g.pos + g.text.length },
    effects: setGhost.of(null),
    userEvent: "input.complete",
  });
  return true;
}

function dismiss(view: EditorView): boolean {
  if (!view.state.field(ghostField)) return false;
  view.dispatch({ effects: setGhost.of(null) });
  return true;
}

export function inlineCompletion(path: string): Extension {
  const plugin = ViewPlugin.fromClass(
    class {
      timer: number | undefined;
      seq = 0;
      constructor(readonly view: EditorView) {}
      update(u: ViewUpdate) {
        if (!u.docChanged) return;
        // 사용자가 직접 입력한 경우만 (수락, 되돌리기, 외부 다시 읽기 제외)
        if (!u.transactions.some((tr) => tr.isUserEvent("input.type") || tr.isUserEvent("delete"))) return;
        clearTimeout(this.timer);
        this.seq++;
        if (disabled) return;
        this.timer = window.setTimeout(() => void this.request(), DEBOUNCE_MS);
      }
      async request() {
        const { state } = this.view;
        const sel = state.selection;
        if (sel.ranges.length > 1 || !sel.main.empty) return;
        const pos = sel.main.head;
        const line = state.doc.lineAt(pos);
        if (!TAIL_OK.test(state.sliceDoc(pos, line.to))) return;
        if (!state.sliceDoc(line.from, pos).trim()) return; // 빈 줄에서는 묻지 않는다
        const my = ++this.seq;
        const docBefore = state.doc;
        try {
          const text = await invoke<string | null>("inline_complete", {
            path,
            prefix: state.sliceDoc(Math.max(0, pos - 6000), pos),
            suffix: state.sliceDoc(pos, Math.min(state.doc.length, pos + 2000)),
          });
          if (text === null) {
            disabled = true;
            return;
          }
          if (my !== this.seq || this.view.state.doc !== docBefore || this.view.state.selection.main.head !== pos || !text.trim()) return;
          this.view.dispatch({ effects: setGhost.of({ pos, text }) });
        } catch {
          /* 네트워크·모델 오류는 조용히 넘긴다 (타이핑을 방해하지 않게) */
        }
      }
      destroy() {
        clearTimeout(this.timer);
      }
    },
  );
  return [
    ghostField,
    plugin,
    Prec.highest(keymap.of([
      { key: "Tab", run: accept },
      { key: "Escape", run: dismiss },
    ])),
  ];
}
