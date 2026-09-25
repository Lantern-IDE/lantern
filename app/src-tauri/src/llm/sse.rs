//! Server-Sent Events 파서. 바이트 청크를 받아 완성된 이벤트만 돌려준다.

#[derive(Debug, Default, PartialEq)]
pub struct SseEvent {
    pub event: Option<String>,
    pub data: String,
}

#[derive(Default)]
pub struct SseParser {
    buf: Vec<u8>,
}

impl SseParser {
    pub fn push(&mut self, chunk: &[u8]) -> Vec<SseEvent> {
        self.buf.extend_from_slice(chunk);
        let mut out = Vec::new();
        while let Some((end, sep_len)) = find_boundary(&self.buf) {
            let raw: Vec<u8> = self.buf.drain(..end + sep_len).take(end).collect();
            let text = String::from_utf8_lossy(&raw);
            let mut ev = SseEvent::default();
            let mut data_lines = Vec::new();
            for line in text.lines() {
                if let Some(v) = line.strip_prefix("event:") {
                    ev.event = Some(v.trim().to_string());
                } else if let Some(v) = line.strip_prefix("data:") {
                    data_lines.push(v.strip_prefix(' ').unwrap_or(v).to_string());
                }
            }
            if !data_lines.is_empty() || ev.event.is_some() {
                ev.data = data_lines.join("\n");
                out.push(ev);
            }
        }
        out
    }
}

/// 빈 줄(`\n\n` 또는 `\r\n\r\n`)의 위치와 길이
fn find_boundary(buf: &[u8]) -> Option<(usize, usize)> {
    let lf = buf.windows(2).position(|w| w == b"\n\n").map(|p| (p, 2));
    let crlf = buf.windows(4).position(|w| w == b"\r\n\r\n").map(|p| (p, 4));
    match (lf, crlf) {
        (Some(a), Some(b)) => Some(if a.0 <= b.0 { a } else { b }),
        (a, b) => a.or(b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_split_chunks() {
        let mut p = SseParser::default();
        assert!(p.push(b"event: message_start\ndata: {\"a\"").is_empty());
        let evs = p.push(b":1}\n\ndata: [DONE]\r\n\r\n");
        assert_eq!(evs.len(), 2);
        assert_eq!(evs[0].event.as_deref(), Some("message_start"));
        assert_eq!(evs[0].data, "{\"a\":1}");
        assert_eq!(evs[1].data, "[DONE]");
    }
}
