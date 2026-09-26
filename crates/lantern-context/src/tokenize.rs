//! 식별자 분해와 검색 질의 생성.
//!
//! `parseUserConfig` → `parse user config` 처럼 코드 식별자를 단어로 나눠 두면,
//! 사용자가 자연어로 물어도 코드와 연결된다.

use std::collections::HashSet;

/// 텍스트에서 `[A-Za-z_][A-Za-z0-9_]*` 형태의 식별자를 뽑는다.
pub fn identifiers(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|w| w.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_'))
}

/// 식별자를 소문자 단어로 분해한다. `HTTPServer_v2` → `http`, `server`, `v2`
pub fn split_identifier(ident: &str) -> Vec<String> {
    let chars: Vec<char> = ident.chars().collect();
    let mut parts = Vec::new();
    let mut cur = String::new();
    for i in 0..chars.len() {
        let c = chars[i];
        if !c.is_ascii_alphanumeric() {
            flush(&mut cur, &mut parts);
            continue;
        }
        if let Some(&prev) = i.checked_sub(1).and_then(|p| chars.get(p)) {
            let next = chars.get(i + 1).copied();
            let boundary = (prev.is_ascii_lowercase() && c.is_ascii_uppercase())
                || (prev.is_ascii_uppercase()
                    && c.is_ascii_uppercase()
                    && next.is_some_and(|n| n.is_ascii_lowercase()))
                || (prev.is_ascii_alphabetic() && c.is_ascii_digit() && cur.len() > 1)
                || (prev.is_ascii_digit() && c.is_ascii_uppercase());
            if boundary {
                flush(&mut cur, &mut parts);
            }
        }
        cur.push(c.to_ascii_lowercase());
    }
    flush(&mut cur, &mut parts);
    parts
}

fn flush(cur: &mut String, parts: &mut Vec<String>) {
    if cur.len() >= 2 {
        parts.push(std::mem::take(cur));
    } else {
        cur.clear();
    }
}

/// 인덱싱용: 텍스트의 모든 식별자와 그 분해 단어 (중복 제거)
pub fn terms_of(text: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for ident in identifiers(text) {
        let lower = ident.to_ascii_lowercase();
        let parts = split_identifier(ident);
        if lower.len() >= 2 && seen.insert(lower.clone()) {
            out.push(lower);
        }
        if parts.len() > 1 {
            for p in parts {
                if seen.insert(p.clone()) {
                    out.push(p);
                }
            }
        }
    }
    out
}

const EN_STOP: &[&str] = &[
    "the", "an", "is", "are", "was", "how", "what", "where", "why", "when", "which", "who",
    "does", "do", "did", "in", "of", "to", "and", "or", "for", "with", "this", "that", "these",
    "it", "its", "be", "on", "by", "from", "as", "at", "we", "you", "can", "could", "should",
    "would", "into", "use", "used", "using", "work", "works", "file", "files", "code", "function",
    "functions", "method", "class", "there", "here", "about", "explain", "find", "show", "me",
    "my", "our", "all", "any", "get", "set", "if", "not", "no", "yes", "please", "happen",
    "happens",
];

const KO_STOP: &[&str] = &[
    "어디", "어떻게", "무엇", "뭐", "무슨", "코드", "함수", "파일", "메서드", "클래스", "설명",
    "부분", "있는", "하는", "되는", "알려", "해줘", "알려줘", "보여줘", "찾아", "찾아줘", "이거",
    "그거", "저거", "여기", "거기", "어느", "왜", "언제", "누가", "좀", "그리고", "또는",
    "어디서", "어디에", "어떤", "무엇을", "뭘", "하려면", "고쳐야", "있어", "있나", "하나", "동작해",
    "보여", "있게", "없게", "되게", "싶어", "같아",
];

/// 부탁하는 끝말. 조사를 떼기 전에 먼저 뗀다 ("추가해줘" → "추가해" → "추가")
const KO_REQUEST: &[&str] = &["해주세요", "주세요", "해줘", "줘"];

/// 한국어 조사·어미를 떼어 어간을 남긴다 (긴 것부터 검사).
const KO_SUFFIX: &[&str] = &[
    "에서는", "으로는", "에서", "으로", "하는", "해서", "하고", "까지", "부터", "에게", "처럼",
    "보다", "이랑", "한테", "은", "는", "이", "가", "을", "를", "에", "의", "로", "와", "과",
    "도", "만", "랑", "해", "한", "할",
];

