//! 확장자 → 언어 판별과 언어별 tree-sitter 태그 설정.

use anyhow::{Context, Result};
use std::cell::OnceCell;
use std::path::Path;
use tree_sitter_tags::TagsConfiguration;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Lang {
    Rust,
    Python,
    TypeScript,
    Tsx,
    JavaScript,
    Java,
    Go,
    CSharp,
    Kotlin,
    C,
    Cpp,
}

impl Lang {
    const COUNT: usize = 11;

    pub fn from_path(path: &Path) -> Option<Lang> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        Some(match ext.as_str() {
            "rs" => Lang::Rust,
            "py" | "pyi" => Lang::Python,
            "ts" | "mts" | "cts" => Lang::TypeScript,
            "tsx" => Lang::Tsx,
            "js" | "mjs" | "cjs" | "jsx" => Lang::JavaScript,
            "java" => Lang::Java,
            "go" => Lang::Go,
            "cs" => Lang::CSharp,
            "kt" | "kts" => Lang::Kotlin,
            "c" => Lang::C,
            // 헤더는 C++ 문법으로 읽는다 (C도 대부분 읽힌다)
            "h" | "cpp" | "cc" | "cxx" | "hpp" | "hh" | "hxx" | "ipp" => Lang::Cpp,
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
            Lang::Java => "java",
            Lang::Go => "go",
            Lang::CSharp => "csharp",
            Lang::Kotlin => "kotlin",
            Lang::C => "c",
            Lang::Cpp => "cpp",
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
            "java" => "java",
            "go" => "go",
            "csharp" => "csharp",
            "kotlin" => "kotlin",
            "c" => "c",
            "cpp" => "cpp",
            _ => "",
        }
    }

    /// 서로 호출할 수 있는 언어끼리 같은 값. 호출 관계는 이름으로 잇기 때문에,
    /// Spring 백엔드의 `getUser`와 React 화면의 `getUser`가 이어지지 않게 계열로 나눈다.
    pub fn family(name: &str) -> &str {
        match name {
            "typescript" | "tsx" | "javascript" => "js",
            "c" | "cpp" => "c",
            other => other,
        }
    }
}

