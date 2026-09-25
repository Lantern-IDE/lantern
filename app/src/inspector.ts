// 인스펙터: 모델에 보낸 시스템 프롬프트, 조립된 맥락, 도구 목록, 토큰과 비용을 요청마다 보여준다.
// 기획서 원칙 02 "숨기지 않는다".
import { api, errorText, on, type AgentEvent } from "./api";
import { $, h } from "./dom";
import * as editor from "./editor";

const MAX_CARDS = 50;
const cards = new Map<number, HTMLElement>();

function fmt(n: number) {
  return n.toLocaleString();
}

function section(title: string, body: string, open = false): HTMLElement {
  const d = h("details", {}, h("summary", {}, title), h("pre", {}, body));
  if (open) d.setAttribute("open", "");
  return d;
}

function handle(ev: AgentEvent) {
  if (ev.kind === "request") {
    const card = h("div", { class: "req", "data-id": String(ev.request_id) },
      h("div", { class: "rhead" },
        h("b", {}, `#${ev.request_id}`),
        h("span", {}, ev.agent),
        h("span", { class: "muted" }, `${ev.model_key} · ${ev.model}`),
        h("span", { class: "muted" }, new Date().toLocaleTimeString())),
      h("div", { class: "usage" }, "응답 대기 중…"),
      ev.context ? section(`조립된 맥락 (약 ${fmt(ev.context_tokens)} 토큰)`, ev.context) : null,
      section("시스템 프롬프트", ev.system),
      section(`도구 ${ev.tools.length}개`, ev.tools.join("\n")));
    cards.set(ev.request_id, card);
    $("#requests").prepend(card);
    while (cards.size > MAX_CARDS) {
      const oldest = Math.min(...cards.keys());
      cards.get(oldest)?.remove();
      cards.delete(oldest);
    }
  } else if (ev.kind === "usage") {
    const u = ev.usage;
    const card = cards.get(ev.request_id);
    if (!card) return;
    const cache = u.cache_read_tokens || u.cache_write_tokens
      ? ` · 캐시 읽기 ${fmt(u.cache_read_tokens)} / 쓰기 ${fmt(u.cache_write_tokens)}` : "";
    const cost = ev.cost_usd === null ? "가격 미설정" : `$${ev.cost_usd.toFixed(4)}`;
    card.querySelector(".usage")!.textContent =
      `입력 ${fmt(u.input_tokens)} · 출력 ${fmt(u.output_tokens)}${cache} · ${cost} · 응답 모델 ${ev.model}`;
  }
}

export function show(requestId?: number) {
  document.querySelector<HTMLElement>('.aux-tabs button[data-ai="inspector"]')?.click();
  if (requestId !== undefined) {
    const card = cards.get(requestId);
    card?.querySelector("details")?.setAttribute("open", "");
    card?.scrollIntoView({ block: "start" });
  }
}

async function preview() {
  const q = $<HTMLInputElement>("#preview-query").value.trim();
  if (!q) return;
  const f = editor.focusInfo();
  const out = h("div", { class: "req" }, h("div", { class: "usage" }, "조립 중…"));
  $("#requests").prepend(out);
  try {
    const r = await api.contextPreview(q, f.file, f.line);
    out.replaceChildren(
      h("div", { class: "rhead" }, h("b", {}, "미리보기"), h("span", {}, q)),
      h("div", { class: "usage" }, `${r.items}개 항목 · 약 ${fmt(r.used_tokens)} 토큰 · ${r.elapsed_ms.toFixed(0)}ms · 모델 호출 없음`),
      section("조립된 맥락", r.markdown, true),
    );
  } catch (e) {
    out.replaceChildren(h("div", { class: "usage" }, errorText(e)));
  }
}

export function init() {
  on<AgentEvent>("agent", handle);
  $("#btn-preview").addEventListener("click", () => void preview());
  $("#preview-query").addEventListener("keydown", (e) => {
    if ((e as KeyboardEvent).key === "Enter") void preview();
  });
}
