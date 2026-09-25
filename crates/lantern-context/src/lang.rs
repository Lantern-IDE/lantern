//! 확장자 → 언어 판별과 언어별 tree-sitter 태그 설정.

use anyhow::{Context, Result};
use std::path::Path;
use tree_sitter_tags::TagsConfiguration;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Lang {
    Rust,
    Python,
    TypeScript,
    Tsx,
    JavaScript,
}

impl Lang {
    pub fn from_path(path: &Path) -> Option<Lang> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        Some(match ext.as_str() {
            "rs" => Lang::Rust,
            "py" | "pyi" => Lang::Python,
            "ts" | "mts" | "cts" => Lang::TypeScript,
            "tsx" => Lang::Tsx,
            "js" | "mjs" | "cjs" | "jsx" => Lang::JavaScript,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Lang::Rust => "rust",
            Lang::Python => "python",
            Lang::TypeScript => "typescript",
            Lang::Tsx => "tsx",
            Lang::JavaScript => "javascript",
        }
    }

    /// Markdown 코드 펜스에 쓰는 언어 표기
    pub fn fence(name: &str) -> &'static str {
        match name {
            "rust" => "rust",
            "python" => "python",
            "typescript" => "ts",
            "tsx" => "tsx",
            "javascript" => "js",
            _ => "",
        }
    }
}

/// Rust는 문법 크레이트의 tags.scm을 쓰지 않고 직접 정의한다.
/// tree-sitter-tags는 이름 노드 하나에 태그 하나만 만드는데, 원본 질의는 `impl Foo`의 `Foo`를
/// 참조로 잡아 버려 impl 블록을 정의로 얻을 수 없기 때문이다.
const RUST_TAGS: &str = r#"
(struct_item name: (type_identifier) @name) @definition.class
(enum_item name: (type_identifier) @name) @definition.class
(union_item name: (type_identifier) @name) @definition.class
(type_item name: (type_identifier) @name) @definition.class
(declaration_list (function_item name: (identifier) @name) @definition.method)
(function_item name: (identifier) @name) @definition.function
(function_signature_item name: (identifier) @name) @definition.method
(trait_item name: (type_identifier) @name) @definition.interface
(mod_item name: (identifier) @name) @definition.module
(macro_definition name: (identifier) @name) @definition.macro
(const_item name: (identifier) @name) @definition.constant
(static_item name: (identifier) @name) @definition.constant
(impl_item type: (type_identifier) @name) @definition.impl
(impl_item type: (generic_type type: (type_identifier) @name)) @definition.impl

(impl_item trait: (type_identifier) @name) @reference.implementation
(call_expression function: (identifier) @name) @reference.call
(call_expression function: (field_expression field: (field_identifier) @name)) @reference.call
(call_expression function: (scoped_identifier name: (identifier) @name)) @reference.call
(macro_invocation macro: (identifier) @name) @reference.call
(struct_expression name: (type_identifier) @name) @reference.class
"#;

const TS_EXTRA: &str = r#"
(type_alias_declaration name: (type_identifier) @name) @definition.type
(enum_declaration name: (identifier) @name) @definition.enum
"#;

pub struct TagConfigs {
    rust: TagsConfiguration,
    python: TagsConfiguration,
    typescript: TagsConfiguration,
    tsx: TagsConfiguration,
    javascript: TagsConfiguration,
}

impl TagConfigs {
    pub fn new() -> Result<Self> {
        // TypeScript 문법은 JavaScript의 상위 집합이라 JS 태그 질의를 함께 쓴다.
        let ts_q = format!(
            "{}\n{}\n{}",
            tree_sitter_javascript::TAGS_QUERY,
            tree_sitter_typescript::TAGS_QUERY,
            TS_EXTRA
        );
        Ok(Self {
            rust: TagsConfiguration::new(tree_sitter_rust::LANGUAGE.into(), RUST_TAGS, "")
                .context("rust 태그 질의")?,
            python: TagsConfiguration::new(
                tree_sitter_python::LANGUAGE.into(),
                tree_sitter_python::TAGS_QUERY,
                "",
            )
            .context("python 태그 질의")?,
            typescript: TagsConfiguration::new(
                tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
                &ts_q,
                "",
            )
            .context("typescript 태그 질의")?,
            tsx: TagsConfiguration::new(tree_sitter_typescript::LANGUAGE_TSX.into(), &ts_q, "")
                .context("tsx 태그 질의")?,
            javascript: TagsConfiguration::new(
                tree_sitter_javascript::LANGUAGE.into(),
                tree_sitter_javascript::TAGS_QUERY,
                "",
            )
            .context("javascript 태그 질의")?,
        })
    }

    pub fn get(&self, lang: Lang) -> &TagsConfiguration {
        match lang {
            Lang::Rust => &self.rust,
            Lang::Python => &self.python,
            Lang::TypeScript => &self.typescript,
            Lang::Tsx => &self.tsx,
            Lang::JavaScript => &self.javascript,
        }
    }
}
