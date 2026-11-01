// E2E 시나리오: 기본 화면과 지도. 지도 위에서 누른 것은 눈앞에 열리고, 보던 지도는 저절로 움직이지 않으며, 카드의 수는 전체에서 센다.

import fs from "node:fs";
import path from "node:path";
import { cdp, DIRS, git, launch, openScm, run, scenario, sleep, stop, waitFor } from "../harness.mjs";
import "../mock-model.mjs"; // 다른 시나리오처럼 이것을 기다려야 run.mjs의 순서대로 등록된다

const toMap = async () => {
  await run(() => {
    document.querySelector("#canvas-switch [data-canvas='map']").click();
    return true;
  });
  await waitFor(() => !document.querySelector("#map-view").classList.contains("hidden") && !!document.querySelector("#map-crumb .cur"), 15000, "지도");
};
const mapHidden = () => run(() => document.querySelector("#map-view").classList.contains("hidden"));
const closePages = () => run(() => {
  for (const b of document.querySelectorAll(".tab.is-page .close")) b.click();
  return true;
});
const key = (k, code, ctrl = true, shift = false) => run((k, code, ctrl, shift) => {
  document.dispatchEvent(new KeyboardEvent("keydown", { key: k, code, ctrlKey: ctrl, shiftKey: shift, bubbles: true, cancelable: true }));
  return true;
}, k, code, ctrl, shift);
/** 빠른 열기 입력에 넣고 Enter (`>`로 시작하면 명령) */
const quickOpen = async (text) => {
  await (text.startsWith(">") ? key("P", "KeyP", true, true) : key("p", "KeyP"));
  await waitFor(() => !document.querySelector("#quickopen").classList.contains("hidden"), 3000, "빠른 열기");
  await run((text) => {
    const i = document.querySelector("#qo-input");
    i.value = text;
    i.dispatchEvent(new Event("input"));
    return true;
  }, text);
  await waitFor(() => !!document.querySelector("#qo-list li.sel"), 8000, `빠른 열기 ${text}`);
  await run(() => {
    document.querySelector("#qo-input").dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    return true;
  });
};

/** 캔버스 그리기를 엿본다: 카메라와 이름표 자리 */
const watchDraw = () => run(() => {
  if (window.__draw) return true;
  const D = (window.__draw = { k: 1, x: 0, y: 0, labels: {}, draws: 0, ks: [] });
  const P = CanvasRenderingContext2D.prototype;
  const { clearRect, translate, scale, fillText } = P;
  P.clearRect = function (...a) {
    D.labels = {};
    D.draws++;
    return clearRect.apply(this, a);
  };
  P.translate = function (x, y) {
    D.x = x;
    D.y = y;
    return translate.call(this, x, y);
  };
  P.scale = function (a, b) {
    D.k = a;
    D.ks.push(a);
    return scale.call(this, a, b);
  };
  P.fillText = function (t, x, y) {
    D.labels[t] = [x, y];
    return fillText.call(this, t, x, y);
  };
  return true;
});
const snapshot = () => run(() => ({ k: __draw.k, x: __draw.x, y: __draw.y, labels: { ...__draw.labels }, draws: __draw.draws }));
const zoomIn = () => run(() => {
  const c = document.querySelector("#map-canvas");
  const r = c.getBoundingClientRect();
  for (let i = 0; i < 3; i++) c.dispatchEvent(new WheelEvent("wheel", { deltaY: -240, clientX: r.left + r.width / 2, clientY: r.top + r.height / 2, bubbles: true, cancelable: true }));
  return true;
});
/** `only`: 비교할 이름표. 전체 구조의 덩어리 이름은 노드가 늘면 따라 움직이므로 파일만 본다 */
function expectStill(a, b, what, only = (t) => t.includes(".")) {
  if (a.k !== b.k || a.x !== b.x || a.y !== b.y) throw new Error(`${what}: 카메라가 바뀜 ${JSON.stringify([a.k, a.x, a.y])} → ${JSON.stringify([b.k, b.x, b.y])}`);
  let common = 0;
  for (const [t, [x, y]] of Object.entries(a.labels)) {
    const q = b.labels[t];
    if (!q || !only(t)) continue;
    common++;
    if (Math.hypot(x - q[0], y - q[1]) * a.k > 1) throw new Error(`${what}: ${t} 이름표가 ${Math.round(Math.hypot(x - q[0], y - q[1]) * a.k)}px 움직임`);
  }
  if (!common) throw new Error(`${what}: 비교할 이름표가 없음`);
}
const nodeCount = () => run(() => Number(document.querySelector(".map-count")?.textContent?.replace(/\D/g, "") || 0));

