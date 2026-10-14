// E2E 시나리오: MCP, 고친 뒤 테스트 확인, 웹 문서, 재시도, 대화 줄이기, @ 멘션, 계획 먼저

import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { app, clickDialog, DIRS, file, git, HERE, launch, openScm, pickMenu, run, scenario, screenshot, sleep, stop, TMP, ui, waitDialog, waitFor } from "../harness.mjs";
import { acpLog, mcpLog, mock, MOCK_PORT } from "../mock-model.mjs";

scenario("외부 MCP 서버: 에이전트 도구로 보이고, 승인 카드 → 실행 → 결과로 답", async () => {
  await run(() => {
    document.querySelector("#btn-new-task").click();
    return true;
  });
  await run(ui.sendTask, "티켓 T-1 확인해줘", "code", false);
  const card = await waitFor(() => {
    const c = [...document.querySelectorAll(".task-log:not(.hidden) .approval:not(.resolved)")].at(-1);
    return c && { title: c.querySelector(".atitle")?.textContent ?? "", plug: !!c.querySelector(".ahead .codicon-plug"), detail: c.querySelector("pre")?.textContent ?? "" };
  }, 30000, "MCP 승인 카드");
  if (!card.title.includes("tracker · lookup_ticket") || !card.plug || !card.detail.includes("T-1")) throw new Error(`승인 카드: ${JSON.stringify(card)}`);
  if (!mock.toolNames.includes("mcp__tracker__lookup_ticket") || !mock.toolNames.includes("mcp__tracker__list_tickets")) throw new Error(`모델에 보인 도구: ${mock.toolNames}`);
  if (mcpLog().some((l) => l.call)) throw new Error("승인 전에 도구가 실행됨");
  await run(() => {
    [...document.querySelectorAll(".task-log:not(.hidden) .approval:not(.resolved) .actions button")].find((b) => b.textContent.includes("실행")).click();
    return true;
  });
  await waitFor(() => document.querySelector(".task-log:not(.hidden)")?.innerText.includes("티켓 내용: T-1: 로그인 쿠키 만료가 너무 짧음"), 20000, "도구 결과로 답");
  const calls = mcpLog().filter((l) => l.call);
  if (calls.length !== 1 || calls[0].call !== "lookup_ticket" || calls[0].args?.id !== "T-1") throw new Error(`MCP 호출: ${JSON.stringify(calls)}`);
  // 설정 화면: 서버 상태와 도구, 도구를 눌러 '승인 없이'로
  await run(() => {
    document.querySelector("#ab-settings").click();
    return true;
  });
  const row = await waitFor(() => {
    const r = document.querySelector('[data-mcp="tracker"]');
    return r?.querySelector(".pill.ok") && r.innerText;
  }, 15000, "MCP 서버 상태");
  if (!row.includes("도구 2개") || !row.includes("lookup_ticket")) throw new Error(`서버 상태: ${row}`);
  await run(() => {
    [...document.querySelectorAll('[data-mcp="tracker"] .tool-chip')].find((b) => b.textContent.includes("lookup_ticket")).click();
    return true;
  });
  const cfgPath = path.join(DIRS.home, "config.toml");
  const end = Date.now() + 5000;
  while (!/auto_approve = \["lookup_ticket"\]/.test(fs.readFileSync(cfgPath, "utf8")) && Date.now() < end) await sleep(100);
  if (!/auto_approve = \["lookup_ticket"\]/.test(fs.readFileSync(cfgPath, "utf8"))) throw new Error(`auto_approve 저장 안 됨:\n${fs.readFileSync(cfgPath, "utf8")}`);
  await run(() => {
    document.querySelector('.ab-item[data-view="tasks"]').click();
    return true;
  });
});

