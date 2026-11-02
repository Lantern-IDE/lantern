// E2E 시나리오: 승인 카드와 완료 카드. 사람이 보기 전에 바뀌지 않고, 본 것이 사실이어야 한다.
// 입력 중 초점, 줄 접기와 버튼 줄, 위험도, 테스트 확인 결과, 승인 대기 중의 지도

import fs from "node:fs";
import path from "node:path";
import { cdp, DIRS, file, git, HERE, launch, run, scenario, sleep, stop, traceLog, ui, waitFor } from "../harness.mjs";
import { mock } from "../mock-model.mjs";

const SESSION = "src/auth/session.ts";
const projCfg = path.join(DIRS.proj, ".lantern", "config.toml");

const newTask = () => run(() => {
  document.querySelector("#btn-new-task").click();
  return true;
});

/** 진짜 키 입력 (브라우저 입력 경로를 거쳐 버튼도 눌린다) */
async function press(key, code, vk, text) {
  const base = { key, code, windowsVirtualKeyCode: vk, nativeVirtualKeyCode: vk };
  await cdp("Input.dispatchKeyEvent", { type: text ? "keyDown" : "rawKeyDown", ...base, ...(text ? { text, unmodifiedText: text } : {}) });
  await cdp("Input.dispatchKeyEvent", { type: "keyUp", ...base });
}
const enter = () => press("Enter", "Enter", 13, "\r");
const space = () => press(" ", "Space", 32, " ");

const pendingCard = (what = "승인 카드") => waitFor(() => {
  const c = document.querySelector(".task-log:not(.hidden) .approval:not(.resolved)");
  return c && (c.dataset.kind ?? "?");
}, 30000, what);
const impactShown = () => waitFor(() => document.querySelector(".task-log:not(.hidden) .approval:not(.resolved) .impact:not(.loading)")?.innerText, 20000, "영향 반경");
const idle = () => waitFor(() => document.querySelector("#btn-stop").classList.contains("hidden"), 20000, "작업 끝");
/** 지금 카드를 거절하고 작업이 끝나기를 기다린다 */
async function refuseAndFinish() {
  await run(() => {
    document.querySelector(".task-log:not(.hidden) .approval:not(.resolved) .actions .btn-secondary")?.click();
    return true;
  });
  await idle();
}
const stillWaiting = () => run(() => !!document.querySelector(".task-log:not(.hidden) .approval:not(.resolved) .actions .btn-primary"));
const focusInCard = () => run(() => !!document.activeElement?.closest(".approval"));

const useSig = () => {
  mock.read = "src/auth/login.ts";
  mock.edit = SESSION;
  mock.old = "v + '.sig'";
  mock.new = "v + '.typed'";
};

scenario("승인 카드 앞에서: 시나리오용 프로젝트로 돌아온다", async () => {
  // 앞 시나리오가 다른 폴더를 열었을 수 있다
  await stop();
  await launch();
  git("checkout", "-q", "--", ".");
  fs.rmSync(projCfg, { force: true });
});

scenario("입력 중에 온 승인 카드: 입력창의 Enter로 적용되지 않는다", async () => {
  await newTask();
  useSig();
  await run(() => {
    document.querySelector("#prompt").focus();
    return true;
  });
  await run(ui.sendTask, "초점 확인: 입력창", "code", false);
  await pendingCard();
  await sleep(200);
  const active = await run(() => document.activeElement?.id);
  if (active !== "prompt") throw new Error(`초점이 입력창에서 ${active}로 옮겨 감`);
  await enter();
  await sleep(600);
  if (!(await stillWaiting())) throw new Error("Enter로 카드가 닫힘");
  if (file(SESSION).includes(".typed")) throw new Error("Enter로 파일이 바뀜");
  await refuseAndFinish();
});

