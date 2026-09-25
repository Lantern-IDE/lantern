// 설정 화면. 원본은 TOML 파일이고, 이 화면은 그 파일을 고치는 편한 수단이다.
// 모든 변경은 바로 저장되며(주석 보존), 저장 위치는 "사용자 전체" 또는 "이 프로젝트"다.
import { api, errorText, type SettingsModel, type SettingsSnapshot } from "./api";
import { h } from "./dom";
import * as editor from "./editor";
import * as i18n from "./i18n";
import { codicon } from "./icons";
import * as prefs from "./prefs";
import * as toast from "./toast";

type Scope = "global" | "project";

interface Ctx {
  hasProject: () => boolean;
  openProjectConfig: () => void;
  onChanged: () => void;
  openPreview: () => void;
}

let ctx: Ctx;
let scope: Scope = "global";
let snap: SettingsSnapshot | null = null;
let host: HTMLElement | null = null;
let pending: string | null = null;

const SECTIONS: [string, string, string][] = [
  ["models", "모델", "sparkle"],
  ["cost", "비용", "credit-card"],
  ["context", "맥락", "references"],
  ["agent", "에이전트", "shield"],
  ["hooks", "훅", "zap"],
  ["appearance", "화면", "color-mode"],
];

const PRESETS: Record<string, { label: string; provider: string; base_url?: string; api_key_env?: string; model: string; price?: [number, number] }> = {
  anthropic: { label: "Anthropic (Claude)", provider: "anthropic", api_key_env: "ANTHROPIC_API_KEY", model: "claude-opus-5", price: [5, 25] },
  openai: { label: "OpenAI", provider: "openai", base_url: "https://api.openai.com/v1", api_key_env: "OPENAI_API_KEY", model: "" },
  openrouter: { label: "OpenRouter", provider: "openai", base_url: "https://openrouter.ai/api/v1", api_key_env: "OPENROUTER_API_KEY", model: "" },
  ollama: { label: "Ollama (로컬)", provider: "openai", base_url: "http://localhost:11434/v1", model: "", price: [0, 0] },
  lmstudio: { label: "LM Studio (로컬)", provider: "openai", base_url: "http://localhost:1234/v1", model: "", price: [0, 0] },
  custom: { label: "기타 OpenAI 호환", provider: "openai", base_url: "", model: "" },
};

async function save(changes: [string, unknown][], status?: HTMLElement): Promise<boolean> {
  try {
    await api.setSettings(scope, changes);
    if (status) {
      status.textContent = "";
      status.append(codicon("check"), "저장됨");
      status.className = "saved show";
      setTimeout(() => status.classList.remove("show"), 1600);
    }
    ctx.onChanged();
    return true;
  } catch (e) {
    if (status) {
      status.className = "ferr";
      status.textContent = errorText(e);
    } else toast.error("설정을 저장하지 못했습니다", errorText(e));
    return false;
  }
}

function field(label: string, help: string, control: HTMLElement, forId?: string): HTMLElement {
  return h("div", { class: "field" },
    h(forId ? "label" : "div", forId ? { for: forId } : { class: "flabel" }, label),
    help ? h("div", { class: "fhelp" }, help) : null,
    control);
}

function numberField(label: string, help: string, path: string, value: number, opts: { min?: number; max?: number; step?: number } = {}): HTMLElement {
  const id = `set-${path.replace(/\./g, "-")}`;
  const status = h("span", { class: "saved" });
  const input = h("input", { type: "number", id, value: String(value), min: opts.min, max: opts.max, step: opts.step ?? 1 }) as HTMLInputElement;
  let timer: number | undefined;
  input.addEventListener("input", () => {
    clearTimeout(timer);
    timer = window.setTimeout(() => {
      const n = Number(input.value);
      if (input.value === "" || Number.isNaN(n)) return;
      void save([[path, n]], status);
    }, 500);
  });
  return field(label, help, h("div", { class: "row" }, input, status), id);
}

