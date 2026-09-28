// 첫 실행 안내: AI 모델 연결 → 폴더 열기 → 사용법. 모달이 아니라 탭이라 언제든 건너뛰고 돌아올 수 있다.
import { api, errorText, type LocalServer } from "./api";
import { h, store } from "./dom";
import * as editor from "./editor";
import { codicon, hex } from "./icons";
import { CLOUD, chatModels, modelKey, provider } from "./providers";

interface Ctx {
  pickProject: () => Promise<void>;
  hasProject: () => boolean;
  showChat: () => void;
  onModelChanged: () => void;
}

let ctx: Ctx;
let host: HTMLElement | null = null;
let localServers: LocalServer[] | null = null;

export function done(): boolean {
  return store.get<boolean>("onboarded", false);
}

function finish() {
  store.set("onboarded", true);
  void editor.closeTab("page:onboarding");
}

function status(el: HTMLElement, kind: "ok" | "bad" | "wait" | "", text: string) {
  el.className = `status-line${kind === "ok" ? " ok" : kind === "bad" ? " bad" : ""}`;
  el.replaceChildren(
    kind === "wait" ? codicon("loading", "codicon-modifier-spin") : kind ? codicon(kind === "ok" ? "pass" : "error") : codicon("info"),
    text);
}

/**
 * 클라우드 모델: 제공자 고르기 → API 키 → 제공자의 모델 목록에서 고르기.
 * 모델 이름은 제공자가 자주 바꾸므로 목록을 받아서 보여준다 (providers.ts).
 */
async function cloudChoice(onDone: () => void): Promise<HTMLElement> {
  const line = h("div", { class: "status-line" });
  const which = h("select", { "aria-label": "제공자" }, ...CLOUD.map((p) => h("option", { value: p.id }, p.label))) as HTMLSelectElement;
  const keyUrl = h("div", { class: "key-url" });
  const input = h("input", { type: "password", autocomplete: "off", spellcheck: "false", "aria-label": "API 키" }) as HTMLInputElement;
  const btn = h("button", { class: "btn btn-secondary" }, "저장하고 확인") as HTMLButtonElement;
  const pick = h("select", { "aria-label": "모델" }) as HTMLSelectElement;
  const use = h("button", { class: "btn btn-primary" }, "이 모델 쓰기") as HTMLButtonElement;
  const pickRow = h("div", { class: "row hidden" }, pick, use);
  const box = h("div", { class: "choice" },
    h("h3", {}, codicon("cloud"), "클라우드 모델"),
    h("p", {}, "Claude, GPT, Gemini 등. 내 API 키로 쓰고, 사용한 만큼만 그 회사에 비용을 냅니다."),
    h("div", { class: "row" }, which),
    keyUrl,
    h("div", { class: "row" }, input, btn),
    pickRow,
    line);

  const current = () => provider(which.value);
  const describe = async () => {
    const p = current();
    input.placeholder = p.key_placeholder ?? "API 키";
    keyUrl.replaceChildren(...(p.key_url ? ["키 발급: ", h("span", { class: "mono", title: "복사해서 브라우저에서 여세요" }, p.key_url)] : []));
    pickRow.classList.add("hidden");
    const m = (await api.getSettings().catch(() => null))?.config.models[modelKey(p)];
    if (m?.key_source && m.provider === p.provider) {
      status(line, "", m.key_source === "env" ? `환경변수 ${m.api_key_env}에서 키를 찾았습니다. ‘저장하고 확인’으로 모델 목록을 받으세요` : "저장된 API 키가 있습니다. ‘저장하고 확인’으로 모델 목록을 받으세요");
    } else status(line, "", "키는 OS 자격 증명 저장소에 저장되고 설정 파일에는 남지 않습니다.");
  };
  which.addEventListener("change", () => void describe());
  await describe();

  // 1) 설정에 이 제공자를 만들고 키를 저장한 뒤, 모델 목록을 받는다
  const connect = async () => {
    const p = current();
    const key = modelKey(p);
    btn.disabled = true;
    try {
      const existing = (await api.getSettings()).config.models[key];
      if (!existing || existing.provider !== p.provider || (p.base_url && existing.base_url !== p.base_url)) {
        const changes: [string, unknown][] = [
          [`models.${key}.provider`, p.provider],
          [`models.${key}.model`, existing?.model ?? p.model ?? ""],
          [`models.${key}.api_key_env`, p.api_key_env],
        ];
        if (p.base_url) changes.push([`models.${key}.base_url`, p.base_url]);
        if (p.price) changes.push([`models.${key}.price_input`, p.price[0]], [`models.${key}.price_output`, p.price[1]]);
        await api.setSettings("global", changes);
      }
      if (input.value.trim()) await api.setApiKey(key, input.value);
      status(line, "wait", "연결을 확인하는 중…");
      const r = await api.testModel(key);
      if (!r.ok) {
        status(line, "bad", r.message);
        return;
      }
      input.value = "";
      const { models, pick: first } = chatModels(p, r.models);
      if (!models.length) {
        status(line, "bad", "연결은 되었지만 모델 목록을 받지 못했습니다. 설정 → 모델에서 모델 ID를 직접 넣으세요");
        return;
      }
      pick.replaceChildren(...models.map((m) => h("option", { value: m, selected: m === first }, m)));
      pickRow.classList.remove("hidden");
      status(line, "ok", `연결되었습니다 · 쓸 수 있는 모델 ${models.length}개 · 하나를 고르세요`);
      pick.focus();
    } catch (e) {
      status(line, "bad", errorText(e));
    } finally {
      btn.disabled = false;
    }
  };
  // 2) 고른 모델을 기본 모델로
  const choose = async () => {
    const p = current();
    const key = modelKey(p);
    use.disabled = true;
    try {
      await api.setSettings("global", [[`models.${key}.model`, pick.value], ["routing.default", key]]);
      status(line, "ok", `기본 모델을 ${pick.value}(으)로 설정했습니다 · ${p.label}`);
      box.classList.add("selected");
      ctx.onModelChanged();
      onDone();
    } catch (e) {
      status(line, "bad", errorText(e));
    } finally {
      use.disabled = false;
    }
  };
  btn.addEventListener("click", () => void connect());
  input.addEventListener("keydown", (e) => e.key === "Enter" && void connect());
  use.addEventListener("click", () => void choose());
  return box;
}

