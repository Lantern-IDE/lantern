// 변경 영향 반경 줄: 승인 카드·Ctrl+K 패널에서 바뀌는 코드의 호출자·테스트·함께 바뀐 파일을 보인다.
// 언어 서버가 켜져 있으면 직접 호출 수를 다시 센다.
import { invoke } from "@tauri-apps/api/core";
import { api } from "./api";
import { h } from "./dom";
import { codicon } from "./icons";
import * as editor from "./editor";
import type { GNode, Impact } from "./map";
import { diffTarget } from "./lib/diff";
import { isTestPath, type TestTarget } from "./lib/testPrompt";
import * as lsp from "./lsp";

/** 영향 반경 줄의 단추가 부를 것 (채팅이 정한다) */
const deps = {
  showImpact: (_i: Impact) => {},
  createTests: async (_t: TestTarget[]) => {},
};

export function configure(d: typeof deps) {
  Object.assign(deps, d);
}

const RISK_LABEL: Record<string, string> = { low: "낮음", medium: "보통", high: "높음" };

/**
 * 언어 서버가 켜져 있으면 바뀌는 심볼의 참조를 다시 센다.
 * - 이름이 여러 곳에 정의되어 있으면 언어 서버 결과를 쓴다 (같은 이름의 다른 심볼을 구분)
 * - 한 곳에만 정의되어 있으면 이름 기준과 합친다. 언어 서버는 열린 파일·설정된 프로젝트만 보고
 *   (예: tsconfig 없는 TypeScript) 일부를 놓치기도 하므로, 결과가 줄어들게 두지 않는다.
 */
async function lspCallers(i: Impact): Promise<{ callers: number; files: number; lspOnly: number } | null> {
  const lang = editor.langOf(i.path)?.lsp;
  if (!lang) return null;
  const touched = i.graph.nodes.filter((n) => n.depth === 0 && n.line).slice(0, 3);
  if (!touched.length) return null;
  const text = await api.readFile(i.path).catch(() => "");
  const lines = text.split("\n");
  const found = new Map<string, GNode>();
  const touchedIds = new Set(touched.map((n) => n.id));
  for (const n of touched) {
    // 정의 줄(또는 그 아래 몇 줄)에서 이름이 시작하는 열
    let pos: { line: number; col: number } | null = null;
    for (let l = n.line! - 1; l < Math.min(lines.length, n.line! + 3); l++) {
      const col = lines[l]?.search(new RegExp(`\\b${n.label.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}\\b`));
      if (col !== undefined && col >= 0) {
        pos = { line: l, col };
        break;
      }
    }
    if (!pos) return null;
    const refs = await lsp.references(lang, i.path, pos.line, pos.col);
    if (!refs) return null;
    const enclosing = await invoke<(GNode | null)[]>("graph_enclosing", { locations: refs.map((r) => [r.path, r.line]) }).catch(() => null);
    if (!enclosing) return null;
    for (const e of enclosing) if (e && !touchedIds.has(e.id)) found.set(e.id, e);
  }
  const lspOnly = [...found.keys()].filter((id) => !i.graph.nodes.some((n) => n.id === id && n.depth === 1)).length;
  if (i.unique) for (const n of i.graph.nodes) if (n.depth === 1) found.set(n.id, n);
  return { callers: found.size, files: new Set([...found.values()].map((n) => n.path).filter((p) => p !== i.path)).size, lspOnly };
}

