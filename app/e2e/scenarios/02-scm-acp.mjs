// E2E 시나리오: 소스 제어, 외부 에이전트, 커밋 전 검토, 테스트 만들기

import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { app, clickDialog, DIRS, file, git, HERE, launch, openScm, pickMenu, run, scenario, screenshot, sleep, stop, TMP, ui, waitDialog, waitFor } from "../harness.mjs";
import { acpLog, mcpLog, mock, MOCK_PORT } from "../mock-model.mjs";

scenario("소스 제어: 원격 연결, 커밋 이력, 푸시, 브랜치 만들기·전환", async () => {
  // 원격 저장소 흉내 (로컬 bare 저장소)
  const bare = path.join(TMP, "remote.git");
  execFileSync("git", ["init", "-q", "--bare", "-b", "main", bare]);
  git("remote", "add", "origin", bare);
  await openScm();
  await waitFor(() => document.querySelector("#scm-sync .scm-branch-btn")?.textContent.includes("main") && document.querySelector("#scm-sync .scm-remote")?.textContent.includes("remote"), 10000, "브랜치와 원격 표시");

  // 커밋 이력: 3개 (init, v2, v3), 커밋을 펼치면 바뀐 파일
  await run(() => {
    [...document.querySelectorAll(".scm-history .scm-group")][0].click();
    return true;
  });
  const subjects = await waitFor(() => {
    const rows = [...document.querySelectorAll(".scm-commit-row .csubject")].map((x) => x.textContent);
    return rows.length >= 3 && rows;
  }, 10000, "커밋 이력");
  if (subjects[0] !== "v3" || !subjects.includes("init")) throw new Error(`커밋 이력: ${subjects.join(", ")}`);
  await run(() => {
    document.querySelector(".scm-commit-row").click();
    return true;
  });
  const files = await waitFor(() => {
    const f = [...document.querySelectorAll(".scm-commit-files .scm-file .fname")].map((x) => x.textContent);
    return f.length && f;
  }, 10000, "커밋의 바뀐 파일");
  if (!files.includes("session.ts")) throw new Error(`v3에서 바뀐 파일: ${files.join(", ")}`);

  // 푸시: 업스트림이 없으면 origin에 올리고 연결
  await run(() => {
    [...document.querySelectorAll("#scm-sync .scm-sync-actions button")].find((b) => b.title.startsWith("푸시")).click();
    return true;
  });
  await waitFor(() => [...document.querySelectorAll(".toast")].some((t) => t.innerText.includes("원격에 올렸습니다")), 20000, "푸시 알림");
  const pushed = execFileSync("git", ["--git-dir", bare, "rev-parse", "main"], { encoding: "utf8" }).trim();
  if (pushed !== git("rev-parse", "HEAD").trim()) throw new Error("원격에 올라간 커밋이 다름");

  // 새 브랜치 만들기 → 상태 표시줄에도 → 다시 main으로
  await run(() => {
    document.querySelector("#scm-sync .scm-branch-btn").click();
    return true;
  });
  await waitFor(() => !!document.querySelector("#menu-popup:not(.hidden) .menu-item"), 5000, "브랜치 메뉴");
  if (!(await pickMenu("새 브랜치 만들기"))) throw new Error("브랜치 메뉴에 '새 브랜치 만들기' 없음");
  // 입력칸은 메뉴를 고른 뒤에 그려진다 (느린 CI에서 바로 찾으면 없을 수 있다)
  await waitFor(() => !!document.querySelector(".scm-newbranch"), 5000, "새 브랜치 입력칸");
  await run(() => {
    const i = document.querySelector(".scm-newbranch");
    i.value = "e2e/feature";
    i.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    return true;
  });
  await waitFor(() => document.querySelector("#scm-sync .scm-branch-btn")?.textContent.includes("e2e/feature") && document.querySelector("#sb-branch")?.textContent.includes("e2e/feature"), 10000, "새 브랜치로 전환");
  if (git("branch", "--show-current").trim() !== "e2e/feature") throw new Error("git 브랜치가 바뀌지 않음");
  await run(() => {
    document.querySelector("#scm-sync .scm-branch-btn").click();
    return true;
  });
  await waitFor(() => !!document.querySelector("#menu-popup:not(.hidden) .menu-item"), 5000, "브랜치 메뉴");
  if (!(await pickMenu("main"))) throw new Error("브랜치 메뉴에 main 없음");
  await waitFor(() => document.querySelector("#scm-sync .scm-branch-btn")?.textContent.includes("main"), 10000, "main으로 전환");
  if (git("branch", "--show-current").trim() !== "main") throw new Error("main으로 돌아가지 않음");
});

