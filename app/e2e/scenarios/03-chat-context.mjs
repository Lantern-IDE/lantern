// E2E 시나리오: 이미지 붙이기, 의미 검색과 그 설정

import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { app, clickDialog, DIRS, file, git, HERE, launch, openScm, pickMenu, run, scenario, screenshot, sleep, stop, TMP, ui, waitDialog, waitFor } from "../harness.mjs";
import { acpLog, mcpLog, mock, MOCK_PORT } from "../mock-model.mjs";

scenario("채팅에 이미지 붙여넣기: 미리보기 → 줄여서 모델에 보냄", async () => {
  await run(() => {
    document.querySelector("#btn-new-task").click();
    return true;
  });
  // 3000×2000 PNG를 붙여 넣는다 (긴 변 1568로 줄어야 한다)
  await run(async () => {
    const c = document.createElement("canvas");
    c.width = 3000;
    c.height = 2000;
    const g = c.getContext("2d");
    g.fillStyle = "#22d3ee";
    g.fillRect(0, 0, 3000, 2000);
    const blob = await new Promise((r) => c.toBlob(r, "image/png"));
    const dt = new DataTransfer();
    dt.items.add(new File([blob], "shot.png", { type: "image/png" }));
    document.querySelector("#prompt").dispatchEvent(new ClipboardEvent("paste", { clipboardData: dt, bubbles: true, cancelable: true }));
    return true;
  });
  await waitFor(() => document.querySelectorAll("#attachments .attach-thumb").length === 1, 10000, "미리보기");
  await run(ui.sendTask, "이 화면에서 버튼 색을 알려줘", "ask", false);
  await waitFor(() => document.querySelector(".task-log:not(.hidden)")?.innerText.includes("이미지를 봤습니다"), 20000, "이미지에 대한 답");
  if (!mock.image || mock.image.prefix !== "data:image/png;base64" || mock.image.width !== 1568 || mock.image.height !== 1045) throw new Error(`보낸 이미지: ${JSON.stringify(mock.image)}`);
  if (!mock.image.text.includes("버튼 색")) throw new Error("이미지와 함께 글이 가지 않음");
  const ui2 = await run(() => ({
    thumbs: document.querySelectorAll(".task-log:not(.hidden) .user-images img").length,
    pending: document.querySelectorAll("#attachments .attach-thumb").length,
  }));
  if (ui2.thumbs !== 1 || ui2.pending !== 0) throw new Error(`화면: ${JSON.stringify(ui2)}`);
});

scenario("의미 검색: 뒤에서 코드 조각을 임베딩하고, 질문에 뜻이 가까운 코드를 붙인다", async () => {
  // 프로젝트를 열면 인덱싱 뒤에 조각 임베딩이 돈다 (코드 파일 4개 → 조각 4개 이상)
  const end = Date.now() + 20000;
  while (mock.embedded < 4 && Date.now() < end) await sleep(250);
  if (mock.embedded < 4) throw new Error(`임베딩한 조각 ${mock.embedded}개`);
  await run(() => {
    document.querySelector("#btn-new-task").click();
    return true;
  });
  await run(ui.sendTask, "뜻으로 찾기 확인", "ask", false);
  await waitFor(() => document.querySelector(".task-log:not(.hidden)")?.innerText.includes("맥락을 받았습니다"), 20000, "답");
  // 키워드가 하나도 안 맞는 질문인데도 뜻이 가까운 코드가 맥락에 들어간다
  if (!/뜻이 가까움 \(\d+위\)/.test(mock.semanticPrompt)) throw new Error(`맥락: ${mock.semanticPrompt.slice(0, 600)}`);
  const hidden = await run(() => document.querySelector("#sb-semantic").classList.contains("hidden"));
  if (!hidden) throw new Error("다 만들었는데 진행률이 남아 있음");
});

scenario("설정 화면에서 의미 검색: 연결 시험 → 끄기 → 직접 입력으로 다시 켜기", async () => {
  const cfgPath = path.join(DIRS.home, "config.toml");
  await run(() => {
    document.querySelector("#ab-settings").click();
    return true;
  });
  // 기본 포트가 아닌 로컬 서버는 '직접 입력'으로 보여야 한다 (로컬 찾기 결과로 덮어쓰지 않게)
  const shown = await waitFor(() => {
    const box = document.querySelector("#set-embed");
    const on = box?.querySelector(".segmented button.on")?.getAttribute("data-kind");
    const url = box?.querySelector('input[aria-label="주소"]')?.value;
    return on && url ? { on, url } : null;
  }, 10000, "의미 검색 설정");
  if (shown.on !== "custom" || !shown.url.includes("127.0.0.1")) throw new Error(`설정 표시: ${JSON.stringify(shown)}`);
  await run(() => {
    [...document.querySelectorAll("#set-embed button")].find((b) => b.textContent.includes("연결 시험")).click();
    return true;
  });
  const ok = await waitFor(() => document.querySelector("#set-embed .test-result.ok")?.textContent, 10000, "연결 시험");
  if (!ok.includes("8차원")) throw new Error(`연결 시험: ${ok}`);
  // 끄면 설정 파일에서 [embeddings]가 빠진다
  await run(() => {
    document.querySelector('#set-embed [data-kind="off"]').click();
    return true;
  });
  const end = Date.now() + 5000;
  while (fs.readFileSync(cfgPath, "utf8").includes("[embeddings]") && Date.now() < end) await sleep(100);
  if (fs.readFileSync(cfgPath, "utf8").includes("[embeddings]")) throw new Error("끈 뒤에도 [embeddings]가 남음");
  // 직접 입력으로 다시 저장
  await run(() => {
    document.querySelector('#set-embed [data-kind="custom"]').click();
    return true;
  });
  await run(() => {
    [...document.querySelectorAll("#set-embed button")].find((b) => b.textContent.trim() === "저장").click();
    return true;
  });
  const end2 = Date.now() + 5000;
  while (!fs.readFileSync(cfgPath, "utf8").includes("[embeddings]") && Date.now() < end2) await sleep(100);
  const cfg = fs.readFileSync(cfgPath, "utf8");
  if (!cfg.includes("[embeddings]") || !cfg.includes(`127.0.0.1:${MOCK_PORT}/v1`) || !cfg.includes('model = "e2e-embed"')) throw new Error(`다시 켠 설정:
${cfg}`);
});
