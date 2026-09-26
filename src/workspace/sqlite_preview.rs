//! Read-only table preview for a SQLite file. Callers pass an already
//! sandboxed path. The client never supplies SQL — table names are taken
//! from `sqlite_master` and then quoted.

use std::path::Path;

use rusqlite::{Connection, OpenFlags, params};
use serde::Serialize;
use serde_json::{Number, Value};

use super::WorkspaceError;

const PAGE_ROWS: u64 = 100;
const MAX_OFFSET: u64 = 1_000_000;
const MAX_CELL_CHARS: usize = 2_000;
const SQLITE_HEADER: &[u8] = b"SQLite format 3\0";

#[derive(Debug, Clone, Serialize)]
pub struct SqlitePreview {
    pub tables: Vec<String>,
    pub table: String,
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
    pub offset: u64,
    pub limit: u64,
    pub truncated: bool,
}

pub fn preview(abs: &Path, table: &str, offset: u64) -> Result<SqlitePreview, WorkspaceError> {
    if offset > MAX_OFFSET {
        return Err(WorkspaceError::PreviewOffset);
    }
    if !is_sqlite_file(abs)? {
        return Err(WorkspaceError::NotSqlite);
    }

    let conn = Connection::open_with_flags(
        abs,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(db_err)?;
    conn.busy_timeout(std::time::Duration::from_secs(2))
        .map_err(db_err)?;
    conn.execute_batch("PRAGMA query_only=ON;")
        .map_err(db_err)?;

    let tables = list_tables(&conn)?;
    let selected = if table.is_empty() {
        tables.first().cloned().unwrap_or_default()
    } else if tables.iter().any(|name| name == table) {
        table.to_string()
    } else {
        return Err(WorkspaceError::UnknownTable);
    };

    if selected.is_empty() {
        return Ok(SqlitePreview {
            tables,
            table: String::new(),
            columns: Vec::new(),
            rows: Vec::new(),
            offset,
            limit: PAGE_ROWS,
            truncated: false,
        });
    }

    let sql = format!(
        "SELECT * FROM {} LIMIT ?1 OFFSET ?2",
        quote_ident(&selected)
    );
    let mut stmt = conn.prepare(&sql).map_err(db_err)?;
    let columns: Vec<String> = stmt
        .column_names()
        .iter()
        .map(|name| (*name).to_string())
        .collect();
    let mut query = stmt.query(params![PAGE_ROWS + 1, offset]).map_err(db_err)?;
    let mut rows = Vec::new();
    while let Some(row) = query.next().map_err(db_err)? {
        if rows.len() == PAGE_ROWS as usize {
            return Ok(SqlitePreview {
                tables,
                table: selected,
                columns,
                rows,
                offset,
                limit: PAGE_ROWS,
                truncated: true,
            });
        }
        let mut cells = Vec::with_capacity(columns.len());
        for index in 0..columns.len() {
            cells.push(cell_to_json(row.get_ref(index).map_err(db_err)?));
        }
        rows.push(cells);
    }

    Ok(SqlitePreview {
        tables,
        table: selected,
        columns,
        rows,
        offset,
        limit: PAGE_ROWS,
        truncated: false,
    })
}

fn is_sqlite_file(abs: &Path) -> Result<bool, WorkspaceError> {
    let mut file = std::fs::File::open(abs)?;
    let mut header = [0_u8; 16];
    use std::io::Read;
    let n = file.read(&mut header)?;
    Ok(n == header.len() && &header == SQLITE_HEADER)
}

fn list_tables(conn: &Connection) -> Result<Vec<String>, WorkspaceError> {
    let mut stmt = conn
        .prepare(
            "SELECT name FROM sqlite_master
             WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite_%'
             ORDER BY name",
        )
        .map_err(db_err)?;
    let names = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(db_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_err)?;
    Ok(names)
}

fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

fn cell_to_json(value: rusqlite::types::ValueRef<'_>) -> Value {
    match value {
        rusqlite::types::ValueRef::Null => Value::Null,
        rusqlite::types::ValueRef::Integer(n) => Value::Number(n.into()),
        rusqlite::types::ValueRef::Real(n) => Number::from_f64(n)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        rusqlite::types::ValueRef::Text(bytes) => {
            let text = String::from_utf8_lossy(bytes);
            let mut chars = text.chars();
            let head: String = chars.by_ref().take(MAX_CELL_CHARS).collect();
            if chars.next().is_some() {
                Value::String(format!("{head}…"))
            } else {
                Value::String(head)
            }
        }
        rusqlite::types::ValueRef::Blob(bytes) => {
            Value::String(format!("[blob {} bytes]", bytes.len()))
        }
    }
}

fn db_err(err: rusqlite::Error) -> WorkspaceError {
    WorkspaceError::Sqlite(err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_db(dir: &Path) -> std::path::PathBuf {
        let path = dir.join("app.db");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE items (id INTEGER, name TEXT);
             INSERT INTO items VALUES (1, 'a');
             INSERT INTO items VALUES (2, 'b');",
        )
        .unwrap();
        path
    }

    #[test]
    fn lists_tables_and_rows() {
        let dir = tempfile::tempdir().unwrap();
        let path = sample_db(dir.path());
        let page = preview(&path, "", 0).unwrap();
        assert_eq!(page.tables, vec!["items".to_string()]);
        assert_eq!(page.table, "items");
        assert_eq!(page.columns, vec!["id".to_string(), "name".to_string()]);
        assert_eq!(page.rows.len(), 2);
        assert_eq!(page.rows[0][1], Value::String("a".into()));
        assert!(!page.truncated);
    }

    #[test]
    fn rejects_unknown_and_injected_table_names() {
        let dir = tempfile::tempdir().unwrap();
        let path = sample_db(dir.path());
        assert!(matches!(
            preview(&path, "missing", 0),
            Err(WorkspaceError::UnknownTable)
        ));
        assert!(matches!(
            preview(&path, "items; DROP TABLE items", 0),
            Err(WorkspaceError::UnknownTable)
        ));
        let conn = Connection::open(&path).unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM items", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn rejects_non_sqlite_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notes.db");
        std::fs::write(&path, b"hello").unwrap();
        assert!(matches!(
            preview(&path, "", 0),
            Err(WorkspaceError::NotSqlite)
        ));
    }

    #[test]
    fn pages_rows() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wide.db");
        let conn = Connection::open(&path).unwrap();
        conn.execute("CREATE TABLE n (id INTEGER)", []).unwrap();
        for id in 0..150 {
            conn.execute("INSERT INTO n VALUES (?1)", params![id])
                .unwrap();
        }
        drop(conn);
        let first = preview(&path, "n", 0).unwrap();
        assert_eq!(first.rows.len(), 100);
        assert!(first.truncated);
        let second = preview(&path, "n", 100).unwrap();
        assert_eq!(second.rows.len(), 50);
        assert!(!second.truncated);
    }
}
