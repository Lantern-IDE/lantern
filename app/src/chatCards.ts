// 채팅에 들어가는 카드와 표시 도우미: Lantern이 본 코드(맥락 카드), 외부 에이전트 로그인 카드,
// 승인 카드 제목, 도구 이름·요약. 작업 상태에 기대지 않고 필요한 것은 인자로 받는다.
import { api, errorText, type AuthMethod } from "./api";
import { basename, dirname, h } from "./dom";
import { codicon, fileIcon, hex } from "./icons";
import * as editor from "./editor";
import { parseContext } from "./lib/context";

export function contextCard(md: string, tokens: number, requestId: number, on: { inspector: (requestId: number) => void; map: () => void }): HTMLElement {
  const { files, memory } = parseContext(md);
  const symbols = files.reduce((n, f) => n + f.symbols.length, 0);
  const list = h("div", { class: "ctx-files" });
  for (const m of memory) {
    list.append(h("div", { class: "ctx-file" }, codicon("book"), h("span", {}, basename(m)), h("span", { class: "dir" }, "프로젝트 메모리")));
  }
  for (const f of files) {
    list.append(h("div", { class: "ctx-file" }, fileIcon(f.path), h("span", {}, basename(f.path)), h("span", { class: "dir" }, dirname(f.path))));
    for (const s of f.symbols) {
      const row = h("div", { class: "ctx-sym", title: `${f.path}:${s.line} · ${s.why}` },
        h("span", { class: "name" }, s.name, s.signatureOnly ? h("span", { class: "sig" }, " 시그니처") : null),
        h("span", { class: "why" }, s.why));
      row.addEventListener("click", () => void editor.openFile(f.path, s.line));
      list.append(row);
    }
  }
  const more = h("span", { class: "link small" }, "인스펙터에서 전문 보기");
  more.addEventListener("click", () => on.inspector(requestId));
  const map = h("span", { class: "link small" }, "지도에서 보기");
  map.addEventListener("click", () => on.map());
  list.append(h("div", { class: "ctx-foot" }, map, more));
  return h("details", { class: "ctx-card" },
    h("summary", {},
      hex("plain"),
      h("span", { class: "title" }, "Lantern이 본 코드"),
      h("span", { class: "meta" }, `파일 ${files.length} · 심볼 ${symbols}${memory.length ? ` · 메모리 ${memory.length}` : ""}`),
      h("span", { class: "tokens" }, `약 ${tokens.toLocaleString()} 토큰`),
      codicon("chevron-right", "chev")),
    list);
}


/** 외부 에이전트 로그인 카드. 로그인하면 `retry`로 다시 보낼 수 있다 */
export function authCard(session: string, agent: string, name: string, methods: AuthMethod[], retry: () => void): HTMLElement {
  const line = h("div", { class: "auth-status" });
  const buttons = methods
    .filter((m) => m.type !== "env_var")
    .map((m) => {
      const b = h("button", { class: "btn btn-secondary", title: m.description ?? "" }, codicon("account"), m.name) as HTMLButtonElement;
      b.addEventListener("click", async () => {
        for (const x of card.querySelectorAll("button")) x.disabled = true;
        line.replaceChildren(codicon("loading", "codicon-modifier-spin"), `${name}이(가) 연 브라우저에서 로그인을 마치세요…`);
        try {
          await api.acpAuthenticate(session, agent, m.id);
          const again = h("button", { class: "btn btn-primary" }, codicon("debug-restart"), "다시 보내기");
          again.addEventListener("click", retry);
          line.replaceChildren(codicon("pass"), "로그인되었습니다", again);
        } catch (e) {
          line.replaceChildren(codicon("error"), errorText(e));
          for (const x of card.querySelectorAll("button")) x.disabled = false;
        }
      });
      return b;
    });
  const envOnly = methods.filter((m) => m.type === "env_var").map((m) => m.name);
  const card = h("div", { class: "auth-card" },
    h("div", { class: "auth-head" }, codicon("lock"), `${name}에 로그인해야 합니다`),
    h("p", {}, "로그인은 이 에이전트가 직접 합니다. Lantern은 로그인 정보를 보거나 저장하지 않습니다."),
    h("div", { class: "row" }, ...buttons),
    envOnly.length ? h("p", { class: "muted" }, `또는 환경변수로: ${envOnly.join(", ")}`) : null,
    line);
  return card;
}


/**
 * 승인 카드 제목. 파일 수정은 "파일 수정: 경로"를 이름과 폴더로 나눠, 이름은 한 덩어리로 두고
 * 폴더는 흐리게 `/` 뒤에서만 줄을 바꾼다 (글자 중간에서 "tokenize.r / s"로 꺾이지 않게).
 */
export function approvalTitle(title: string, isEdit: boolean): HTMLElement {
  const m = isEdit ? title.match(/^(.+?): (.+)$/) : null;
  if (!m) return h("span", { class: "atitle" }, title);
  const [, label, path] = m;
  const dir = dirname(path);
  const parts = dir ? dir.split("/").flatMap((p) => [p + "/", h("wbr")]) : [];
  return h("span", { class: "atitle", title: path },
    h("span", { class: "alabel" }, label), " ",
    h("span", { class: "aname" }, basename(path)),
    dir ? h("span", { class: "adir" }, ...parts) : null);
}


/** mcp__서버__도구 → "서버 · 도구" */
export function toolLabel(name: string): string {
  if (name === "run_tests") return "관련 테스트로 확인";
  if (name === "fetch_url") return "웹 페이지";
  const m = name.match(/^mcp__(.+?)__(.+)$/);
  return m ? `${m[1]} · ${m[2]}` : name;
}

export function toolSummary(input: Record<string, unknown>): string {
  const v = input.path ?? input.query ?? input.name ?? input.command ?? input.url ?? input.topic ?? "";
  return typeof v === "string" ? v : JSON.stringify(v);
}