/// 한국어 개발 용어 → 코드에서 흔히 쓰는 영어 단어.
/// 코드는 영어 식별자로 쓰고 질문은 한국어로 하므로, 이 연결이 없으면 한국어 질문은
/// 한국어 주석에만 걸린다. 특정 프로젝트의 업무 용어가 아니라 일반적인 개발 용어만 둔다.
const KO_EN: &[(&str, &[&str])] = &[
    // 인증·권한
    ("로그인", &["login", "signin", "auth"]),
    ("로그아웃", &["logout", "signout"]),
    ("회원가입", &["signup", "register"]),
    ("인증", &["auth", "authenticate"]),
    ("인가", &["authorize", "permission"]),
    ("권한", &["role", "permission", "authorize"]),
    ("관리자", &["admin"]),
    ("운영자", &["operator", "admin"]),
    ("사용자", &["user"]),
    ("회원", &["user", "member"]),
    ("계정", &["account", "user"]),
    ("세션", &["session"]),
    ("쿠키", &["cookie"]),
    ("토큰", &["token"]),
    ("발급", &["issue", "sign", "create"]),
    ("재발급", &["refresh", "reissue", "rotate"]),
    ("갱신", &["refresh", "renew", "update"]),
    ("만료", &["expire", "expiry", "ttl"]),
    ("비밀번호", &["password"]),
    ("암호", &["password", "crypto"]),
    ("암호화", &["encrypt", "hash", "crypto"]),
    ("해시", &["hash"]),
    ("서명", &["sign", "signature"]),
    ("게스트", &["guest"]),
    ("익명", &["anonymous", "guest"]),
    ("소유자", &["owner"]),
    ("식별", &["identify", "resolve"]),
    ("공개", &["public"]),
    ("비공개", &["private"]),
    ("검사", &["check", "validate", "guard"]),
    ("검증", &["validate", "verify", "validation"]),
    ("방어", &["guard", "protect"]),
    // 요청·응답·오류
    ("요청", &["request"]),
    ("응답", &["response"]),
    ("에러", &["error"]),
    ("오류", &["error"]),
    ("실패", &["fail", "failure", "error"]),
    ("예외", &["exception", "error"]),
    ("재시도", &["retry"]),
    ("라우팅", &["route", "router"]),
    ("경로", &["route", "path"]),
    ("컨트롤러", &["controller"]),
    ("서비스", &["service"]),
    ("미들웨어", &["middleware"]),
    ("가드", &["guard"]),
    ("필터", &["filter"]),
    ("인터셉터", &["interceptor"]),
    ("캐시", &["cache"]),
    ("업로드", &["upload"]),
    ("다운로드", &["download"]),
    ("이미지", &["image"]),
    ("사진", &["image", "photo"]),
    // 데이터
    ("조회", &["find", "get", "query", "fetch"]),
    ("검색", &["search", "find", "query"]),
    ("목록", &["list"]),
    ("추가", &["add", "create"]),
    ("생성", &["create"]),
    ("삭제", &["delete", "remove"]),
    ("수정", &["update", "edit"]),
    ("저장", &["save", "store", "persist"]),
    ("저장소", &["repository", "store"]),
    ("레포지토리", &["repository"]),
    ("데이터베이스", &["database", "db"]),
    ("디비", &["db", "database"]),
    ("테이블", &["table"]),
    ("컬럼", &["column"]),
    ("스키마", &["schema"]),
    ("마이그레이션", &["migration", "migrate"]),
    ("트랜잭션", &["transaction"]),
    ("중복", &["duplicate", "dedupe"]),
    ("정렬", &["sort", "order"]),
    ("페이지네이션", &["pagination", "page", "cursor"]),
    ("매핑", &["map", "mapper"]),
    ("변환", &["convert", "transform", "map"]),
    // 설정·실행
    ("설정", &["config", "setting"]),
    ("환경", &["env", "environment", "config"]),
    ("변수", &["variable", "var"]),
    ("시작", &["start", "bootstrap", "init"]),
    ("부팅", &["boot", "bootstrap", "startup"]),
    ("초기화", &["init", "initialize", "reset"]),
    ("종료", &["shutdown", "close", "exit"]),
    ("배포", &["deploy", "release"]),
    ("배치", &["batch", "job"]),
    ("작업", &["job", "task"]),
    ("스케줄", &["schedule", "cron"]),
    ("크론", &["cron", "schedule"]),
    ("잠금", &["lock"]),
    ("락", &["lock"]),
    ("동기화", &["sync", "synchronize"]),
    ("비동기", &["async"]),
    ("큐", &["queue"]),
    ("이벤트", &["event"]),
    ("알림", &["notification", "notify", "alert"]),
    ("구독", &["subscribe", "subscription"]),
    ("구독자", &["subscriber", "subscription"]),
    ("로그", &["log", "logger"]),
    ("지표", &["metric", "metrics"]),
    ("모니터링", &["monitor", "metric"]),
    ("테스트", &["test", "spec"]),
    ("스크립트", &["script"]),
    // 화면
    ("화면", &["view", "screen", "page"]),
    ("페이지", &["page"]),
    ("표시", &["display", "show", "render"]),
    ("버튼", &["button"]),
    ("입력", &["input", "form"]),
    ("폼", &["form"]),
    ("모달", &["modal", "dialog"]),
    ("상태", &["state", "status"]),
    // 일반
    ("처리", &["handle", "process"]),
    ("흐름", &["flow"]),
    ("계산", &["calculate", "calc", "compute"]),
    ("점수", &["score"]),
    ("위험", &["risk", "danger"]),
    ("위험도", &["risk"]),
    ("단계", &["level", "stage", "step"]),
    ("등급", &["level", "grade"]),
    ("구간", &["range", "threshold"]),
    ("기준", &["threshold", "criteria"]),
    ("임계", &["threshold"]),
    ("주의", &["caution", "warning"]),
    ("경고", &["warn", "warning"]),
    ("안전", &["safe", "safety"]),
    ("가중치", &["weight"]),
    ("요인", &["factor"]),
    ("신고", &["report"]),
    ("제보", &["report"]),
    ("보고서", &["report"]),
    ("통계", &["stats", "statistics"]),
    ("날짜", &["date"]),
    ("시간", &["time"]),
    ("관측", &["observation", "observe"]),
    ("기상", &["weather"]),
    ("주인", &["owner"]),
    ("이름", &["name"]),
    ("정보", &["info", "detail"]),
    ("상세", &["detail"]),
    ("기록", &["record", "history", "log"]),
    ("이력", &["history", "log"]),
    // 검색·맥락
    ("검색어", &["query", "term", "keyword"]),
    ("질문", &["query", "question", "prompt"]),
    ("단어", &["word", "term", "token"]),
    ("색인", &["index"]),
    ("인덱스", &["index"]),
    ("순위", &["rank", "score"]),
    ("예산", &["budget"]),
    ("맥락", &["context"]),
    ("그래프", &["graph"]),
    ("지도", &["map", "graph"]),
    ("노드", &["node"]),
    ("트리", &["tree"]),
    ("파싱", &["parse", "parser"]),
    ("파서", &["parser"]),
    ("분석", &["analyze", "parse"]),
    ("토큰화", &["tokenize"]),
    // 코드·자료구조
    ("할당", &["alloc", "allocate", "capacity"]),
    ("메모리", &["memory", "alloc"]),
    ("성능", &["performance", "perf"]),
    ("속도", &["speed", "performance"]),
    ("크기", &["size", "len", "length"]),
    ("길이", &["length", "len"]),
    ("개수", &["count", "len"]),
    ("문자열", &["string", "str"]),
    ("배열", &["array", "vec", "list"]),
    ("키", &["key"]),
    ("값", &["value"]),
    ("반환", &["return"]),
    ("호출", &["call", "invoke"]),
    ("호출자", &["caller"]),
    ("인자", &["arg", "param"]),
    ("매개변수", &["param", "parameter"]),
    ("타입", &["type"]),
    ("복사", &["copy", "clone"]),
    ("연결", &["connect", "link", "edge"]),
    ("압축", &["compress", "compact"]),
    ("정규식", &["regex"]),
    ("스레드", &["thread"]),
    ("쓰레드", &["thread"]),
    ("병렬", &["parallel"]),
    ("동시", &["concurrent", "parallel"]),
    ("실행", &["run", "exec", "execute"]),
    ("프로세스", &["process"]),
    ("명령", &["command"]),
    ("서버", &["server"]),
    ("클라이언트", &["client"]),
    ("주소", &["address", "url"]),
    ("포트", &["port"]),
    ("소켓", &["socket"]),
    // 편집기·화면
    ("편집기", &["editor"]),
    ("터미널", &["terminal"]),
    ("창", &["window"]),
    ("탭", &["tab"]),
    ("메뉴", &["menu"]),
    ("단축키", &["keybinding", "shortcut", "keymap"]),
    ("테마", &["theme"]),
    ("색", &["color"]),
    ("색상", &["color"]),
    ("글꼴", &["font"]),
    ("글자", &["text", "char", "font"]),
    ("번역", &["i18n", "translate", "locale"]),
    ("언어", &["lang", "language", "locale"]),
    ("모델", &["model"]),
    ("프롬프트", &["prompt"]),
    ("에이전트", &["agent"]),
    ("도구", &["tool"]),
    ("승인", &["approve", "approval"]),
    ("거부", &["reject", "deny"]),
    // 흔한 업무 용어 (쇼핑·금융·게시판처럼 여러 프로젝트에 두루 나오는 것만)
    ("결제", &["payment", "pay", "checkout"]),
    ("주문", &["order"]),
    ("상품", &["product", "item"]),
    ("장바구니", &["cart"]),
    ("가격", &["price"]),
    ("금액", &["amount"]),
    ("계좌", &["account"]),
    ("이체", &["transfer"]),
    ("송금", &["transfer", "remit"]),
    ("잔액", &["balance"]),
    ("거래", &["transaction", "trade"]),
    ("고객", &["customer", "client"]),
    ("직원", &["employee", "staff"]),
    ("부서", &["department", "dept"]),
    ("게시글", &["post", "article"]),
    ("게시물", &["post", "article"]),
    ("게시판", &["board"]),
    ("댓글", &["comment", "reply"]),
    ("메시지", &["message"]),
    ("채팅", &["chat"]),
    ("이메일", &["email", "mail"]),
    ("메일", &["mail", "email"]),
    ("쿠폰", &["coupon"]),
    ("포인트", &["point"]),
    ("배송", &["delivery", "shipping"]),
    ("환불", &["refund"]),
    ("예약", &["reservation", "booking"]),
];