scenario("고친 뒤 관련 테스트로 확인: 실패 → 출력을 받아 다시 고침 → 통과", async () => {
  // 이 프로젝트만: 테스트 명령 (프로젝트 설정은 승인 없이 실행할 명령을 늘리지 못하므로 실행도 승인한다)
  const projCfg = path.join(DIRS.proj, ".lantern", "config.toml");
  const check = path.join(HERE, "check-test.mjs").replace(/\\/g, "/");
  fs.writeFileSync(projCfg, `[agent]\ntest_command = "node ${check} {files}"\n`);
  await run(() => {
    document.querySelector("#btn-new-task").click();
    return true;
  });
  await run(ui.sendTask, "검증 시나리오: 서명 접미사를 바꿔줘", "code", false);
  // 틀린 수정 → 테스트 실행 → (실패) → 되돌리는 수정 → 테스트 실행: 승인 카드가 하나씩 차례로 온다
  for (let i = 0; i < 4; i++) {
    await waitFor(() => !!document.querySelector(".task-log:not(.hidden) .approval:not(.resolved)"), 30000, `승인 카드 ${i + 1}`);
    const kind = await run(() => {
      const card = document.querySelector(".task-log:not(.hidden) .approval:not(.resolved)");
      card.querySelector(".actions .btn-primary").click();
      return card.dataset.kind;
    });
    if (kind !== (i % 2 ? "command" : "edit")) throw new Error(`${i + 1}번째 승인 카드 종류: ${kind}`);
    await waitFor((n) => document.querySelectorAll(".task-log:not(.hidden) .approval.resolved").length === n, 10000, `승인 반영 ${i + 1}`, i + 1);
  }
  const cards = await waitFor(() => {
    const c = [...document.querySelectorAll(".task-log:not(.hidden) details.tool")].filter((d) => d.querySelector(".tname")?.textContent.includes("관련 테스트로 확인"));
    return c.length >= 2 && !c.some((d) => d.querySelector(".tstate.run")) && c.map((d) => ({ ok: !!d.querySelector(".tstate.ok"), text: d.querySelector("pre")?.textContent ?? "" }));
  }, 30000, "테스트 확인 두 번");
  if (cards.length !== 2 || cards[0].ok || !cards[1].ok) throw new Error(`테스트 카드: ${JSON.stringify(cards)}`);
  if (!cards[1].text.includes("PASS") || !cards[0].text.includes("tests/session.test.ts")) throw new Error(`테스트 출력: ${JSON.stringify(cards)}`);
  if (!mock.verifyFailure.includes("FAIL testSign")) throw new Error("실패 출력이 모델에 가지 않음");
  if (!file("src/auth/session.ts").includes("v + '.sig'")) throw new Error("다시 고친 결과가 파일에 없음");
  fs.rmSync(projCfg);
  git("checkout", "--", "src/auth/session.ts");
});

scenario("웹 문서 가져오기: 처음 여는 사이트만 승인 → 본문 글로 읽기", async () => {
  await run(() => {
    document.querySelector("#btn-new-task").click();
    return true;
  });
  await run(ui.sendTask, "쿠키 서명 문서를 찾아봐", "ask", false);
  const card = await waitFor(() => {
    const c = document.querySelector(".task-log:not(.hidden) .approval:not(.resolved)");
    return c && { title: c.querySelector(".atitle")?.textContent ?? "", globe: !!c.querySelector(".ahead .codicon-globe"), detail: c.querySelector("pre")?.textContent ?? "" };
  }, 20000, "사이트 승인 카드");
  if (!card.title.includes("127.0.0.1") || !card.globe || !card.detail.includes("/docs/cookie")) throw new Error(`승인 카드: ${JSON.stringify(card)}`);
  if (mock.fetched.length) throw new Error("승인 전에 가져옴");
  await run(() => {
    [...document.querySelectorAll(".task-log:not(.hidden) .approval:not(.resolved) .actions button")].find((b) => b.textContent.includes("가져오기")).click();
    return true;
  });
  await waitFor(() => document.querySelector(".task-log:not(.hidden)")?.innerText.includes("문서 요약"), 20000, "요약");
  // 같은 사이트의 두 번째 페이지는 묻지 않는다
  const approvals = await run(() => document.querySelectorAll(".task-log:not(.hidden) .approval").length);
  if (approvals !== 1 || mock.fetched.join() !== "/docs/cookie,/docs/session") throw new Error(`승인 ${approvals}개, 가져온 곳 ${mock.fetched}`);
  if (!mock.fetchResult.includes("Cookie signing") || !mock.fetchResult.includes("HMAC") || mock.fetchResult.includes("steal()") || mock.fetchResult.includes("<p>")) throw new Error(`본문: ${mock.fetchResult}`);
});

scenario("일시적 오류: 과부하(529)면 기다렸다가 다시 보내 답을 받는다", async () => {
  await run(() => {
    document.querySelector("#btn-new-task").click();
    return true;
  });
  await run(ui.sendTask, "재시도 확인", "ask", false);
  const r = await waitFor(() => {
    const log = document.querySelector(".task-log:not(.hidden)");
    const note = log?.querySelector(".note.retry:not(.waiting)");
    return note && log.querySelector(".md") && { note: note.textContent, done: !log.querySelector(".err-box") };
  }, 20000, "다시 보낸 뒤 답");
  if (!mock.overloaded || !r.note.includes("다시 보냈습니다") || !r.done) throw new Error(`재시도: ${JSON.stringify(r)}`);
});

scenario("긴 작업: 대화가 모델 입력 한도에 가까우면 줄이고 계속한다", async () => {
  const big = path.join(DIRS.proj, "big.txt");
  fs.writeFileSync(big, Array.from({ length: 4000 }, (_, i) => `line ${i} ${"lorem ipsum ".repeat(4)}`).join("\n"));
  await run(() => {
    document.querySelector("#btn-new-task").click();
    return true;
  });
  await run(ui.sendTask, "긴 작업 확인", "ask", false);
  const r = await waitFor(() => {
    const log = document.querySelector(".task-log:not(.hidden)");
    return log?.innerText.includes("다 읽었습니다") && { note: log.querySelector(".note.compacted")?.textContent ?? "", err: !!log.querySelector(".err-box") };
  }, 60000, "긴 작업 끝");
  fs.rmSync(big);
  if (!r.note.includes("줄였습니다") || r.err) throw new Error(`대화 줄이기: ${JSON.stringify(r)}`);
  if (!mock.compactSeen) throw new Error("줄인 표시가 모델에 가지 않음");
  // 8번 × 2만 자를 그대로 보냈다면 16만 자를 넘는다
  if (mock.compactMax > 140000) throw new Error(`줄인 뒤에도 요청이 큼: ${mock.compactMax}자`);
});

