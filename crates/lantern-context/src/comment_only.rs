//! 변경이 주석·공백만 건드리는지. 승인 카드가 위험도 대신 그렇게 말할 때 쓴다.
//! '주석만'이라고 잘못 말하면 코드 변경을 못 보고 넘기게 되므로, 장담할 수 없으면 false다.

use crate::lang::Lang;
use std::path::Path;
use tree_sitter::Parser;

/// 이보다 큰 파일은 따지지 않는다
const MAX_BYTES: usize = 2_000_000;
/// 도구가 읽는 주석 (빌드 제약, 타입 검사·린트 끄기, 번들러 지시, 파일 인코딩). 주석이어도 동작을 바꾼다
const DIRECTIVES: &[&str] = &[
    "go:", "+build", "@ts-", "eslint", "noqa", "type:", "pragma", "#!", "rustfmt", "clippy", "nolint", "prettier", "@flow", "@jsx",
    "<reference", "__PURE__", "__NO_SIDE_EFFECTS__", "webpack", "@vite-ignore", "coding:", "coding=",
];
/// 이 노드 안에서는 공백도 값이다 (문자열·서식·JSX 글자)
const VALUE_NODES: &[&str] = &["string", "char", "rune", "format", "regex", "jsx"];
/// 노드 종류 대신 넣는 표시: 자식이 덮지 않는 글자, 또는 코틀린의 줄바꿈
const GAP: u16 = u16::MAX;