/** 이름으로 노드를 찾아 누른다 (마우스를 격자로 움직여 제목을 읽는다) */
const clickNode = async (name) => {
  const at = await run((name) => {
    const c = document.querySelector("#map-canvas");
    const r = c.getBoundingClientRect();
    for (let y = r.top + 2; y < r.bottom - 2; y += 4) {
      for (let x = r.left + 2; x < r.right - 2; x += 4) {
        c.dispatchEvent(new MouseEvent("mousemove", { clientX: x, clientY: y, bubbles: true }));
        if (c.title.startsWith(name)) {
          c.dispatchEvent(new MouseEvent("mousedown", { button: 0, clientX: x, clientY: y, bubbles: true }));
          window.dispatchEvent(new MouseEvent("mouseup", { button: 0, bubbles: true }));
          return true;
        }
      }
    }
    return false;
  }, name);
  if (!at) throw new Error(`지도에 ${name} 노드가 없음`);
  return waitFor((name) => {
    if (document.querySelector("#map-card .mc-title")?.textContent !== name) return null;
    const cells = [...document.querySelectorAll("#map-card .mc-stat")].map((s) => [s.querySelector("span").textContent, s.querySelector("b").textContent]);
    return cells.length && cells.every((c) => c[1] !== "–") && Object.fromEntries(cells);
  }, 8000, `${name} 카드의 수`, name);
};

scenario("기본 화면: 지도에서 톱니, Ctrl+,, 시작하기를 누르면 지도가 닫히고 그 화면이 보인다", async () => {
  await stop();
  await launch();
  git("checkout", "-q", "--", ".");
  const settingsShown = () => waitFor(() => !!document.querySelector(".settings")?.offsetParent && document.querySelector("#map-view").classList.contains("hidden"), 3000, "설정이 눈앞에");
  await toMap();
  await run(() => {
    document.querySelector("#ab-settings").click();
    return true;
  });
  await settingsShown();
  // 이미 열린 설정 탭으로 돌아갈 때도
  await toMap();
  await key(",", "Comma");
  await settingsShown();
  await toMap();
  await quickOpen(">시작하기");
  await waitFor(() => document.querySelector(".tab.active")?.textContent.includes("시작하기") && document.querySelector("#map-view").classList.contains("hidden"), 3000, "시작하기가 눈앞에");
  await closePages();
});

