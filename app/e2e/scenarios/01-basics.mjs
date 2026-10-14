// E2E 시나리오: 기본: 지도, 제목 표시줄, 에이전트 수정·되돌리기·격리, 바꾸기, 지도 검색

import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { app, clickDialog, DIRS, file, git, HERE, launch, openScm, pickMenu, run, scenario, screenshot, sleep, stop, TMP, ui, waitDialog, waitFor } from "../harness.mjs";
import { acpLog, mcpLog, mock, MOCK_PORT } from "../mock-model.mjs";

scenario("프로젝트를 열면 코드 지도와 빈 작업 목록", async () => {
  const s = await run(() => ({
    crumb: document.querySelector("#map-crumb")?.textContent ?? "",
    nodes: Number(document.querySelector(".map-count")?.textContent?.replace(/\D/g, "") || 0),
    empty: !!document.querySelector("#task-list .empty-view"),
  }));
  if (!s.crumb.includes("전체 구조")) throw new Error(`지도 제목: ${s.crumb}`);
  if (s.nodes < 4) throw new Error(`지도 노드 ${s.nodes}개`);
  if (!s.empty) throw new Error("작업 목록이 비어 있지 않음");
});

scenario("제목 표시줄의 빈 곳은 어디든 창을 끌 수 있다", async () => {
  // 창 테두리를 직접 그려서, data-tauri-drag-region이 붙은 요소를 직접 눌러야 끌린다(자식은 해당 없음).
  // 제목 표시줄을 가로로 훑어 단추·입력이 아닌 곳이 모두 끌기 영역인지 본다.
  const bad = await run(() => {
    const bar = document.querySelector("#titlebar").getBoundingClientRect();
    const out = [];
    for (let x = 1; x < bar.width - 1; x += 4) {
      for (const y of [bar.top + 3, bar.top + bar.height / 2, bar.bottom - 3]) {
        const el = document.elementFromPoint(x, y);
        if (!el || el.closest("button, input, select, a, [role=tab], [role=tablist], [role=menuitem]")) continue;
        if (!el.hasAttribute("data-tauri-drag-region")) out.push(`x=${Math.round(x)} ${el.tagName.toLowerCase()}${el.id ? "#" + el.id : ""}${typeof el.className === "string" && el.className ? "." + el.className.split(" ")[0] : ""}`);
      }
    }
    return [...new Set(out)].slice(0, 8);
  });
  if (bad.length) throw new Error(`끌리지 않는 빈 곳: ${bad.join(", ")}`);
});

scenario("에이전트 수정: 승인 카드의 영향 반경 → 적용 → 발자취", async () => {
  mock.old = "v + '.sig'";
  mock.new = "v + '.signed'";
  await run(ui.sendTask, "서명 접미사를 바꿔줘", "code", false);
  const impact = await waitFor(() => document.querySelector(".task-log:not(.hidden) .approval .impact:not(.loading)")?.innerText, 20000, "영향 반경");
  if (!impact.includes("signCookie") || !impact.includes("직접 호출")) throw new Error(`영향 반경: ${impact}`);
  if (file("src/auth/session.ts").includes(".signed")) throw new Error("승인 전에 파일이 바뀜");
  await run(ui.approve);
  await waitFor(() => !!document.querySelector(".task-log:not(.hidden) .changed"), 15000, "완료");
  if (!file("src/auth/session.ts").includes("'.signed'")) throw new Error("적용 후 파일이 그대로");
  const meta = await run(() => document.querySelector("#task-list .task.active .task-meta")?.textContent ?? "");
  if (!meta.includes("수정 1") || !meta.includes("읽음 1")) throw new Error(`발자취: ${meta}`);
});

scenario("되돌리기: 확인 후 원래 파일", async () => {
  await run(() => {
    setTimeout(() => document.querySelector(".task-log:not(.hidden) .ch-undo")?.click(), 50);
    return true;
  });
  await waitDialog(0);
  await waitFor(() => document.querySelector(".task-log:not(.hidden) .changed")?.classList.contains("reverted"), 8000, "되돌림 표시");
  if (!file("src/auth/session.ts").includes("'.sig'")) throw new Error("파일이 되돌아가지 않음");
});

scenario("격리 작업: 작업 공간에서만 바뀌고, 적용해야 프로젝트에 반영", async () => {
  await run(() => {
    document.querySelector("#btn-new-task").click();
    return true;
  });
  mock.old = "v + '.sig'";
  mock.new = "v + '.iso'";
  await run(ui.sendTask, "격리해서 바꿔줘", "code", true);
  await waitFor(() => !!document.querySelector(".task-log:not(.hidden) .approval .actions"), 20000, "승인 카드");
  await run(ui.approve);
  await waitFor(() => !!document.querySelector(".task-log:not(.hidden) .iso-box .iso-actions"), 20000, "격리 결과 상자");
  if (file("src/auth/session.ts").includes(".iso")) throw new Error("적용 전에 프로젝트가 바뀜");
  await run(() => {
    [...document.querySelectorAll(".task-log:not(.hidden) .iso-box button")].find((b) => b.textContent.includes("프로젝트에 적용"))?.click();
    return true;
  });
  await waitFor(() => document.querySelector(".task-log:not(.hidden) .iso-box")?.classList.contains("done"), 15000, "적용 완료");
  if (!file("src/auth/session.ts").includes("'.iso'")) throw new Error("적용 후 프로젝트가 그대로");
  git("checkout", "-q", "--", ".");
});

