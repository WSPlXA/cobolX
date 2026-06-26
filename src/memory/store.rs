use rusqlite::{Connection, OpenFlags};
use std::error::Error;
use std::path::{Path, PathBuf};

use super::memories::CodexMemories;

type StoreResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

#[allow(dead_code)]
pub struct MemoryPaths {
    pub root: PathBuf,
    pub base_dir: PathBuf,
    pub db_path: PathBuf,
    pub blobs_dir: PathBuf,
    pub docs_dir: PathBuf,
    pub runs_dir: PathBuf,
    pub skills_dir: PathBuf,
}

#[allow(dead_code)]
pub struct MemoryStore {
    conn: Connection,
    paths: MemoryPaths,
}

impl MemoryPaths {
    pub fn for_root(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        let base_dir = root.join(".cobolx");
        let docs_dir = root.join("docs");
        let memory_dir = base_dir.join("memory");

        Self {
            root,
            base_dir: base_dir.clone(),
            db_path: memory_dir.join("project.db"),
            blobs_dir: base_dir.join("blobs"),
            docs_dir,
            runs_dir: base_dir.join("runs"),
            skills_dir: base_dir.join("skills"),
        }
    }
}

#[allow(dead_code)]
impl MemoryStore {
    pub fn open_or_create(root: impl Into<PathBuf>) -> StoreResult<Self> {
        let paths = MemoryPaths::for_root(root);
        create_dirs(&paths)?;

        let conn = Connection::open_with_flags(
            &paths.db_path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;

        configure_connection(&conn)?;
        migrate_schema(&conn)?;

        Ok(Self { conn, paths })
    }

    pub fn project_root(&self) -> &Path {
        &self.paths.root
    }

    pub fn db_path(&self) -> &Path {
        &self.paths.db_path
    }

    pub fn docs_dir(&self) -> &Path {
        &self.paths.docs_dir
    }

    pub fn skills_dir(&self) -> &Path {
        &self.paths.skills_dir
    }

    pub fn runs_dir(&self) -> &Path {
        &self.paths.runs_dir
    }

    pub fn codex_memories(&self) -> CodexMemories {
        CodexMemories::for_project(&self.paths.base_dir, &self.paths.root)
    }

    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    pub fn connection_mut(&mut self) -> &mut Connection {
        &mut self.conn
    }

    pub fn query_readonly(&self, sql: &str) -> StoreResult<serde_json::Value> {
        let trimmed = sql.trim();
        if !trimmed.to_ascii_uppercase().starts_with("SELECT") {
            return Err("Only SELECT queries are allowed for security reasons".into());
        }

        let mut stmt = self.conn.prepare(trimmed)?;
        let col_count = stmt.column_count();
        let col_names: Vec<String> = stmt
            .column_names()
            .into_iter()
            .map(|s| s.to_string())
            .collect();

        let mut rows = stmt.query([])?;
        let mut result_rows = Vec::new();

        while let Some(row) = rows.next()? {
            let mut map = serde_json::Map::new();
            for i in 0..col_count {
                let col_name = &col_names[i];
                let value_ref = row.get_ref(i)?;
                let value = match value_ref {
                    rusqlite::types::ValueRef::Null => serde_json::Value::Null,
                    rusqlite::types::ValueRef::Integer(v) => {
                        serde_json::Value::Number(serde_json::Number::from(v))
                    }
                    rusqlite::types::ValueRef::Real(v) => {
                        if let Some(num) = serde_json::Number::from_f64(v) {
                            serde_json::Value::Number(num)
                        } else {
                            serde_json::Value::Null
                        }
                    }
                    rusqlite::types::ValueRef::Text(v) => {
                        let s = std::str::from_utf8(v).unwrap_or("");
                        serde_json::Value::String(s.to_string())
                    }
                    rusqlite::types::ValueRef::Blob(v) => {
                        let hex_str = v.iter().map(|b| format!("{:02x}", b)).collect::<String>();
                        serde_json::Value::String(hex_str)
                    }
                };
                map.insert(col_name.clone(), value);
            }
            result_rows.push(serde_json::Value::Object(map));
        }

        Ok(serde_json::Value::Array(result_rows))
    }

    pub fn project_index_is_empty(&self) -> StoreResult<bool> {
        let file_count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM files", [], |row| row.get(0))?;
        Ok(file_count == 0)
    }
}

fn create_dirs(paths: &MemoryPaths) -> StoreResult<()> {
    std::fs::create_dir_all(paths.db_path.parent().unwrap())?;
    std::fs::create_dir_all(&paths.blobs_dir)?;
    std::fs::create_dir_all(&paths.docs_dir)?;
    std::fs::create_dir_all(&paths.runs_dir)?;
    std::fs::create_dir_all(&paths.skills_dir)?;
    let memories = CodexMemories::for_project(&paths.base_dir, &paths.root);
    memories.ensure_layout()?;
    Ok(())
}

fn configure_connection(conn: &Connection) -> rusqlite::Result<()> {
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.pragma_update(None, "busy_timeout", 5000_i64)?;
    Ok(())
}

fn migrate_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS schema_migrations (
            version INTEGER PRIMARY KEY,
            applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );

        CREATE TABLE IF NOT EXISTS files (
            id INTEGER PRIMARY KEY,
            path TEXT NOT NULL UNIQUE,
            kind TEXT NOT NULL,
            size_bytes INTEGER NOT NULL,
            mtime_unix INTEGER NOT NULL,
            sha256 BLOB,
            indexed_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );

        CREATE TABLE IF NOT EXISTS programs (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            file_id INTEGER NOT NULL,
            start_offset INTEGER NOT NULL,
            byte_len INTEGER NOT NULL,
            FOREIGN KEY(file_id) REFERENCES files(id) ON DELETE CASCADE
        );

        CREATE INDEX IF NOT EXISTS idx_programs_name ON programs(name);
        CREATE INDEX IF NOT EXISTS idx_programs_file ON programs(file_id);

        CREATE TABLE IF NOT EXISTS copybook_uses (
            id INTEGER PRIMARY KEY,
            from_file_id INTEGER NOT NULL,
            copybook_name TEXT NOT NULL,
            start_offset INTEGER NOT NULL,
            byte_len INTEGER NOT NULL,
            resolved_file_id INTEGER,
            resolve_status TEXT NOT NULL DEFAULT 'unknown',
            replacing_text TEXT,
            FOREIGN KEY(from_file_id) REFERENCES files(id) ON DELETE CASCADE,
            FOREIGN KEY(resolved_file_id) REFERENCES files(id) ON DELETE SET NULL
        );

        CREATE INDEX IF NOT EXISTS idx_copybook_uses_from_file ON copybook_uses(from_file_id);
        CREATE INDEX IF NOT EXISTS idx_copybook_uses_name ON copybook_uses(copybook_name);

        CREATE TABLE IF NOT EXISTS call_edges (
            id INTEGER PRIMARY KEY,
            caller_program_id INTEGER NOT NULL,
            callee_name TEXT NOT NULL,
            start_offset INTEGER NOT NULL,
            byte_len INTEGER NOT NULL,
            kind TEXT NOT NULL DEFAULT 'static',
            using_count INTEGER NOT NULL DEFAULT 0,
            FOREIGN KEY(caller_program_id) REFERENCES programs(id) ON DELETE CASCADE
        );

        CREATE INDEX IF NOT EXISTS idx_call_edges_caller ON call_edges(caller_program_id);
        CREATE INDEX IF NOT EXISTS idx_call_edges_callee ON call_edges(callee_name);

        CREATE TABLE IF NOT EXISTS data_items (
            id INTEGER PRIMARY KEY,
            program_id INTEGER NOT NULL,
            source_file_id INTEGER,
            name TEXT NOT NULL,
            level INTEGER NOT NULL,
            parent_name TEXT,
            pic TEXT,
            usage_clause TEXT,
            occurs INTEGER,
            redefines TEXT,
            section TEXT,
            byte_offset INTEGER,
            byte_size INTEGER,
            storage_kind TEXT,
            layout_status TEXT,
            start_offset INTEGER NOT NULL,
            byte_len INTEGER NOT NULL,
            FOREIGN KEY(program_id) REFERENCES programs(id) ON DELETE CASCADE,
            FOREIGN KEY(source_file_id) REFERENCES files(id) ON DELETE SET NULL
        );

        CREATE INDEX IF NOT EXISTS idx_data_items_program ON data_items(program_id);
        CREATE INDEX IF NOT EXISTS idx_data_items_source_file ON data_items(source_file_id);
        CREATE INDEX IF NOT EXISTS idx_data_items_name ON data_items(name);