scenario("입력 중에 온 승인 카드: 편집기의 Space로 적용되지 않는다", async () => {
  await newTask();
  await run(() => {
    document.querySelector('.ab-item[data-view="explorer"]').click();
    document.querySelector("#canvas-switch [data-canvas='editor']").click();
    return true;
  });
  // 트리의 자식은 비동기로 채워진다: 다음 줄이 나타날 때까지 기다리며 펼친다
  for (const p of ["src", "src/auth", "src/auth/login.ts"]) {
    await waitFor((p) => {
      const n = [...document.querySelectorAll("#tree .node[data-path]")].find((e) => e.dataset.path === p);
      if (!n) return false;
      if (p.endsWith(".ts")) {
        n.click();
        n.dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
      } else if (n.getAttribute("aria-expanded") !== "true") n.click();
      return true;
    }, 10000, `트리의 ${p}`, p);
  }
  await waitFor(() => [...document.querySelectorAll(".cm-editor")].some((e) => e.offsetParent && e.textContent.includes("handleLogin")), 10000, "편집기");
  useSig();
  await run(() => {
    [...document.querySelectorAll(".cm-editor")].find((e) => e.offsetParent).querySelector(".cm-content").focus();
    return true;
  });
  await run(ui.sendTask, "초점 확인: 편집기", "code", false);
  await pendingCard();
  await sleep(200);
  if (await focusInCard()) throw new Error("초점이 편집기에서 승인 카드로 옮겨 감");
  await space();
  await sleep(600);
  if (!(await stillWaiting())) throw new Error("Space로 카드가 닫힘");
  if (file(SESSION).includes(".typed")) throw new Error("Space로 파일이 바뀜");
  await refuseAndFinish();
  // 편집기에 들어간 빈칸을 되돌리고 저장해 '저장 안 됨' 표시를 지운다
  await run(() => {
    const view = [...document.querySelectorAll(".cm-editor")].find((e) => e.offsetParent).querySelector(".cm-content").cmTile.root.view;
    view.focus();
    view.contentDOM.dispatchEvent(new KeyboardEvent("keydown", { key: "z", code: "KeyZ", ctrlKey: true, bubbles: true }));
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "s", code: "KeyS", ctrlKey: true, bubbles: true }));
    return true;
  });
  await sleep(400);
  await run(() => {
    document.querySelector('.ab-item[data-view="tasks"]').click();
    return true;
  });
  git("checkout", "-q", "--", ".");
});

scenario("막 키를 눌렀으면 어디에 있든 초점을 옮기지 않고, 조용하면 키보드로 결정할 수 있게 옮긴다", async () => {
  await newTask();
  useSig();
  await run(() => {
    document.activeElement?.blur();
    return true;
  });
  await run(ui.sendTask, "초점 확인: 방금 친 키", "code", false);
  for (let i = 0; i < 200 && !(await run(() => !!document.querySelector(".task-log:not(.hidden) .approval:not(.resolved)"))); i++) {
    await press("Shift", "ShiftLeft", 16);
    await sleep(100);
  }
  await sleep(200);
  if (await focusInCard()) throw new Error("키를 누른 직후인데 초점이 승인 카드로 옮겨 감");
  await enter();
  await sleep(500);
  if (!(await stillWaiting())) throw new Error("Enter로 카드가 닫힘");
  await refuseAndFinish();

  // 조용한 경우: 1.5초 넘게 키를 누르지 않았고 초점이 입력칸 밖이면 '적용'으로 간다. 보내기는 키 없이 버튼으로
  await newTask();
  await run(() => {
    document.activeElement?.blur();
    return true;
  });
  await sleep(1700);
  await run(() => {
    const sel = document.querySelector("#agent-select");
    sel.value = "code";
    sel.dispatchEvent(new Event("change"));
    const ta = document.querySelector("#prompt");
    ta.value = "초점 확인: 조용할 때";
    ta.dispatchEvent(new Event("input"));
    document.querySelector("#btn-send").click();
    return true;
  });
  await pendingCard();
  await sleep(200);
  if (!(await focusInCard())) throw new Error("키보드 사용자를 위한 초점 이동이 사라짐");
  await enter();
  await waitFor(() => document.querySelector(".task-log:not(.hidden) .approval .verdict")?.textContent === "적용됨", 10000, "Enter로 적용");
  await idle();
  git("checkout", "-q", "--", ".");
});