function localChoice(onDone: () => void): HTMLElement {
  const line = h("div", { class: "status-line" });
  const select = h("select", { "aria-label": "로컬 모델" }) as HTMLSelectElement;
  const use = h("button", { class: "btn btn-secondary" }, "이 모델 쓰기") as HTMLButtonElement;
  const rescan = h("button", { class: "btn btn-secondary", title: "다시 찾기" }, codicon("refresh"), "다시 찾기");
  const row = h("div", { class: "row" }, select, use);
  const box = h("div", { class: "choice" },
    h("h3", {}, codicon("device-desktop"), "로컬 모델 (무료)"),
    h("p", {}, "비용 없이, 코드가 이 컴퓨터 밖으로 나가지 않게 쓰고 싶을 때. Ollama나 LM Studio가 실행 중이어야 합니다."),
    row, h("div", { class: "row" }, line, rescan));

  const scan = async () => {
    status(line, "wait", "로컬 모델 서버를 찾는 중…");
    row.classList.add("hidden");
    localServers = await api.probeLocal().catch(() => []);
    const options = localServers.filter((s) => s.running).flatMap((s) => s.models.map((m) => ({ s, m })));
    if (!options.length) {
      const running = localServers.find((s) => s.running);
      status(line, "bad", running ? `${running.name}은(는) 실행 중이지만 받은 모델이 없습니다` : "실행 중인 Ollama나 LM Studio를 찾지 못했습니다");
      return;
    }
    select.replaceChildren(...options.map(({ s, m }, i) => h("option", { value: String(i) }, `${m} · ${s.name}`)));
    select.dataset.options = JSON.stringify(options.map(({ s, m }) => [s.base_url, m]));
    row.classList.remove("hidden");
    status(line, "ok", `모델 ${options.length}개를 찾았습니다`);
  };
  use.addEventListener("click", async () => {
    const opts = JSON.parse(select.dataset.options ?? "[]") as [string, string][];
    const [base, model] = opts[Number(select.value)] ?? [];
    if (!model) return;
    use.disabled = true;
    try {
      await api.setSettings("global", [["models.local.base_url", base], ["models.local.model", model], ["routing.default", "local"]]);
      const r = await api.testModel("local");
      status(line, r.ok ? "ok" : "bad", r.ok ? `기본 모델을 ${model}(으)로 설정했습니다` : r.message);
      if (r.ok) {
        box.classList.add("selected");
        ctx.onModelChanged();
        onDone();
      }
    } catch (e) {
      status(line, "bad", errorText(e));
    } finally {
      use.disabled = false;
    }
  });
  rescan.addEventListener("click", () => void scan());
  void scan();
  return box;
}