scenario("@로 파일·심볼 집어 넣기: 후보 → 칩 → 맥락에 꼭 붙음", async () => {
  await run(() => {
    document.querySelector("#btn-new-task").click();
    return true;
  });
  const typeAt = (text) => run((t) => {
    const p = document.querySelector("#prompt");
    p.focus();
    p.value = t;
    p.setSelectionRange(t.length, t.length);
    p.dispatchEvent(new Event("input", { bubbles: true }));
    return true;
  }, text);
  const pick = async (want) => {
    await waitFor((w) => [...document.querySelectorAll("#mention-pop:not(.hidden) .mention-item")].some((i) => i.textContent.includes(w)), 10000, `후보 ${want}`, want);
    await run((w) => {
      const items = [...document.querySelectorAll("#mention-pop .mention-item")];
      const i = items.findIndex((x) => x.querySelector(".mi-label").textContent === w);
      const p = document.querySelector("#prompt");
      for (let k = 0; k < i; k++) p.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }));
      p.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
      return true;
    }, want);
  };
  await typeAt("멘션 확인 @sess");
  await pick("session.ts");
  await typeAt("멘션 확인 @session.ts @issueS");
  await pick("issueSession");
  const chipsBefore = await run(() => [...document.querySelectorAll("#mentions .mention-chip")].map((c) => c.textContent));
  if (chipsBefore.join() !== "@session.ts,@issueSession") throw new Error(`칩: ${chipsBefore}`);
  // Enter가 후보 고르기에 쓰였고 아직 보내지 않았다
  if (mock.mentionPrompt) throw new Error("후보를 고르는 Enter에 메시지가 나감");
  await run(ui.sendTask, "멘션 확인 @session.ts @issueSession ", "ask", false);
  await waitFor(() => document.querySelector(".task-log:not(.hidden)")?.innerText.includes("지정한 코드를 받았습니다"), 20000, "답");
  const p = mock.mentionPrompt;
  if (!p.includes("# 사용자가 지정한 코드") || !p.includes("### `session.ts` (file, L1-") || !/### `issueSession` \(function, L\d+-\d+\) — 사용자가 지정/.test(p)) throw new Error(`맥락: ${p.slice(0, 800)}`);
  const ui2 = await run(() => ({
    sent: [...document.querySelectorAll(".task-log:not(.hidden) .user-mentions .mention-chip")].map((c) => c.textContent),
    left: document.querySelectorAll("#mentions .mention-chip").length,
  }));
  if (ui2.sent.join() !== "@session.ts,@issueSession" || ui2.left !== 0) throw new Error(`보낸 뒤 칩: ${JSON.stringify(ui2)}`);
});

scenario("계획 먼저: 읽기만 해서 계획 → 계획대로 진행하면 모든 도구", async () => {
  await run(() => {
    document.querySelector("#btn-new-task").click();
    const sel = document.querySelector("#agent-select");
    sel.value = "code";
    sel.dispatchEvent(new Event("change"));
    const box = document.querySelector("#plan-first");
    if (!box.checked) box.click();
    return true;
  });
  await run(ui.sendTask, "계획 시나리오: 서명 바꾸기", "code", false);
  await waitFor(() => !!document.querySelector(".task-log:not(.hidden) .plan-actions"), 20000, "계획과 진행 버튼");
  const [planTools] = mock.planTools;
  if (planTools.includes("edit_file") || planTools.includes("run_command") || !planTools.includes("read_file")) throw new Error(`계획 단계 도구: ${planTools}`);
  if (!mock.planSystem.includes("Plan first")) throw new Error("계획 지시가 시스템 프롬프트에 없음");
  await run(() => {
    [...document.querySelectorAll(".task-log:not(.hidden) .plan-actions button")].find((b) => b.textContent.includes("계획대로 진행")).click();
    return true;
  });
  await waitFor(() => document.querySelector(".task-log:not(.hidden)")?.innerText.includes("계획대로 진행했습니다"), 20000, "진행");
  const goTools = mock.planTools[1];
  if (!goTools?.includes("edit_file")) throw new Error(`진행 단계 도구: ${goTools}`);
  const left = await run(() => document.querySelectorAll(".task-log:not(.hidden) .plan-actions").length);
  if (left) throw new Error("진행한 뒤에도 진행 버튼이 남음");
  await run(() => {
    const box = document.querySelector("#plan-first");
    if (box.checked) box.click();
    return true;
  });
});