/// 동사: 활용형 앞부분 → 영어 단어. "바꿀", "바꿔", "바꾸는"처럼 모양이 바뀌어서 어간 대신 앞부분으로 찾는다.
/// 한 글자 활용형("셀", "뺄")은 낱말 전체가 같을 때만 ("셀러"는 count가 아니다).
/// "찾아줘"처럼 '찾아 달라'는 부탁은 기능 설명이 아니라서 "찾" 대신 "찾는/찾을/찾기"만 둔다.
const KO_VERB: &[(&[&str], &[&str])] = &[
    (&["만들", "만드", "만든"], &["build", "make", "create"]),
    (&["바꾸", "바꿔", "바꿀", "바꾼", "바뀌", "바뀐", "바뀔"], &["change", "update", "replace"]),
    (&["줄이", "줄여", "줄일", "줄인"], &["reduce", "shrink"]),
    (&["늘리", "늘려", "늘릴", "늘린"], &["increase", "extend", "grow"]),
    (&["찾는", "찾을", "찾기", "찾은"], &["find", "search", "lookup"]),
    (&["고치", "고쳐", "고칠", "고친"], &["fix"]),
    (&["지우", "지워", "지울", "지운"], &["delete", "remove", "clear"]),
    (&["읽는", "읽을", "읽어", "읽기", "읽은"], &["read", "load"]),
    (&["보내", "보낼", "보낸"], &["send"]),
    (&["받는", "받을", "받아", "받은"], &["receive", "fetch"]),
    (&["나누", "나눠", "나눌", "나눈"], &["split", "divide"]),
    (&["합치", "합쳐", "합칠", "합친"], &["merge", "join"]),
    (&["묶는", "묶어", "묶을", "묶기"], &["group", "bundle"]),
    (&["부르", "불러", "부를", "부른"], &["call", "invoke", "load"]),
    (&["넣는", "넣어", "넣을", "넣기", "넣은"], &["insert", "add", "put"]),
    (&["빼는", "빼고", "빼", "뺄", "뺀"], &["remove", "exclude"]),
    (&["옮기", "옮겨", "옮길"], &["move"]),
    (&["막아", "막는", "막을", "막기"], &["block", "prevent", "guard"]),
    (&["세는", "셀", "세어", "세기"], &["count"]),
    (&["느려", "느리", "느린"], &["slow", "performance"]),
    (&["빠르", "빨리", "빠른"], &["fast", "performance"]),
    (&["깨져", "깨지", "깨진"], &["broken", "encoding", "render"]),
];