function tagField(label: string, help: string, path: string, values: string[], placeholder: string): HTMLElement {
  const list = [...values];
  const status = h("span", { class: "saved" });
  const box = h("div", { class: "taglist" });
  const input = h("input", { placeholder, "aria-label": `${label} 추가` }) as HTMLInputElement;
  const render = () => {
    box.replaceChildren(
      ...list.map((v, i) => h("span", { class: "tag" }, v,
        h("button", { title: "삭제", "aria-label": `${v} 삭제`, onclick: () => { list.splice(i, 1); render(); void save([[path, list]], status); } }, codicon("close")))),
      input);
  };
  input.addEventListener("keydown", (e) => {
    if (e.key === "Enter" && input.value.trim()) {
      list.push(input.value.trim());
      input.value = "";
      render();
      input.focus();
      void save([[path, list]], status);
    }
  });
  render();
  return field(label, help, h("div", { class: "row" }, box, status));
}

function keySourceText(m: SettingsModel): [string, string] {
  switch (m.key_source) {
    case "env": return ["ok", `환경변수 ${m.api_key_env}`];
    case "keychain": return ["ok", "자격 증명 저장소"];
    case "config": return ["ok", "설정 파일"];
    default: return m.needs_key ? ["bad", "API 키 없음"] : ["", "키 필요 없음"];
  }
}

function modelRow(key: string, m: SettingsModel, isDefault: boolean): HTMLElement {
  const [cls, text] = keySourceText(m);
  const result = h("div", { class: "test-result hidden" });
  const keyForm = h("div", { class: "key-form hidden" });
  const row = h("div", { class: "model" },
    h("div", {},
      h("div", { class: "m-title" }, key, isDefault ? h("span", { class: "pill default" }, codicon("star-full"), "기본") : null,
        h("span", { class: "pill" }, m.provider === "anthropic" ? "Anthropic" : "OpenAI 호환")),
      h("div", { class: "m-sub", title: m.base_url ?? "" }, `${m.model}${m.base_url ? " · " + m.base_url : ""}`)),
    h("div", { class: "m-actions" }),
    h("div", { class: "m-status" },
      m.needs_key || m.key_source ? h("span", { class: `pill ${cls}` }, codicon(cls === "bad" ? "warning" : "key"), text) : null,
      m.price_input != null ? h("span", { class: "muted num" }, `100만 토큰당 입력 $${m.price_input} · 출력 $${m.price_output ?? 0}`) : h("span", { class: "muted" }, "가격 미설정 (비용 미터에 토큰만 표시)")),
    keyForm, result);

  const actions = row.querySelector(".m-actions")!;
  if (!isDefault) {
    actions.append(h("button", { class: "btn btn-secondary", onclick: () => void save([["routing.default", key]]).then(() => refresh()) }, "기본으로"));
  }
  actions.append(h("button", {
    class: "btn btn-secondary",
    onclick: async () => {
      result.className = "test-result";
      result.replaceChildren(codicon("loading", "codicon-modifier-spin"), "확인 중…");
      const r = await api.testModel(key).catch((e) => ({ ok: false, message: errorText(e), ms: 0, models: [] }));
      result.className = `test-result ${r.ok ? "ok" : "bad"}`;
      result.replaceChildren(codicon(r.ok ? "pass" : "error"), `${r.message}${r.ms ? ` · ${r.ms}ms` : ""}`);
    },
  }, codicon("plug"), "연결 확인"));
  if (m.api_key_env) {
    actions.append(h("button", { class: "btn btn-secondary", onclick: () => keyForm.classList.toggle("hidden") }, codicon("key"), "API 키"));
    const input = h("input", { type: "password", placeholder: `${m.api_key_env} 값`, autocomplete: "off", spellcheck: "false", "aria-label": "API 키" }) as HTMLInputElement;
    keyForm.append(input,
      h("button", {
        class: "btn btn-primary",
        onclick: async () => {
          try {
            await api.setApiKey(key, input.value);
            input.value = "";
            toast.success("API 키를 자격 증명 저장소에 저장했습니다", "설정 파일에는 키가 남지 않습니다.");
            ctx.onChanged();
            await refresh();
          } catch (e) {
            toast.error("API 키를 저장하지 못했습니다", errorText(e));
          }
        },
      }, "저장"),
      ...(m.key_source === "keychain"
        ? [h("button", { class: "btn btn-secondary", onclick: async () => { await api.deleteApiKey(key).catch(() => {}); ctx.onChanged(); await refresh(); } }, "저장된 키 삭제")]
        : []));
    input.addEventListener("keydown", (e) => e.key === "Enter" && (keyForm.querySelector(".btn-primary") as HTMLButtonElement).click());
  }
  return row;
}