        CREATE TABLE IF NOT EXISTS program_features (
            program_id INTEGER PRIMARY KEY,
            source_file_id INTEGER NOT NULL,
            incoming_call_count INTEGER NOT NULL DEFAULT 0,
            outgoing_call_count INTEGER NOT NULL DEFAULT 0,
            static_call_count INTEGER NOT NULL DEFAULT 0,
            dynamic_call_count INTEGER NOT NULL DEFAULT 0,
            copybook_use_count INTEGER NOT NULL DEFAULT 0,
            distinct_copybook_count INTEGER NOT NULL DEFAULT 0,
            referenced_by_file_count INTEGER NOT NULL DEFAULT 0,
            is_entrypoint INTEGER NOT NULL DEFAULT 0,
            has_heavy_copy_usage INTEGER NOT NULL DEFAULT 0,
            data_item_count INTEGER NOT NULL DEFAULT 0,
            paragraph_count INTEGER NOT NULL DEFAULT 0,
            external_op_count INTEGER NOT NULL DEFAULT 0,
            identifier_count INTEGER NOT NULL DEFAULT 0,
            literal_count INTEGER NOT NULL DEFAULT 0,
            FOREIGN KEY(program_id) REFERENCES programs(id) ON DELETE CASCADE,
            FOREIGN KEY(source_file_id) REFERENCES files(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS code_blocks (
            id INTEGER PRIMARY KEY,
            program_id INTEGER NOT NULL,
            source_file_id INTEGER NOT NULL,
            name TEXT NOT NULL,
            kind TEXT NOT NULL,
            parent_section TEXT,
            sequence_no INTEGER NOT NULL,
            statement_count INTEGER NOT NULL DEFAULT 0,
            start_offset INTEGER NOT NULL,
            byte_len INTEGER NOT NULL,
            FOREIGN KEY(program_id) REFERENCES programs(id) ON DELETE CASCADE,
            FOREIGN KEY(source_file_id) REFERENCES files(id) ON DELETE CASCADE
        );

        CREATE INDEX IF NOT EXISTS idx_code_blocks_program ON code_blocks(program_id);
        CREATE INDEX IF NOT EXISTS idx_code_blocks_kind ON code_blocks(kind);

        CREATE TABLE IF NOT EXISTS external_ops (
            id INTEGER PRIMARY KEY,
            program_id INTEGER NOT NULL,
            source_file_id INTEGER NOT NULL,
            kind TEXT NOT NULL,
            verb TEXT NOT NULL,
            target TEXT,
            start_offset INTEGER NOT NULL,
            byte_len INTEGER NOT NULL,
            FOREIGN KEY(program_id) REFERENCES programs(id) ON DELETE CASCADE,
            FOREIGN KEY(source_file_id) REFERENCES files(id) ON DELETE CASCADE
        );

        CREATE INDEX IF NOT EXISTS idx_external_ops_program ON external_ops(program_id);
        CREATE INDEX IF NOT EXISTS idx_external_ops_kind ON external_ops(kind);
        CREATE INDEX IF NOT EXISTS idx_external_ops_target ON external_ops(target);

        CREATE TABLE IF NOT EXISTS identifiers (
            id INTEGER PRIMARY KEY,
            program_id INTEGER NOT NULL,
            source_file_id INTEGER NOT NULL,
            kind TEXT NOT NULL,
            value TEXT NOT NULL,
            occurrences INTEGER NOT NULL DEFAULT 1,
            first_offset INTEGER NOT NULL,
            FOREIGN KEY(program_id) REFERENCES programs(id) ON DELETE CASCADE,
            FOREIGN KEY(source_file_id) REFERENCES files(id) ON DELETE CASCADE
        );

        CREATE INDEX IF NOT EXISTS idx_identifiers_program ON identifiers(program_id);
        CREATE INDEX IF NOT EXISTS idx_identifiers_kind_value ON identifiers(kind, value);

        CREATE TABLE IF NOT EXISTS literals (
            id INTEGER PRIMARY KEY,
            program_id INTEGER NOT NULL,
            source_file_id INTEGER NOT NULL,
            kind TEXT NOT NULL,
            value TEXT NOT NULL,
            occurrences INTEGER NOT NULL DEFAULT 1,
            first_offset INTEGER NOT NULL,
            FOREIGN KEY(program_id) REFERENCES programs(id) ON DELETE CASCADE,
            FOREIGN KEY(source_file_id) REFERENCES files(id) ON DELETE CASCADE
        );

        CREATE INDEX IF NOT EXISTS idx_literals_program ON literals(program_id);
        CREATE INDEX IF NOT EXISTS idx_literals_kind_value ON literals(kind, value);

        CREATE TABLE IF NOT EXISTS copybook_features (
            copybook_file_id INTEGER PRIMARY KEY,
            copybook_name TEXT NOT NULL,
            used_by_program_count INTEGER NOT NULL DEFAULT 0,
            used_by_file_count INTEGER NOT NULL DEFAULT 0,
            replacing_use_count INTEGER NOT NULL DEFAULT 0,
            data_item_count INTEGER NOT NULL DEFAULT 0,
            contains_header_fields INTEGER NOT NULL DEFAULT 0,
            contains_error_fields INTEGER NOT NULL DEFAULT 0,
            FOREIGN KEY(copybook_file_id) REFERENCES files(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS runs (
            id TEXT PRIMARY KEY,
            started_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
            status TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS run_events (
            id INTEGER PRIMARY KEY,
            run_id TEXT NOT NULL,
            seq INTEGER NOT NULL,
            kind TEXT NOT NULL,
            payload_json TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
            FOREIGN KEY(run_id) REFERENCES runs(id) ON DELETE CASCADE,
            UNIQUE(run_id, seq)
        );

        CREATE TABLE IF NOT EXISTS skills (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            version TEXT NOT NULL,
            path TEXT NOT NULL UNIQUE,
            sha256 BLOB NOT NULL,
            tags_json TEXT NOT NULL
        );

        INSERT OR IGNORE INTO schema_migrations(version) VALUES (1);
        "#,
    )?;

    ensure_column(
        conn,
        "copybook_uses",
        "resolve_status",
        "ALTER TABLE copybook_uses ADD COLUMN resolve_status TEXT NOT NULL DEFAULT 'unknown'",
    )?;
    ensure_column(
        conn,
        "copybook_uses",
        "replacing_text",
        "ALTER TABLE copybook_uses ADD COLUMN replacing_text TEXT",
    )?;
    ensure_column(
        conn,
        "call_edges",
        "kind",
        "ALTER TABLE call_edges ADD COLUMN kind TEXT NOT NULL DEFAULT 'static'",
    )?;
    ensure_column(
        conn,
        "call_edges",
        "using_count",
        "ALTER TABLE call_edges ADD COLUMN using_count INTEGER NOT NULL DEFAULT 0",
    )?;
    ensure_column(
        conn,
        "data_items",
        "source_file_id",
        "ALTER TABLE data_items ADD COLUMN source_file_id INTEGER",
    )?;
    ensure_column(
        conn,
        "data_items",
        "parent_name",
        "ALTER TABLE data_items ADD COLUMN parent_name TEXT",
    )?;
    ensure_column(
        conn,
        "data_items",
        "occurs",
        "ALTER TABLE data_items ADD COLUMN occurs INTEGER",
    )?;
    ensure_column(
        conn,
        "data_items",
        "redefines",
        "ALTER TABLE data_items ADD COLUMN redefines TEXT",
    )?;
    ensure_column(
        conn,
        "data_items",
        "section",
        "ALTER TABLE data_items ADD COLUMN section TEXT",
    )?;
    ensure_column(
        conn,
        "data_items",
        "byte_offset",
        "ALTER TABLE data_items ADD COLUMN byte_offset INTEGER",
    )?;
    ensure_column(
        conn,
        "data_items",
        "byte_size",
        "ALTER TABLE data_items ADD COLUMN byte_size INTEGER",
    )?;
    ensure_column(
        conn,
        "data_items",
        "storage_kind",
        "ALTER TABLE data_items ADD COLUMN storage_kind TEXT",
    )?;
    ensure_column(
        conn,
        "data_items",
        "layout_status",
        "ALTER TABLE data_items ADD COLUMN layout_status TEXT",
    )?;

    Ok(())
}

fn ensure_column(
    conn: &Connection,
    table: &str,
    column: &str,
    alter_sql: &str,
) -> rusqlite::Result<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({})", table))?;
    let mut rows = stmt.query([])?;

    while let Some(row) = rows.next()? {
        let name: String = row.get(1)?;
        if name == column {
            return Ok(());
        }
    }

    conn.execute_batch(alter_sql)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_query_readonly_safety() {
        let temp_dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::open_or_create(temp_dir.path()).unwrap();

        // 1. Valid read-only query
        let res = store.query_readonly("SELECT * FROM files");
        assert!(res.is_ok());
        let val = res.unwrap();
        assert!(val.is_array());

        // 2. Reject modifying queries
        let res_insert = store.query_readonly("INSERT INTO files (path, kind, size_bytes, mtime_unix) VALUES ('test.cob', 'source', 10, 0)");
        assert!(res_insert.is_err());
        assert!(
            res_insert
                .unwrap_err()
                .to_string()
                .contains("Only SELECT queries are allowed")
        );

        let res_drop = store.query_readonly("DROP TABLE files");
        assert!(res_drop.is_err());
        assert!(
            res_drop
                .unwrap_err()
                .to_string()
                .contains("Only SELECT queries are allowed")
        );
    }
}
