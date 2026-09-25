//! SQLite 인덱스 저장소.

use crate::parse::Parsed;
use crate::tokenize;
use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;

/// 스키마가 바뀌면 올린다. 버전이 다르면 인덱스를 새로 만든다.
const SCHEMA_VERSION: &str = "1";

const SCHEMA: &str = r#"
CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE files(
    id INTEGER PRIMARY KEY,
    path TEXT NOT NULL UNIQUE,
    lang TEXT NOT NULL,
    hash TEXT NOT NULL,
    mtime INTEGER NOT NULL,
    size INTEGER NOT NULL,
    indexed_at INTEGER NOT NULL
);
CREATE TABLE symbols(
    id INTEGER PRIMARY KEY,
    file_id INTEGER NOT NULL,
    name TEXT NOT NULL,
    kind TEXT NOT NULL,
    start_line INTEGER NOT NULL,
    end_line INTEGER NOT NULL,
    start_byte INTEGER NOT NULL,
    end_byte INTEGER NOT NULL,
    signature TEXT NOT NULL,
    doc TEXT
);
CREATE INDEX symbols_name ON symbols(name);
CREATE INDEX symbols_file ON symbols(file_id);
CREATE TABLE refs(
    id INTEGER PRIMARY KEY,
    file_id INTEGER NOT NULL,
    name TEXT NOT NULL,
    kind TEXT NOT NULL,
    line INTEGER NOT NULL,
    enclosing_symbol_id INTEGER
);
CREATE INDEX refs_name ON refs(name);
CREATE INDEX refs_enclosing ON refs(enclosing_symbol_id);
CREATE INDEX refs_file ON refs(file_id);
CREATE VIRTUAL TABLE symbols_fts USING fts5(
    name, path, terms, body,
    tokenize = 'unicode61 remove_diacritics 2'
);
CREATE TABLE cochange(
    file_a TEXT NOT NULL,
    file_b TEXT NOT NULL,
    count INTEGER NOT NULL,
    PRIMARY KEY(file_a, file_b)
);
CREATE INDEX cochange_b ON cochange(file_b);
"#;

const TABLES: &[&str] = &["meta", "files", "symbols", "refs", "symbols_fts", "cochange"];

const SYMBOL_COLS: &str = "s.id, s.file_id, f.path, f.lang, s.name, s.kind, s.start_line, s.end_line, \
     s.start_byte, s.end_byte, s.signature, s.doc";

#[derive(Debug, Clone)]
pub struct FileRow {
    pub id: i64,
    pub mtime: i64,
    pub size: i64,
    pub hash: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SymbolRow {
    pub id: i64,
    pub file_id: i64,
    pub path: String,
    pub lang: String,
    pub name: String,
    pub kind: String,
    pub start_line: u32,
    pub end_line: u32,
    pub start_byte: usize,
    pub end_byte: usize,
    pub signature: String,
    pub doc: Option<String>,
}

impl SymbolRow {
    fn from_row(r: &Row) -> rusqlite::Result<Self> {
        Ok(Self {
            id: r.get(0)?,
            file_id: r.get(1)?,
            path: r.get(2)?,
            lang: r.get(3)?,
            name: r.get(4)?,
            kind: r.get(5)?,
            start_line: r.get(6)?,
            end_line: r.get(7)?,
            start_byte: r.get::<_, i64>(8)? as usize,
            end_byte: r.get::<_, i64>(9)? as usize,
            signature: r.get(10)?,
            doc: r.get(11)?,
        })
    }

