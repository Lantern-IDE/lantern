//! 비밀 정보 보호: 모델로 보내는 텍스트에서 키·토큰을 가리고, 비밀 파일을 알아본다.
//!
//! 완벽한 탐지기가 아니라 흔한 실수(코드에 박힌 키, .env 파일)를 막는 안전망이다.

use regex::Regex;
use std::sync::OnceLock;

pub const MASK: &str = "«가려진 비밀»";

/// 이름만 보고 비밀 파일로 판단한다. 에이전트가 읽으려 하면 사용자 승인을 받는다.
pub fn is_secret_path(path: &str) -> bool {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path).to_ascii_lowercase();
    if name == ".env" || (name.starts_with(".env.") && !name.ends_with(".example") && !name.ends_with(".sample") && !name.ends_with(".template")) {
        return true;
    }
    const EXACT: &[&str] = &[
        "id_rsa", "id_dsa", "id_ecdsa", "id_ed25519", ".npmrc", ".pypirc", ".netrc", "credentials",
        "credentials.json", "service-account.json", "secrets.json", "secrets.yaml", "secrets.yml", ".git-credentials",
    ];
    const EXT: &[&str] = &[".pem", ".key", ".p12", ".pfx", ".jks", ".keystore", ".kdbx"];
    EXACT.contains(&name.as_str()) || EXT.iter().any(|e| name.ends_with(e))
}

fn patterns() -> &'static [Regex] {
    static P: OnceLock<Vec<Regex>> = OnceLock::new();
    P.get_or_init(|| {
        [
            // 개인 키 블록 전체
            r"-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----",
            // 널리 쓰이는 키 접두사
            r"\bsk-ant-[A-Za-z0-9_\-]{20,}",
            r"\bsk-(?:proj-)?[A-Za-z0-9_\-]{20,}",
            r"\bAKIA[0-9A-Z]{16}\b",
            r"\bgh[pousr]_[A-Za-z0-9]{30,}\b",
            r"\bgithub_pat_[A-Za-z0-9_]{30,}\b",
            r"\bxox[abposr]-[A-Za-z0-9-]{10,}\b",
            r"\bAIza[0-9A-Za-z_\-]{35}\b",
            r"\beyJ[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{10,}\b",
        ]
        .iter()
        .map(|p| Regex::new(p).expect("비밀 패턴"))
        .collect()
    })
}

fn assignment() -> &'static Regex {
    static A: OnceLock<Regex> = OnceLock::new();
    // password = "...", API_KEY: '...', secret="..." 같은 대입의 값 부분
    A.get_or_init(|| {
        Regex::new(r#"(?i)((?:password|passwd|pwd|secret|api[_-]?key|access[_-]?key|auth[_-]?token|client[_-]?secret|private[_-]?key)["']?\s*[:=]\s*["'])([^"'\s]{8,})(["'])"#)
            .expect("대입 패턴")
    })
}

/// 텍스트 안의 비밀 값을 가리고, 가린 개수를 돌려준다.
pub fn redact(text: &str) -> (String, usize) {
    let mut out = text.to_string();
    let mut count = 0;
    for re in patterns() {
        let n = re.find_iter(&out).count();
        if n > 0 {
            count += n;
            out = re.replace_all(&out, MASK).into_owned();
        }
    }
    let n = assignment().captures_iter(&out).filter(|c| !c[2].contains("process.env") && !c[2].starts_with("${")).count();
    if n > 0 {
        count += n;
        out = assignment()
            .replace_all(&out, |c: &regex::Captures| {
                if c[2].contains("process.env") || c[2].starts_with("${") {
                    c[0].to_string()
                } else {
                    format!("{}{}{}", &c[1], MASK, &c[3])
                }
            })
            .into_owned();
    }
    (out, count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_secret_files() {
        for p in [".env", "app/.env.local", "certs/server.pem", "id_rsa", "deploy/key.p12", "~/.npmrc"] {
            assert!(is_secret_path(p), "{p}");
        }
        for p in [".env.example", "src/env.ts", "README.md", "keys.rs", "public.pub"] {
            assert!(!is_secret_path(p), "{p}");
        }
    }

    #[test]
    fn redacts_common_secrets() {
        let src = r#"
const key = "sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123";
aws = "AKIAABCDEFGHIJKLMNOP"
password = "hunter2hunter2"
token: process.env.TOKEN
const safe = "hello world";
-----BEGIN RSA PRIVATE KEY-----
MIIEowIBAAKCAQEA
-----END RSA PRIVATE KEY-----
"#;
        let (out, n) = redact(src);
        assert_eq!(n, 4, "{out}");
        assert!(!out.contains("sk-ant-api03"));
        assert!(!out.contains("AKIAABCDEFGHIJKLMNOP"));
        assert!(!out.contains("hunter2"));
        assert!(!out.contains("MIIEow"));
        assert!(out.contains("process.env.TOKEN"));
        assert!(out.contains("hello world"));
        assert!(out.contains("password = \"«가려진 비밀»\""));
    }
}