fn english_for(stem: &str) -> &'static [&'static str] {
    if let Some((_, v)) = KO_EN.iter().find(|(k, _)| *k == stem) {
        return v;
    }
    // 합성어는 사전에 있는 가장 긴 앞말로 ("검색창" → "검색")
    KO_EN
        .iter()
        .filter(|(k, _)| k.chars().count() >= 2 && stem.starts_with(k))
        .max_by_key(|(k, _)| k.len())
        .map(|(_, v)| *v)
        .unwrap_or(&[])
}

fn verb_english(word: &str) -> &'static [&'static str] {
    let hit = |f: &&str| if f.chars().count() == 1 { word == *f } else { word.starts_with(f) };
    KO_VERB.iter().find(|(forms, _)| forms.iter().any(hit)).map(|(_, v)| *v).unwrap_or(&[])
}

#[derive(Debug, Clone, PartialEq)]
pub struct QueryTerm {
    pub text: String,
    pub prefix: bool,
    /// 질문에 없던, 한국어 개발 용어에서 옮긴 영어 단어. 질문의 단어보다 약하게 친다.
    pub expanded: bool,
}

fn is_hangul(c: char) -> bool {
    ('\u{AC00}'..='\u{D7A3}').contains(&c)
}

/// 질문을 검색어 목록으로 바꾼다.
pub fn query_terms(query: &str) -> Vec<QueryTerm> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    let mut push = |text: String, prefix: bool, expanded: bool, out: &mut Vec<QueryTerm>| {
        if seen.insert(text.clone()) {
            out.push(QueryTerm { text, prefix, expanded });
        }
    };

    for ident in identifiers(query) {
        let lower = ident.to_ascii_lowercase();
        let parts = split_identifier(ident);
        if parts.len() > 1 && !EN_STOP.contains(&lower.as_str()) {
            push(lower, false, false, &mut out);
        }
        for p in parts {
            if !EN_STOP.contains(&p.as_str()) {
                let prefix = p.len() >= 4;
                push(p, prefix, false, &mut out);
            }
        }
    }

    // 한국어: 어간을 그대로 찾고(한국어 주석·문서), 개발 용어면 영어 단어도 함께 찾는다
    let mut english: Vec<&str> = Vec::new();
    for word in query.split(|c: char| !is_hangul(c)).filter(|w| !w.is_empty()) {
        let word = KO_REQUEST.iter().find_map(|r| word.strip_suffix(r).filter(|w| !w.is_empty())).unwrap_or(word);
        let mut stem = word;
        for suf in KO_SUFFIX {
            if let Some(s) = stem.strip_suffix(suf) {
                if s.chars().count() >= 2 {
                    stem = s;
                    break;
                }
            }
        }
        if (stem.chars().count() >= 2 || !english_for(stem).is_empty()) && !KO_STOP.contains(&stem) {
            push(stem.to_string(), true, false, &mut out);
        }
        // "위험도"처럼 사전에 그대로 있거나, 조사를 떼기 전 낱말이 사전에 있으면
        let found = [stem, word].into_iter().flat_map(english_for).chain(verb_english(word));
        for en in found {
            if !english.contains(en) {
                english.push(en);
            }
        }
    }
    for en in english {
        push(en.to_string(), en.len() >= 4, true, &mut out);
    }

    out.truncate(28);
    out
}

