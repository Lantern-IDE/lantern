// @vitest-environment happy-dom
// 알림이 한꺼번에 여러 개 떠도 멈추지 않는다 (E2E에서 찾은 무한 반복 버그의 회귀 테스트)
import { beforeAll, describe, expect, it } from "vitest";

let toast: typeof import("../../src/toast");

beforeAll(async () => {
  document.body.innerHTML = `
    <div id="toasts"></div>
    <div id="notif-center" class="hidden"><div id="notif-list"></div><button id="nc-close"></button><button id="nc-clear"></button></div>
    <button id="sb-bell"></button>`;
  toast = await import("../../src/toast");
});

describe("알림", () => {
  it("닫히는 중인 알림이 있어도 넘치는 알림을 정리한다", () => {
    const box = document.querySelector("#toasts")!;
    // 4개째에서 첫 알림이 "닫히는 중"이 되고, 5개째에서 예전 코드는 무한 반복에 빠졌다
    for (let i = 0; i < 8; i++) toast.success(`알림 ${i}`);
    const live = [...box.children].filter((c) => !c.classList.contains("leaving"));
    expect(live.length).toBe(3);
    expect(live.map((c) => c.textContent)).toEqual(["알림 5", "알림 6", "알림 7"]);
  });

  it("승인을 기다리는 동안에는 오류가 아닌 알림을 띄우지 않고 기록만 한다", () => {
    const approval = document.createElement("div");
    approval.className = "approval";
    document.body.append(approval);
    const before = document.querySelectorAll("#toasts .toast:not(.leaving)").length;
    toast.info("숨겨질 알림");
    expect(document.querySelectorAll("#toasts .toast:not(.leaving)").length).toBe(before);
    approval.remove();
  });
});