scenario("승인 카드: 긴 줄은 접히고 달라진 곳만 칠해지며 버튼은 대화 창 안에 있다", async () => {
  await newTask();
  useSig();
  const tail = " + '".concat("긴꼬리".repeat(40), "'");
  mock.new = `v + '.sig'${tail}`;
  await run(ui.sendTask, "긴 줄 확인", "code", false);
  await pendingCard();
  await impactShown();
  await sleep(300);
  const m = await run(() => {
    const c = document.querySelector(".task-log:not(.hidden) .approval:not(.resolved)");
    const box = document.querySelector("#messages").getBoundingClientRect();
    const add = c.querySelector(".diff .add:not(.meta)");
    const a = c.querySelector(".actions").getBoundingClientRect();
    const d = c.querySelector(".diff");
    return { addH: add.getBoundingClientRect().height, chg: add.querySelector(".chg")?.textContent ?? "", delChg: c.querySelector(".diff .del .chg")?.textContent ?? null, sideways: d.scrollWidth - d.clientWidth, over: a.bottom - box.bottom, position: getComputedStyle(c.querySelector(".actions")).position };
  });
  if (m.addH <= 18) throw new Error(`긴 줄이 접히지 않음: 높이 ${m.addH}`);
  if (m.sideways > 0) throw new Error(`Diff가 옆으로 ${m.sideways}px 넘침`);
  if (!m.chg.includes("긴꼬리") || m.chg.includes("signCookie")) throw new Error(`줄 안 강조: ${m.chg}`);
  if (m.delChg !== null) throw new Error(`덧붙이기만 했는데 지운 줄에 강조: ${m.delChg}`);
  if (m.over > 0.5) throw new Error(`버튼 줄이 ${m.over}px 가려짐`);
  if (m.position !== "sticky") throw new Error(`버튼 줄 위치: ${m.position}`);
  await refuseAndFinish();
});

scenario("승인 카드: 좁은 창에서도 줄 끝의 변경과 버튼이 스크롤 없이 보인다", async () => {
  const LONG = "src/long.ts";
  const args = Array.from({ length: 14 }, (_, i) => `argument${i}: string`).join(", ");
  fs.writeFileSync(path.join(DIRS.proj, LONG), `// 긴 줄\nexport function longSignature(${args}, last: string) {\n  return last;\n}\n`);
  await cdp("Emulation.setDeviceMetricsOverride", { width: 900, height: 560, deviceScaleFactor: 0, mobile: false });
  try {
    await newTask();
    mock.read = LONG;
    mock.edit = LONG;
    mock.old = "last: string) {";
    mock.new = "last: string[]) {";
    await run(ui.sendTask, "좁은 창 확인", "code", false);
    await pendingCard();
    await sleep(1500); // 영향 반경으로 카드가 커질 때까지
    const m = await run(() => {
      const c = document.querySelector(".task-log:not(.hidden) .approval:not(.resolved)");
      const onTop = (el) => {
        const b = el.getBoundingClientRect();
        // 잠깐 뜨는 알림이 겹칠 수 있어 알림은 빼고 본다
        const t = document.elementsFromPoint(b.left + b.width / 2, b.top + b.height / 2).find((e) => !e.closest(".toast"));
        return !!t && (t === el || el.contains(t));
      };
      const chg = c.querySelector(".diff .add .chg");
      return { width: innerWidth, chg: chg?.textContent ?? null, chgShown: !!chg && onTop(chg), apply: onTop(c.querySelector(".actions .btn-primary")), refuse: onTop(c.querySelector(".actions .btn-secondary")) };
    });
    if (m.width !== 900) throw new Error(`창 폭을 줄이지 못함: ${m.width}`);
    if (m.chg !== "[]" || !m.chgShown) throw new Error(`줄 끝의 변경이 안 보임: ${JSON.stringify(m)}`);
    if (!m.apply || !m.refuse) throw new Error(`버튼이 가려짐: ${JSON.stringify(m)}`);
    await refuseAndFinish();
  } finally {
    await cdp("Emulation.clearDeviceMetricsOverride");
    fs.rmSync(path.join(DIRS.proj, LONG), { force: true });
  }
});