/// SQL에서 언어 열(`col`)을 [`Lang::family`]와 같은 값으로 바꾸는 식
pub fn family_sql(col: &str) -> String {
    format!("(CASE WHEN {col} IN ('typescript', 'tsx', 'javascript') THEN 'js' WHEN {col} IN ('c', 'cpp') THEN 'c' ELSE {col} END)")
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

/// 문법 크레이트의 질의에 없는 정의: 생성자, enum, record, 애너테이션 타입
const JAVA_EXTRA: &str = r#"
(constructor_declaration name: (identifier) @name) @definition.method
(enum_declaration name: (identifier) @name) @definition.enum
(record_declaration name: (identifier) @name) @definition.class
(annotation_type_declaration name: (identifier) @name) @definition.interface
"#;

/// 문법 크레이트의 질의는 `obj.Foo()`만 잡고 `Foo()`는 놓친다. struct·enum·record·생성자도 없다.
const CSHARP_EXTRA: &str = r#"
(invocation_expression function: (identifier) @name) @reference.call
(struct_declaration name: (identifier) @name) @definition.class
(enum_declaration name: (identifier) @name) @definition.enum
(record_declaration name: (identifier) @name) @definition.class
(constructor_declaration name: (identifier) @name) @definition.method
"#;

/// Kotlin 문법 크레이트에는 태그 질의가 없어 직접 정의한다.
/// 클래스 본문 안의 함수는 메서드로 (먼저 맞는 패턴이 이긴다), 호출은 `foo()`와 `obj.foo()`의 `foo`.
const KOTLIN_TAGS: &str = r#"
(class_declaration name: (identifier) @name) @definition.class
(object_declaration name: (identifier) @name) @definition.class
(companion_object name: (identifier) @name) @definition.class
(class_body (function_declaration name: (identifier) @name) @definition.method)
(function_declaration name: (identifier) @name) @definition.function
(type_alias type: (identifier) @name) @definition.type
(call_expression (identifier) @name) @reference.call
(call_expression (navigation_expression (_) . (identifier) @name .)) @reference.call
"#;

/// C·C++ 문법 크레이트의 질의는 함수 이름(선언자)만 잡아 본문이 심볼 범위에 들지 않고, 호출도 없다.
/// 본문까지 잡는 정의를 앞에 두어 먼저 맞게 하고(같은 이름 노드는 앞 패턴이 이긴다), 호출을 더한다.
const C_EXTRA: &str = r#"
(function_definition declarator: (function_declarator declarator: (identifier) @name)) @definition.function
(function_definition declarator: (pointer_declarator declarator: (function_declarator declarator: (identifier) @name))) @definition.function
(call_expression function: (identifier) @name) @reference.call
(call_expression function: (field_expression field: (field_identifier) @name)) @reference.call
"#;

const CPP_EXTRA: &str = r#"
(function_definition declarator: (function_declarator declarator: (identifier) @name)) @definition.function
(function_definition declarator: (function_declarator declarator: (field_identifier) @name)) @definition.method
(function_definition declarator: (function_declarator declarator: (qualified_identifier name: (identifier) @name))) @definition.method
(function_definition declarator: (pointer_declarator declarator: (function_declarator declarator: (identifier) @name))) @definition.function
(function_definition declarator: (reference_declarator (function_declarator declarator: (identifier) @name))) @definition.function
(call_expression function: (identifier) @name) @reference.call
(call_expression function: (field_expression field: (field_identifier) @name)) @reference.call
(call_expression function: (qualified_identifier name: (identifier) @name)) @reference.call
(call_expression function: (template_function name: (identifier) @name)) @reference.call
(new_expression type: (type_identifier) @name) @reference.class
"#;

/// 언어별 태그 설정. 질의를 만드는 데 시간이 들어 (11개 언어 400ms쯤) 그 언어를 처음 읽을 때 만든다.
#[derive(Default)]
pub struct TagConfigs {
    cells: [OnceCell<TagsConfiguration>; Lang::COUNT],
}

fn build_config(lang: Lang) -> Result<TagsConfiguration> {
    // TypeScript 문법은 JavaScript의 상위 집합이라 JS 태그 질의를 함께 쓴다.
    let ts_q = || format!("{}\n{}\n{}", tree_sitter_javascript::TAGS_QUERY, tree_sitter_typescript::TAGS_QUERY, TS_EXTRA);
    let built = match lang {
        Lang::Rust => TagsConfiguration::new(tree_sitter_rust::LANGUAGE.into(), RUST_TAGS, ""),
        Lang::Python => TagsConfiguration::new(tree_sitter_python::LANGUAGE.into(), tree_sitter_python::TAGS_QUERY, ""),
        Lang::TypeScript => TagsConfiguration::new(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(), &ts_q(), ""),
        Lang::Tsx => TagsConfiguration::new(tree_sitter_typescript::LANGUAGE_TSX.into(), &ts_q(), ""),
        Lang::JavaScript => TagsConfiguration::new(tree_sitter_javascript::LANGUAGE.into(), tree_sitter_javascript::TAGS_QUERY, ""),
        Lang::Java => TagsConfiguration::new(tree_sitter_java::LANGUAGE.into(), &format!("{}\n{}", tree_sitter_java::TAGS_QUERY, JAVA_EXTRA), ""),
        Lang::Go => TagsConfiguration::new(tree_sitter_go::LANGUAGE.into(), tree_sitter_go::TAGS_QUERY, ""),
        // 원본 질의의 `@module` 캡처는 tree-sitter-tags가 받지 않아 그 줄만 뺀다
        Lang::CSharp => TagsConfiguration::new(
            tree_sitter_c_sharp::LANGUAGE.into(),
            &format!(
                "{}\n{}",
                tree_sitter_c_sharp::TAGS_QUERY.lines().filter(|l| !l.trim_end().ends_with("@module")).collect::<Vec<_>>().join("\n"),
                CSHARP_EXTRA
            ),
            "",
        ),
        Lang::Kotlin => TagsConfiguration::new(tree_sitter_kotlin_ng::LANGUAGE.into(), KOTLIN_TAGS, ""),
        Lang::C => TagsConfiguration::new(tree_sitter_c::LANGUAGE.into(), &format!("{C_EXTRA}\n{}", tree_sitter_c::TAGS_QUERY), ""),
        Lang::Cpp => TagsConfiguration::new(tree_sitter_cpp::LANGUAGE.into(), &format!("{CPP_EXTRA}\n{}", tree_sitter_cpp::TAGS_QUERY), ""),
    };
    built.with_context(|| format!("{} 태그 질의", lang.name()))
}

impl TagConfigs {
    pub fn new() -> Result<Self> {
        Ok(Self::default())
    }

    pub fn get(&self, lang: Lang) -> Result<&TagsConfiguration> {
        let cell = &self.cells[lang as usize];
        if let Some(cfg) = cell.get() {
            return Ok(cfg);
        }
        let cfg = build_config(lang)?;
        Ok(cell.get_or_init(|| cfg))
    }
}