function addModelForm(): HTMLElement {
  const preset = h("select", { "aria-label": "종류" }, ...Object.entries(PRESETS).map(([k, p]) => h("option", { value: k }, p.label))) as HTMLSelectElement;
  const name = h("input", { placeholder: "예: fast, local2", spellcheck: "false" }) as HTMLInputElement;
  const model = h("input", { placeholder: "모델 ID", spellcheck: "false" }) as HTMLInputElement;
  const base = h("input", { placeholder: "https://…/v1", spellcheck: "false" }) as HTMLInputElement;
  const envName = h("input", { placeholder: "예: OPENAI_API_KEY (없으면 비움)", spellcheck: "false" }) as HTMLInputElement;
  const pin = h("input", { type: "number", step: "0.01", placeholder: "입력 $/100만" }) as HTMLInputElement;
  const pout = h("input", { type: "number", step: "0.01", placeholder: "출력 $/100만" }) as HTMLInputElement;
  // 미리 채운 값처럼 보이지 않게, 프리셋은 자리표시자로만 보여주고 비워 두면 프리셋 값을 쓴다.
  const fill = () => {
    const p = PRESETS[preset.value];
    for (const el of [model, base, envName, pin, pout]) el.value = "";
    model.placeholder = p.model ? `예: ${p.model}` : "모델 ID";
    base.placeholder = p.base_url || "https://…/v1";
    envName.placeholder = p.api_key_env ? `기본값 ${p.api_key_env}` : "키가 없으면 비워 두세요";
    pin.placeholder = p.price ? `기본값 ${p.price[0]}` : "입력 $/100만";
    pout.placeholder = p.price ? `기본값 ${p.price[1]}` : "출력 $/100만";
    base.disabled = p.provider === "anthropic";
  };
  preset.addEventListener("change", fill);
  fill();
  const add = h("button", {
    class: "btn btn-primary",
    onclick: async () => {
      const key = name.value.trim();
      if (!/^[A-Za-z0-9_-]+$/.test(key)) return toast.warn("이름은 영문, 숫자, -, _ 만 쓸 수 있습니다");
      const p = PRESETS[preset.value];
      const modelId = model.value.trim() || p.model;
      if (!modelId) return toast.warn("모델 ID를 입력하세요");
      const baseUrl = base.value.trim() || p.base_url || "";
      if (p.provider !== "anthropic" && !baseUrl) return toast.warn("주소(base_url)를 입력하세요");
      const env = envName.value.trim() || p.api_key_env || "";
      const priceIn = pin.value !== "" ? Number(pin.value) : p.price?.[0];
      const priceOut = pout.value !== "" ? Number(pout.value) : p.price?.[1];
      const changes: [string, unknown][] = [
        [`models.${key}.provider`, p.provider],
        [`models.${key}.model`, modelId],
      ];
      if (p.provider !== "anthropic") changes.push([`models.${key}.base_url`, baseUrl]);
      if (env) changes.push([`models.${key}.api_key_env`, env]);
      if (priceIn !== undefined) changes.push([`models.${key}.price_input`, priceIn]);
      if (priceOut !== undefined) changes.push([`models.${key}.price_output`, priceOut]);
      if (await save(changes)) {
        toast.success(`모델 '${key}'을(를) 추가했습니다`);
        await refresh();
      }
    },
  }, codicon("add"), "추가");
  const form = h("div", { class: "add-model hidden" },
    h("label", {}, "종류", preset),
    h("label", {}, "이름", name),
    h("label", {}, "모델 ID", model),
    h("label", {}, "주소 (base_url)", base),
    h("label", { class: "full" }, "API 키 환경변수 이름", envName),
    h("label", {}, "입력 가격", pin),
    h("label", {}, "출력 가격", pout),
    h("div", { class: "full row" }, add, h("button", { class: "btn btn-secondary", onclick: () => { form.classList.add("hidden"); toggle.classList.remove("hidden"); } }, "취소")));
  const toggle = h("button", { class: "btn btn-secondary", style: "margin-top:12px" }, codicon("add"), "모델 추가");
  toggle.addEventListener("click", () => {
    toggle.classList.add("hidden");
    form.classList.remove("hidden");
    name.focus();
  });
  return h("div", {}, toggle, form);
}