scenario("외부 에이전트(ACP): 로그인 카드 → 로그인 → 읽기 → 승인 카드 → 적용", async () => {
  await run(() => {
    document.querySelector("#btn-new-task").click();
    return true;
  });
  await run(ui.sendTask, "로그인 경로를 /signin으로 바꿔줘", "acp:e2e", false);
  // 로그인 전: 로그인 카드 (환경변수 방법은 버튼이 아니라 안내)
  const card = await waitFor(() => {
    const c = document.querySelector(".task-log:not(.hidden) .auth-card");
    return c && { buttons: [...c.querySelectorAll("button")].map((b) => b.textContent), text: c.innerText };
  }, 30000, "로그인 카드");
  if (card.buttons.length !== 1 || !card.buttons[0].includes("Log in with E2E") || !card.text.includes("E2E_KEY")) throw new Error(`로그인 카드: ${JSON.stringify(card)}`);
  const init = acpLog().find((l) => l.clientCapabilities);
  if (!init?.clientCapabilities?.fs?.writeTextFile || init.clientCapabilities.terminal !== false) throw new Error(`클라이언트 기능: ${JSON.stringify(init)}`);
  if (acpLog().find((l) => "mcp" in l)?.mcp !== "--mcp") throw new Error("Lantern MCP 서버를 넘기지 않음");
  await run(() => {
    document.querySelector(".task-log:not(.hidden) .auth-card button").click();
    return true;
  });
  await waitFor(() => document.querySelector(".task-log:not(.hidden) .auth-card .auth-status")?.textContent.includes("로그인되었습니다"), 15000, "로그인");
  await run(() => {
    [...document.querySelectorAll(".task-log:not(.hidden) .auth-card button")].find((b) => b.textContent.includes("다시 보내기")).click();
    return true;
  });
  // 수정 권한 요청 → 기존과 같은 승인 카드와 영향 반경
  const impact = await waitFor(() => document.querySelector(".task-log:not(.hidden) .approval .impact:not(.loading)")?.innerText, 30000, "영향 반경");
  if (!impact.includes("직접 호출") && !impact.includes("찾지 못했습니다")) throw new Error(`영향 반경: ${impact}`);
  if (file("src/api.ts").includes("/signin")) throw new Error("승인 전에 파일이 바뀜");
  if (!acpLog().some((l) => l.hasContext) || !acpLog().some((l) => typeof l.read === "string" && l.read.includes("/login"))) throw new Error("맥락 또는 파일 읽기가 에이전트에 가지 않음");
  await run(ui.approve);
  await waitFor(() => !!document.querySelector(".task-log:not(.hidden) .changed"), 15000, "완료");
  if (!file("src/api.ts").includes("'/signin'")) throw new Error("적용 후 파일이 그대로");
  if (!acpLog().some((l) => l.outside === "rejected") || fs.existsSync(path.join(TMP, "acp-escape.txt"))) throw new Error("프로젝트 밖 쓰기를 막지 못함");
  const meta = await run(() => document.querySelector("#task-list .task.active .task-meta")?.textContent ?? "");
  if (!meta.includes("수정 1")) throw new Error(`발자취: ${meta}`);
});

