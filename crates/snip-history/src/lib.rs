//! History storage: SQLite database + PNG file storage + JPEG thumbnails.

use anyhow::Result;
use chrono::Utc;
use ns_common::history::HistoryEntry;
use ns_common::paths;
use ns_common::PixelBuffer;
use rusqlite::{params, Connection};
use std::path::PathBuf;
use std::sync::mpsc;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

/// Commands sent to the background writer thread.
enum WriterCommand {
    Save {
        entry: HistoryEntry,
        pixels: PixelBuffer,
    },
    Delete(Uuid),
    Cleanup,
    Shutdown,
}

/// Handle to the history system. All writes are async via mpsc channel.
pub struct HistoryStore {
    conn: Connection,
    writer_tx: mpsc::Sender<WriterCommand>,
    writer_thread: Option<std::thread::JoinHandle<()>>,
}

impl HistoryStore {
    /// Open or create the history database and start the background writer.
    pub fn new() -> Result<Self> {
        let db_path = paths::history_db();
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let conn = Connection::open(&db_path)?;
        Self::init_schema(&conn)?;

        let (tx, rx) = mpsc::channel::<WriterCommand>();

        let writer_db_path = db_path.clone();
        let writer_thread = std::thread::Builder::new()
            .name("history-writer".into())
            .spawn(move || {
                Self::writer_loop(writer_db_path, rx);
            })?;

        info!("History store opened");

        Ok(Self {
            conn,
            writer_tx: tx,
            writer_thread: Some(writer_thread),
        })
    }