scenario("위험도: 테스트 호출자는 세지 않고, 주석만 바꾸면 그렇게 말한다", async () => {
  await newTask();
  useSig();
  await run(ui.sendTask, "위험도 확인: 코드 변경", "code", false);
  let row = await impactShown();
  // signCookie: 호출자 issueSession, testSign(테스트). 테스트는 호출자로 세지 않고 따로 보인다
  if (!/직접 호출/.test(row) || !/테스트 1/.test(row) || /코드 변경 없음/.test(row)) throw new Error(`코드 변경의 영향 반경: ${row}`);
  const direct = await run(() => document.querySelector(".task-log:not(.hidden) .approval:not(.resolved) .impact .istat b")?.textContent);
  if (direct !== "1") throw new Error(`직접 호출 ${direct} (테스트를 뺀 값이 아님)`);
  await refuseAndFinish();

  await newTask();
  mock.old = "v + '.sig';";
  mock.new = "v + '.sig'; // 서명을 붙인다";
  await run(ui.sendTask, "위험도 확인: 주석", "code", false);
  row = await impactShown();
  if (!row.includes("코드 변경 없음") || row.includes("위험도")) throw new Error(`주석만 바꾼 영향 반경: ${row}`);
  const crumb = await run(() => document.querySelector("#map-crumb")?.textContent ?? "");
  if (crumb.includes("영향 반경")) throw new Error(`주석만 바꿨는데 지도가 영향 반경으로 바뀜: ${crumb}`);
  await refuseAndFinish();
});

/** 검증 시나리오를 돌린다. 명령 승인 카드에서 answers 차례대로 답한다 */
async function verifyRun(prompt, answers) {
  await newTask();
  await run(ui.sendTask, prompt, "code", false);
  for (const [i, yes] of answers.entries()) {
    await pendingCard(`승인 카드 ${i + 1}`);
    await run((yes) => {
      document.querySelector(`.task-log:not(.hidden) .approval:not(.resolved) .actions ${yes ? ".btn-primary" : ".btn-secondary"}`).click();
      return true;
    }, yes);
    await waitFor((n) => document.querySelectorAll(".task-log:not(.hidden) .approval.resolved").length === n, 10000, `승인 반영 ${i + 1}`, i + 1);
  }
  await waitFor(() => !!document.querySelector(".task-log:not(.hidden) .changed"), 30000, "완료 카드");
  await idle();
  return run(() => {
    const box = document.querySelector(".task-log:not(.hidden) .changed");
    const row = box.querySelector(".ch-test");
    const tests = [...document.querySelectorAll(".task-log:not(.hidden) details.tool")].filter((d) => d.querySelector(".tname")?.textContent.includes("관련 테스트로 확인"));
    return {
      state: row ? row.className.replace("ch-test", "").trim() : null,
      text: row?.textContent ?? "",
      head: box.querySelector(".ch-head .codicon")?.className ?? "",
      tool: tests.at(-1)?.querySelector(".tstate")?.className ?? null,
      task: document.querySelector("#task-list .task.active")?.className ?? "",
    };
  });
}

