// 알림 토스트 (오른쪽 아래) 와 알림 기록 (상태 표시줄 종 아이콘)
import { $, h } from "./dom";
import { codicon } from "./icons";

export type Kind = "info" | "success" | "warn" | "error";

export interface ToastOptions {
  message: string;
  detail?: string;
  kind?: Kind;
  actions?: { label: string; primary?: boolean; run: () => unknown }[];
  /** ms. 0이면 사용자가 닫을 때까지 남는다. 기본: 오류·경고는 남고 나머지는 6초 */
  timeout?: number;
}

const ICON: Record<Kind, string> = { info: "info", success: "pass", warn: "warning", error: "error" };
const MAX_VISIBLE = 3;
const history: { kind: Kind; message: string; detail?: string; at: Date }[] = [];

function dismiss(el: HTMLElement) {
  if (el.classList.contains("leaving")) return;
  el.classList.add("leaving");
  el.addEventListener("animationend", () => el.remove(), { once: true });
  setTimeout(() => el.remove(), 400);
}

export function show(opts: ToastOptions): () => void {
  const kind = opts.kind ?? "info";
  history.unshift({ kind, message: opts.message, detail: opts.detail, at: new Date() });
  history.length = Math.min(history.length, 50);
  if ($("#notif-center").classList.contains("hidden")) $("#sb-bell").classList.add("unread");
  else renderCenter();
  // 변경 승인을 기다리는 동안에는 승인 버튼을 가리지 않도록 오류가 아닌 알림은 기록만 한다.
  if (kind !== "error" && document.querySelector(".approval:not(.resolved)")) return () => {};

  const close = h("button", { class: "icon-btn", title: "닫기", "aria-label": "알림 닫기" }, codicon("close"));
  const body = h("div", {},
    h("div", { class: "t-msg" }, opts.message),
    opts.detail ? h("div", { class: "t-detail" }, opts.detail) : null);
  const el = h("div", { class: `toast ${kind}`, role: kind === "error" ? "alert" : "status" }, codicon(ICON[kind]), body, close);
  if (opts.actions?.length) {
    body.append(h("div", { class: "t-actions" }, ...opts.actions.map((a) =>
      h("button", { class: `btn ${a.primary ? "btn-primary" : "btn-secondary"}`, onclick: () => { dismiss(el); void a.run(); } }, a.label))));
  }
  close.addEventListener("click", () => dismiss(el));

  const box = $("#toasts");
  box.append(el);
  // 넘치면 오래된 것부터 닫는다. 닫히는 중인(애니메이션 중) 알림은 세지 않는다:
  // 그것까지 세면 이미 닫히는 알림을 다시 닫으려다 개수가 줄지 않아 무한 반복된다.
  const live = () => [...box.children].filter((c) => !c.classList.contains("leaving")) as HTMLElement[];
  for (let alive = live(); alive.length > MAX_VISIBLE; alive = live()) dismiss(alive[0]);
  const timeout = opts.timeout ?? (kind === "error" || kind === "warn" ? 0 : 6000);
  if (timeout > 0) {
    let timer = window.setTimeout(() => dismiss(el), timeout);
    el.addEventListener("mouseenter", () => clearTimeout(timer));
    el.addEventListener("mouseleave", () => (timer = window.setTimeout(() => dismiss(el), 2500)));
  }
  return () => dismiss(el);
}

export const info = (message: string, detail?: string) => show({ message, detail, kind: "info" });
export const success = (message: string, detail?: string) => show({ message, detail, kind: "success" });
export const warn = (message: string, detail?: string) => show({ message, detail, kind: "warn" });
export const error = (message: string, detail?: string) => show({ message, detail, kind: "error" });

function renderCenter() {
  const list = $("#notif-list");
  if (!history.length) {
    list.replaceChildren(h("div", { class: "nc-empty" }, "새 알림이 없습니다."));
    return;
  }
  list.replaceChildren(...history.map((n) =>
    h("div", { class: `nc-item ${n.kind}` },
      codicon(ICON[n.kind]),
      h("div", {}, n.message, n.detail ? h("div", { class: "muted small" }, n.detail) : null),
      h("time", {}, n.at.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })))));
}

export function toggleCenter(force?: boolean) {
  const c = $("#notif-center");
  const open = force ?? c.classList.contains("hidden");
  c.classList.toggle("hidden", !open);
  if (open) {
    renderCenter();
    $("#sb-bell").classList.remove("unread");
    for (const t of [...$("#toasts").children]) dismiss(t as HTMLElement);
  }
}

export function init() {
  $("#sb-bell").addEventListener("click", () => toggleCenter());
  $("#nc-close").addEventListener("click", () => toggleCenter(false));
  $("#nc-clear").addEventListener("click", () => {
    history.length = 0;
    renderCenter();
  });
  window.addEventListener("keydown", (e) => {
    if (e.key === "Escape" && !$("#notif-center").classList.contains("hidden")) toggleCenter(false);
  });
}
