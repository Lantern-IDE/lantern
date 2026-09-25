// 화면 언어. 원문(한국어)을 그대로 키로 쓰고, 영어일 때만 사전으로 바꾼다.
// 화면 요소를 일일이 감싸지 않도록 DOM 변화를 지켜보며 글자·툴팁·자리표시자를 바꾼다.
// 편집기, 터미널, 채팅 본문, Diff처럼 사용자·모델의 내용은 건드리지 않는다.
import en from "./locales/en.json";
import { store } from "./dom";

export type Lang = "ko" | "en";

const SKIP = ".cm-editor, .xterm, .md, .user-text, .diff, pre, code, #tree, .kb-id, [data-no-i18n]";
const ATTRS = ["title", "placeholder", "aria-label"];
const HANGUL = /[가-힣]/;

const saved = store.get<Lang | null>("lang", null);
export const lang: Lang = saved ?? (navigator.language.toLowerCase().startsWith("ko") ? "ko" : "en");

const unescape = (s: string) => s.replace(/\\n/g, "\n");
const exact = new Map<string, string>(Object.entries(en.exact as Record<string, string>).map(([k, v]) => [unescape(k), unescape(v)]));

interface Pattern { re: RegExp; out: string; literal: number }
// 글자가 많이 고정된(구체적인) 템플릿부터 맞춘다. "맥락 {}{}"가 "맥락 엔진 인덱스: …"를 먼저 삼키지 않게.
const patterns: Pattern[] = Object.entries(en.templates as Record<string, string>)
  .map(([ko, out]) => {
    const parts = unescape(ko).split("{}");
    const src = parts.map((p) => p.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")).join("([\\s\\S]*?)");
    return { re: new RegExp(`^${src}$`), out: unescape(out), literal: parts.join("").length };
  })
  .sort((a, b) => b.literal - a.literal);

function fromPattern(s: string): string | null {
  for (const p of patterns) {
    const m = p.re.exec(s);
    if (!m) continue;
    const args = m.slice(1).map((a) => t(a)); // 안쪽 값도 원문이면 번역 (예: 상태 이름)
    let i = 0;
    const out = p.out.replace(/\{(\d*)\}/g, (_, n: string) => (n ? args[Number(n) - 1] : args[i++]) ?? "");
    // 한국어는 복수형이 없어 "1 files"가 되기 쉽다
    return out.replace(/(^|[^\d.,])1 (file|result|occurrence|symbol|item|request|token|line|change|match)(?:es|s)\b/g, (_, pre: string, noun: string) => `${pre}1 ${noun}`);
  }
  return null;
}

/** 원문을 현재 언어로. 사전에 없으면 그대로 */
export function t(s: string): string {
  if (lang === "ko" || !HANGUL.test(s)) return s;
  const trimmed = s.trim();
  const hit = exact.get(trimmed) ?? fromPattern(trimmed);
  if (hit === null || hit === undefined) return s;
  if (trimmed === s) return hit;
  // 앞뒤 공백은 유지 (h()로 이어 붙인 조각들)
  const lead = s.slice(0, s.indexOf(trimmed));
  return lead + hit + s.slice(lead.length + trimmed.length);
}

export function setLang(next: Lang) {
  store.set("lang", next);
  location.reload();
}

function skip(node: Node): boolean {
  const el = node.nodeType === Node.ELEMENT_NODE ? (node as Element) : node.parentElement;
  return !el || !!el.closest(SKIP);
}

function translateText(node: Text) {
  const v = node.data;
  if (!HANGUL.test(v) || skip(node)) return;
  const out = t(v);
  if (out !== v) node.data = out;
}

function translateAttrs(el: Element) {
  for (const a of ATTRS) {
    const v = el.getAttribute(a);
    if (v && HANGUL.test(v)) {
      const out = t(v);
      if (out !== v) el.setAttribute(a, out);
    }
  }
}

function translateTree(root: Node) {
  if (root.nodeType === Node.TEXT_NODE) return translateText(root as Text);
  if (root.nodeType !== Node.ELEMENT_NODE || skip(root)) return;
  translateAttrs(root as Element);
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_ELEMENT | NodeFilter.SHOW_TEXT, {
    acceptNode: (n) => (n.nodeType === Node.ELEMENT_NODE && (n as Element).matches(SKIP) ? NodeFilter.FILTER_REJECT : NodeFilter.FILTER_ACCEPT),
  });
  for (let n = walker.nextNode(); n; n = walker.nextNode()) {
    if (n.nodeType === Node.TEXT_NODE) translateText(n as Text);
    else translateAttrs(n as Element);
  }
}

export function init() {
  document.documentElement.lang = lang;
  if (lang === "ko") return;
  translateTree(document.body);
  // #tree 안의 파일 이름은 건드리지 않지만, 트리 자체의 접근성 이름은 번역한다
  const tree = document.querySelector("#tree");
  if (tree) {
    const label = tree.getAttribute("aria-label");
    if (label) tree.setAttribute("aria-label", t(label));
  }
  new MutationObserver((records) => {
    for (const r of records) {
      if (r.type === "childList") r.addedNodes.forEach(translateTree);
      else if (r.type === "characterData") translateText(r.target as Text);
      else if (r.type === "attributes" && r.target.nodeType === Node.ELEMENT_NODE && !skip(r.target)) translateAttrs(r.target as Element);
    }
  }).observe(document.body, { childList: true, subtree: true, characterData: true, attributes: true, attributeFilter: ATTRS });
}
