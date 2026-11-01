// E2E 시나리오: 새로 고침이 사용자가 하던 일을 망치지 않는다. 트리의 이름 입력칸, 소스 제어의 새 브랜치 입력칸, 여는 중의 창 새로 고침

import fs from "node:fs";
import path from "node:path";
import { cdp, DIRS, git, launch, openScm, pickMenu, run, scenario, sleep, stop, TMP, waitFor } from "../harness.mjs";
import "../mock-model.mjs"; // 다른 시나리오처럼 이것을 기다려야 run.mjs의 순서대로 등록된다

const proj = (name) => path.join(DIRS.proj, name);
const type = (text) => cdp("Input.insertText", { text });
async function press(key, vk) {
  const base = { key, code: key, windowsVirtualKeyCode: vk, nativeVirtualKeyCode: vk };
  await cdp("Input.dispatchKeyEvent", { type: "rawKeyDown", ...base });
  await cdp("Input.dispatchKeyEvent", { type: "keyUp", ...base });
}
const enter = () => press("Enter", 13);
const esc = () => press("Escape", 27);

/** 밖에서 파일을 만들고 앱이 새로 고칠 때까지 기다린다. 트리와 같은 때 다시 읽는 소스 제어 목록으로 안다 */
async function changeOutside(name) {
  fs.writeFileSync(proj(name), "export const outside = 1;\n");
  await waitFor((name) => [...document.querySelectorAll("#scm-body .scm-file .fname")].some((f) => f.textContent === name), 20000, `${name} 변경 반영`, name);
  await sleep(400); // 트리도 이때 목록을 읽어 그린다
}

/** 입력칸이 그대로이고, 친 글자가 남고, 초점도 그 칸에 있어야 한다 */
async function expectTyping(selector, value, what) {
  const s = await run((sel) => {
    const i = document.querySelector(sel);
    return { value: i?.value ?? null, focused: !!i && document.activeElement === i };
  }, selector);
  if (s.value !== value || !s.focused) throw new Error(`${what}: 입력칸이 망가짐 ${JSON.stringify(s)}`);
}

const openMenuOn = (selector) => run((sel) => {
  document.querySelector(sel).dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 200, clientY: 200 }));
  return true;
}, selector);
const menuShown = () => waitFor(() => !!document.querySelector("#menu-popup:not(.hidden) .menu-item"), 15000, "메뉴");
const focusedIn = (cls) => waitFor((c) => document.activeElement?.classList.contains(c), 5000, `${cls} 입력칸`, cls);
const treeHas = (...names) => waitFor((names) => {
  const rows = [...document.querySelectorAll("#tree .node[data-path]")].map((n) => n.dataset.path);
  return !document.querySelector(".rename-input") && names.every((n) => rows.includes(n));
}, 8000, `트리의 ${names.join(", ")}`, names);

scenario("입력 중 새로 고침: 트리의 새 파일·이름 바꾸기 입력칸이 남고 치다 만 이름이 파일이 되지 않는다", async () => {
  await run(() => {
    if (!document.querySelector("#view-explorer")?.offsetParent) document.querySelector('.ab-item[data-view="explorer"]').click();
    return true;
  });
  await waitFor(() => !!document.querySelector('#tree .node[data-path="src"]'), 8000, "트리");

  // 새 파일: 치는 중에 밖에서 파일이 바뀐다
  await openMenuOn("#tree");
  await menuShown();
  if (!(await pickMenu("새 파일"))) throw new Error("메뉴에 '새 파일' 없음");
  await focusedIn("rename-input");
  await type("half");
  await changeOutside("outside-a.ts");
  await expectTyping(".rename-input", "half", "새 파일");
  if (fs.existsSync(proj("half"))) throw new Error("치다 만 이름으로 파일이 생김");
  // 이어 친 글자는 같은 칸으로 가고, Enter면 끝까지 친 이름 하나만 생긴다. 미뤘던 새로 고침도 이때 그린다
  await type("-typed.ts");
  await enter();
  await treeHas("half-typed.ts", "outside-a.ts");
  if (!fs.existsSync(proj("half-typed.ts")) || fs.existsSync(proj("half"))) throw new Error("끝까지 친 이름으로 만들지 않음");

  // 이름 바꾸기: 치는 중에 밖에서 파일이 바뀌어도 커밋되지 않는다 (확장자 앞까지 선택된 채로 시작한다)
  await openMenuOn('#tree .node[data-path="half-typed.ts"]');
  await menuShown();
  if (!(await pickMenu("이름 바꾸기"))) throw new Error("메뉴에 '이름 바꾸기' 없음");
  await focusedIn("rename-input");
  await type("renamed");
  await changeOutside("outside-b.ts");
  await expectTyping(".rename-input", "renamed.ts", "이름 바꾸기");
  if (fs.existsSync(proj("renamed.ts")) || !fs.existsSync(proj("half-typed.ts"))) throw new Error("치던 이름으로 파일 이름이 바뀜");
  await esc();
  await treeHas("half-typed.ts", "outside-b.ts");
  if (fs.existsSync(proj("renamed.ts"))) throw new Error("Esc인데 이름이 바뀜");

  await run(() => {
    for (const b of document.querySelectorAll(".tab .close")) b.click();
    return true;
  });
  for (const f of ["half-typed.ts", "outside-a.ts", "outside-b.ts"]) fs.rmSync(proj(f));
});

