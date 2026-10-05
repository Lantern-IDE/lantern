//! 웹 문서 가져오기 (에이전트의 fetch_url 도구): 라이브러리 문서, 오류 메시지 설명 등을 글로 읽는다.
//!
//! 사설·로컬 주소는 막는다 (주소를 미리 풀어서 확인하고, 그 주소로만 접속해 DNS를 바꿔치기해도 못 들어가게).
//! 리다이렉트도 한 번씩 같은 확인을 거친다. 크기·시간을 제한하고 HTML은 본문 글로 바꾼다.

use anyhow::{bail, Context, Result};
use futures_util::StreamExt;
use reqwest::Url;
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

const MAX_BYTES: usize = 2 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(20);
const MAX_REDIRECTS: usize = 5;
/// HTML을 글로 바꿀 때 한 줄 폭
const TEXT_WIDTH: usize = 100;

pub struct Fetched {
    pub url: String,
    pub text: String,
}

/// 인터넷의 공개 주소가 아니면 true (루프백·사설·링크 로컬·CGNAT·문서용 등)
pub fn blocked_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => {
            let o = v.octets();
            v.is_loopback()
                || v.is_private()
                || v.is_link_local()
                || v.is_unspecified()
                || v.is_multicast()
                || v.is_broadcast()
                || v.is_documentation()
                || o[0] == 0
                || (o[0] == 100 && (o[1] & 0xc0) == 64) // 100.64.0.0/10
                || (o[0] == 192 && o[1] == 0 && o[2] == 0) // 192.0.0.0/24
                || o[0] >= 240
        }
        IpAddr::V6(v) => {
            if let Some(v4) = v.to_ipv4_mapped() {
                return blocked_ip(IpAddr::V4(v4));
            }
            let s = v.segments();
            v.is_loopback()
                || v.is_unspecified()
                || v.is_multicast()
                || (s[0] & 0xfe00) == 0xfc00 // 고유 로컬
                || (s[0] & 0xffc0) == 0xfe80 // 링크 로컬
                || (s[0] == 0x2001 && s[1] == 0x0db8) // 문서용
        }
    }
}

/// 테스트(E2E)에서만 로컬 서버를 허용한다
fn allow_local() -> bool {
    std::env::var("LANTERN_FETCH_ALLOW_LOCAL").is_ok_and(|v| v == "1")
}

/// 주소를 풀어 공개 주소 하나를 고른다
async fn resolve(url: &Url) -> Result<SocketAddr> {
    let host = url.host_str().context("주소에 호스트가 없습니다")?;
    let port = url.port_or_known_default().unwrap_or(443);
    let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host, port)).await.with_context(|| format!("{host}의 주소를 찾지 못했습니다"))?.collect();
    let first = *addrs.first().with_context(|| format!("{host}의 주소를 찾지 못했습니다"))?;
    if !allow_local() && addrs.iter().any(|a| blocked_ip(a.ip())) {
        bail!("{host}는 내부망·로컬 주소라 가져오지 않습니다");
    }
    Ok(first)
}

pub fn check_url(url: &str) -> Result<Url> {
    let u = Url::parse(url.trim()).context("올바른 주소가 아닙니다")?;
    if !matches!(u.scheme(), "http" | "https") {
        bail!("http·https 주소만 가져옵니다");
    }
    if !u.username().is_empty() || u.password().is_some() {
        bail!("주소에 계정 정보를 넣을 수 없습니다");
    }
    u.host_str().context("주소에 호스트가 없습니다")?;
    Ok(u)
}