scenario("완료 카드: 통과·실패·찾지 못함·명령 모름·거절이 서로 다르게 보인다", async () => {
  const check = path.join(HERE, "check-test.mjs").replace(/\\/g, "/");
  git("checkout", "-q", "--", ".");

  // 거절: 초록 표시가 없고 돌리지 않았다고 말한다
  fs.writeFileSync(projCfg, `[agent]\ntest_command = "node ${check} {files}"\n`);
  let r = await verifyRun("검증 시나리오: 테스트를 거절한다", [true, false]);
  if (r.state !== "skipped" || !r.text.includes("돌리지 않았습니다")) throw new Error(`거절: ${JSON.stringify(r)}`);
  if (r.head.includes("codicon-pass")) throw new Error(`거절했는데 머리에 초록 체크: ${r.head}`);
  if (!r.tool?.includes("skip") || r.tool.includes(" ok")) throw new Error(`거절한 테스트 카드: ${r.tool}`);
  if (!r.task.includes("s-unverified")) throw new Error(`작업 목록 표시: ${r.task}`);
  git("checkout", "-q", "--", ".");

  // 통과: 실패 → 다시 고침 → 통과
  r = await verifyRun("검증 시나리오: 고쳐서 통과한다", [true, true, true, true]);
  if (r.state !== "passed" || !r.tool?.includes(" ok")) throw new Error(`통과: ${JSON.stringify(r)}`);
  if (r.task.includes("s-unverified")) throw new Error(`통과인데 작업 목록 표시: ${r.task}`);
  git("checkout", "-q", "--", ".");

  // 실패: 고쳐도 같은 실패가 반복되면 멈춘다
  fs.writeFileSync(projCfg, `[agent]\ntest_command = "node ${check} --fail {files}"\n`);
  r = await verifyRun("검증 시나리오: 계속 실패한다", [true, true, true, true]);
  if (r.state !== "failed" || !r.text.includes("실패")) throw new Error(`실패: ${JSON.stringify(r)}`);
  if (!r.task.includes("s-unverified")) throw new Error(`실패인데 작업 목록 표시: ${r.task}`);
  git("checkout", "-q", "--", ".");
  fs.rmSync(projCfg, { force: true });

  // 명령 모름: 관련 테스트는 있는데 이 프로젝트의 테스트 명령을 알 수 없다
  useSig();
  await newTask();
  await run(ui.sendTask, "완료 카드 확인: 명령 모름", "code", false);
  await pendingCard();
  await run(ui.approve);
  const row = (cls) => waitFor((c) => document.querySelector(`.task-log:not(.hidden) .changed .ch-test.${c}`)?.textContent, 20000, `완료 카드 ${cls}`, cls);
  await row("no_command");
  await idle();
  git("checkout", "-q", "--", ".");

  // 찾지 못함: 연결된 테스트가 없는 함수 (route)
  mock.edit = "src/api.ts";
  mock.old = "export function route";
  mock.new = "export function routed";
  await newTask();
  await run(ui.sendTask, "완료 카드 확인: 테스트 없음", "code", false);
  await pendingCard();
  await run(ui.approve);
  await row("none_found");
  await idle();
  git("checkout", "-q", "--", ".");
});

scenario("승인 대기 중 중단: 카드가 닫히고, 남은 버튼을 눌러도 적용됨으로 바뀌지 않는다", async () => {
  git("checkout", "-q", "--", ".");
  await newTask();
  useSig();
  await run(ui.sendTask, "중단 확인", "code", false);
  await pendingCard();
  await run(() => {
    window.__stopped = document.querySelector(".task-log:not(.hidden) .approval:not(.resolved)");
    window.__apply = window.__stopped.querySelector(".actions .btn-primary");
    document.querySelector("#btn-stop").click();
    return true;
  });
  await idle();
  await waitFor(() => window.__stopped.querySelector(".verdict").textContent === "중단됨", 5000, "중단됨 표시");
  await run(() => {
    window.__apply.click();
    return true;
  });
  await sleep(600);
  const m = await run(() => ({ verdict: window.__stopped.querySelector(".verdict").textContent, shown: !!window.__apply.offsetParent, meta: document.querySelector("#task-list .task.active .task-meta")?.textContent ?? "" }));
  if (m.verdict !== "중단됨" || m.shown) throw new Error(`중단한 카드: ${JSON.stringify(m)}`);
  if (m.meta.includes("수정")) throw new Error(`적용하지 않았는데 발자취에 수정: ${m.meta}`);
  if (file(SESSION).includes(".typed")) throw new Error("중단했는데 파일이 바뀜");
});