scenario("기본 화면: diff는 지도 위로 열리고, 다른 탭을 열면 뒤로 숨는다", async () => {
  fs.appendFileSync(path.join(DIRS.proj, "src/api.ts"), "// diff 확인\n");
  await openScm();
  await waitFor(() => [...document.querySelectorAll(".scm-file")].some((f) => f.textContent.includes("api.ts")), 15000, "변경 목록");
  await toMap();
  await run(() => {
    [...document.querySelectorAll(".scm-file")].find((f) => f.textContent.includes("api.ts")).click();
    return true;
  });
  await waitFor(() => !!document.querySelector(".diff-page.active .diff"), 8000, "diff 탭");
  if (!(await mapHidden())) throw new Error("diff가 지도 뒤에서 열림");

  await quickOpen("login.ts");
  await waitFor(() => [...document.querySelectorAll(".editor-host.active .cm-line")].some((l) => l.textContent.includes("handleLogin")), 8000, "파일 탭");
  const shown = await run(() => ({
    diff: getComputedStyle(document.querySelector(".diff-page")).display,
    hosts: [...document.querySelectorAll(".editor-host")].filter((e) => getComputedStyle(e).display !== "none").length,
  }));
  if (shown.diff !== "none" || shown.hosts !== 1) throw new Error(`diff가 다른 탭 위에 겹침: ${JSON.stringify(shown)}`);

  await run(() => {
    [...document.querySelectorAll(".tab.is-page")].find((t) => t.textContent.includes("api.ts")).click();
    return true;
  });
  const back = await run(() => {
    const d = document.querySelector(".diff-page");
    return { display: getComputedStyle(d).display, dir: getComputedStyle(d).flexDirection };
  });
  if (back.display !== "flex" || back.dir !== "column") throw new Error(`돌아온 diff 배치: ${JSON.stringify(back)}`);
  await closePages();
  git("checkout", "-q", "--", ".");
  await sleep(1500);
});

scenario("기본 화면: 파일이 바뀌어도 확대해 둔 지도와 고른 노드는 그대로다", async () => {
  await toMap();
  await watchDraw();
  await sleep(2500); // 배치가 가라앉을 때까지
  await zoomIn();
  await sleep(400);
  const before = await snapshot();

  // 저장: 구조는 그대로
  fs.appendFileSync(path.join(DIRS.proj, "src/auth/login.ts"), "// 지도 확인\n");
  await waitFor((n) => __draw.draws > n, 8000, "저장 뒤 다시 그림", before.draws);
  await sleep(1200);
  expectStill(before, await snapshot(), "파일 저장");

  // 새 파일: 새 노드만 자리를 찾는다
  const count = await nodeCount();
  const extra = path.join(DIRS.proj, "src/extra.ts");
  fs.writeFileSync(extra, "import { route } from './api';\n\nexport function extraRoute() {\n  return route('/x');\n}\n");
  await waitFor((n) => Number(document.querySelector(".map-count")?.textContent?.replace(/\D/g, "") || 0) > n, 10000, "새 노드", count);
  await sleep(600);
  expectStill(before, await snapshot(), "새 파일");

  // 고른 노드: 다시 그려도 카드와 수가 남는다
  await run(() => {
    document.querySelector("#map-fit").click();
    return true;
  });
  await sleep(500);
  const card = await clickNode("api.ts");
  fs.rmSync(extra);
  await waitFor((n) => Number(document.querySelector(".map-count")?.textContent?.replace(/\D/g, "") || 0) === n, 10000, "노드 빠짐", count);
  await sleep(600);
  const after = await run(() => document.querySelector("#map-card:not(.hidden) .mc-title")?.textContent);
  if (after !== "api.ts") throw new Error(`고른 노드가 풀림: ${after} (${JSON.stringify(card)})`);
  await run(() => {
    document.querySelector("#map-card .icon-btn").click();
    return true;
  });
  git("checkout", "-q", "--", ".");
});

