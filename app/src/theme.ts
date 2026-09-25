// 편집기 모양. 다크는 Lantern 색 바탕에 VS Code "Dark+" 문법 색, 라이트는 "Light+" 문법 색.
// 문법 색은 개발자가 이미 익숙한 VS Code 값을 그대로 쓴다 (낯선 조작·낯선 코드 색을 피한다).
import { EditorView } from "@codemirror/view";
import { HighlightStyle, syntaxHighlighting } from "@codemirror/language";
import { tags as t } from "@lezer/highlight";
import type { Extension } from "@codemirror/state";

function chrome(dark: boolean) {
  const c = dark
    ? { bg: "#1b1c22", fg: "#d5d8e0", gutter: "#5f6474", gutterActive: "#d5d8e0", activeLine: "rgba(160,168,200,0.06)", sel: "rgba(108,115,245,0.32)", caret: "#9aa3ff", widget: "#202128", border: "#3a3d49", selItem: "rgba(108,115,245,0.3)", match: "#8f97ff", bracket: "rgba(160,168,200,0.5)", selMatch: "rgba(160,168,200,0.14)", search: "rgba(220,174,58,0.3)", searchSel: "rgba(220,174,58,0.55)", input: "#1f2027" }
    : { bg: "#ffffff", fg: "#2a2d37", gutter: "#9a9fad", gutterActive: "#2a2d37", activeLine: "rgba(40,48,90,0.04)", sel: "rgba(79,85,216,0.2)", caret: "#4f55d8", widget: "#ffffff", border: "#c7cad4", selItem: "rgba(79,85,216,0.14)", match: "#4147c4", bracket: "rgba(40,48,90,0.35)", selMatch: "rgba(40,48,90,0.08)", search: "rgba(234,170,0,0.3)", searchSel: "rgba(234,170,0,0.55)", input: "#ffffff" };
  return EditorView.theme(
    {
      "&": { color: c.fg, backgroundColor: c.bg },
      ".cm-content": { caretColor: c.caret, padding: "4px 0" },
      ".cm-cursor, .cm-dropCursor": { borderLeft: `2px solid ${c.caret}` },
      "&.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground, .cm-selectionBackground, .cm-content ::selection":
        { backgroundColor: c.sel },
      ".cm-activeLine": { backgroundColor: c.activeLine },
      ".cm-gutters": { backgroundColor: c.bg, color: c.gutter, border: "none" },
      ".cm-activeLineGutter": { backgroundColor: "transparent", color: c.gutterActive },
      ".cm-lineNumbers .cm-gutterElement": { padding: "0 10px 0 18px", minWidth: "46px" },
      ".cm-foldGutter .cm-gutterElement": { color: c.gutter, padding: "0 4px" },
      ".cm-foldPlaceholder": { backgroundColor: c.selMatch, border: "none", color: c.fg, padding: "0 4px", borderRadius: "3px" },
      ".cm-matchingBracket, &.cm-focused .cm-matchingBracket": { backgroundColor: "transparent", outline: `1px solid ${c.bracket}`, borderRadius: "2px" },
      ".cm-nonmatchingBracket": { color: "#f2656a" },
      ".cm-selectionMatch": { backgroundColor: c.selMatch },
      ".cm-searchMatch": { backgroundColor: c.search, borderRadius: "2px" },
      ".cm-searchMatch.cm-searchMatch-selected": { backgroundColor: c.searchSel },
      ".cm-tooltip": { backgroundColor: c.widget, border: `1px solid ${c.border}`, color: c.fg, borderRadius: "6px", boxShadow: dark ? "0 8px 24px rgba(0,0,0,.4)" : "0 8px 24px rgba(20,24,40,.12)" },
      ".cm-tooltip-autocomplete > ul": { fontFamily: "var(--code-font)", padding: "4px" },
      ".cm-tooltip-autocomplete > ul > li": { padding: "2px 8px", borderRadius: "4px" },
      ".cm-tooltip-autocomplete > ul > li[aria-selected]": { backgroundColor: c.selItem, color: c.fg },
      ".cm-completionMatchedText": { color: c.match, textDecoration: "none", fontWeight: "700" },
      ".cm-panels": { backgroundColor: c.widget, color: c.fg },
      ".cm-panels.cm-panels-top": { borderBottom: `1px solid ${c.border}` },
      ".cm-panels.cm-panels-bottom": { borderTop: `1px solid ${c.border}` },
      ".cm-panel.cm-search": { padding: "6px 10px", fontFamily: "var(--ui-font)" },
      ".cm-panel.cm-search [name=close]": { color: c.fg, fontSize: "16px" },
      ".cm-panel label": { fontSize: "12px" },
      ".cm-button": { backgroundImage: "none", backgroundColor: "transparent", border: `1px solid ${c.border}`, color: c.fg, borderRadius: "4px", fontSize: "12px" },
      ".cm-textfield": { backgroundColor: c.input, border: `1px solid ${c.border}`, color: c.fg, borderRadius: "4px", fontSize: "12.5px" },
      ".cm-diagnostic": { padding: "4px 10px" },
      ".cm-diagnostic-error": { borderLeft: "2px solid #f2656a" },
      ".cm-diagnostic-warning": { borderLeft: "2px solid #dcae3a" },
      ".cm-diagnostic-info": { borderLeft: "2px solid #6aa8f8" },
      ".cm-lsp-documentation": { fontFamily: "var(--ui-font)" },
    },
    { dark },
  );
}

