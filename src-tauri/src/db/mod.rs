use crate::error::AppError;
use crate::models::{Build, RecentProject};
use crate::Result;
use chrono::{DateTime, Utc};
use rusqlite::Connection;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Builds 缓存的 SQLite 持久化（docs/01 §6.1：Rust 内存 + SQLite）。
/// 重启后从库里恢复版本列表，无需重新爬取。
/// Connection 非线程安全（!Sync），用 Mutex 包装以放入 AppState。
pub struct Db {
    conn: Mutex<Connection>,
}

impl Db {
    /// 打开（必要时创建）数据库文件并初始化 schema。
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(AppError::from)?;
        }
        let conn =
            Connection::open(path).map_err(|err| AppError::DatabaseError(err.to_string()))?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(|err| AppError::DatabaseError(err.to_string()))?;
        let db = Self {
            conn: Mutex::new(conn),
        };
        db.init_schema()?;
        Ok(db)
    }

    fn init_schema(&self) -> Result<()> {
        self.conn
            .lock()
            .unwrap()
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS builds (
                    id         TEXT PRIMARY KEY,
                    payload    TEXT NOT NULL,
                    fetched_at INTEGER NOT NULL
                );
                CREATE TABLE IF NOT EXISTS recent_projects (
                    blend_file  TEXT PRIMARY KEY,
                    build_id    TEXT NOT NULL,
                    last_opened INTEGER NOT NULL
                );
                CREATE TABLE IF NOT EXISTS favorites (
                    build_id TEXT PRIMARY KEY,
                    added_at INTEGER NOT NULL
                );",
            )
            .map_err(|err| AppError::DatabaseError(err.to_string()))?;
        Ok(())
    }

    /// 收藏的版本 id 列表（最新在前）。
    pub fn favorite_ids(&self) -> Result<Vec<String>> {
        let conn = self.conn.lock().unwrap();
        let mut statement = conn
            .prepare("SELECT build_id FROM favorites ORDER BY added_at DESC")
            .map_err(|err| AppError::DatabaseError(err.to_string()))?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|err| AppError::DatabaseError(err.to_string()))?;
        let mut ids = Vec::new();
        for row in rows {
            ids.push(row.map_err(|err| AppError::DatabaseError(err.to_string()))?);
        }
        Ok(ids)
    }

    /// 加/取消收藏。
    pub fn set_favorite(&self, build_id: &str, favorite: bool) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        if favorite {
            conn.execute(
                "INSERT OR IGNORE INTO favorites (build_id, added_at) VALUES (?1, ?2)",
                rusqlite::params![build_id, Utc::now().timestamp()],
            )
        } else {
            conn.execute("DELETE FROM favorites WHERE build_id = ?1", [build_id])
        }
        .map_err(|err| AppError::DatabaseError(err.to_string()))?;
        Ok(())
    }

    /// 记录一个最近打开的 .blend（同文件更新时间为准，docs/01 §3.3）。
    pub fn record_recent_project(&self, blend_file: &str, build_id: &str) -> Result<()> {
        self.conn
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO recent_projects (blend_file, build_id, last_opened)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(blend_file) DO UPDATE SET
                     build_id = excluded.build_id,
                     last_opened = excluded.last_opened",
                rusqlite::params![blend_file, build_id, Utc::now().timestamp()],
            )
            .map_err(|err| AppError::DatabaseError(err.to_string()))?;
        Ok(())
    }

    /// 最近项目，最新在前，最多 `limit` 条。
    pub fn recent_projects(&self, limit: usize) -> Result<Vec<RecentProject>> {
        let conn = self.conn.lock().unwrap();
        let mut statement = conn
            .prepare(
                "SELECT blend_file, build_id, last_opened
                 FROM recent_projects
                 ORDER BY last_opened DESC
                 LIMIT ?1",
            )
            .map_err(|err| AppError::DatabaseError(err.to_string()))?;
        let rows = statement
            .query_map([limit as i64], |row| {
                Ok(RecentProject {
                    blend_file: row.get(0)?,
                    build_id: row.get(1)?,
                    last_opened: row.get(2)?,
                })
            })
            .map_err(|err| AppError::DatabaseError(err.to_string()))?;

        let mut projects = Vec::new();
        for row in rows {
            projects.push(row.map_err(|err| AppError::DatabaseError(err.to_string()))?);
        }
        Ok(projects)
    }

    /// 文件已不存在时清理条目。
    pub fn forget_recent_project(&self, blend_file: &str) -> Result<()> {
        self.conn
            .lock()
            .unwrap()
            .execute(
                "DELETE FROM recent_projects WHERE blend_file = ?1",
                [blend_file],
            )
            .map_err(|err| AppError::DatabaseError(err.to_string()))?;
        Ok(())
    }

    /// 读取缓存的版本列表与最近一次爬取时间。
    pub fn load_builds(&self) -> Result<(Vec<Build>, Option<DateTime<Utc>>)> {
        let conn = self.conn.lock().unwrap();
        let mut statement = conn
            .prepare("SELECT payload, fetched_at FROM builds")
            .map_err(|err| AppError::DatabaseError(err.to_string()))?;
        let mut builds = Vec::new();
        let mut latest: Option<DateTime<Utc>> = None;
        let rows = statement
            .query_map([], |row| {
                let payload: String = row.get(0)?;
                let fetched_at: i64 = row.get(1)?;
                Ok((payload, fetched_at))
            })
            .map_err(|err| AppError::DatabaseError(err.to_string()))?;

        for row in rows {
            let (payload, fetched_at) =
                row.map_err(|err| AppError::DatabaseError(err.to_string()))?;
            let build: Build =
                serde_json::from_str(&payload).map_err(AppError::SerializationError)?;
            let time = DateTime::from_timestamp(fetched_at, 0);
            if latest.is_none_or(|current| time.is_some_and(|time| time > current)) {
                latest = time;
            }
            builds.push(build);
        }
        Ok((builds, latest))
    }

    /// 全量替换缓存（含 is_installed 状态，见 library 扫描）。
    pub fn save_builds(&self, builds: &[Build]) -> Result<()> {
        let fetched_at = Utc::now().timestamp();
        let mut conn = self.conn.lock().unwrap();
        let tx = conn
            .transaction()
            .map_err(|err| AppError::DatabaseError(err.to_string()))?;
        tx.execute("DELETE FROM builds", [])
            .map_err(|err| AppError::DatabaseError(err.to_string()))?;
        for build in builds {
            let payload = serde_json::to_string(build).map_err(AppError::SerializationError)?;
            tx.execute(
                "INSERT INTO builds (id, payload, fetched_at) VALUES (?1, ?2, ?3)",
                rusqlite::params![build.id, payload, fetched_at],
            )
            .map_err(|err| AppError::DatabaseError(err.to_string()))?;
        }
        tx.commit()
            .map_err(|err| AppError::DatabaseError(err.to_string()))?;
        Ok(())
    }
}

