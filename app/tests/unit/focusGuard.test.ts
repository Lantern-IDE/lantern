// @vitest-environment happy-dom
// 입력 중에 온 승인 카드: 초점이 '적용'으로 옮겨 가면 치던 Enter·Space가 변경을 적용해 버린다 (감사에서 재현한 버그)
import { beforeAll, describe, expect, it } from "vitest";
import { isEditable, mayTakeFocus, watchKeys } from "../../src/lib/focusGuard";

const later = (ms: number) => performance.now() + ms;

function add(html: string): HTMLElement {
  const box = document.createElement("div");
  box.innerHTML = html;
  document.body.append(box);
  return box.firstElementChild as HTMLElement;
}

beforeAll(() => watchKeys());

describe("결정 버튼으로 초점 옮기기", () => {
  it("입력칸, 편집기, 터미널에 초점이 있으면 옮기지 않는다", () => {
    for (const html of [
      `<input id="f">`,
      `<textarea id="prompt"></textarea>`,
      `<div contenteditable="true" tabindex="0"></div>`,
      `<div class="cm-editor"><div class="cm-content" tabindex="0"></div></div>`,
      `<div class="xterm"><textarea class="xterm-helper-textarea"></textarea></div>`,
    ]) {
      const box = add(html);
      const el = box.querySelector<HTMLElement>("[tabindex], textarea") ?? box;
      el.focus();
      expect(document.activeElement, html).toBe(el);
      expect(isEditable(document.activeElement), html).toBe(true);
      expect(mayTakeFocus(later(10_000)), html).toBe(false);
      el.blur();
      box.remove();
    }
  });

  it("버튼이나 빈 곳에 있고 한동안 키를 안 눌렀으면 옮긴다", () => {
    const b = add(`<button>열기</button>`);
    b.focus();
    expect(isEditable(b)).toBe(false);
    expect(mayTakeFocus(later(10_000))).toBe(true);
    b.blur();
    b.remove();
    expect(mayTakeFocus(later(10_000))).toBe(true);
  });

  it("어디서든 마지막 키 입력 뒤 1.5초 안에는 옮기지 않는다", () => {
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown" }));
    expect(mayTakeFocus(later(0))).toBe(false);
    expect(mayTakeFocus(later(1400))).toBe(false);
    expect(mayTakeFocus(later(1600))).toBe(true);
  });

  it("전파를 막는 처리기가 있어도 키 입력을 안다", () => {
    const b = add(`<button>열기</button>`);
    b.addEventListener("keydown", (e) => e.stopPropagation());
    b.dispatchEvent(new KeyboardEvent("keydown", { key: "a", bubbles: true }));
    expect(mayTakeFocus(later(100))).toBe(false);
    b.remove();
  });
});