const darkPlus = HighlightStyle.define([
  { tag: [t.keyword, t.self, t.null, t.bool, t.atom, t.definitionKeyword, t.modifier], color: "#569cd6" },
  { tag: [t.controlKeyword, t.moduleKeyword, t.operatorKeyword], color: "#c586c0" },
  { tag: [t.string, t.special(t.string), t.character], color: "#ce9178" },
  { tag: t.regexp, color: "#d16969" },
  { tag: t.escape, color: "#d7ba7d" },
  { tag: [t.number, t.integer, t.float], color: "#b5cea8" },
  { tag: [t.comment, t.lineComment, t.blockComment, t.docComment], color: "#6a9955" },
  { tag: [t.function(t.variableName), t.function(t.propertyName), t.function(t.definition(t.variableName)), t.macroName], color: "#dcdcaa" },
  { tag: [t.typeName, t.className, t.namespace, t.definition(t.typeName), t.standard(t.typeName)], color: "#4ec9b0" },
  { tag: [t.variableName, t.propertyName, t.definition(t.variableName), t.definition(t.propertyName), t.attributeName, t.labelName], color: "#9cdcfe" },
  { tag: [t.constant(t.variableName), t.constant(t.propertyName)], color: "#4fc1ff" },
  { tag: [t.operator, t.punctuation, t.separator, t.bracket, t.derefOperator], color: "#d4d4d4" },
  { tag: [t.tagName, t.angleBracket], color: "#569cd6" },
  { tag: t.heading, color: "#569cd6", fontWeight: "bold" },
  { tag: [t.link, t.url], color: "#ce9178" },
  { tag: t.emphasis, fontStyle: "italic" },
  { tag: t.strong, fontWeight: "bold" },
  { tag: t.strikethrough, textDecoration: "line-through" },
  { tag: [t.meta, t.annotation, t.processingInstruction], color: "#9b9b9b" },
  { tag: t.invalid, color: "#f44747" },
]);

const lightPlus = HighlightStyle.define([
  { tag: [t.keyword, t.self, t.null, t.bool, t.atom, t.definitionKeyword, t.modifier], color: "#0000ff" },
  { tag: [t.controlKeyword, t.moduleKeyword, t.operatorKeyword], color: "#af00db" },
  { tag: [t.string, t.special(t.string), t.character], color: "#a31515" },
  { tag: t.regexp, color: "#811f3f" },
  { tag: t.escape, color: "#ee0000" },
  { tag: [t.number, t.integer, t.float], color: "#098658" },
  { tag: [t.comment, t.lineComment, t.blockComment, t.docComment], color: "#008000" },
  { tag: [t.function(t.variableName), t.function(t.propertyName), t.function(t.definition(t.variableName)), t.macroName], color: "#795e26" },
  { tag: [t.typeName, t.className, t.namespace, t.definition(t.typeName), t.standard(t.typeName)], color: "#267f99" },
  { tag: [t.variableName, t.propertyName, t.definition(t.variableName), t.definition(t.propertyName), t.labelName], color: "#001080" },
  { tag: t.attributeName, color: "#e50000" },
  { tag: [t.constant(t.variableName), t.constant(t.propertyName)], color: "#0070c1" },
  { tag: [t.operator, t.punctuation, t.separator, t.bracket, t.derefOperator], color: "#000000" },
  { tag: [t.tagName, t.angleBracket], color: "#800000" },
  { tag: t.heading, color: "#800000", fontWeight: "bold" },
  { tag: [t.link, t.url], color: "#a31515" },
  { tag: t.emphasis, fontStyle: "italic" },
  { tag: t.strong, fontWeight: "bold" },
  { tag: t.strikethrough, textDecoration: "line-through" },
  { tag: [t.meta, t.annotation, t.processingInstruction], color: "#6e6e6e" },
  { tag: t.invalid, color: "#cd3131" },
]);

const dark = [chrome(true), syntaxHighlighting(darkPlus)];
const light = [chrome(false), syntaxHighlighting(lightPlus)];

export function editorAppearance(isDark: boolean): Extension {
  return isDark ? dark : light;
}