export function impactRow(diff: string, onTouched: (names: string[]) => void = () => {}): HTMLElement | null {
  const target = diffTarget(diff);
  if (!target) return null;
  const row = h("div", { class: "impact loading" }, codicon("loading", "codicon-modifier-spin"), h("span", {}, "영향 반경 계산 중…"));
  void invoke<Impact>("graph_impact", { path: target.path, ranges: target.ranges }).then((i) => {
    onTouched(i.touched);
    row.classList.remove("loading");
    const nothing = !i.touched.length && !i.callers && !i.cochanged.length;
    const stats: HTMLElement[] = [];
    const stat = (n: number, label: string) => stats.push(h("span", { class: "istat" }, h("b", {}, String(n)), label));
    if (i.touched.length) stats.push(h("span", { class: "itouch", title: i.touched.join(", ") }, codicon("symbol-method"), i.touched.slice(0, 2).join(", ") + (i.touched.length > 2 ? ` 외 ${i.touched.length - 2}` : "")));
    stat(i.callers, "직접 호출");
    if (i.callers2) stat(i.callers2, "간접 호출");
    if (i.files) stat(i.files, "파일에 걸침");
    if (i.cochanged.length) stat(i.cochanged.length, "함께 바뀌던 파일");
    stats.push(h("span", { class: `istat ${i.tests.length ? "tests" : "no-tests"}` }, codicon(i.tests.length ? "beaker" : "warning"), i.tests.length ? `테스트 ${i.tests.length}` : "테스트 없음"));
    if (!i.tests.length && i.touched.length && !isTestPath(target.path)) {
      const make = h("button", { class: "istat-action", title: "기존 테스트 형식을 따라 이 코드의 테스트를 새 작업으로 만듭니다" }, codicon("beaker"), "테스트 만들기");
      make.addEventListener("click", () => {
        make.disabled = true;
        void deps.createTests([{ path: target.path, symbols: i.touched }]);
      });
      stats.push(make);
    }
    const view = h("button", { class: "btn btn-ghost", title: "영향 범위를 지도에서 보기" }, codicon("type-hierarchy"), "지도에서 보기");
    view.addEventListener("click", () => deps.showImpact(i));
    // 숫자를 사실로 믿지 않게: 무엇을 기준으로 셌고 무엇을 놓치는지
    const basis = h("span", { class: "ibasis", title: "이름으로 찾은 호출 관계(정적 분석)입니다. DI·리플렉션·이벤트·라우팅처럼 실행 중에 정해지는 호출과, URL로 부르는 테스트는 세지 않습니다." }, codicon("info"));
    row.replaceChildren(...([
      h("div", { class: "ihead" },
        codicon("pulse"),
        h("span", { class: "ititle" }, "영향 반경"),
        basis,
        i.supported ? h("span", { class: `risk risk-${i.risk}` }, `위험도 ${RISK_LABEL[i.risk]}`) : null,
        i.graph.nodes.length > 1 ? view : null),
      !i.supported
        ? h("div", { class: "inote" }, "이 언어는 호출 관계를 분석하지 않아 영향 범위를 알 수 없습니다. 바뀌는 곳을 직접 확인하세요.")
        : nothing
          ? h("div", { class: "inote" }, "이 변경을 호출하는 다른 코드를 찾지 못했습니다.")
          : h("div", { class: "istats" }, ...stats),
      i.framework.length ? h("div", { class: "inote framework" }, codicon("warning"), `${i.framework.join(", ")}: 프레임워크가 부르는 코드라 호출자·테스트 수에 잡히지 않을 수 있습니다`) : null,
      i.ambiguous.length ? h("div", { class: "inote ambiguous" }, `이름이 흔해 호출자를 셀 수 없음: ${i.ambiguous.join(", ")}`) : null,
    ].filter(Boolean) as Node[]));
    if (i.supported) row.classList.add(`risk-${i.risk}`);
    // 언어 서버로 다시 확인 (켜져 있을 때만). 이름 기준 결과와 다르면 고쳐 보여준다.
    void lspCallers(i).then((r) => {
      if (!r) return;
      const direct = row.querySelector(".istat b");
      if (direct) direct.textContent = String(r.callers);
      row.querySelector(".inote.ambiguous")?.remove();
      const note = r.callers === i.callers ? "언어 서버로 확인함" : `언어 서버로 확인함 (이름 기준 ${i.callers})`;
      if (r.lspOnly && i.unique) row.querySelector(".istats")?.append(h("span", { class: "istat" }, h("b", {}, `+${r.lspOnly}`), "언어 서버가 더 찾음"));
      row.querySelector(".ihead")!.insertBefore(h("span", { class: "lsp-ok", title: "언어 서버의 참조 찾기로 다시 센 직접 호출 수입니다" }, codicon("verified"), note), row.querySelector(".ihead .btn"));
    });
  }).catch(() => row.remove());
  return row;
}