/// 默认数据库路径：`<cache_dir>/blender-launcher.db`。
pub fn default_db_path(cache_dir: &Path) -> PathBuf {
    cache_dir.join("blender-launcher.db")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Architecture, BuildBranch, BuildFork, Platform};
    use std::path::PathBuf;

    fn sample_build(id: &str, installed: bool) -> Build {
        Build {
            id: id.to_string(),
            version: "4.5.0".into(),
            branch: BuildBranch::Stable,
            fork: BuildFork::Official,
            platform: Platform::Windows,
            architecture: Architecture::X64,
            url: format!("https://example.com/{id}.zip"),
            size_bytes: 123,
            release_date: Utc::now(),
            release_notes_url: None,
            breaking_changes: None,
            is_installed: installed,
            install_path: None,
            update_build_id: None,
        }
    }

    #[tokio::test]
    async fn test_roundtrip() {
        let dir = std::env::temp_dir().join(format!("bl-db-{}", uuid::Uuid::new_v4()));
        let path = default_db_path(&dir);
        let db = Db::open(&path).unwrap();

        let builds = vec![sample_build("a", false), sample_build("b", true)];
        db.save_builds(&builds).unwrap();

        let (loaded, last) = Db::open(&path).unwrap().load_builds().unwrap();
        assert_eq!(loaded.len(), 2);
        assert!(loaded.iter().any(|b| b.id == "b" && b.is_installed));
        assert!(last.is_some());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn test_default_db_path() {
        assert_eq!(
            default_db_path(Path::new("C:/cache")),
            PathBuf::from("C:/cache/blender-launcher.db")
        );
    }
}