function section(id: string, title: string, desc: string, ...children: (HTMLElement | null)[]): HTMLElement {
  return h("section", { class: "set-section", id: `set-${id}` }, h("h2", {}, title), h("p", { class: "desc" }, desc), ...children);
}

async function refresh() {
  if (!host) return;
  try {
    snap = await api.getSettings();
  } catch (e) {
    host.querySelector(".settings-body")!.replaceChildren(h("div", { class: "err-box" }, h("div", { class: "err-msg" }, codicon("error"), h("span", {}, `설정 파일을 읽을 수 없습니다: ${errorText(e)}`))));
    return;
  }
  const info = await api.modelInfo().catch(() => null);
  const c = snap.config;
  const body = host.querySelector<HTMLElement>(".settings-body")!;
  const scrollTop = body.scrollTop;
  if (!ctx.hasProject()) scope = "global";

  const scopeBtns = h("div", { class: "segmented", role: "group", "aria-label": "저장 위치" },
    ...(["global", "project"] as Scope[]).map((s) => {
      const b = h("button", { class: scope === s ? "on" : "", "aria-pressed": String(scope === s) }, s === "global" ? "사용자 (모든 프로젝트)" : "이 프로젝트");
      if (s === "project" && !ctx.hasProject()) (b as HTMLButtonElement).disabled = true;
      b.addEventListener("click", () => { scope = s; void refresh(); });
      return b;
    }));
  const path = scope === "global" ? snap.global_path : snap.project_path;
  const openFile = scope === "project" ? h("button", { class: "btn btn-ghost", onclick: () => ctx.openProjectConfig() }, codicon("go-to-file"), "파일로 열기") : null;

  const month = info?.usage;
  const limit = c.budget.monthly_usd_limit;
  const pct = limit > 0 && month ? Math.min(100, (month.cost_usd / limit) * 100) : 0;

  body.replaceChildren(
    h("h1", {}, "설정"),
    h("p", { class: "lead" }, "여기서 바꾼 값은 바로 TOML 설정 파일에 저장됩니다. 파일에 적어 둔 주석과 순서는 그대로 남습니다."),
    h("div", { class: "scope-bar" }, h("span", { class: "muted small" }, "저장 위치"), scopeBtns,
      h("code", { class: "muted small", title: path ?? "" }, path ?? ""), openFile),

    section("models", "모델", "AI 요청에 쓰는 모델입니다. 기본 모델은 에이전트 정의에 모델이 없을 때 쓰입니다. API 키는 환경변수나 OS 자격 증명 저장소에 두고, 설정 파일에는 적지 않는 것을 권합니다.",
      h("div", { class: "model-list" }, ...Object.entries(c.models).map(([k, m]) => modelRow(k, m, k === c.routing.default))),
      addModelForm(),
      (() => {
        const status = h("span", { class: "saved" });
        const cur = c.routing.completion ?? "";
        const select = h("select", { id: "set-completion" },
          h("option", { value: "" }, "끔"),
          ...Object.keys(c.models).map((k) => h("option", { value: k }, k))) as HTMLSelectElement;
        select.value = c.models[cur] ? cur : "";
        select.addEventListener("change", () => void save([["routing.completion", select.value]], status));
        return field("인라인 자동 완성", "타이핑을 멈추면 이어질 코드를 흐리게 제안합니다 (Tab 수락, Esc 무시). 요청이 자주 가므로 로컬 모델처럼 빠르고 싼 모델을 권합니다.", h("div", { class: "row" }, select, status), "set-completion");
      })()),

    section("cost", "비용", "모델별 가격으로 계산한 이번 달 사용 비용입니다. 한도를 넘으면 AI 요청을 막습니다.",
      month ? h("div", { class: "field" },
        h("div", { class: "usage-line" },
          h("span", {}, h("b", {}, `$${month.cost_usd.toFixed(2)}`), limit > 0 ? `한도 $${limit.toFixed(0)} 중` : "한도 없음"),
          h("span", {}, h("b", {}, month.requests.toLocaleString()), "요청"),
          h("span", {}, h("b", {}, (month.input_tokens + month.output_tokens).toLocaleString()), "토큰")),
        limit > 0 ? h("div", { class: `meter${pct >= 100 ? " err" : pct >= c.budget.warn_at_percent ? " warn" : ""}` }, h("i", { style: `width:${pct}%` })) : null) : null,
      numberField("월 한도 (USD)", "0이면 제한하지 않습니다.", "budget.monthly_usd_limit", c.budget.monthly_usd_limit, { min: 0, step: 1 }),
      numberField("경고 기준 (%)", "이 비율을 넘으면 상태 표시줄의 비용이 경고색으로 바뀝니다.", "budget.warn_at_percent", c.budget.warn_at_percent, { min: 1, max: 100 })),

    section("context", "맥락", "질문마다 맥락 엔진이 관련 코드를 골라 붙입니다. 예산이 클수록 더 많은 코드를 보내고 토큰을 더 씁니다.",
      numberField("맥락 토큰 예산", "보통 4,000~16,000 사이가 적당합니다.", "context.budget_tokens", c.context.budget_tokens, { min: 500, step: 500 }),
      h("div", { class: "field" }, h("div", { class: "row" },
        h("button", { class: "btn btn-secondary", onclick: () => ctx.openPreview() }, codicon("eye"), "맥락 미리보기 열기"),
        h("span", { class: "muted small" }, "모델을 부르지 않고 어떤 코드가 골라지는지 확인합니다.")))),

    section("agent", "에이전트", "‘코드 작성’ 같은 에이전트가 파일을 바꾸거나 명령을 실행할 때의 권한입니다. 목록 밖의 작업은 항상 승인을 받습니다.",
      numberField("최대 단계", "한 번의 요청에서 도구를 부를 수 있는 최대 횟수입니다.", "agent.max_steps", c.agent.max_steps, { min: 1, max: 200 }),
      tagField("승인 없이 실행할 명령", "앞부분이 일치하면 바로 실행합니다. &, |, ; 같은 연결 기호가 있으면 항상 묻습니다.", "agent.allowed_commands", c.agent.allowed_commands, "명령 입력 후 Enter"),
      tagField("승인 없이 수정할 파일", "glob 형식. 예: docs/**, **/*.test.ts", "agent.auto_approve", c.agent.auto_approve, "패턴 입력 후 Enter")),

    section("hooks", "훅", "정해진 순간에 실행할 명령입니다. 결과는 출력 패널에 표시됩니다.",
      tagField("저장할 때", "", "hooks.on_save", c.hooks.on_save, "예: npx prettier --check ."),
      tagField("에이전트가 파일을 바꾼 뒤", "", "hooks.on_agent_done", c.hooks.on_agent_done, "예: cargo check")),

    section("appearance", "화면", "이 컴퓨터에만 적용되는 개인 설정입니다.",
      field("테마", "", (() => {
        const cur = prefs.get().theme;
        return h("div", { class: "segmented", role: "group", "aria-label": "테마" },
          ...([["system", "시스템"], ["dark", "다크"], ["light", "라이트"]] as const).map(([v, label]) => {
            const b = h("button", { class: cur === v ? "on" : "", "aria-pressed": String(cur === v) }, label);
            b.addEventListener("click", () => { prefs.set({ theme: v }); void refresh(); });
            return b;
          }));
      })()),
      field("언어", "바꾸면 화면을 다시 불러옵니다. 저장하지 않은 파일은 먼저 저장하세요.", (() => {
        const langs = [["ko", "한국어"], ["en", "English"]] as const;
        return h("div", { class: "segmented", role: "group", "aria-label": "언어", "data-no-i18n": true },
          ...langs.map(([v, label]) => {
            const b = h("button", { class: i18n.lang === v ? "on" : "", "aria-pressed": String(i18n.lang === v) }, label);
            b.addEventListener("click", async () => {
              if (i18n.lang === v) return;
              if (editor.hasDirty()) await editor.saveAll();
              i18n.setLang(v);
            });
            return b;
          }));
      })()),
      (() => {
        const input = h("input", { type: "number", id: "set-font-size", min: 10, max: 24, value: String(prefs.get().fontSize) }) as HTMLInputElement;
        input.addEventListener("change", () => {
          const n = Math.min(24, Math.max(10, Number(input.value) || 14));
          prefs.set({ fontSize: n });
        });
        return field("편집기 글꼴 크기", "10~24px", input, "set-font-size");
      })()),
  );
  body.scrollTop = scrollTop;
  if (pending) {
    body.querySelector(`#set-${pending}`)?.scrollIntoView({ block: "start" });
    pending = null;
  }
}