    pub fn contains(&self, other: &SymbolRow) -> bool {
        self.file_id == other.file_id
            && self.start_byte <= other.start_byte
            && other.end_byte <= self.end_byte
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RefRow {
    pub path: String,
    pub line: u32,
    pub kind: String,
    pub enclosing: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Stats {
    pub files: i64,
    pub symbols: i64,
    pub refs: i64,
    pub cochange_pairs: i64,
    pub by_lang: Vec<(String, i64)>,
}

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        Self::init(Connection::open(path)?)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.execute_batch(
            // 기본 캐시(2MB)로는 초기 인덱싱의 큰 트랜잭션이 디스크로 계속 넘쳐 8배 느려진다.
            "PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL; PRAGMA temp_store = MEMORY;
             PRAGMA cache_size = -65536;",
        )?;
        let version: Option<String> = conn
            .query_row("SELECT value FROM meta WHERE key = 'schema_version'", [], |r| r.get(0))
            .optional()
            .unwrap_or(None);
        if version.as_deref() != Some(SCHEMA_VERSION) {
            for t in TABLES {
                conn.execute_batch(&format!("DROP TABLE IF EXISTS {t};"))?;
            }
            conn.execute_batch(SCHEMA)?;
            conn.execute(
                "INSERT INTO meta(key, value) VALUES ('schema_version', ?1)",
                [SCHEMA_VERSION],
            )?;
        }
        Ok(Self { conn })
    }

    pub fn begin(&self) -> Result<()> {
        self.conn.execute_batch("BEGIN")?;
        Ok(())
    }

    pub fn commit(&self) -> Result<()> {
        self.conn.execute_batch("COMMIT")?;
        Ok(())
    }

    pub fn rollback(&self) {
        let _ = self.conn.execute_batch("ROLLBACK");
    }

    pub fn meta(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
            .optional()?)
    }

    pub fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO meta(key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [key, value],
        )?;
        Ok(())
    }

    // ── 쓰기 ────────────────────────────────────────────────

