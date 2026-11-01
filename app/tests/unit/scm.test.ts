// @vitest-environment happy-dom
// 새 브랜치 이름을 치는 중에 소스 제어가 새로 고쳐지면 입력칸이 사라져 치던 이름도 사라졌다 (#146)
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

const REPO = { path: "", name: "proj", branch: "main", upstream: null, ahead: 0, behind: 0, changes: 0, remote: null, prefix: "" };
const STATUS = { repo: true, branch: "main", ahead: 0, behind: 0, files: [] };
type MenuItem = { label: string; run: () => void } | "-";

const { invoke, menu } = vi.hoisted(() => ({ invoke: vi.fn(), menu: { items: [] as unknown[] } }));
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("../../src/editor", () => ({}));
vi.mock("../../src/dialog", () => ({ ask: vi.fn() }));
vi.mock("../../src/commands", () => ({ showContextMenu: (_x: number, _y: number, items: unknown[]) => (menu.items = items) }));
vi.mock("../../src/toast", () => ({ error: vi.fn(), success: vi.fn(), warn: vi.fn() }));

let scm: typeof import("../../src/scm");

const tick = () => new Promise((r) => setTimeout(r));
const input = () => document.querySelector<HTMLInputElement>(".scm-newbranch");
const branchBtn = () => document.querySelector<HTMLElement>("#scm-sync .scm-branch-btn");
const press = (key: string) => input()!.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true }));
const calls = (name: string) => invoke.mock.calls.filter((c) => c[0] === name);

function backend(over: Record<string, () => unknown> = {}) {
  invoke.mockImplementation(async (cmd: string) => {
    if (over[cmd]) return over[cmd]();
    if (cmd === "git_repos") return [{ ...REPO }];
    if (cmd === "git_status") return { ...STATUS };
    if (cmd === "git_branches") return [{ name: "main", remote: false, current: true, upstream: null, date: "" }];
    return "ok";
  });
}

/** 브랜치 메뉴에서 '새 브랜치 만들기'를 골라 치던 중인 입력칸을 연다 */
async function typing(name: string): Promise<HTMLInputElement> {
  branchBtn()!.click();
  await tick();
  (menu.items as MenuItem[]).find((m): m is Exclude<MenuItem, "-"> => m !== "-" && m.label.includes("새 브랜치"))!.run();
  const i = input()!;
  i.value = name;
  return i;
}

beforeAll(async () => {
  document.body.innerHTML = `
    <div id="scm-repos"></div><div id="scm-branch"></div><div id="scm-sync"></div>
    <div id="scm-commit"></div><div id="scm-review"></div><div id="scm-body"></div>`;
  scm = await import("../../src/scm");
});

beforeEach(async () => {
  if (input()) press("Escape");
  await tick();
  invoke.mockReset();
  backend();
  scm.reset();
  await scm.refresh({ repos: true });
  invoke.mockClear();
});

describe("새 브랜치 입력칸이 열린 동안의 새로 고침", () => {
  it("입력칸과 치던 이름, 초점이 그대로 남는다", async () => {
    const i = await typing("feat/ha");
    await scm.refresh();
    await scm.refresh({ repos: true });
    expect(input()).toBe(i);
    expect(i.value).toBe("feat/ha");
    expect(document.activeElement).toBe(i);
    expect(branchBtn()).toBeNull();
    expect(calls("git_checkout")).toEqual([]);
  });

  it("상태를 읽는 사이에 입력칸이 열려도 지우지 않는다", async () => {
    let answer: (v: unknown) => void = () => {};
    backend({ git_status: () => new Promise((r) => (answer = r)) });
    const reading = scm.refresh();
    await tick();
    backend();
    const i = await typing("feat/ha");
    answer({ ...STATUS });
    await reading;
    expect(input()).toBe(i);
    expect(i.value).toBe("feat/ha");
  });

  it("Esc로 닫으면 만들지 않고 브랜치 줄을 다시 그린다", async () => {
    await typing("feat/ha");
    await scm.refresh();
    press("Escape");
    await tick();
    await tick();
    expect(input()).toBeNull();
    expect(branchBtn()?.textContent).toContain("main");
    expect(calls("git_checkout")).toEqual([]);
  });

  it("다 친 뒤 Enter면 끝까지 친 이름으로 만든다", async () => {
    const i = await typing("feat/ha");
    await scm.refresh();
    i.value = "feat/half";
    press("Enter");
    await tick();
    await tick();
    expect(calls("git_checkout").map((c) => c[1])).toEqual([{ repo: "", name: "feat/half", create: true, remote: false }]);
    expect(input()).toBeNull();
    expect(branchBtn()).not.toBeNull();
  });

  it("프로젝트를 바꾸면 입력칸은 만들지 않고 닫힌다", async () => {
    await typing("feat/ha");
    scm.reset();
    await tick();
    await scm.refresh({ repos: true });
    await tick();
    expect(input()).toBeNull();
    expect(branchBtn()).not.toBeNull();
    expect(calls("git_checkout")).toEqual([]);
  });

  it("입력칸이 없으면 평소대로 브랜치 줄을 다시 그린다", async () => {
    backend({ git_status: () => ({ ...STATUS, branch: "dev" }) });
    await scm.refresh();
    expect(branchBtn()?.textContent).toContain("dev");
  });
});