export function init(c: Ctx) {
  ctx = c;
}

export function open(sectionId?: string) {
  pending = sectionId ?? null;
  // openPage는 탭을 활성화하면서 onShow(refresh)를 부르므로, host는 render 안에서 먼저 정해 둔다.
  editor.openPage("settings", "설정", "settings-gear", (el) => {
    host = el;
    const nav = h("nav", { class: "settings-nav", "aria-label": "설정 항목" },
      ...SECTIONS.map(([id, label, icon]) => {
        const b = h("button", { "data-sec": id, class: id === SECTIONS[0][0] ? "active" : "" }, codicon(icon), label);
        b.addEventListener("click", () => el.querySelector(`#set-${id}`)?.scrollIntoView({ block: "start", behavior: "smooth" }));
        return b;
      }));
    const body = h("div", { class: "settings-body page-content scroll" });
    body.addEventListener("scroll", () => {
      let cur = SECTIONS[0][0];
      for (const [id] of SECTIONS) {
        const s = body.querySelector<HTMLElement>(`#set-${id}`);
        if (s && s.offsetTop - body.scrollTop < 120) cur = id;
      }
      // 끝까지 내렸으면 마지막 섹션 (짧은 마지막 섹션은 위쪽 기준선에 닿지 못한다)
      if (body.scrollTop + body.clientHeight >= body.scrollHeight - 2) cur = SECTIONS[SECTIONS.length - 1][0];
      for (const b of nav.querySelectorAll<HTMLElement>("button")) b.classList.toggle("active", b.dataset.sec === cur);
    });
    el.append(h("div", { class: "settings" }, nav, body));
    el.style.overflow = "hidden";
  }, () => void refresh());
}

/** 설정이 다른 곳에서 바뀌었을 때 (설정 화면이 열려 있으면 다시 그린다) */
export function refreshIfOpen() {
  if (host?.isConnected) void refresh();
}