/// `old`(원문)에 `diff`를 적용해도 코드는 그대로인가
pub fn only_comments(path: &str, old: &str, diff: &str) -> bool {
    let Some(lang) = Lang::from_path(Path::new(path)) else { return false };
    if old.len() > MAX_BYTES {
        return false;
    }
    // cgo: `import "C"` 앞의 주석은 C 코드이고 `//export`도 주석으로 쓴다
    if lang == Lang::Go && old.contains("\"C\"") {
        return false;
    }
    let changed = |l: &&str| (l.starts_with('+') && !l.starts_with("+++")) || (l.starts_with('-') && !l.starts_with("---"));
    if diff.lines().filter(changed).any(|l| DIRECTIVES.iter().any(|d| l.contains(d))) {
        return false;
    }
    let Some(new) = apply(old, diff) else { return false };
    match (code_shape(lang, old), code_shape(lang, &new)) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

/// unified diff를 적용한다. 원문과 문맥이 맞지 않으면 None
fn apply(old: &str, diff: &str) -> Option<String> {
    let lines: Vec<&str> = old.split('\n').collect();
    let same = |at: usize, text: &str| lines.get(at).is_some_and(|l| l.trim_end_matches('\r') == text.trim_end_matches('\r'));
    let mut out: Vec<&str> = Vec::new();
    let mut at = 0usize;
    let mut in_hunk = false;
    for l in diff.split('\n') {
        if let Some(h) = l.strip_prefix("@@ -") {
            let old_side = h.split(' ').next()?;
            let (start, count) = old_side.split_once(',').unwrap_or((old_side, "1"));
            let (start, count): (usize, usize) = (start.parse().ok()?, count.parse().ok()?);
            // 넣기만 한 덩어리(-a,0)는 a번째 줄 뒤에 붙는다
            let to = if count == 0 { start } else { start.checked_sub(1)? };
            out.extend(lines.get(at..to)?);
            at = to;
            in_hunk = true;
        } else if in_hunk {
            match l.as_bytes().first() {
                Some(b' ') => {
                    if !same(at, &l[1..]) {
                        return None;
                    }
                    out.push(lines[at]);
                    at += 1;
                }
                Some(b'-') => {
                    if !same(at, &l[1..]) {
                        return None;
                    }
                    at += 1;
                }
                Some(b'+') => out.push(&l[1..]),
                Some(b'\\') | None => {}
                Some(_) => return None,
            }
        }
    }
    out.extend(lines.get(at..)?);
    Some(out.join("\n"))
}

/// 주석을 뺀 구문 트리: (깊이, 노드 종류, 잎이면 글자). 들여쓰기로 구조가 바뀌면(Python) 깊이에서 드러난다.
/// 구문 오류가 있으면 None
fn code_shape(lang: Lang, src: &str) -> Option<Vec<(usize, u16, &str)>> {
    let mut parser = Parser::new();
    parser.set_language(&lang.grammar()).ok()?;
    let tree = parser.parse(src, None)?;
    if tree.root_node().has_error() {
        return None;
    }
    let mut out = Vec::new();
    let mut c = tree.walk();
    let mut depth = 0usize;
    let mut last_row = None;
    loop {
        let n = c.node();
        if !n.kind().contains("comment") {
            let leaf = n.child_count() == 0;
            // 코틀린은 줄바꿈이 문장을 끝내는데 (`return a` 다음 줄 `- b`) 문법은 한 식으로 읽는다
            if leaf && lang == Lang::Kotlin {
                if last_row.is_some_and(|r| r != n.start_position().row) {
                    out.push((depth, GAP, "\n"));
                }
                last_row = Some(n.end_position().row);
            }
            out.push((depth, n.kind_id(), if leaf { n.utf8_text(src.as_bytes()).ok()? } else { "" }));
            if !leaf {
                push_gaps(n, src, depth, &mut out)?;
            }
            if c.goto_first_child() {
                depth += 1;
                continue;
            }
        }
        while !c.goto_next_sibling() {
            if !c.goto_parent() {
                return Some(out);
            }
            depth -= 1;
        }
    }
}

/// 자식이 덮지 않는 글자: 문법이 이름 없이 삼킨 토큰 (`f"{x:>3}"`의 `>3`, 코틀린 `'a'`의 `a`)
fn push_gaps<'a>(n: tree_sitter::Node, src: &'a str, depth: usize, out: &mut Vec<(usize, u16, &'a str)>) -> Option<()> {
    let value = VALUE_NODES.iter().any(|k| n.kind().contains(k));
    let mut w = n.walk();
    let mut at = n.start_byte();
    let spans = n.children(&mut w).map(|c| (c.start_byte(), c.end_byte()));
    for (start, end) in spans.chain([(n.end_byte(), n.end_byte())]) {
        let gap = src.get(at..start.max(at))?;
        let gap = if value { gap } else { gap.trim() };
        if !gap.is_empty() {
            out.push((depth, GAP, gap));
        }
        at = end.max(at);
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 한 줄 바꾸기 diff
    fn replace_line(old: &str, n: usize, new_line: &str) -> String {
        let line = old.lines().nth(n - 1).unwrap();
        format!("--- a/x\n+++ b/x\n@@ -{n},1 +{n},1 @@\n-{line}\n+{new_line}\n")
    }

    #[test]
    fn comment_changes_by_language() {
        let go = "package a\n\nfunc f() {\n\tfor i := 0; i < 3; i++ {\n\t\tg()\n\t}\n}\n";
        assert!(only_comments("a.go", go, &replace_line(go, 4, "\tfor i := 0; i < 3; i++ { // 세 번")), "줄 끝 주석");
        assert!(only_comments("a.go", go, "@@ -4,0 +5,2 @@\n+\t\t// 부른다\n+\n"), "주석과 빈 줄 넣기");
        assert!(!only_comments("a.go", go, &replace_line(go, 4, "\tfor i := 0; i < 4; i++ {")), "숫자 하나도 코드다");

        let rs = "/// 인사\npub fn a() -> u32 {\n    1 // 하나\n}\n";
        assert!(only_comments("a.rs", rs, &replace_line(rs, 1, "/// 인사말")), "문서 주석");
        assert!(only_comments("a.rs", rs, &replace_line(rs, 3, "    1 // 일")));
        assert!(!only_comments("a.rs", rs, &replace_line(rs, 3, "    2 // 하나")));

        let ts = "export function a() {\n  return 1;\n}\n";
        assert!(only_comments("a.ts", ts, "@@ -1,0 +2,3 @@\n+  /*\n+   * 여러 줄\n+   */\n"), "여러 줄 주석");
        assert!(!only_comments("a.ts", ts, &replace_line(ts, 2, "  return 2;")));

        let java = "class A {\n    int a() { return 1; }\n}\n";
        assert!(only_comments("A.java", java, "@@ -1,0 +2,1 @@\n+    // 하나를 돌려준다\n"));
    }

    #[test]
    fn strings_that_look_like_comments_are_code() {
        // 여러 줄 문자열 안의 // 와 # 은 주석이 아니다
        let ts = "export const q = `\n  // a\n`;\n";
        assert!(!only_comments("a.ts", ts, &replace_line(ts, 2, "  // b")));
        let py = "Q = \"\"\"\n# a\n\"\"\"\n";
        assert!(!only_comments("a.py", py, &replace_line(py, 2, "# b")));
        // 독스트링도 문자열이다 (함수 안에서 읽을 수 있다)
        let doc = "def f():\n    \"\"\"하나\"\"\"\n    return 1\n";
        assert!(!only_comments("a.py", doc, &replace_line(doc, 2, "    \"\"\"둘\"\"\"")));
    }

    #[test]
    fn python_indentation_is_code() {
        let py = "def f(x):\n    if x:\n        a()\n        b()\n";
        assert!(!only_comments("a.py", py, &replace_line(py, 4, "    b()")), "들여쓰기가 구조를 바꾼다");
        assert!(only_comments("a.py", py, "@@ -2,0 +3,1 @@\n+    # 켜졌을 때만\n"));
    }

    #[test]
    fn tokens_the_grammar_swallows_are_code() {
        let py = "x = 1\ns = f\"{x:>3}\"\n";
        assert!(!only_comments("a.py", py, &replace_line(py, 2, "s = f\"{x:<3}\"")), "f-string 서식");
        assert!(!only_comments("a.py", py, &replace_line(py, 2, "s = f\"{x: >3}\"")), "서식 안의 공백");
        let py = "s = f\"{{a}}\"\n";
        assert!(!only_comments("a.py", py, &replace_line(py, 1, "s = f\"{{b}}\"")), "f-string 이스케이프");
        let kt = "val a = 'a'\n";
        assert!(!only_comments("a.kt", kt, &replace_line(kt, 1, "val a = 'b'")), "코틀린 문자");
        let cs = "class A {\n    string a = $\"{1:F2}\";\n}\n";
        assert!(!only_comments("a.cs", cs, &replace_line(cs, 2, "    string a = $\"{1:F3}\";")), "C# 서식");
        let go = "package a\n\nvar S = \"a b\"\n";
        assert!(!only_comments("a.go", go, &replace_line(go, 3, "var S = \"a  b\"")), "문자열 안의 공백");
        let tsx = "export const a = <div>x</div>;\n";
        assert!(!only_comments("a.tsx", tsx, &replace_line(tsx, 1, "export const a = <div> x</div>;")), "JSX 글자 앞의 공백");
    }

    #[test]
    fn whitespace_between_tokens_is_not_code() {
        let go = "package a\n\nfunc f(a, b int) {\n}\n";
        assert!(only_comments("a.go", go, &replace_line(go, 3, "func f(a,  b int) {")));
        let ts = "export const a = f(1, 2);\n";
        assert!(only_comments("a.ts", ts, &replace_line(ts, 1, "export const a = f(1,  2);")));
        let kt = "fun f(a: Int, b: Int) = a\n";
        assert!(only_comments("a.kt", kt, &replace_line(kt, 1, "fun f(a: Int,  b: Int) = a")));
        let py = "x = f(1, 2)\n";
        assert!(only_comments("a.py", py, &replace_line(py, 1, "x = f(1,  2)")));
        // 틈을 비교해도 값이 아닌 곳의 주석은 그대로 주석이다
        let lit = "package a\n\nvar T = S{\n\ta: 1,\n}\n";
        assert!(only_comments("a.go", lit, "@@ -3,0 +4,1 @@\n+\t// 하나\n"), "복합 리터럴");
        let cpp = "template <typename T>\nT f(T a) { return a; }\n";
        assert!(only_comments("a.cpp", cpp, "@@ -1,0 +2,1 @@\n+// 그대로 돌려준다\n"), "템플릿");
        let kt = "val g = { x: Int ->\n    x\n}\n";
        assert!(only_comments("a.kt", kt, "@@ -1,0 +2,1 @@\n+    // 그대로\n"), "람다");
        let py = "x = {\n    \"a\": 1,\n}\n";
        assert!(only_comments("a.py", py, "@@ -1,0 +2,1 @@\n+    # 하나\n"), "사전");
        let tsx = "export const a = (\n  <div>\n    <b />\n  </div>\n);\n";
        assert!(only_comments("a.tsx", tsx, "@@ -0,0 +1,1 @@\n+// 굵게\n"), "JSX 바깥");
    }

    #[test]
    fn kotlin_line_breaks_end_statements() {
        let kt = "fun f(a: Int, b: Int): Int {\n    return a - b\n}\n";
        let split = "@@ -2,1 +2,2 @@\n-    return a - b\n+    return a\n+        - b\n";
        assert!(!only_comments("a.kt", kt, split), "- b가 따로 떨어진다");
        assert!(only_comments("a.kt", kt, &replace_line(kt, 2, "    return a - b // 뺀다")), "줄 끝 주석");
        assert!(only_comments("a.kt", kt, "@@ -1,0 +2,2 @@\n+    // 뺀다\n+\n"), "주석과 빈 줄");
    }

    #[test]
    fn comments_that_tools_read_are_code() {
        let go = "package a\n\nimport \"C\"\n\nfunc f() {}\n";
        assert!(!only_comments("a.go", go, "@@ -1,0 +2,1 @@\n+// #include <stdio.h>\n"), "cgo 앞 주석");
        assert!(!only_comments("a.go", go, "@@ -4,0 +5,1 @@\n+//export f\n"), "cgo 내보내기");
        let ts = "export const a = 1;\n";
        assert!(!only_comments("a.ts", ts, "@@ -0,0 +1,1 @@\n+/// <reference types=\"node\" />\n"));
        let lazy = "export const a = f();\n";
        assert!(!only_comments("a.ts", lazy, &replace_line(lazy, 1, "export const a = /*#__PURE__*/ f();")));
        assert!(!only_comments("a.ts", lazy, &replace_line(lazy, 1, "export const a = /* webpackChunkName: \"b\" */ f();")));
        let py = "x = 1\n";
        assert!(!only_comments("a.py", py, "@@ -0,0 +1,1 @@\n+# -*- coding: latin-1 -*-\n"));
    }

    #[test]
    fn unsure_means_false() {
        let go = "package a\n\nfunc f() {\n}\n";
        assert!(!only_comments("a.rb", "x = 1\n", "@@ -1,0 +2,1 @@\n+# 설명\n"), "분석하지 않는 언어");
        assert!(!only_comments("a.go", go, "@@ -9,1 +9,1 @@\n-없는 줄\n+// 주석\n"), "문맥이 맞지 않음");
        assert!(!only_comments("a.go", "func {{{\n", "@@ -1,0 +2,1 @@\n+// 주석\n"), "구문 오류");
        assert!(!only_comments("a.go", go, "@@ -1,0 +2,1 @@\n+//go:build linux\n"), "빌드 제약");
        assert!(!only_comments("a.ts", "const a = 1;\n", "@@ -0,0 +1,1 @@\n+// @ts-ignore\n"), "타입 검사 끄기");
    }

    #[test]
    fn applies_hunks_and_keeps_line_endings_out_of_the_comparison() {
        let crlf = "package a\r\n\r\nfunc f() {\r\n}\r\n";
        assert!(only_comments("a.go", crlf, "@@ -3,1 +3,1 @@\n-func f() {\r\n+func f() { // 설명\r\n"));
        assert_eq!(apply("a\nb\nc\n", "--- a/x\n+++ b/x\n@@ -1,2 +1,2 @@\n a\n-b\n+B\n"), Some("a\nB\nc\n".to_string()));
        assert_eq!(apply("a\nb\n", "@@ -0,0 +1,1 @@\n+z\n"), Some("z\na\nb\n".to_string()), "맨 앞에 넣기");
        assert_eq!(apply("a\nb\n", "@@ -1,1 +1,1 @@\n-x\n+y\n"), None);
    }
}
