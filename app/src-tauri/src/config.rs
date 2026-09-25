//! 설정: 기본값 ← `~/.lantern/config.toml` ← `<프로젝트>/.lantern/config.toml` 순으로 덮어쓴다.
//!
//! 모든 동작은 사람이 읽고 고칠 수 있는 TOML 파일로 정의한다 (기획서 원칙 03).

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// 프로젝트에 설정 파일이 없을 때 `.lantern/config.toml`로 만들어 주는 템플릿.
/// 이 문자열이 곧 기본값이기도 하다.
pub const DEFAULT_CONFIG: &str = r#"# Lantern 설정. 이 파일을 고치면 다음 요청부터 바로 반영됩니다.
# 전역 설정은 ~/.lantern/config.toml 에 두고, 여기서는 이 프로젝트에만 필요한 값만 덮어써도 됩니다.
# API 키는 파일에 적지 말고 환경변수 이름(api_key_env)으로 지정하세요.

# ── 모델 ─────────────────────────────────────────────
# provider = "anthropic" : Anthropic Messages API
# provider = "openai"    : OpenAI 호환 API (OpenAI, Ollama, LM Studio, vLLM, OpenRouter 등)
# price_input / price_output : 100만 토큰당 USD. 비용 미터에 쓰입니다 (없으면 토큰만 표시).

[models.smart]
provider = "anthropic"
model = "claude-opus-5"
api_key_env = "ANTHROPIC_API_KEY"
max_tokens = 32000
effort = "high"          # low | medium | high | xhigh | max
fallbacks = "default"    # 안전 분류기가 요청을 거절하면 서버가 다른 모델로 자동 재시도
price_input = 5.0
price_output = 25.0

[models.local]
provider = "openai"
base_url = "http://localhost:11434/v1"   # Ollama
model = "qwen2.5-coder:7b"
max_tokens = 8192
price_input = 0.0
price_output = 0.0

# 에이전트 정의 파일(.lantern/agents/*.md)에 model 이 없을 때 쓰는 모델
[routing]
default = "smart"
completion = ""   # 인라인 자동 완성(고스트 텍스트)에 쓸 모델. 예: "local". 비워 두면 끔

# ── 비용 한도 ─────────────────────────────────────────
[budget]
monthly_usd_limit = 20.0   # 넘으면 요청을 막습니다. 0 이면 제한 없음
warn_at_percent = 80

# ── 맥락 엔진 ─────────────────────────────────────────
[context]
budget_tokens = 8000       # 질문마다 자동으로 붙이는 코드 맥락의 토큰 예산

# ── 에이전트 권한 ─────────────────────────────────────
[agent]
max_steps = 30
# 승인 없이 수정해도 되는 파일 (glob). 예: ["**/*.test.ts", "docs/**"]
auto_approve = []
# 승인 없이 실행해도 되는 명령 (앞부분 일치)
allowed_commands = ["git status", "git diff", "git log", "cargo check", "cargo test", "npm test", "npm run lint"]

# ── 훅 ───────────────────────────────────────────────
[hooks]
on_save = []          # 파일 저장 후 실행. 예: ["npx prettier --check ."]
on_agent_done = []    # 에이전트가 파일을 바꾸고 끝났을 때 실행. 예: ["cargo check"]

# ── 언어 서버 ─────────────────────────────────────────
# 설치되어 있지 않으면 조용히 건너뜁니다.
[lsp.rust]
command = "rust-analyzer"
args = []

[lsp.typescript]
command = "typescript-language-server"
args = ["--stdio"]

[lsp.javascript]
command = "typescript-language-server"
args = ["--stdio"]

[lsp.python]
command = "pyright-langserver"
args = ["--stdio"]
"#;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    pub provider: String,
    pub model: String,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub api_key_env: Option<String>,
    /// 가능하면 쓰지 말 것. 전역 설정 파일에만 두는 용도.
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    #[serde(default)]
    pub effort: Option<String>,
    #[serde(default)]
    pub fallbacks: Option<String>,
    #[serde(default)]
    pub price_input: Option<f64>,
    #[serde(default)]
    pub price_output: Option<f64>,
}

fn default_max_tokens() -> u32 {
    16000
}

/// OS 자격 증명 저장소(Windows 자격 증명 관리자 등)에서 쓰는 서비스 이름
pub const KEYRING_SERVICE: &str = "lantern";

impl ModelConfig {
    /// API 키 찾는 순서: 환경변수 → OS 자격 증명 저장소 → 설정 파일의 api_key
    pub fn resolve_api_key(&self) -> Option<String> {
        self.api_key_with_source().map(|(k, _)| k)
    }

    pub fn api_key_with_source(&self) -> Option<(String, &'static str)> {
        if let Some(env) = &self.api_key_env {
            if let Ok(v) = std::env::var(env) {
                if !v.trim().is_empty() {
                    return Some((v.trim().to_string(), "env"));
                }
            }
            if let Some(k) = keyring_get(env) {
                return Some((k, "keychain"));
            }
        }
        self.api_key.clone().filter(|k| !k.trim().is_empty()).map(|k| (k, "config"))
    }
}

/// 예전 이름(KHALA) 시절에 저장한 키도 읽는다
const LEGACY_KEYRING_SERVICE: &str = "khala";

