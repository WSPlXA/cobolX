use rusqlite::{Connection, OpenFlags};
use std::error::Error;
use std::path::{Path, PathBuf};

type StoreResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

#[allow(dead_code)]
pub struct MemoryPaths {
    pub root: PathBuf,
    pub base_dir: PathBuf,
    pub db_path: PathBuf,
    pub blobs_dir: PathBuf,
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
        let memory_dir = base_dir.join("memory");

        Self {
            root,
            base_dir: base_dir.clone(),
            db_path: memory_dir.join("project.db"),
            blobs_dir: base_dir.join("blobs"),
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

    pub fn connection(&self) -> &Connection {
        &self.conn
    }
}

fn create_dirs(paths: &MemoryPaths) -> StoreResult<()> {
    std::fs::create_dir_all(paths.db_path.parent().unwrap())?;
    std::fs::create_dir_all(&paths.blobs_dir)?;
    std::fs::create_dir_all(&paths.runs_dir)?;
    std::fs::create_dir_all(&paths.skills_dir)?;
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
            FOREIGN KEY(caller_program_id) REFERENCES programs(id) ON DELETE CASCADE
        );

        CREATE INDEX IF NOT EXISTS idx_call_edges_caller ON call_edges(caller_program_id);
        CREATE INDEX IF NOT EXISTS idx_call_edges_callee ON call_edges(callee_name);

        CREATE TABLE IF NOT EXISTS data_items (
            id INTEGER PRIMARY KEY,
            program_id INTEGER NOT NULL,
            name TEXT NOT NULL,
            level INTEGER NOT NULL,
            pic TEXT,
            usage_clause TEXT,
            start_offset INTEGER NOT NULL,
            byte_len INTEGER NOT NULL,
            FOREIGN KEY(program_id) REFERENCES programs(id) ON DELETE CASCADE
        );

        CREATE INDEX IF NOT EXISTS idx_data_items_program ON data_items(program_id);
        CREATE INDEX IF NOT EXISTS idx_data_items_name ON data_items(name);

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
    )
}
