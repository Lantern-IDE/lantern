// 로그와 문제 보고. 로그는 이 컴퓨터에만 남고, 보고서는 사용자가 직접 복사해 보낸다.
import { invoke } from "@tauri-apps/api/core";
import { errorText } from "./api";
import { h } from "./dom";
import { codicon } from "./icons";
import * as editor from "./editor";
import * as toast from "./toast";

export function log(level: "info" | "error", message: string) {
  void invoke("log_frontend", { level, message }).catch(() => {});
}

export function open() {
  editor.openPage("report", "문제 보고", "report", async (el) => {
    const pre = h("pre", { class: "report-text" }, "보고서를 만드는 중…");
    const copy = h("button", { class: "btn btn-primary" }, codicon("copy"), "보고서 복사");
    copy.addEventListener("click", async () => {
      try {
        await navigator.clipboard.writeText(pre.textContent ?? "");
        toast.success("보고서를 복사했습니다", "이슈나 메일에 붙여 넣고, 무엇을 하다가 생긴 문제인지 한두 줄 적어 주세요.");
      } catch (e) {
        toast.error("복사하지 못했습니다", errorText(e));
      }
    });
    const folder = h("button", { class: "btn btn-secondary" }, codicon("folder-opened"), "로그 폴더 열기");
    folder.addEventListener("click", openLogDir);
    el.append(
      h("div", { class: "page-content report-page scroll" },
        h("h1", {}, "문제 보고"),
        h("p", { class: "lead" },
          "Lantern은 사용 기록을 어디에도 보내지 않습니다. 아래 보고서는 이 컴퓨터의 로그로 만들었고, API 키나 비밀번호처럼 보이는 값은 ",
          h("code", {}, "«가려진 비밀»"), "로 바꿨습니다. 보내기 전에 내용을 확인하세요."),
        h("div", { class: "report-actions" }, copy, folder),
        pre),
    );
    try {
      pre.textContent = await invoke<string>("problem_report");
    } catch (e) {
      pre.textContent = errorText(e);
    }
  });
}

export function init() {
  window.addEventListener("error", (e) => log("error", `${e.message} @ ${e.filename}:${e.lineno}:${e.colno}`));
  window.addEventListener("unhandledrejection", (e) => {
    const r = e.reason;
    let text: string;
    if (r instanceof Error) text = `${r.message}\n${r.stack ?? ""}`;
    else if (typeof r === "object" && r !== null) {
      try {
        text = JSON.stringify(r);
      } catch {
        text = String(r);
      }
    } else text = String(r);
    log("error", `처리되지 않은 오류: ${text}`);
  });
  void invoke<string | null>("pending_crash").then((crash) => {
    if (!crash) return;
    toast.show({
      kind: "warn",
      message: "지난번에 Lantern이 비정상 종료되었습니다",
      detail: "충돌 보고서를 남겨 두었습니다. 문제 보고를 보내 주시면 고치는 데 큰 도움이 됩니다.",
      actions: [{ label: "문제 보고", primary: true, run: open }],
    });
  }).catch(() => {});
}

export function openLogDir() {
  void invoke("open_log_dir").catch((e) => toast.error("폴더를 열지 못했습니다", errorText(e)));
}