scenario("기본 화면: 카드의 호출 수는 보이는 간선이 아니라 프로젝트 전체에서 센다", async () => {
  await toMap();
  await run(() => {
    const i = document.querySelector("#map-search");
    i.value = "signCookie";
    i.dispatchEvent(new Event("input"));
    return true;
  });
  await waitFor(() => !!document.querySelector("#map-results .mr-row"), 5000, "지도 검색");
  await run(() => {
    document.querySelector("#map-results .mr-row").dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
    return true;
  });
  await waitFor(() => document.querySelector("#map-crumb .cur")?.textContent === "signCookie", 8000, "signCookie 주변");
  await sleep(2500);
  // issueSession을 부르는 handleLogin은 이 지도에 없다. 보이는 간선만 세면 0이다
  const up = await clickNode("issueSession");
  if (!(Number(up["부르는 곳"]) >= 1) || !(Number(up["부르는 것"]) >= 1)) throw new Error(`issueSession 카드: ${JSON.stringify(up)}`);
  const sig = await clickNode("signCookie");
  if (!(Number(sig["부르는 곳"]) >= 1) || !(Number(sig["테스트"]) >= 1)) throw new Error(`signCookie 카드: ${JSON.stringify(sig)}`);
  // 테스트 노드(testSign)는 물러나 있고, 범례가 그 뜻을 말한다
  if (!(await run(() => !!document.querySelector("#map-legend .sw.faded")))) throw new Error("테스트 범례가 없음");
  await run(() => {
    document.querySelector("#map-card .icon-btn").click();
    return true;
  });

  // 중심 심볼의 파일을 저장해 심볼 id가 바뀌어도 지도가 비거나 흔들리지 않는다
  await watchDraw();
  await zoomIn();
  await sleep(400);
  const before = await snapshot();
  const count = await nodeCount();
  fs.appendFileSync(path.join(DIRS.proj, "src/auth/session.ts"), "// 주변 지도 확인\n// 한 줄 더\n");
  await waitFor((n) => __draw.draws > n, 8000, "저장 뒤 다시 그림", before.draws);
  await sleep(1500);
  if ((await nodeCount()) !== count) throw new Error(`저장 뒤 주변 노드 ${await nodeCount()}개 (전에는 ${count}개)`);
  expectStill(before, await snapshot(), "심볼 파일 저장", () => true);
  git("checkout", "-q", "--", ".");
  await sleep(1500);

  await run(() => {
    document.querySelector("#map-crumb .crumb-link").click();
    return true;
  });
  await waitFor(() => document.querySelector("#map-crumb .cur")?.textContent.includes("전체 구조"), 8000, "전체 구조");
});

scenario("기본 화면: 움직임 줄이기를 켜면 지도 카메라가 바로 옮겨 간다", async () => {
  await toMap();
  await watchDraw();
  await sleep(2500);
  const fitSteps = async () => {
    await zoomIn();
    await sleep(200);
    await run(() => {
      __draw.ks = [];
      document.querySelector("#map-fit").click();
      return true;
    });
    await sleep(500);
    return run(() => new Set(__draw.ks).size);
  };
  // 운영체제 설정과 상관없이 둘 다 본다 (원격 데스크톱은 움직임 줄이기가 켜져 있기도 하다)
  const motion = (value) => cdp("Emulation.setEmulatedMedia", { features: [{ name: "prefers-reduced-motion", value }] });
  try {
    await motion("no-preference");
    const normal = await fitSteps();
    if (normal < 3) throw new Error(`평소에는 부드럽게 옮겨 가야 함: 단계 ${normal}`);
    await motion("reduce");
    const calm = await fitSteps();
    if (calm > 1) throw new Error(`움직임 줄이기: 단계 ${calm}`);
  } finally {
    await cdp("Emulation.setEmulatedMedia", { features: [] });
  }
});

scenario("기본 화면: 인덱스 이벤트가 다시 와도 빈 채팅 안내의 육각형은 그대로다", async () => {
  await run(() => {
    document.querySelector("#btn-new-task").click();
    return true;
  });
  await waitFor(() => !!document.querySelector(".task-log:not(.hidden) .empty-chat .index-note"), 15000, "인덱스 안내");
  await run((p) => window.__TAURI_INTERNALS__.invoke("open_project", { path: p }).then(() => true), DIRS.proj);
  await sleep(2500);
  const note = await run(() => {
    const n = document.querySelector(".task-log:not(.hidden) .empty-chat .index-note");
    const hex = n?.querySelector(".hex");
    return { svg: !!hex?.querySelector("svg"), hexText: hex?.textContent ?? "", text: n?.innerText ?? "" };
  });
  if (!note.svg || note.hexText || !/인덱/.test(note.text)) throw new Error(`안내가 깨짐: ${JSON.stringify(note)}`);
});