pub fn keyring_get(name: &str) -> Option<String> {
    let read = |service: &str| {
        keyring::Entry::new(service, name).ok()?.get_password().ok().filter(|k| !k.trim().is_empty())
    };
    read(KEYRING_SERVICE).or_else(|| {
        let old = read(LEGACY_KEYRING_SERVICE)?;
        // 새 이름으로 옮겨 둔다 (실패해도 예전 자리에서 계속 읽힌다)
        if let Ok(e) = keyring::Entry::new(KEYRING_SERVICE, name) {
            let _ = e.set_password(&old);
        }
        Some(old)
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Routing {
    #[serde(default)]
    pub default: String,
    /// 인라인 자동 완성에 쓸 모델 키. 비어 있으면 끔
    #[serde(default)]
    pub completion: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Budget {
    #[serde(default)]
    pub monthly_usd_limit: f64,
    #[serde(default)]
    pub warn_at_percent: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextConfig {
    #[serde(default = "default_ctx_budget")]
    pub budget_tokens: usize,
}

fn default_ctx_budget() -> usize {
    8000
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentConfig {
    #[serde(default)]
    pub max_steps: usize,
    #[serde(default)]
    pub auto_approve: Vec<String>,
    #[serde(default)]
    pub allowed_commands: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Hooks {
    #[serde(default)]
    pub on_save: Vec<String>,
    #[serde(default)]
    pub on_agent_done: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspConfig {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub models: BTreeMap<String, ModelConfig>,
    #[serde(default)]
    pub routing: Routing,
    #[serde(default)]
    pub budget: Budget,
    pub context: ContextConfig,
    #[serde(default)]
    pub agent: AgentConfig,
    #[serde(default)]
    pub hooks: Hooks,
    #[serde(default)]
    pub lsp: BTreeMap<String, LspConfig>,
}

impl Config {
    pub fn model(&self, key: &str) -> Result<(&str, &ModelConfig)> {
        let key = if key.is_empty() { self.routing.default.as_str() } else { key };
        self.models
            .get_key_value(key)
            .map(|(k, v)| (k.as_str(), v))
            .with_context(|| format!("config.toml에 [models.{key}]가 없습니다"))
    }
}

/// 사용자 전체 설정 파일. `LANTERN_HOME`을 주면 `~/.lantern` 대신 그 폴더를 쓴다 (테스트 격리, 휴대용 설치).
pub fn global_path() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("LANTERN_HOME").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(d).join("config.toml"));
    }
    dirs::home_dir().map(|h| h.join(".lantern").join("config.toml"))
}

pub fn project_path(root: &Path) -> PathBuf {
    root.join(".lantern").join("config.toml")
}

/// 테이블은 재귀적으로 합치고, 나머지 값은 덮어쓴다.
fn merge(base: &mut toml::Value, over: toml::Value) {
    match (base, over) {
        (toml::Value::Table(b), toml::Value::Table(o)) => {
            for (k, v) in o {
                match b.get_mut(&k) {
                    Some(existing) => merge(existing, v),
                    None => {
                        b.insert(k, v);
                    }
                }
            }
        }
        (b, o) => *b = o,
    }
}

fn read_toml(path: &Path) -> Result<Option<toml::Value>> {
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(path)?;
    let v: toml::Value =
        toml::from_str(&text).with_context(|| format!("{} 형식 오류", path.display()))?;
    Ok(Some(v))
}

pub fn load(root: Option<&Path>) -> Result<Config> {
    let mut value: toml::Value = toml::from_str(DEFAULT_CONFIG).expect("기본 설정은 유효한 TOML");
    if let Some(p) = global_path() {
        if let Some(v) = read_toml(&p)? {
            merge(&mut value, v);
        }
    }
    if let Some(root) = root {
        if let Some(v) = read_toml(&project_path(root))? {
            merge(&mut value, v);
        }
    }
    let cfg: Config = value.try_into().context("설정 값 오류")?;
    Ok(cfg)
}

/// 프로젝트 설정 파일과 예시 에이전트가 없으면 만든다. 설정 파일 경로를 돌려준다.
pub fn ensure_project_files(root: &Path) -> Result<PathBuf> {
    let dir = root.join(".lantern");
    std::fs::create_dir_all(dir.join("agents"))?;
    std::fs::create_dir_all(dir.join("memory"))?;
    let path = project_path(root);
    if !path.exists() {
        std::fs::write(&path, DEFAULT_CONFIG)?;
    }
    let reviewer = dir.join("agents").join("reviewer.md");
    if !reviewer.exists() && std::fs::read_dir(dir.join("agents"))?.next().is_none() {
        std::fs::write(&reviewer, crate::agents::EXAMPLE_AGENT)?;
    }
    let conventions = dir.join("memory").join("conventions.md");
    if !conventions.exists() && std::fs::read_dir(dir.join("memory"))?.next().is_none() {
        std::fs::write(
            &conventions,
            "# 코딩 규칙\n\n이 파일의 내용은 AI에게 질문할 때마다 함께 전달됩니다.\n\
             프로젝트의 규칙, 자주 하는 설명, 설계 결정을 적어 두세요.\n",
        )?;
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_parses() {
        let cfg = load(None).unwrap();
        assert_eq!(cfg.routing.default, "smart");
        assert_eq!(cfg.model("").unwrap().1.model, "claude-opus-5");
        assert_eq!(cfg.context.budget_tokens, 8000);
        assert!(cfg.lsp.contains_key("rust"));
    }

    #[test]
    fn project_overrides_merge() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".lantern")).unwrap();
        std::fs::write(
            project_path(dir.path()),
            "[routing]\ndefault = \"local\"\n[models.local]\nmodel = \"llama\"\n",
        )
        .unwrap();
        let cfg = load(Some(dir.path())).unwrap();
        let (key, m) = cfg.model("").unwrap();
        assert_eq!(key, "local");
        assert_eq!(m.model, "llama");
        // 덮어쓰지 않은 필드는 기본값 유지
        assert_eq!(m.provider, "openai");
    }
}