pub async fn fetch(url: &str) -> Result<Fetched> {
    let mut url = check_url(url)?;
    for _ in 0..=MAX_REDIRECTS {
        let addr = resolve(&url).await?;
        let host = url.host_str().unwrap_or_default().to_string();
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(TIMEOUT)
            .resolve(&host, addr)
            .user_agent(concat!("Lantern-IDE/", env!("CARGO_PKG_VERSION")))
            .build()?;
        let resp = client.get(url.clone()).header("Accept", "text/html, text/plain, text/markdown, application/json;q=0.9, */*;q=0.5").send().await.with_context(|| format!("{host}에 연결하지 못했습니다"))?;
        let status = resp.status();
        if status.is_redirection() {
            let loc = resp.headers().get(reqwest::header::LOCATION).and_then(|v| v.to_str().ok()).context("리다이렉트 주소가 없습니다")?;
            url = check_url(url.join(loc)?.as_str())?;
            continue;
        }
        if !status.is_success() {
            bail!("{url} 응답 {status}");
        }
        let ctype = resp.headers().get(reqwest::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("").to_ascii_lowercase();
        let mut body = Vec::new();
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            body.extend_from_slice(&chunk[..chunk.len().min(MAX_BYTES - body.len())]);
            if body.len() >= MAX_BYTES {
                break;
            }
        }
        let text = to_text(&ctype, &body)?;
        return Ok(Fetched { url: url.to_string(), text });
    }
    bail!("리다이렉트가 너무 많습니다")
}

/// 응답 본문을 모델에 줄 글로
pub fn to_text(ctype: &str, body: &[u8]) -> Result<String> {
    let looks_html = ctype.contains("html") || (ctype.is_empty() && String::from_utf8_lossy(&body[..body.len().min(512)]).to_ascii_lowercase().contains("<html"));
    if looks_html {
        return html2text::config::plain().string_from_read(body, TEXT_WIDTH).context("HTML을 글로 바꾸지 못했습니다");
    }
    if ctype.is_empty() || ctype.starts_with("text/") || ctype.contains("json") || ctype.contains("xml") || ctype.contains("javascript") {
        return Ok(String::from_utf8_lossy(body).into_owned());
    }
    bail!("글이 아닌 문서라 읽지 않습니다 ({ctype})")
}

/// 이 호스트를 승인 없이 열어도 되는지 (설정의 허용 목록: 도메인과 그 하위 도메인)
pub fn host_allowed(host: &str, allowed: &[String]) -> bool {
    let host = host.to_ascii_lowercase();
    allowed.iter().map(|d| d.trim().trim_start_matches("*.").to_ascii_lowercase()).any(|d| !d.is_empty() && (host == d || host.ends_with(&format!(".{d}"))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_internal_addresses() {
        for ip in ["127.0.0.1", "10.0.0.5", "192.168.1.1", "172.16.0.1", "169.254.169.254", "100.64.0.1", "0.0.0.0", "::1", "fd00::1", "fe80::1", "::ffff:127.0.0.1"] {
            assert!(blocked_ip(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["93.184.216.34", "1.1.1.1", "2606:4700:4700::1111"] {
            assert!(!blocked_ip(ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn checks_urls_and_hosts() {
        assert!(check_url("https://docs.rs/serde").is_ok());
        assert!(check_url("file:///etc/passwd").is_err());
        assert!(check_url("ftp://x.org").is_err());
        assert!(check_url("https://user:pw@x.org").is_err());
        let allowed = vec!["docs.rs".to_string(), "*.mozilla.org".to_string()];
        assert!(host_allowed("docs.rs", &allowed) && host_allowed("developer.mozilla.org", &allowed));
        assert!(!host_allowed("evil-docs.rs", &allowed) && !host_allowed("example.com", &allowed));
    }

    #[test]
    fn turns_html_into_text() {
        let t = to_text("text/html; charset=utf-8", b"<html><head><style>x{}</style><script>bad()</script></head><body><h1>Title</h1><p>Hello <b>world</b></p></body></html>").unwrap();
        assert!(t.contains("Title") && t.contains("Hello") && t.contains("world"));
        assert!(!t.contains("bad()") && !t.contains("x{}"));
        assert_eq!(to_text("application/json", b"{\"a\":1}").unwrap(), "{\"a\":1}");
        assert!(to_text("image/png", b"\x89PNG").is_err());
    }
}
