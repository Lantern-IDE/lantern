// E2E 시나리오: Ctrl+K, 다시 켜기, 외부 에이전트 이어가기, 영어 화면, 여러 저장소

import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { app, clickDialog, DIRS, file, git, HERE, launch, openScm, pickMenu, run, scenario, screenshot, sleep, stop, TMP, ui, waitDialog, waitFor } from "../harness.mjs";
import { acpLog, mcpLog, mock, MOCK_PORT } from "../mock-model.mjs";

scenario("편집기 안 즉시 수정 (Ctrl+K): diff·영향 반경 → 적용 → 되돌리기", async () => {
  await run(() => {
    document.querySelector('.ab-item[data-view="explorer"]').click();
    document.querySelector("#canvas-switch [data-canvas='editor']").click();
    return true;
  });
  // session.ts를 열고 signCookie 본문의 v + '.sig'를 선택
  await run(async () => {
    for (const p of ["src", "src/auth"]) {
      const n = [...document.querySelectorAll("#tree .node[data-path]")].find((e) => e.dataset.path === p);
      if (n && n.getAttribute("aria-expanded") !== "true") n.click();
      await new Promise((r) => setTimeout(r, 400));
    }
    const f = [...document.querySelectorAll("#tree .node[data-path]")].find((e) => e.dataset.path === "src/auth/session.ts");
    f.click();
    f.dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
    return true;
  });
  await waitFor(() => [...document.querySelectorAll(".cm-editor")].some((e) => e.offsetParent && e.textContent.includes("signCookie")), 10000, "편집기");
  await run(() => {
    const ed = [...document.querySelectorAll(".cm-editor")].find((e) => e.offsetParent && e.textContent.includes("signCookie"));
    const view = ed.querySelector(".cm-content").cmTile.root.view;
    const text = view.state.doc.toString();
    const from = text.indexOf("v + '.sig'");
    view.dispatch({ selection: { anchor: from, head: from + "v + '.sig'".length } });
    view.focus();
    view.contentDOM.dispatchEvent(new KeyboardEvent("keydown", { key: "k", code: "KeyK", ctrlKey: true, bubbles: true }));
    return true;
  });
  await waitFor(() => !!document.querySelector(".inline-edit .ie-input"), 5000, "Ctrl+K 패널");
  await run(() => {
    const i = document.querySelector(".inline-edit .ie-input");
    i.value = "접미사를 .inline으로";
    i.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    return true;
  });
  const panel = await waitFor(() => {
    const p = document.querySelector(".inline-edit");
    const impact = p?.querySelector(".impact:not(.loading)");
    return impact && { diff: p.querySelector(".diff")?.innerText ?? "", impact: impact.innerText };
  }, 20000, "diff와 영향 반경");
  if (!panel.diff.includes(".inline") || !panel.impact.includes("signCookie")) throw new Error(`Ctrl+K 결과: ${JSON.stringify(panel)}`);
  if (!mock.inlinePrompt.includes("<selection>v + '.sig'</selection>")) throw new Error("선택 영역이 프롬프트에 없음");
  await run(() => {
    [...document.querySelectorAll(".inline-edit button")].find((b) => b.textContent.includes("적용")).click();
    return true;
  });
  const doc = () => run(() => [...document.querySelectorAll(".cm-editor")].find((e) => e.offsetParent && e.textContent.includes("signCookie")).querySelector(".cm-content").cmTile.root.view.state.doc.toString());
  if (!(await doc()).includes("v + '.inline'")) throw new Error("적용 후 편집기에 반영되지 않음");
  if (file("src/auth/session.ts").includes(".inline")) throw new Error("저장 전에 파일이 바뀜");
  // Ctrl+Z
  await run(() => {
    const view = [...document.querySelectorAll(".cm-editor")].find((e) => e.offsetParent && e.textContent.includes("signCookie")).querySelector(".cm-content").cmTile.root.view;
    view.focus();
    view.contentDOM.dispatchEvent(new KeyboardEvent("keydown", { key: "z", code: "KeyZ", ctrlKey: true, bubbles: true }));
    return true;
  });
  await sleep(300);
  if ((await doc()).includes(".inline")) throw new Error("Ctrl+Z로 되돌아가지 않음");
  // 되돌린 내용(원래와 같음)을 저장해 탭의 '저장 안 됨' 표시를 지운다 (다음 시나리오가 창을 닫는다)
  await run(() => {
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "s", code: "KeyS", ctrlKey: true, bubbles: true }));
    return true;
  });
  await sleep(500);
  if (file("src/auth/session.ts").includes(".inline")) throw new Error("되돌린 뒤 저장했는데 파일에 남음");
  await run(() => {
    document.querySelector('.ab-item[data-view="tasks"]').click();
    return true;
  });
});

scenario("다시 켜면 작업이 복원된다", async () => {
  await run(() => {
    setTimeout(() => document.querySelector("#win-close").click(), 50);
    return true;
  });
  const end = Date.now() + 10000;
  while (app.exitCode === null && Date.now() < end) await sleep(200);
  if (app.exitCode === null) throw new Error("닫기(X)로 창이 닫히지 않음");
  await launch();
  const titles = await waitFor(() => {
    const t = [...document.querySelectorAll("#task-list .task .task-title")].map((x) => x.textContent);
    return t.length >= 2 && t;
  }, 10000, "복원된 작업");
  if (!titles.includes("서명 접미사를 바꿔줘") || !titles.includes("격리해서 바꿔줘")) throw new Error(`작업 목록: ${titles.join(", ")}`);
});