function step(n: number, title: string, desc: string, ...content: HTMLElement[]): HTMLElement {
  return h("div", { class: "step", "data-step": String(n) },
    h("div", { class: "step-mark" }, h("span", {}, String(n)), codicon("check")),
    h("div", {}, h("h2", {}, title), h("p", { class: "desc" }, desc), ...content));
}

async function render(el: HTMLElement) {
  const markDone = (n: number) => el.querySelector(`.step[data-step="${n}"]`)?.classList.add("done");
  const folderBtn = h("button", { class: "btn btn-secondary" }, codicon("folder-opened"), "폴더 열기");
  folderBtn.addEventListener("click", async () => {
    await ctx.pickProject();
    if (ctx.hasProject()) markDone(2);
  });
  const chatBtn = h("button", { class: "btn btn-secondary" }, codicon("comment-discussion"), "채팅 열기");
  chatBtn.addEventListener("click", () => ctx.showChat());
  const finishBtn = h("button", { class: "btn btn-primary" }, "시작하기");
  finishBtn.addEventListener("click", finish);
  const skip = h("button", { class: "btn btn-ghost" }, "건너뛰기");
  skip.addEventListener("click", finish);

  const models = h("div", { class: "choices" }, await cloudChoice(() => markDone(1)), localChoice(() => markDone(1)));
  el.append(h("div", { class: "onboard page-content" },
    h("div", { class: "onboard-head" }, hex("brand"), h("h1", {}, "Lantern 시작하기"), h("div", { class: "spacer" }), skip),
    h("p", { class: "lead" }, "세 단계면 됩니다. 나중에 설정에서 언제든 바꿀 수 있습니다."),
    h("div", { class: "steps" },
      step(1, "AI 모델 연결", "클라우드 모델과 로컬 모델 중 하나를 고르세요. 둘 다 연결해 두고 작업마다 바꿔 써도 됩니다.", models),
      step(2, "프로젝트 폴더 열기", "폴더를 열면 맥락 엔진이 코드를 인덱싱합니다. 대부분 몇 초면 끝납니다.", h("div", { class: "row" }, folderBtn)),
      step(3, "이렇게 씁니다", "질문하면 Lantern이 관련 코드를 골라 붙이고, 답변 위 ‘Lantern이 본 코드’에서 무엇을 왜 보냈는지 보여줍니다. 파일을 바꾸는 작업은 Diff를 보고 승인해야 적용됩니다.",
        h("div", { class: "row", style: "gap:8px" }, chatBtn, finishBtn)))));

  if (ctx.hasProject()) markDone(2);
  // 1단계는 기본 모델이 실제로 연결 확인을 통과했을 때만 완료로 표시한다.
  const info = await api.modelInfo().catch(() => null);
  const def = info?.models.find((m) => m.key === info.default);
  if (def?.has_key) {
    const r = await api.testModel(def.key).catch(() => null);
    if (r?.ok) {
      markDone(1);
      el.querySelector('.step[data-step="1"] .desc')?.after(h("div", { class: "status-line ok", style: "margin:-6px 0 12px" }, codicon("pass"), `연결됨: ${def.model} (기본 모델 ‘${def.key}’)`));
    }
  }
}

export function init(c: Ctx) {
  ctx = c;
}

export function open() {
  host = editor.openPage("onboarding", "시작하기", "rocket", (el) => void render(el));
  void host;
}