/** 지도의 카메라 (--trace 없이 진단 기록을 켜서 읽는다) */
async function camera() {
  const line = [...traceLog].reverse().find((l) => l.includes("draw k="));
  const m = line?.match(/k=([\d.]+) x=(-?[\d.]+) y=(-?[\d.]+)/);
  return m && { k: Number(m[1]), x: Number(m[2]), y: Number(m[3]) };
}

scenario("승인 대기: 지도가 스스로 영향 반경으로 바뀌고, 결정하면 보던 화면과 카메라로 돌아온다", async () => {
  await newTask();
  useSig();
  await run(() => {
    document.querySelector("#canvas-switch [data-canvas='map']").click();
    return true;
  });
  await waitFor(() => (document.querySelector("#map-crumb .cur")?.textContent ?? "").includes("전체 구조"), 15000, "전체 구조");
  await sleep(2500); // 배치가 가라앉을 때까지
  await cdp("Runtime.enable");
  await run(() => {
    localStorage.setItem("lantern.trace", "1");
    return true;
  });
  await sleep(1300);
  // 사용자가 확대해 둔다
  const zoom = () => run(() => {
    const c = document.querySelector("#map-canvas");
    const r = c.getBoundingClientRect();
    for (let i = 0; i < 3; i++) c.dispatchEvent(new WheelEvent("wheel", { deltaY: -240, clientX: r.left + r.width / 2, clientY: r.top + r.height / 2, bubbles: true, cancelable: true }));
    return true;
  });
  await zoom();
  await sleep(300);
  const before = await camera();
  if (!before) throw new Error(`카메라를 읽지 못함: ${traceLog.slice(-3).join(" / ")}`);

  await run(ui.sendTask, "지도 확인: 승인 대기", "code", false);
  await pendingCard();
  const view = await waitFor(() => {
    const crumb = document.querySelector("#map-crumb")?.textContent ?? "";
    return crumb.includes("영향 반경") && crumb.includes("signCookie") && { crumb, legend: !!document.querySelector("#map-legend .sw.impact0"), stats: document.querySelector("#map-footprint")?.textContent ?? "" };
  }, 15000, "지도가 영향 반경으로");
  if (!view.legend) throw new Error(`영향 지도 범례: ${JSON.stringify(view)}`);
  await sleep(300);
  const during = await camera();
  if (during && during.k === before.k && during.x === before.x) throw new Error("영향 지도가 맞춰지지 않음");

  // 적용: 전체 구조로 돌아오고, 파일 변경으로 지도를 다시 그려도 카메라는 그대로다
  await run(ui.approve);
  await waitFor(() => {
    const crumb = document.querySelector("#map-crumb")?.textContent ?? "";
    return crumb.includes("전체 구조") && !crumb.includes("영향 반경");
  }, 10000, "전체 구조로 돌아옴");
  await idle();
  await sleep(4000); // 다시 그리기(refreshSoon)와 배치가 끝날 때까지
  const after = await camera();
  await run(() => {
    localStorage.removeItem("lantern.trace");
    return true;
  });
  if (!after || after.k !== before.k || after.x !== before.x || after.y !== before.y) throw new Error(`카메라가 바뀜: ${JSON.stringify(before)} → ${JSON.stringify(after)}`);
  git("checkout", "-q", "--", ".");
});