    fn init_schema(conn: &Connection) -> Result<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS captures (
                id TEXT PRIMARY KEY,
                timestamp_ms INTEGER NOT NULL,
                mode TEXT NOT NULL,
                region_x INTEGER NOT NULL,
                region_y INTEGER NOT NULL,
                region_w INTEGER NOT NULL,
                region_h INTEGER NOT NULL,
                file_path TEXT NOT NULL,
                file_size INTEGER NOT NULL,
                width INTEGER NOT NULL,
                height INTEGER NOT NULL,
                ocr_text TEXT,
                annotated INTEGER NOT NULL DEFAULT 0,
                thumbnail BLOB
            );
            CREATE INDEX IF NOT EXISTS idx_captures_timestamp ON captures(timestamp_ms DESC);",
        )?;
        Ok(())
    }

    /// Queue a capture for async saving (never blocks the capture hot path).
    pub fn save_async(&self, entry: HistoryEntry, pixels: PixelBuffer) {
        if let Err(e) = self.writer_tx.send(WriterCommand::Save { entry, pixels }) {
            error!("Failed to send to history writer: {e}");
        }
    }

    /// Delete a history entry.
    pub fn delete_async(&self, id: Uuid) {
        if let Err(e) = self.writer_tx.send(WriterCommand::Delete(id)) {
            error!("Failed to send delete to history writer: {e}");
        }
    }

    /// Trigger retention cleanup.
    pub fn cleanup_async(&self) {
        let _ = self.writer_tx.send(WriterCommand::Cleanup);
    }

    /// Query history entries (synchronous, for UI).
    pub fn query(
        &self,
        offset: u32,
        limit: u32,
        search: Option<&str>,
    ) -> Result<(Vec<HistoryEntry>, u32)> {
        let (where_clause, search_param) = if let Some(q) = search {
            ("WHERE (file_path LIKE ?1 OR mode LIKE ?1 OR ocr_text LIKE ?1)".to_string(), Some(format!("%{q}%")))
        } else {
            (String::new(), None)
        };

        let count_sql = format!("SELECT COUNT(*) FROM captures {where_clause}");
        let total: u32 = if let Some(ref p) = search_param {
            self.conn.query_row(&count_sql, params![p], |row| row.get(0))?
        } else {
            self.conn.query_row(&count_sql, [], |row| row.get(0))?
        };

        let query_sql = format!(
            "SELECT id, timestamp_ms, mode, region_x, region_y, region_w, region_h,
                    file_path, file_size, width, height, ocr_text, annotated
             FROM captures {where_clause}
             ORDER BY timestamp_ms DESC
             LIMIT ?{limit_p} OFFSET ?{offset_p}",
            limit_p = if search_param.is_some() { "2" } else { "1" },
            offset_p = if search_param.is_some() { "3" } else { "2" },
        );

        let mut stmt = self.conn.prepare(&query_sql)?;
        let rows = if let Some(ref p) = search_param {
            stmt.query_map(params![p, limit, offset], Self::row_to_entry)?
        } else {
            stmt.query_map(params![limit, offset], Self::row_to_entry)?
        };

        let entries: Vec<HistoryEntry> = rows.filter_map(|r| r.ok()).collect();
        Ok((entries, total))
    }

    /// Get thumbnail BLOB for a given entry ID.
    pub fn get_thumbnail(&self, id: &Uuid) -> Result<Option<Vec<u8>>> {
        let mut stmt = self
            .conn
            .prepare("SELECT thumbnail FROM captures WHERE id = ?1")?;
        let result = stmt.query_row(params![id.to_string()], |row| {
            row.get::<_, Option<Vec<u8>>>(0)
        })?;
        Ok(result)
    }

    fn row_to_entry(row: &rusqlite::Row) -> rusqlite::Result<HistoryEntry> {
        let id_str: String = row.get(0)?;
        let mode_str: String = row.get(2)?;
        Ok(HistoryEntry {
            id: Uuid::parse_str(&id_str).unwrap_or_default(),
            timestamp_ms: row.get(1)?,
            mode: serde_json::from_str(&format!("\"{mode_str}\"")).unwrap_or(ns_common::CaptureMode::Region),
            region: ns_common::Rect::new(row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?),
            file_path: row.get(7)?,
            file_size: row.get(8)?,
            width: row.get(9)?,
            height: row.get(10)?,
            ocr_text: row.get(11)?,
            annotated: row.get::<_, i32>(12)? != 0,
        })
    }

    fn writer_loop(db_path: PathBuf, rx: mpsc::Receiver<WriterCommand>) {
        let conn = match Connection::open(&db_path) {
            Ok(c) => c,
            Err(e) => {
                error!("History writer failed to open DB: {e}");
                return;
            }
        };

        for cmd in rx {
            match cmd {
                WriterCommand::Save { entry, pixels } => {
                    if let Err(e) = Self::do_save(&conn, &entry, &pixels) {
                        error!("Failed to save capture: {e}");
                    }
                }
                WriterCommand::Delete(id) => {
                    if let Err(e) = Self::do_delete(&conn, &id) {
                        error!("Failed to delete entry: {e}");
                    }
                }
                WriterCommand::Cleanup => {
                    if let Err(e) = Self::do_cleanup(&conn) {
                        error!("Retention cleanup failed: {e}");
                    }
                }
                WriterCommand::Shutdown => break,
            }
        }

        debug!("History writer thread exiting");
    }

    fn do_save(conn: &Connection, entry: &HistoryEntry, pixels: &PixelBuffer) -> Result<()> {
        // Save PNG file
        let captures_dir = paths::captures_dir();
        let now = Utc::now();
        let dir = captures_dir
            .join(now.format("%Y").to_string())
            .join(now.format("%m").to_string());
        std::fs::create_dir_all(&dir)?;

        let png_path = dir.join(format!("{}.png", entry.id));

        // Encode BGRA to PNG
        let img = image::RgbaImage::from_fn(pixels.width, pixels.height, |x, y| {
            let offset = (y * pixels.stride + x * 4) as usize;
            if offset + 3 < pixels.data.len() {
                let b = pixels.data[offset];
                let g = pixels.data[offset + 1];
                let r = pixels.data[offset + 2];
                let a = pixels.data[offset + 3];
                image::Rgba([r, g, b, a])
            } else {
                image::Rgba([0, 0, 0, 255])
            }
        });
        img.save(&png_path)?;

        let file_size = std::fs::metadata(&png_path)?.len();

        // Generate thumbnail (200px wide)
        let thumb = image::imageops::resize(
            &img,
            200,
            (200.0 * pixels.height as f32 / pixels.width as f32) as u32,
            image::imageops::FilterType::Triangle,
        );
        let thumb_rgb = image::DynamicImage::ImageRgba8(thumb).to_rgb8();
        let mut thumb_bytes = Vec::new();
        let mut cursor = std::io::Cursor::new(&mut thumb_bytes);
        thumb_rgb.write_to(&mut cursor, image::ImageFormat::Jpeg)?;

        // Relative path from captures dir
        let rel_path = png_path
            .strip_prefix(&captures_dir)
            .unwrap_or(&png_path)
            .to_string_lossy()
            .to_string();

        let mode_str = serde_json::to_string(&entry.mode)?;
        let mode_str = mode_str.trim_matches('"');

        conn.execute(
            "INSERT INTO captures (id, timestamp_ms, mode, region_x, region_y, region_w, region_h,
                                   file_path, file_size, width, height, ocr_text, annotated, thumbnail)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                entry.id.to_string(),
                entry.timestamp_ms,
                mode_str,
                entry.region.x,
                entry.region.y,
                entry.region.width,
                entry.region.height,
                rel_path,
                file_size,
                entry.width,
                entry.height,
                entry.ocr_text,
                entry.annotated as i32,
                thumb_bytes,
            ],
        )?;

        info!("Saved capture {}", entry.id);
        Ok(())
    }

    fn do_delete(conn: &Connection, id: &Uuid) -> Result<()> {
        // Get file path before deleting
        let file_path: Option<String> = conn
            .query_row(
                "SELECT file_path FROM captures WHERE id = ?1",
                params![id.to_string()],
                |row| row.get(0),
            )
            .ok();

        conn.execute(
            "DELETE FROM captures WHERE id = ?1",
            params![id.to_string()],
        )?;

        // Delete the PNG file (with path traversal protection)
        if let Some(path) = file_path {
            safe_remove_in_captures_dir(&path);
        }

        info!("Deleted capture {id}");
        Ok(())
    }

    fn do_cleanup(conn: &Connection) -> Result<()> {
        let config = ns_common::config::NsConfig::load();
        let retention = &config.retention;

        if retention.max_age_days > 0 {
            let cutoff_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as i64
                - (retention.max_age_days as i64 * 86_400_000);

            let paths: Vec<String> = {
                let mut stmt = conn.prepare(
                    "SELECT file_path FROM captures WHERE timestamp_ms < ?1",
                )?;
                let rows = stmt.query_map(params![cutoff_ms], |row| row.get(0))?;
                rows.filter_map(|r| r.ok()).collect()
            };

            for path in &paths {
                safe_remove_in_captures_dir(path);
            }

            conn.execute(
                "DELETE FROM captures WHERE timestamp_ms < ?1",
                params![cutoff_ms],
            )?;

            info!("Retention cleanup: removed {} old entries", paths.len());
        }

        // Size-based cleanup
        if retention.max_size_mb > 0 {
            let max_bytes = retention.max_size_mb as i64 * 1024 * 1024;
            let mut stmt = conn.prepare(
                "SELECT id, file_path, file_size FROM captures ORDER BY timestamp_ms DESC",
            )?;
            let rows: Vec<(String, String, i64)> = {
                let mapped = stmt.query_map([], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                })?;
                mapped.filter_map(|r| r.ok()).collect()
            };

            let mut total: i64 = 0;
            let mut to_delete = Vec::new();
            for (id, file_path, file_size) in &rows {
                total += file_size;
                if total > max_bytes {
                    to_delete.push((id.clone(), file_path.clone()));
                }
            }

            for (id, file_path) in &to_delete {
                safe_remove_in_captures_dir(file_path);
                conn.execute("DELETE FROM captures WHERE id = ?1", params![id])?;
            }

            if !to_delete.is_empty() {
                info!("Size retention cleanup: removed {} entries (limit {}MB)", to_delete.len(), retention.max_size_mb);
            }
        }

        Ok(())
    }
}

/// Remove a file only if it resolves to within the captures directory.
/// Prevents path traversal attacks via crafted relative paths in the DB.
fn safe_remove_in_captures_dir(rel_path: &str) {
    let captures_dir = paths::captures_dir();
    let full_path = captures_dir.join(rel_path);
    let Ok(canonical) = full_path.canonicalize() else { return };
    let Ok(base) = captures_dir.canonicalize() else { return };
    if !canonical.starts_with(&base) {
        warn!("Path traversal blocked: refusing to delete outside captures dir");
        return;
    }
    let _ = std::fs::remove_file(&canonical);
}

impl Drop for HistoryStore {
    fn drop(&mut self) {
        let _ = self.writer_tx.send(WriterCommand::Shutdown);
        if let Some(handle) = self.writer_thread.take() {
            let _ = handle.join();
        }
    }
}