scenario("입력 중 새로 고침: 새 브랜치 이름을 치는 중에 파일이 바뀌어도 입력칸이 남는다 (#146)", async () => {
  await openScm();
  await waitFor(() => !!document.querySelector("#scm-sync .scm-branch-btn"), 10000, "브랜치 버튼");
  await run(() => {
    document.querySelector("#scm-sync .scm-branch-btn").click();
    return true;
  });
  await menuShown();
  if (!(await pickMenu("새 브랜치 만들기"))) throw new Error("메뉴에 '새 브랜치 만들기' 없음");
  await focusedIn("scm-newbranch");
  await type("e2e/ha");
  await changeOutside("outside-c.ts");
  await expectTyping(".scm-newbranch", "e2e/ha", "파일 변경");
  // 창에 초점이 돌아올 때의 새로 고침도 같다
  await run(() => {
    window.dispatchEvent(new Event("focus"));
    return true;
  });
  await sleep(600);
  await type("lf");
  await expectTyping(".scm-newbranch", "e2e/half", "창 초점");
  // 닫으면 미뤘던 브랜치 줄을 그린다
  await esc();
  await waitFor(() => !document.querySelector(".scm-newbranch") && !!document.querySelector("#scm-sync .scm-branch-btn"), 8000, "브랜치 줄");
  if (git("branch", "--list", "e2e/half").trim()) throw new Error("Esc인데 브랜치가 생김");
  fs.rmSync(proj("outside-c.ts"));
});

scenario("여는 중 새로 고침: 인덱싱 중에 창을 새로 고쳐도 인덱스가 잠기지 않는다", async () => {
  // 첫 인덱싱이 새로 고침보다 오래 걸릴 만큼의 파일. 인덱싱이 먼저 끝나는 빠른 환경에서는 그냥 통과한다
  const big = path.join(TMP, "big");
  for (let i = 0; i < 1500; i++) {
    const dir = path.join(big, `m${i % 50}`);
    fs.mkdirSync(dir, { recursive: true });
    let body = "";
    for (let k = 0; k < 12; k++) body += `export function f${i}_${k}(a: number) {\n  return g${i}_${k}(a + ${k});\n}\nfunction g${i}_${k}(n: number) {\n  return n * 2;\n}\n`;
    fs.writeFileSync(path.join(dir, `file${i}.ts`), body);
  }
  const trusted = JSON.parse(fs.readFileSync(path.join(DIRS.data, "trusted.json"), "utf8"));
  fs.writeFileSync(path.join(DIRS.data, "trusted.json"), JSON.stringify([...trusted, big.replace(/\\/g, "/").toLowerCase()]));

  await stop();
  await launch(big, { waitProject: false });
  await waitFor(() => !!document.querySelector("#ctx-status")?.textContent, 30000, "인덱스 상태");
  // 창을 새로 고치면 앞선 열기가 도는 채로 같은 폴더를 또 연다 (언어를 바꿀 때도 같다)
  for (let i = 0; i < 2; i++) {
    await sleep(400);
    await run(() => {
      setTimeout(() => location.reload(), 10);
      return true;
    });
    await sleep(1200);
    await waitFor(() => document.readyState !== "loading" && !!document.documentElement.lang, 30000, "다시 불러옴");
  }
  const state = () => run(() => ({
    status: document.querySelector("#ctx-status")?.textContent ?? "",
    toasts: [...document.querySelectorAll(".toast")].map((t) => t.innerText.replace(/\s+/g, " ")),
  }));
  const end = Date.now() + 180000;
  let s = await state();
  while (!s.status.startsWith("맥락") && !s.status.includes("오류") && !s.toasts.some((t) => t.includes("인덱스를 만들지 못했습니다")) && Date.now() < end) {
    await sleep(200);
    s = (await state()) ?? s;
  }
  if (!s.status.startsWith("맥락")) throw new Error(`인덱스 상태: ${JSON.stringify(s)}`);
  // 차례를 기다리던 열기들의 인덱싱도 끝난다
  await sleep(2500);
  s = await state();
  if (!s.status.startsWith("맥락") || s.toasts.some((t) => t.includes("인덱스를 만들지 못했습니다"))) throw new Error(`인덱스 상태: ${JSON.stringify(s)}`);
  await waitFor(() => !!document.querySelector("#map-crumb .cur"), 30000, "지도");
  await stop();
  await launch();
});