scenario("커밋 전 영향 검토와 AI 커밋 메시지", async () => {
  // 사람이 직접 고친 변경: signCookie 본문 (호출자 issueSession, 테스트 session.test.ts)
  fs.writeFileSync(path.join(DIRS.proj, "src/auth/session.ts"), file("src/auth/session.ts").replace("v + '.sig'", "v + '.sig2'"));
  await run(() => {
    document.querySelector('.ab-item[data-view="scm"]').click();
    return true;
  });
  await waitFor(() => !document.querySelector("#scm-commit").classList.contains("hidden"), 15000, "소스 제어");
  await run(() => {
    document.querySelector("#scm-review-btn").click();
    return true;
  });
  const rv = await waitFor(() => {
    const box = document.querySelector("#scm-review:not(.hidden)");
    const rows = box ? [...box.querySelectorAll(".rv-file")].map((r) => r.innerText.replace(/\s+/g, " ")) : [];
    return rows.length ? { head: box.querySelector(".rv-head").innerText, rows } : null;
  }, 20000, "영향 검토 결과");
  const row = rv.rows.find((r) => r.includes("session.ts"));
  // signCookie를 부르는 곳: issueSession, 테스트의 testSign
  if (!row || !/직접 2/.test(row) || !/테스트 1/.test(row)) throw new Error(`영향 검토: ${JSON.stringify(rv)}`);
  // 파일을 누르면 지도에서 영향 범위
  await run(() => {
    [...document.querySelectorAll("#scm-review .rv-file")].find((r) => r.innerText.includes("session.ts")).click();
    return true;
  });
  await waitFor(() => document.querySelector("#map-crumb")?.textContent.includes("영향"), 10000, "지도의 영향 범위");
  // AI 커밋 메시지
  await run(() => {
    document.querySelector("#scm-gen-msg").click();
    return true;
  });
  const msg = await waitFor(() => document.querySelector("#scm-message").value, 15000, "커밋 메시지");
  if (msg !== "FIX: 로그인 경로 이름 변경") throw new Error(`커밋 메시지: ${msg}`);
  if (!mock.commitPrompt.includes(".sig2") || !mock.commitPrompt.includes("Recent commit subjects")) throw new Error("커밋 메시지 요청에 diff·최근 커밋이 없음");
  // 다음 시나리오를 위해 되돌린다
  fs.writeFileSync(path.join(DIRS.proj, "src/auth/session.ts"), file("src/auth/session.ts").replace("'.sig2'", "'.sig'"));
  await run(() => {
    document.querySelector("#scm-message").value = "";
    document.querySelector('.ab-item[data-view="tasks"]').click();
    return true;
  });
});

scenario("테스트 없는 변경 → 테스트 만들기: 기존 테스트를 예시로 새 작업", async () => {
  // handleLogin은 route가 부르지만 테스트가 없다
  fs.writeFileSync(path.join(DIRS.proj, "src/auth/login.ts"), file("src/auth/login.ts").replace("return s;", "return s.trim();"));
  await run(() => {
    document.querySelector('.ab-item[data-view="scm"]').click();
    return true;
  });
  await waitFor(() => !document.querySelector("#scm-commit").classList.contains("hidden"), 15000, "소스 제어");
  await run(() => {
    document.querySelector("#scm-review-btn").click();
    return true;
  });
  await waitFor(() => {
    const b = [...document.querySelectorAll("#scm-review .rv-actions button")].find((x) => x.textContent.includes("테스트 만들기"));
    if (!b) return false;
    b.click();
    b.click(); // 두 번 눌러도 작업은 하나
    return true;
  }, 20000, "테스트 만들기 버튼");
  const prompt = await waitFor(() => document.querySelector("#messages")?.innerText.includes("테스트를 만들었습니다") && true, 20000, "테스트 작업의 응답");
  if (!prompt || !mock.testPrompt.includes("src/auth/login.ts: handleLogin") || !mock.testPrompt.includes("tests/session.test.ts")) throw new Error(`테스트 요청: ${mock.testPrompt}`);
  if (mock.testPrompt.includes("session.ts:")) throw new Error("테스트가 있는 파일까지 요청함");
  const made = await run(() => [...document.querySelectorAll("#task-list .task .task-title")].filter((x) => x.textContent.includes("테스트를 만들어줘")).length);
  if (made !== 1) throw new Error(`테스트 작업 ${made}개`);
  git("checkout", "--", "src/auth/login.ts");
  await run(() => {
    document.querySelector("#scm-review .rv-head .codicon-close")?.closest("button")?.click();
    document.querySelector('.ab-item[data-view="tasks"]').click();
    return true;
  });
});
