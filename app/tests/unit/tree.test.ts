// @vitest-environment happy-dom
// 입력칸이 열린 채 트리가 새로 고쳐지면 행을 갈아 끼우며 입력칸이 사라지고, 그때의 blur가 치다 만 이름을 파일로 만든다 (감사에서 재현한 버그)
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

const { invoke, listDir } = vi.hoisted(() => ({ invoke: vi.fn(async (..._: unknown[]) => undefined), listDir: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("../../src/api", () => ({ api: { listDir }, errorText: String }));
vi.mock("../../src/dialog", () => ({ ask: vi.fn() }));
vi.mock("../../src/commands", () => ({ showContextMenu: vi.fn() }));
vi.mock("../../src/toast", () => ({ error: vi.fn(), info: vi.fn() }));

let tree: typeof import("../../src/tree");

const entry = (name: string) => ({ name, path: name, is_dir: false });
const tick = () => new Promise((r) => setTimeout(r));
const input = () => document.querySelector<HTMLInputElement>(".rename-input");
const rows = () => [...document.querySelectorAll<HTMLElement>("#tree .node[data-path]")].map((n) => n.dataset.path);
const press = (key: string) => input()!.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true }));

beforeAll(async () => {
  document.body.innerHTML = `<div id="tree" tabindex="0"></div>`;
  tree = await import("../../src/tree");
  tree.init();
});

beforeEach(async () => {
  // 앞 테스트의 입력칸이 남아 있으면 닫고 처음 상태로
  if (input()) press("Escape");
  await tick();
  invoke.mockClear();
  listDir.mockReset();
  listDir.mockResolvedValue([entry("a.ts")]);
  await tree.refresh();
});

/** 치던 중인 입력칸을 연다 */
function typing(name: string): HTMLInputElement {
  tree.newFile(false);
  const i = input()!;
  i.value = name;
  return i;
}

describe("입력칸이 열린 동안의 새로 고침", () => {
  it("입력칸을 지우지 않고 치다 만 이름을 커밋하지 않는다", async () => {
    const i = typing("half");
    listDir.mockResolvedValue([entry("a.ts"), entry("b.ts")]);
    await tree.refresh();
    expect(input()).toBe(i);
    expect(i.value).toBe("half");
    expect(document.activeElement).toBe(i);
    expect(invoke).not.toHaveBeenCalled();
    expect(rows()).toEqual(["a.ts"]);
  });

  it("목록을 읽는 사이에 입력칸이 열려도 지우지 않는다", async () => {
    let answer: (v: unknown) => void = () => {};
    listDir.mockReturnValueOnce(new Promise((r) => (answer = r)));
    const reading = tree.refresh();
    const i = typing("half");
    answer([entry("a.ts"), entry("b.ts")]);
    await reading;
    expect(input()).toBe(i);
    expect(invoke).not.toHaveBeenCalled();
  });

  it("목록을 못 읽어도 입력칸을 오류 문구로 덮지 않는다", async () => {
    let refuse: (e: unknown) => void = () => {};
    listDir.mockReturnValueOnce(new Promise((_, r) => (refuse = r)));
    const reading = tree.refresh();
    const i = typing("half");
    refuse(new Error("x"));
    await reading;
    expect(input()).toBe(i);
  });

  it("Esc로 닫으면 커밋 없이 미뤘던 내용을 그린다", async () => {
    typing("half");
    listDir.mockResolvedValue([entry("a.ts"), entry("b.ts")]);
    await tree.refresh();
    press("Escape");
    await tick();
    expect(input()).toBeNull();
    expect(rows()).toEqual(["a.ts", "b.ts"]);
    expect(invoke).not.toHaveBeenCalled();
  });

  it("다 친 뒤 Enter면 끝까지 친 이름 하나만 만든다", async () => {
    const i = typing("half");
    await tree.refresh();
    i.value = "half-typed.ts";
    press("Enter");
    await tick();
    expect(invoke.mock.calls.map((c) => c[0])).toEqual(["create_path"]);
    expect(invoke.mock.calls[0][1]).toEqual({ path: "half-typed.ts", dir: false });
    expect(input()).toBeNull();
  });

  it("커밋 중에도 입력칸이 있다가, 실패하면 친 글자를 그대로 두고 다시 고치게 한다", async () => {
    const i = typing("half.ts");
    let refuse: (e: unknown) => void = () => {};
    invoke.mockImplementationOnce(() => new Promise((_, r) => (refuse = r)));
    press("Enter");
    await tree.refresh();
    expect(input()).toBe(i);
    refuse(new Error("이미 있음"));
    await tick();
    expect(input()).toBe(i);
    expect(i.value).toBe("half.ts");
  });

  it("이름 바꾸기 입력칸도 지우지 않고 커밋하지 않으며, Esc면 행이 돌아온다", async () => {
    const box = document.querySelector<HTMLElement>("#tree")!;
    document.querySelector<HTMLElement>('#tree .node[data-path="a.ts"]')!.click();
    box.dispatchEvent(new KeyboardEvent("keydown", { key: "F2", bubbles: true }));
    const i = input()!;
    expect(i.value).toBe("a.ts");
    i.value = "renamed.ts";
    listDir.mockResolvedValue([entry("a.ts"), entry("b.ts")]);
    await tree.refresh();
    expect(input()).toBe(i);
    expect(invoke).not.toHaveBeenCalled();
    press("Escape");
    await tick();
    expect(input()).toBeNull();
    expect(rows()).toEqual(["a.ts", "b.ts"]);
    expect(invoke).not.toHaveBeenCalled();
  });

  it("프로젝트를 바꾸면 입력칸은 커밋 없이 닫히고 새 프로젝트를 그린다", async () => {
    typing("half");
    tree.reset();
    expect(input()).toBeNull();
    listDir.mockResolvedValue([entry("other.ts")]);
    await tree.refresh();
    expect(rows()).toEqual(["other.ts"]);
    expect(invoke).not.toHaveBeenCalled();
  });

  it("입력칸이 없으면 평소대로 바로 그린다", async () => {
    listDir.mockResolvedValue([entry("a.ts"), entry("c.ts")]);
    await tree.refresh();
    expect(rows()).toEqual(["a.ts", "c.ts"]);
  });
});