scenario("파일에서 바꾸기와 되돌리기", async () => {
  await run(() => {
    document.querySelector('.ab-item[data-view="search"]').click();
    const q = document.querySelector("#search-input");
    q.value = "handleLogin";
    q.dispatchEvent(new Event("input"));
    return true;
  });
  await waitFor(() => /2개 파일/.test(document.querySelector("#search-summary")?.textContent ?? ""), 8000, "검색 결과");
  await run(() => {
    if (document.querySelector("#replace-row").classList.contains("hidden")) document.querySelector("#search-expand").click();
    document.querySelector("#replace-input").value = "handleSignIn";
    setTimeout(() => document.querySelector("#replace-all").click(), 50);
    return true;
  });
  await waitDialog(0);
  // "곳을 바꿨습니다"로 찾는다. "바꿨습니다"만으로는 에이전트 수정 알림("Lantern이 파일 N개를 바꿨습니다")도 걸려서,
  // CI처럼 느린 환경에서 앞 시나리오의 알림이 남아 있으면 되돌리기 단추가 없는 알림을 고른다.
  try {
    await waitFor(() => [...document.querySelectorAll(".toast")].some((t) => t.innerText.includes("곳을 바꿨습니다")), process.env.CI ? 20000 : 8000, "바꾸기 알림");
  } catch (e) {
    const dialog = execFileSync("powershell", ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", path.join(HERE, "dialog.ps1"), "-ProcId", String(app.pid)], { encoding: "utf8" }).trim();
    const toasts = await run(() => [...document.querySelectorAll(".toast")].map((t) => t.innerText.replace(/\s+/g, " ")).join(" | "));
    throw new Error(`${e.message}\n      알림: ${toasts || "없음"}\n      파일: ${file("src/api.ts").includes("handleSignIn") ? "바뀜" : "그대로"}\n      ${dialog.replace(/\n/g, "\n      ")}`);
  }
  if (!file("src/api.ts").includes("handleSignIn") || !file("src/auth/login.ts").includes("handleSignIn")) throw new Error("바꾸지 않음");
  const clicked = await run(() => {
    const t = [...document.querySelectorAll(".toast")].find((x) => x.innerText.includes("곳을 바꿨습니다"));
    const b = t && [...t.querySelectorAll("button")].find((x) => x.textContent.includes("되돌리기"));
    b?.click();
    return !!b;
  });
  if (!clicked) throw new Error("바꾸기 알림에 되돌리기 단추가 없음");
  try {
    await waitFor(() => [...document.querySelectorAll(".toast")].some((t) => t.innerText.includes("되돌렸습니다")), process.env.CI ? 20000 : 8000, "되돌림 알림");
  } catch (e) {
    // 네이티브 대화상자(예: 충돌 확인)는 화면 캡처에 찍히지 않으니 문구를 남긴다
    const dialog = execFileSync("powershell", ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", path.join(HERE, "dialog.ps1"), "-ProcId", String(app.pid)], { encoding: "utf8" }).trim();
    const toasts = await run(() => [...document.querySelectorAll(".toast")].map((t) => t.innerText.replace(/\s+/g, " ")).join(" | "));
    throw new Error(`${e.message}\n      알림: ${toasts || "없음"}\n      ${dialog.replace(/\n/g, "\n      ")}`);
  }
  if (file("src/api.ts").includes("handleSignIn")) throw new Error("되돌리지 않음");
});

scenario("지도: 검색해서 주변 보기", async () => {
  await run(() => {
    document.querySelector('#canvas-switch [data-canvas="map"]').click();
    const i = document.querySelector("#map-search");
    i.value = "signCookie";
    i.dispatchEvent(new Event("input"));
    return true;
  });
  await waitFor(() => !!document.querySelector("#map-results .mr-row"), 5000, "지도 검색 결과");
  await run(() => {
    document.querySelector("#map-results .mr-row").dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
    return true;
  });
  const crumb = await waitFor(() => (document.querySelector("#map-crumb")?.textContent ?? "").includes("signCookie") && document.querySelector("#map-crumb").textContent, 8000, "주변 보기");
  const n = Number(crumb.replace(/.*signCookie/, "").replace(/\D/g, ""));
  if (n < 3) throw new Error(`주변 노드 ${n}개 (호출자 issueSession, testSign과 파일이 있어야 함)`);
});

/** 소스 제어 뷰를 연다 (이미 보이는 뷰의 아이콘을 다시 누르면 사이드바가 접히므로 안 보일 때만 누른다) */