scenario("외부 에이전트: 다시 켠 뒤에도 같은 대화를 이어간다 (session/load)", async () => {
  await run(() => {
    [...document.querySelectorAll("#task-list .task")].find((t) => t.textContent.includes("/signin으로 바꿔줘")).click();
    return true;
  });
  await run(ui.sendTask, "이어서 확인해줘", "acp:e2e", false);
  await waitFor(() => document.querySelector(".task-log:not(.hidden)")?.innerText.includes("앞의 대화를 기억합니다"), 30000, "이어진 대화의 답");
  const log = acpLog();
  const loads = log.filter((l) => "load" in l);
  if (loads.length !== 1 || loads[0].load !== "s1") throw new Error(`session/load: ${JSON.stringify(loads)}`);
  if (log.filter((l) => "mcp" in l).length !== 2) throw new Error("다시 켠 뒤 새 세션을 만듦"); // 로그인 전·후 두 번뿐
  if (!log.some((l) => l.followUp === "s1")) throw new Error("이어간 세션에 보내지 않음");
  const text = await run(() => document.querySelector(".task-log:not(.hidden)").innerText);
  if (text.includes("REPLAYED") || text.includes("새 대화로 시작")) throw new Error("다시 보낸 지난 대화가 화면에 쌓이거나 이어가지 못했다고 나옴");
});

scenario("영어 화면: 보이는 한국어가 없다", async () => {
  await run(() => {
    localStorage.setItem("lang", JSON.stringify("en"));
    setTimeout(() => location.reload(), 50);
    return true;
  });
  await sleep(1500);
  await waitFor(() => document.documentElement.lang === "en" && !!document.querySelector("#map-view:not(.hidden)"), 20000, "영어로 다시 불러옴");
  const left = await run(() => {
    const hangul = /[가-힣]/;
    const out = [];
    const w = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
    for (let n = w.nextNode(); n; n = w.nextNode()) {
      const el = n.parentElement;
      if (!hangul.test(n.data) || !el || el.closest(".cm-editor, .xterm, .md, .user-text, .diff, pre, code, #tree, [data-no-i18n], .task-log, .task-title, .th-title, .fp-title, .mem-text, .mem-topic-head, .mc-title, .mc-note")) continue;
      if (!el.offsetParent) continue;
      out.push(n.data.trim().slice(0, 40));
    }
    return out;
  });
  await run(() => {
    localStorage.setItem("lang", JSON.stringify("ko"));
    return true;
  });
  if (left.length) throw new Error(`번역 안 된 글자: ${left.slice(0, 5).join(" | ")}`);
});

scenario("여러 저장소를 담은 폴더: 저장소를 모두 찾고 골라서 본다", async () => {
  // 저장소 두 개를 담은 작업 폴더 (폴더 자체는 저장소가 아님)
  const ws = path.join(TMP, "ws");
  for (const [name, file] of [["api", "server.ts"], ["web", "app.ts"]]) {
    const dir = path.join(ws, name);
    fs.mkdirSync(dir, { recursive: true });
    const g = (...a) => execFileSync("git", ["-C", dir, ...a], { encoding: "utf8" });
    g("init", "-q", "-b", "main");
    g("config", "user.email", "e2e@example.com");
    g("config", "user.name", "e2e");
    fs.writeFileSync(path.join(dir, file), `export const ${name} = 1;\n`);
    g("add", "-A");
    g("commit", "-qm", `${name} 시작`);
  }
  fs.writeFileSync(path.join(ws, "web", "app.ts"), "export const web = 2;\n"); // web에만 변경 1개
  const trusted = JSON.parse(fs.readFileSync(path.join(DIRS.data, "trusted.json"), "utf8"));
  fs.writeFileSync(path.join(DIRS.data, "trusted.json"), JSON.stringify([...trusted, ws.replace(/\\/g, "/").toLowerCase()]));

  await stop();
  await launch(ws);
  await openScm();
  const names = await waitFor(() => {
    const r = [...document.querySelectorAll("#scm-repos .scm-repo .rname")].map((x) => x.textContent);
    return r.length === 2 && r;
  }, 15000, "저장소 목록");
  if (names.join(",") !== "api,web") throw new Error(`저장소 목록: ${names.join(", ")}`);
  if (await run(() => !!document.querySelector("#scm-body .btn-primary"))) throw new Error("하위에 저장소가 있는데 '저장소 만들기'를 권함");
  // 변경이 있는 저장소(web)를 먼저 고른다
  await waitFor(() => document.querySelector("#scm-repos .scm-repo.active .rname")?.textContent === "web" && document.querySelectorAll("#scm-body .scm-file").length === 1, 10000, "web 저장소의 변경");
  if (!(await run(() => document.querySelector("#sb-branch")?.textContent.includes("web: main")))) throw new Error("상태 표시줄에 저장소·브랜치가 없음");
  await run(() => {
    [...document.querySelectorAll("#scm-repos .scm-repo")].find((r) => r.textContent.includes("api")).click();
    return true;
  });
  await waitFor(() => document.querySelector("#scm-repos .scm-repo.active .rname")?.textContent === "api" && document.querySelectorAll("#scm-body .scm-file").length === 0, 10000, "api 저장소로 전환");
});