/// FTS5 MATCH 식. 단어를 따옴표로 감싸 문법 오류를 막는다.
pub fn fts_query(terms: &[QueryTerm]) -> String {
    terms
        .iter()
        .map(|t| {
            let q = t.text.replace('"', "");
            if t.prefix {
                format!("\"{q}\"*")
            } else {
                format!("\"{q}\"")
            }
        })
        .collect::<Vec<_>>()
        .join(" OR ")
}

/// 대략적인 토큰 수. ASCII는 4자당 1토큰, 한글 등은 1.5자당 1토큰으로 본다.
pub fn estimate_tokens(s: &str) -> usize {
    let (mut ascii, mut other) = (0usize, 0usize);
    for c in s.chars() {
        if c.is_ascii() {
            ascii += 1;
        } else {
            other += 1;
        }
    }
    ascii / 4 + other * 2 / 3 + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_identifiers() {
        assert_eq!(split_identifier("parseUserConfig"), ["parse", "user", "config"]);
        assert_eq!(split_identifier("HTTPServer"), ["http", "server"]);
        assert_eq!(split_identifier("load_index_db"), ["load", "index", "db"]);
        assert_eq!(split_identifier("getV2Token"), ["get", "v2", "token"]);
        assert_eq!(split_identifier("XMLHttpRequest"), ["xml", "http", "request"]);
        assert_eq!(split_identifier("sha256Hash"), ["sha", "256", "hash"]);
        assert_eq!(split_identifier("getX"), ["get"]); // 한 글자 조각은 버린다.
    }

    #[test]
    fn builds_query_terms() {
        let terms: Vec<_> = query_terms("How does refreshToken work?")
            .into_iter()
            .map(|t| t.text)
            .collect();
        assert_eq!(terms, ["refreshtoken", "refresh", "token"]);

        let ko: Vec<_> = query_terms("로그인은 어디서 처리해?").into_iter().map(|t| t.text).collect();
        assert!(ko.contains(&"로그인".to_string()));
        assert!(ko.contains(&"처리".to_string()));
        // 한국어 개발 용어는 코드의 영어 단어로도 찾는다
        assert!(ko.contains(&"login".to_string()) && ko.contains(&"handle".to_string()), "{ko:?}");
        let risk: Vec<_> = query_terms("위험도 계산 로직").into_iter().map(|t| t.text).collect();
        assert!(risk.contains(&"risk".to_string()) && risk.contains(&"calculate".to_string()), "{risk:?}");
    }

    fn texts(q: &str) -> Vec<String> {
        query_terms(q).into_iter().map(|t| t.text).collect()
    }

    #[test]
    fn korean_actions_become_code_words() {
        // #33: 동작을 묘사한 질문도 코드 쪽 단어로 넓힌다
        let t = texts("검색어를 만들 때 할당을 줄여줘");
        for w in ["query", "term", "build", "alloc", "reduce"] {
            assert!(t.contains(&w.to_string()), "{w} 없음: {t:?}");
        }
        // 부탁하는 끝말을 떼야 조사도 떨어진다
        let t = texts("주인 정보를 저장할 때 검증을 추가해줘");
        assert!(t.contains(&"추가".to_string()) && t.contains(&"add".to_string()), "{t:?}");
        assert!(!t.iter().any(|w| w.ends_with('줘')), "{t:?}");
        // 활용형: 바꿀, 바꿔, 바꾸는
        for q in ["크기를 바꿀 때", "크기를 바꿔줘", "크기를 바꾸는 곳"] {
            let t = texts(q);
            assert!(t.contains(&"change".to_string()) && t.contains(&"size".to_string()), "{q}: {t:?}");
        }
    }

    #[test]
    fn korean_compounds_and_short_forms() {
        // 사전에 없는 합성어는 가장 긴 앞말로
        assert!(texts("검색창 열기").contains(&"search".to_string()));
        // 한 글자 활용형은 낱말 전체가 같을 때만: "셀러"는 count가 아니다
        assert!(texts("호출자를 셀 때").contains(&"count".to_string()));
        assert!(!texts("셀러 목록").contains(&"count".to_string()));
        // '찾아 달라'는 부탁은 기능 설명이 아니다
        assert!(!texts("로그인 코드 찾아줘").contains(&"search".to_string()));
        assert!(texts("주인을 찾을 때").contains(&"find".to_string()));
    }

    #[test]
    fn fts_query_is_quoted() {
        let q = fts_query(&[
            QueryTerm { text: "user".into(), prefix: true, expanded: false },
            QueryTerm { text: "id".into(), prefix: false, expanded: false },
        ]);
        assert_eq!(q, r#""user"* OR "id""#);
    }
}