    pub fn files(&self) -> Result<HashMap<String, FileRow>> {
        let mut stmt = self.conn.prepare("SELECT id, path, mtime, size, hash FROM files")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(1)?,
                FileRow { id: r.get(0)?, mtime: r.get(2)?, size: r.get(3)?, hash: r.get(4)? },
            ))
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn touch_file(&self, id: i64, mtime: i64, size: i64) -> Result<()> {
        self.conn
            .execute("UPDATE files SET mtime = ?2, size = ?3 WHERE id = ?1", params![id, mtime, size])?;
        Ok(())
    }

    fn clear_file_children(&self, file_id: i64) -> Result<()> {
        self.conn.execute(
            "DELETE FROM symbols_fts WHERE rowid IN (SELECT id FROM symbols WHERE file_id = ?1)",
            [file_id],
        )?;
        self.conn.execute("DELETE FROM refs WHERE file_id = ?1", [file_id])?;
        self.conn.execute("DELETE FROM symbols WHERE file_id = ?1", [file_id])?;
        Ok(())
    }

    pub fn remove_file(&self, file_id: i64) -> Result<()> {
        self.clear_file_children(file_id)?;
        self.conn.execute("DELETE FROM files WHERE id = ?1", [file_id])?;
        Ok(())
    }

    /// 파일 하나의 심볼·참조를 통째로 교체한다.
    pub fn replace_file(
        &self,
        path: &str,
        lang: &str,
        hash: &str,
        mtime: i64,
        size: i64,
        parsed: &Parsed,
    ) -> Result<()> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let existing: Option<i64> = self
            .conn
            .query_row("SELECT id FROM files WHERE path = ?1", [path], |r| r.get(0))
            .optional()?;
        let file_id = match existing {
            Some(id) => {
                self.clear_file_children(id)?;
                self.conn.execute(
                    "UPDATE files SET lang = ?2, hash = ?3, mtime = ?4, size = ?5, indexed_at = ?6
                     WHERE id = ?1",
                    params![id, lang, hash, mtime, size, now],
                )?;
                id
            }
            None => {
                self.conn.execute(
                    "INSERT INTO files(path, lang, hash, mtime, size, indexed_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![path, lang, hash, mtime, size, now],
                )?;
                self.conn.last_insert_rowid()
            }
        };

        let mut ins_sym = self.conn.prepare_cached(
            "INSERT INTO symbols(file_id, name, kind, start_line, end_line, start_byte, end_byte, signature, doc)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        )?;
        let mut ins_fts = self.conn.prepare_cached(
            "INSERT INTO symbols_fts(rowid, name, path, terms, body) VALUES (?1, ?2, ?3, ?4, ?5)",
        )?;
        let mut ids = Vec::with_capacity(parsed.symbols.len());
        for s in &parsed.symbols {
            ins_sym.execute(params![
                file_id,
                s.name,
                s.kind,
                s.start_line,
                s.end_line,
                s.start_byte as i64,
                s.end_byte as i64,
                s.signature,
                s.doc
            ])?;
            let id = self.conn.last_insert_rowid();
            ids.push(id);
            let name_col = format!("{} {}", s.name, tokenize::split_identifier(&s.name).join(" "));
            ins_fts.execute(params![id, name_col, path, s.terms, s.body])?;
        }

        let mut ins_ref = self.conn.prepare_cached(
            "INSERT INTO refs(file_id, name, kind, line, enclosing_symbol_id) VALUES (?1, ?2, ?3, ?4, ?5)",
        )?;
        for r in &parsed.refs {
            ins_ref.execute(params![file_id, r.name, r.kind, r.line, r.enclosing.map(|i| ids[i])])?;
        }
        Ok(())
    }

    pub fn replace_cochange(&self, pairs: &[(String, String, u32)]) -> Result<()> {
        self.conn.execute("DELETE FROM cochange", [])?;
        let mut stmt =
            self.conn.prepare_cached("INSERT INTO cochange(file_a, file_b, count) VALUES (?1, ?2, ?3)")?;
        for (a, b, n) in pairs {
            stmt.execute(params![a, b, n])?;
        }
        Ok(())
    }

    // ── 읽기 ────────────────────────────────────────────────

    /// FTS5 BM25 검색. 점수는 클수록 관련도가 높다.
    pub fn search(&self, fts_query: &str, limit: usize) -> Result<Vec<(SymbolRow, f64)>> {
        let sql = format!(
            "SELECT {SYMBOL_COLS}, bm25(symbols_fts, 10.0, 1.5, 4.0, 1.0) AS rank
             FROM symbols_fts
             JOIN symbols s ON s.id = symbols_fts.rowid
             JOIN files f ON f.id = s.file_id
             WHERE symbols_fts MATCH ?1
             ORDER BY rank LIMIT ?2"
        );
        let mut stmt = self.conn.prepare_cached(&sql)?;
        let rows = stmt.query_map(params![fts_query, limit as i64], |r| {
            Ok((SymbolRow::from_row(r)?, -r.get::<_, f64>(12)?))
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn symbol(&self, id: i64) -> Result<Option<SymbolRow>> {
        let sql = format!("SELECT {SYMBOL_COLS} FROM symbols s JOIN files f ON f.id = s.file_id WHERE s.id = ?1");
        Ok(self.conn.prepare_cached(&sql)?.query_row([id], SymbolRow::from_row).optional()?)
    }

    pub fn symbols_by_name(&self, name: &str, limit: usize) -> Result<Vec<SymbolRow>> {
        let sql = format!(
            "SELECT {SYMBOL_COLS} FROM symbols s JOIN files f ON f.id = s.file_id
             WHERE s.name = ?1 ORDER BY f.path, s.start_line LIMIT ?2"
        );
        let mut stmt = self.conn.prepare_cached(&sql)?;
        let rows = stmt.query_map(params![name, limit as i64], SymbolRow::from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn count_defs(&self, name: &str) -> Result<i64> {
        Ok(self
            .conn
            .prepare_cached("SELECT COUNT(*) FROM symbols WHERE name = ?1")?
            .query_row([name], |r| r.get(0))?)
    }

    pub fn symbols_in_file(&self, path: &str) -> Result<Vec<SymbolRow>> {
        let sql = format!(
            "SELECT {SYMBOL_COLS} FROM symbols s JOIN files f ON f.id = s.file_id
             WHERE f.path = ?1 ORDER BY s.start_line"
        );
        let mut stmt = self.conn.prepare_cached(&sql)?;
        let rows = stmt.query_map([path], SymbolRow::from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// 심볼 본문 안에서 참조하는 이름들
    pub fn callee_names(&self, symbol_id: i64) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT name FROM refs WHERE enclosing_symbol_id = ?1 GROUP BY name ORDER BY MIN(line)",
        )?;
        let rows = stmt.query_map([symbol_id], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// `name`을 참조하는 심볼 id들
    pub fn caller_ids(&self, name: &str, limit: usize) -> Result<Vec<i64>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT DISTINCT enclosing_symbol_id FROM refs
             WHERE name = ?1 AND enclosing_symbol_id IS NOT NULL LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![name, limit as i64], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn references(&self, name: &str, limit: usize) -> Result<Vec<RefRow>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT f.path, r.line, r.kind, s.name FROM refs r
             JOIN files f ON f.id = r.file_id
             LEFT JOIN symbols s ON s.id = r.enclosing_symbol_id
             WHERE r.name = ?1 ORDER BY f.path, r.line LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![name, limit as i64], |r| {
            Ok(RefRow { path: r.get(0)?, line: r.get(1)?, kind: r.get(2)?, enclosing: r.get(3)? })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// `path`와 함께 자주 바뀐 파일과 횟수
    pub fn cochanged(&self, path: &str, limit: usize) -> Result<Vec<(String, u32)>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT file_b, count FROM cochange WHERE file_a = ?1
             UNION ALL
             SELECT file_a, count FROM cochange WHERE file_b = ?1
             ORDER BY 2 DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![path, limit as i64], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    // ── 지식그래프용 ────────────────────────────────────────

    /// 파일별 언어와 심볼 수
    pub fn file_summaries(&self) -> Result<Vec<(String, String, i64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT f.path, f.lang, COUNT(s.id) FROM files f LEFT JOIN symbols s ON s.file_id = f.id
             GROUP BY f.id ORDER BY f.path",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// 파일 A가 파일 B에 정의된 이름을 참조한 횟수 (이름 기반).
    /// 정의가 `max_defs`개를 넘는 흔한 이름(new, get 등)은 잡음이라 뺀다.
    pub fn file_ref_edges(&self, max_defs: i64) -> Result<Vec<(String, String, i64)>> {
        let mut stmt = self.conn.prepare(
            "WITH d AS (SELECT name FROM symbols GROUP BY name HAVING COUNT(*) <= ?1)
             SELECT fr.path, fs.path, COUNT(*) FROM refs r
             JOIN d ON d.name = r.name
             JOIN symbols s ON s.name = r.name
             JOIN files fr ON fr.id = r.file_id
             JOIN files fs ON fs.id = s.file_id
             WHERE r.file_id != s.file_id
             GROUP BY r.file_id, s.file_id",
        )?;
        let rows = stmt.query_map([max_defs], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// 함께 바뀐 파일 쌍 (횟수가 `min` 이상)
    pub fn cochange_pairs(&self, min: u32) -> Result<Vec<(String, String, u32)>> {
        let mut stmt = self.conn.prepare("SELECT file_a, file_b, count FROM cochange WHERE count >= ?1")?;
        let rows = stmt.query_map([min], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// 이름이 심볼로 정의되어 있는지 (기억 연결용)
    pub fn has_symbol(&self, name: &str) -> Result<bool> {
        Ok(self.count_defs(name)? > 0)
    }

    pub fn stats(&self) -> Result<Stats> {
        let count = |sql: &str| -> Result<i64> { Ok(self.conn.query_row(sql, [], |r| r.get(0))?) };
        let mut stmt = self.conn.prepare("SELECT lang, COUNT(*) FROM files GROUP BY lang ORDER BY 2 DESC")?;
        let by_lang = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(Stats {
            files: count("SELECT COUNT(*) FROM files")?,
            symbols: count("SELECT COUNT(*) FROM symbols")?,
            refs: count("SELECT COUNT(*) FROM refs")?,
            cochange_pairs: count("SELECT COUNT(*) FROM cochange")?,
            by_lang,
        })
    }
}
