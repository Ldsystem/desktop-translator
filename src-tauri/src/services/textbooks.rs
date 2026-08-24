//! Native-only textbook catalog, validation, and local storage boundary.

use std::{
    collections::{BTreeMap, HashSet},
    fs,
    io::Write,
    path::Path,
    sync::Mutex,
    time::Duration,
};

use opencc_fmmseg::{OpenCC, OpenccConfig};
use reqwest::Url;
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use sha2::{Digest, Sha256};

use crate::contracts::{
    AppError, AppErrorCode, InstalledTextbook, TextbookCatalogItem, TextbookEntry,
    TextbookEntryPage, TextbookPromotionResult, ValidateContract,
};

const MAX_ARTIFACT_BYTES: u64 = 64 * 1024 * 1024;
const MAX_ENTRIES: u64 = 500_000;
const MAX_SOURCE_BYTES: usize = 2_048;
const MAX_TRANSLATION_BYTES: usize = 8_192;
const MAX_PAGE_SIZE: u64 = 500;
const WIKDICT_HOST: &str = "download.wikdict.com";

/// Returns the deliberately small, pinned catalog compiled into the application.
pub fn curated_catalog() -> Vec<TextbookCatalogItem> {
    vec![TextbookCatalogItem {
        id: "wikdict-en-zh-2026-06".into(),
        title: "WikDict English - Chinese".into(),
        source_language: "en".into(),
        target_language: "zh-CN".into(),
        version: "2_2026-06".into(),
        download_url: "https://download.wikdict.com/dictionaries/sqlite/2_2026-06/en-zh.sqlite3"
            .into(),
        expected_bytes: 5_169_152,
        sha256: "16cf69dc8037a8d4dc6bde260142bf0181f9ff0a008d457f26452f1d80ca5ecd".into(),
        license: "CC BY-SA 4.0".into(),
        attribution: "WikDict, Wiktionary and DBnary contributors".into(),
        source_url: "https://www.wikdict.com/page/download".into(),
    }]
}

/// Versioned application-local textbook database. Source artifact paths never cross IPC.
pub struct TextbookStore {
    connection: Mutex<Connection>,
}

impl TextbookStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, AppError> {
        let connection = Connection::open(path).map_err(storage_error)?;
        connection
            .execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")
            .map_err(storage_error)?;
        migrate(&connection)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    /// Validates a pinned WikDict SQLite artifact before atomically replacing a version.
    pub fn install_sqlite(
        &self,
        catalog: &TextbookCatalogItem,
        artifact_path: impl AsRef<Path>,
        now_ms: u64,
    ) -> Result<InstalledTextbook, AppError> {
        validate_catalog(catalog)?;
        let artifact_path = artifact_path.as_ref();
        let metadata = fs::metadata(artifact_path).map_err(|_| invalid_package())?;
        if metadata.len() != catalog.expected_bytes || metadata.len() > MAX_ARTIFACT_BYTES {
            return Err(invalid_package());
        }
        let bytes = fs::read(artifact_path).map_err(|_| invalid_package())?;
        self.install_verified_bytes(catalog, &bytes, now_ms)
    }

    /// Downloads only a compiled catalog item with bounded native networking and staging.
    pub async fn download_and_install(
        &self,
        catalog_id: &str,
        staging_directory: &Path,
        now_ms: u64,
    ) -> Result<InstalledTextbook, AppError> {
        let catalog = curated_catalog()
            .into_iter()
            .find(|item| item.id == catalog_id)
            .ok_or_else(|| internal("Textbook catalog item was not found"))?;
        validate_catalog(&catalog)?;
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(download_error)?;
        let mut response = client
            .get(&catalog.download_url)
            .send()
            .await
            .map_err(download_error)?;
        if !response.status().is_success()
            || response
                .content_length()
                .is_some_and(|length| length > catalog.expected_bytes)
        {
            return Err(download_error_message());
        }
        fs::create_dir_all(staging_directory).map_err(|_| download_error_message())?;
        let mut staged = tempfile::NamedTempFile::new_in(staging_directory)
            .map_err(|_| download_error_message())?;
        let mut received = 0_u64;
        while let Some(chunk) = response.chunk().await.map_err(download_error)? {
            received = received
                .checked_add(chunk.len() as u64)
                .ok_or_else(download_error_message)?;
            if received > catalog.expected_bytes || received > MAX_ARTIFACT_BYTES {
                return Err(invalid_package());
            }
            staged
                .write_all(&chunk)
                .map_err(|_| download_error_message())?;
        }
        if received != catalog.expected_bytes {
            return Err(invalid_package());
        }
        staged.flush().map_err(|_| download_error_message())?;
        staged
            .as_file()
            .sync_all()
            .map_err(|_| download_error_message())?;
        let bytes = fs::read(staged.path()).map_err(|_| download_error_message())?;
        self.install_verified_bytes(&catalog, &bytes, now_ms)
    }

    fn install_verified_bytes(
        &self,
        catalog: &TextbookCatalogItem,
        bytes: &[u8],
        now_ms: u64,
    ) -> Result<InstalledTextbook, AppError> {
        if bytes.len() as u64 != catalog.expected_bytes || bytes.len() as u64 > MAX_ARTIFACT_BYTES {
            return Err(invalid_package());
        }
        let digest = format!("{:x}", Sha256::digest(bytes));
        if digest != catalog.sha256 {
            return Err(invalid_package());
        }
        let mut private_artifact = tempfile::NamedTempFile::new().map_err(|_| invalid_package())?;
        private_artifact
            .write_all(bytes)
            .map_err(|_| invalid_package())?;
        private_artifact.flush().map_err(|_| invalid_package())?;
        let imported = read_wikdict(private_artifact.path())?;
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| internal("Textbook database lock failed"))?;
        let tx = connection.transaction().map_err(storage_error)?;
        let was_active = tx
            .query_row(
                "SELECT active FROM textbooks WHERE id = ?1",
                params![catalog.id],
                |row| row.get::<_, bool>(0),
            )
            .optional()
            .map_err(storage_error)?
            .unwrap_or(false);
        tx.execute(
            "INSERT INTO textbooks (
               id, title, source_language, target_language, version, download_url,
               expected_bytes, sha256, license, attribution, source_url,
               installed_at_epoch_ms, active, entry_count
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
             ON CONFLICT(id) DO UPDATE SET
               title = excluded.title,
               source_language = excluded.source_language,
               target_language = excluded.target_language,
               version = excluded.version,
               download_url = excluded.download_url,
               expected_bytes = excluded.expected_bytes,
               sha256 = excluded.sha256,
               license = excluded.license,
               attribution = excluded.attribution,
               source_url = excluded.source_url,
               installed_at_epoch_ms = excluded.installed_at_epoch_ms,
               entry_count = excluded.entry_count",
            params![
                catalog.id,
                catalog.title,
                catalog.source_language,
                catalog.target_language,
                catalog.version,
                catalog.download_url,
                i64_from_u64(catalog.expected_bytes)?,
                catalog.sha256,
                catalog.license,
                catalog.attribution,
                catalog.source_url,
                i64_from_u64(now_ms)?,
                was_active,
                i64_from_u64(imported.len() as u64)?,
            ],
        )
        .map_err(storage_error)?;
        tx.execute(
            "DELETE FROM textbook_entries WHERE textbook_id = ?1",
            params![catalog.id],
        )
        .map_err(storage_error)?;
        {
            let mut insert = tx
                .prepare(
                    "INSERT INTO textbook_entries (
                       textbook_id, normalized_source, source_text, translated_text,
                       original_translations, source_language, target_language
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                )
                .map_err(storage_error)?;
            let mut insert_alias = tx
                .prepare(
                    "INSERT OR IGNORE INTO textbook_entry_aliases (
                       textbook_entry_id, alias, normalized_alias
                     ) VALUES (?1, ?2, ?3)",
                )
                .map_err(storage_error)?;
            for entry in imported {
                insert
                    .execute(params![
                        catalog.id,
                        entry.normalized_source,
                        entry.source_text,
                        entry.translated_text,
                        entry.original_translations.join(" | "),
                        catalog.source_language,
                        catalog.target_language,
                    ])
                    .map_err(storage_error)?;
                let textbook_entry_id = tx.last_insert_rowid();
                for alias in entry.aliases {
                    insert_alias
                        .execute(params![textbook_entry_id, alias, normalize(&alias)])
                        .map_err(storage_error)?;
                }
            }
        }
        tx.commit().map_err(storage_error)?;
        drop(connection);
        self.get_installed(&catalog.id)?
            .ok_or_else(|| storage_error(rusqlite::Error::QueryReturnedNoRows))
    }

    pub fn list_installed(&self) -> Result<Vec<InstalledTextbook>, AppError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| internal("Textbook database lock failed"))?;
        let mut statement = connection
            .prepare(
                "SELECT id, title, source_language, target_language, version, license,
                        attribution, source_url, entry_count, installed_at_epoch_ms, active
                 FROM textbooks ORDER BY title COLLATE NOCASE, id",
            )
            .map_err(storage_error)?;
        let rows = statement
            .query_map([], row_to_installed)
            .map_err(storage_error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(storage_error)
    }

    pub fn get_installed(&self, id: &str) -> Result<Option<InstalledTextbook>, AppError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| internal("Textbook database lock failed"))?;
        connection
            .query_row(
                "SELECT id, title, source_language, target_language, version, license,
                        attribution, source_url, entry_count, installed_at_epoch_ms, active
                 FROM textbooks WHERE id = ?1",
                params![id],
                row_to_installed,
            )
            .optional()
            .map_err(storage_error)
    }

    pub fn set_active(&self, id: Option<&str>) -> Result<(), AppError> {
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| internal("Textbook database lock failed"))?;
        let tx = connection.transaction().map_err(storage_error)?;
        tx.execute("UPDATE textbooks SET active = 0 WHERE active = 1", [])
            .map_err(storage_error)?;
        if let Some(id) = id {
            let changed = tx
                .execute("UPDATE textbooks SET active = 1 WHERE id = ?1", params![id])
                .map_err(storage_error)?;
            if changed != 1 {
                return Err(internal("Textbook was not found"));
            }
        }
        tx.commit().map_err(storage_error)
    }

    pub fn remove(&self, id: &str) -> Result<(), AppError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| internal("Textbook database lock failed"))?;
        connection
            .execute("DELETE FROM textbooks WHERE id = ?1", params![id])
            .map_err(storage_error)?;
        Ok(())
    }

    pub fn list_entries(
        &self,
        textbook_id: &str,
        search: Option<&str>,
        offset: u64,
        limit: u64,
    ) -> Result<TextbookEntryPage, AppError> {
        if limit == 0 || limit > MAX_PAGE_SIZE || offset > 9_007_199_254_740_991 {
            return Err(internal("Textbook page is outside the allowed range"));
        }
        let search = search
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| format!("%{}%", escape_like(&normalize(value))));
        let connection = self
            .connection
            .lock()
            .map_err(|_| internal("Textbook database lock failed"))?;
        let total = connection
            .query_row(
                "SELECT count(*) FROM textbook_entries e
                 WHERE textbook_id = ?1 AND
                       (?2 IS NULL OR normalized_source LIKE ?2 ESCAPE '\\'
                        OR lower(translated_text) LIKE ?2 ESCAPE '\\'
                        OR EXISTS (
                          SELECT 1 FROM textbook_entry_aliases a
                          WHERE a.textbook_entry_id = e.id
                            AND a.normalized_alias LIKE ?2 ESCAPE '\\'
                        ))",
                params![textbook_id, search],
                |row| row.get::<_, i64>(0),
            )
            .map_err(storage_error)? as u64;
        let mut statement = connection
            .prepare(
                "SELECT id, textbook_id, source_text, translated_text, phonetic_symbols,
                        source_language, target_language
                 FROM textbook_entries
                 WHERE textbook_id = ?1 AND
                       (?2 IS NULL OR normalized_source LIKE ?2 ESCAPE '\\'
                        OR lower(translated_text) LIKE ?2 ESCAPE '\\'
                        OR EXISTS (
                          SELECT 1 FROM textbook_entry_aliases a
                          WHERE a.textbook_entry_id = textbook_entries.id
                            AND a.normalized_alias LIKE ?2 ESCAPE '\\'
                        ))
                 ORDER BY normalized_source, id LIMIT ?3 OFFSET ?4",
            )
            .map_err(storage_error)?;
        let rows = statement
            .query_map(
                params![
                    textbook_id,
                    search,
                    i64_from_u64(limit)?,
                    i64_from_u64(offset)?
                ],
                |row| {
                    Ok(TextbookEntry {
                        id: row.get(0)?,
                        textbook_id: row.get(1)?,
                        source_text: row.get(2)?,
                        translated_text: row.get(3)?,
                        phonetic_symbols: row.get(4)?,
                        source_language: row.get(5)?,
                        target_language: row.get(6)?,
                    })
                },
            )
            .map_err(storage_error)?;
        Ok(TextbookEntryPage {
            entries: rows.collect::<Result<Vec<_>, _>>().map_err(storage_error)?,
            total,
            offset,
            limit,
        })
    }

    pub fn promote_entry(
        &self,
        textbook_entry_id: i64,
        now_ms: u64,
    ) -> Result<TextbookPromotionResult, AppError> {
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| internal("Textbook database lock failed"))?;
        let tx = connection.transaction().map_err(storage_error)?;
        let entry = tx
            .query_row(
                "SELECT e.textbook_id, e.normalized_source, e.source_text, e.translated_text,
                        e.original_translations, e.source_language, e.target_language, t.title,
                        t.version, t.license, t.attribution, t.source_url
                 FROM textbook_entries e JOIN textbooks t ON t.id = e.textbook_id
                 WHERE e.id = ?1",
                params![textbook_entry_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, String>(7)?,
                        row.get::<_, String>(8)?,
                        row.get::<_, String>(9)?,
                        row.get::<_, String>(10)?,
                        row.get::<_, String>(11)?,
                    ))
                },
            )
            .optional()
            .map_err(storage_error)?
            .ok_or_else(|| internal("Textbook entry was not found"))?;
        let (
            book_id,
            normalized_source,
            source_text,
            translated_text,
            original_translations,
            source_language,
            target_language,
            title,
            version,
            license,
            attribution,
            source_url,
        ) = entry;
        let existing_id = tx
            .query_row(
                "SELECT id FROM vocabulary_entries
             WHERE normalized_text = ?1 AND target_language = ?2
               AND requested_source_language IN ('auto', ?3)
             ORDER BY CASE requested_source_language WHEN 'auto' THEN 0 ELSE 1 END, id
             LIMIT 1",
                params![normalized_source, target_language, source_language],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(storage_error)?;
        let changed = if existing_id.is_none() {
            tx.execute(
                "INSERT INTO vocabulary_entries (
               normalized_text, source_text, requested_source_language, target_language,
               translated_text, detected_source_language, effective_source_language,
               lookup_count, first_seen_epoch_ms, last_seen_epoch_ms
             ) VALUES (?1, ?2, 'auto', ?3, ?4, ?5, ?5, 1, ?6, ?6)
             ON CONFLICT(normalized_text, requested_source_language, target_language) DO NOTHING",
                params![
                    normalized_source,
                    source_text,
                    target_language,
                    translated_text,
                    source_language,
                    i64_from_u64(now_ms)?
                ],
            )
            .map_err(storage_error)?
        } else {
            0
        };
        let vocabulary_entry_id = if let Some(id) = existing_id {
            id
        } else {
            tx.query_row(
                "SELECT id FROM vocabulary_entries
                 WHERE normalized_text = ?1 AND requested_source_language = 'auto' AND target_language = ?2",
                params![normalized_source, target_language],
                |row| row.get::<_, i64>(0),
            ).map_err(storage_error)?
        };
        tx.execute(
            "INSERT OR IGNORE INTO vocabulary_textbook_provenance (
                   vocabulary_entry_id, textbook_id, textbook_title, textbook_version,
                   license, attribution, source_url, source_text, translated_text,
                   original_translations, promoted_at_epoch_ms
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                vocabulary_entry_id,
                book_id,
                title,
                version,
                license,
                attribution,
                source_url,
                source_text,
                translated_text,
                original_translations,
                i64_from_u64(now_ms)?
            ],
        )
        .map_err(storage_error)?;
        tx.commit().map_err(storage_error)?;
        Ok(TextbookPromotionResult {
            vocabulary_entry_id,
            inserted: changed == 1,
        })
    }
}

#[derive(Debug)]
struct ImportedEntry {
    normalized_source: String,
    source_text: String,
    translated_text: String,
    original_translations: Vec<String>,
    aliases: Vec<String>,
}

struct LegacyProvenance {
    id: i64,
    vocabulary_entry_id: i64,
    textbook_id: String,
    textbook_title: String,
    textbook_version: String,
    license: String,
    attribution: String,
    source_url: String,
    source_text: String,
    translated_text: String,
    promoted_at_epoch_ms: i64,
}

fn validate_catalog(catalog: &TextbookCatalogItem) -> Result<(), AppError> {
    catalog.validate()?;
    if catalog.expected_bytes > MAX_ARTIFACT_BYTES {
        return Err(invalid_package());
    }
    let url = Url::parse(&catalog.download_url).map_err(|_| invalid_package())?;
    if url.scheme() != "https"
        || url.host_str() != Some(WIKDICT_HOST)
        || url.username() != ""
        || url.password().is_some()
        || url.port().is_some()
    {
        return Err(invalid_package());
    }
    Ok(())
}

fn read_wikdict(path: &Path) -> Result<Vec<ImportedEntry>, AppError> {
    let converter = OpenCC::new();
    let source = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|_| invalid_package())?;
    let columns = source
        .prepare("PRAGMA table_info(simple_translation)")
        .and_then(|mut statement| {
            statement
                .query_map([], |row| row.get::<_, String>(1))?
                .collect::<Result<Vec<_>, _>>()
        })
        .map_err(|_| invalid_package())?;
    if !columns.iter().any(|column| column == "written_rep")
        || !columns.iter().any(|column| column == "trans_list")
    {
        return Err(invalid_package());
    }
    let count = source
        .query_row("SELECT count(*) FROM simple_translation", [], |row| {
            row.get::<_, i64>(0)
        })
        .map_err(|_| invalid_package())?;
    if count <= 0 || count as u64 > MAX_ENTRIES {
        return Err(invalid_package());
    }
    let mut statement = source
        .prepare("SELECT written_rep, trans_list FROM simple_translation ORDER BY written_rep")
        .map_err(|_| invalid_package())?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|_| invalid_package())?;
    let mut distinct_sources = HashSet::with_capacity(count as usize);
    let mut imported = BTreeMap::<String, ImportedEntry>::new();
    for row in rows {
        let (source_text, translated_text) = row.map_err(|_| invalid_package())?;
        let source_text = clean(&source_text);
        let translated_text = clean(&translated_text);
        let key = normalize(&source_text);
        if source_text.is_empty()
            || translated_text.is_empty()
            || source_text.len() > MAX_SOURCE_BYTES
            || translated_text.len() > MAX_TRANSLATION_BYTES
            || !distinct_sources.insert(source_text.clone())
        {
            return Err(invalid_package());
        }
        let (display, originals, aliases) = normalized_zh_cn(&converter, &translated_text)?;
        if let Some(existing) = imported.get_mut(&key) {
            append_unique(&mut existing.original_translations, originals);
            append_unique(&mut existing.aliases, aliases);
            if source_text == key && existing.source_text != key {
                existing.source_text = source_text;
            }
        } else {
            imported.insert(
                key.clone(),
                ImportedEntry {
                    normalized_source: key,
                    source_text,
                    translated_text: display,
                    original_translations: originals,
                    aliases,
                },
            );
        }
    }
    Ok(imported.into_values().collect())
}

fn migrate(connection: &Connection) -> Result<(), AppError> {
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS textbook_schema_migrations (
           version INTEGER PRIMARY KEY,
           applied_at_epoch_ms INTEGER NOT NULL DEFAULT 0
         );",
        )
        .map_err(storage_error)?;
    let version = connection
        .query_row(
            "SELECT coalesce(max(version), 0) FROM textbook_schema_migrations",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(storage_error)?;
    if version < 1 {
        let tx = connection.unchecked_transaction().map_err(storage_error)?;
        tx.execute_batch(
            "CREATE TABLE textbooks (
               id TEXT PRIMARY KEY,
               title TEXT NOT NULL,
               source_language TEXT NOT NULL,
               target_language TEXT NOT NULL,
               version TEXT NOT NULL,
               download_url TEXT NOT NULL,
               expected_bytes INTEGER NOT NULL,
               sha256 TEXT NOT NULL,
               license TEXT NOT NULL,
               attribution TEXT NOT NULL,
               source_url TEXT NOT NULL,
               installed_at_epoch_ms INTEGER NOT NULL,
               active INTEGER NOT NULL DEFAULT 0 CHECK(active IN (0, 1)),
               entry_count INTEGER NOT NULL
             );
             CREATE UNIQUE INDEX textbooks_one_active ON textbooks(active) WHERE active = 1;
             CREATE TABLE textbook_entries (
               id INTEGER PRIMARY KEY,
               textbook_id TEXT NOT NULL REFERENCES textbooks(id) ON DELETE CASCADE,
               normalized_source TEXT NOT NULL,
               source_text TEXT NOT NULL,
               translated_text TEXT NOT NULL,
               phonetic_symbols TEXT,
               source_language TEXT NOT NULL,
               target_language TEXT NOT NULL,
               UNIQUE(textbook_id, normalized_source)
             );
             CREATE INDEX textbook_entries_search ON textbook_entries(textbook_id, normalized_source);
             CREATE TABLE vocabulary_textbook_provenance (
               id INTEGER PRIMARY KEY,
               vocabulary_entry_id INTEGER NOT NULL REFERENCES vocabulary_entries(id) ON DELETE CASCADE,
               textbook_id TEXT NOT NULL,
               textbook_title TEXT NOT NULL,
               textbook_version TEXT NOT NULL,
               license TEXT NOT NULL,
               attribution TEXT NOT NULL,
               source_url TEXT NOT NULL,
               source_text TEXT NOT NULL,
               translated_text TEXT NOT NULL,
               promoted_at_epoch_ms INTEGER NOT NULL,
               UNIQUE(vocabulary_entry_id, textbook_id, source_text, translated_text)
             );
             INSERT INTO textbook_schema_migrations(version) VALUES (1);",
        ).map_err(storage_error)?;
        tx.commit().map_err(storage_error)?;
    }
    if version < 2 {
        let tx = connection.unchecked_transaction().map_err(storage_error)?;
        tx.execute_batch(
            "ALTER TABLE textbook_entries
               ADD COLUMN original_translations TEXT NOT NULL DEFAULT '';
             CREATE TABLE textbook_entry_aliases (
               id INTEGER PRIMARY KEY,
               textbook_entry_id INTEGER NOT NULL REFERENCES textbook_entries(id) ON DELETE CASCADE,
               alias TEXT NOT NULL,
               normalized_alias TEXT NOT NULL,
               UNIQUE(textbook_entry_id, alias)
             );
             CREATE INDEX textbook_entry_aliases_search
               ON textbook_entry_aliases(normalized_alias, textbook_entry_id);",
        )
        .map_err(storage_error)?;
        let rows = {
            let mut statement = tx
                .prepare(
                    "SELECT id, translated_text FROM textbook_entries WHERE target_language = 'zh-CN'",
                )
                .map_err(storage_error)?;
            let rows = statement
                .query_map([], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(storage_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(storage_error)?;
            rows
        };
        let converter = OpenCC::new();
        for (id, original) in rows {
            let (display, originals, aliases) = normalized_zh_cn(&converter, &original)?;
            tx.execute(
                "UPDATE textbook_entries
                 SET translated_text = ?1, original_translations = ?2 WHERE id = ?3",
                params![display, originals.join(" | "), id],
            )
            .map_err(storage_error)?;
            for alias in aliases {
                tx.execute(
                    "INSERT OR IGNORE INTO textbook_entry_aliases (
                       textbook_entry_id, alias, normalized_alias
                     ) VALUES (?1, ?2, ?3)",
                    params![id, alias, normalize(&alias)],
                )
                .map_err(storage_error)?;
            }
        }
        tx.execute_batch(
            "ALTER TABLE vocabulary_textbook_provenance
               RENAME TO vocabulary_textbook_provenance_v1;
             CREATE TABLE vocabulary_textbook_provenance (
               id INTEGER PRIMARY KEY,
               vocabulary_entry_id INTEGER NOT NULL REFERENCES vocabulary_entries(id) ON DELETE CASCADE,
               textbook_id TEXT NOT NULL,
               textbook_title TEXT NOT NULL,
               textbook_version TEXT NOT NULL,
               license TEXT NOT NULL,
               attribution TEXT NOT NULL,
               source_url TEXT NOT NULL,
               source_text TEXT NOT NULL,
               translated_text TEXT NOT NULL,
               original_translations TEXT NOT NULL DEFAULT '',
               promoted_at_epoch_ms INTEGER NOT NULL,
               UNIQUE(
                 vocabulary_entry_id, textbook_id, source_text,
                 translated_text, original_translations
               )
             );",
        )
        .map_err(storage_error)?;
        let provenance = {
            let mut statement = tx
                .prepare(
                    "SELECT id, vocabulary_entry_id, textbook_id, textbook_title,
                            textbook_version, license, attribution, source_url,
                            source_text, translated_text, promoted_at_epoch_ms
                     FROM vocabulary_textbook_provenance_v1 ORDER BY id",
                )
                .map_err(storage_error)?;
            let rows = statement
                .query_map([], |row| {
                    Ok(LegacyProvenance {
                        id: row.get(0)?,
                        vocabulary_entry_id: row.get(1)?,
                        textbook_id: row.get(2)?,
                        textbook_title: row.get(3)?,
                        textbook_version: row.get(4)?,
                        license: row.get(5)?,
                        attribution: row.get(6)?,
                        source_url: row.get(7)?,
                        source_text: row.get(8)?,
                        translated_text: row.get(9)?,
                        promoted_at_epoch_ms: row.get(10)?,
                    })
                })
                .map_err(storage_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(storage_error)?;
            rows
        };
        for row in provenance {
            let (display, originals, _) = normalized_zh_cn(&converter, &row.translated_text)?;
            tx.execute(
                "INSERT INTO vocabulary_textbook_provenance (
                   id, vocabulary_entry_id, textbook_id, textbook_title, textbook_version,
                   license, attribution, source_url, source_text, translated_text,
                   original_translations, promoted_at_epoch_ms
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    row.id,
                    row.vocabulary_entry_id,
                    row.textbook_id,
                    row.textbook_title,
                    row.textbook_version,
                    row.license,
                    row.attribution,
                    row.source_url,
                    row.source_text,
                    display,
                    originals.join(" | "),
                    row.promoted_at_epoch_ms,
                ],
            )
            .map_err(storage_error)?;
        }
        tx.execute_batch("DROP TABLE vocabulary_textbook_provenance_v1;")
            .map_err(storage_error)?;
        tx.execute(
            "INSERT INTO textbook_schema_migrations(version) VALUES (2)",
            [],
        )
        .map_err(storage_error)?;
        tx.commit().map_err(storage_error)?;
    }
    Ok(())
}

fn row_to_installed(row: &rusqlite::Row<'_>) -> rusqlite::Result<InstalledTextbook> {
    Ok(InstalledTextbook {
        id: row.get(0)?,
        title: row.get(1)?,
        source_language: row.get(2)?,
        target_language: row.get(3)?,
        version: row.get(4)?,
        license: row.get(5)?,
        attribution: row.get(6)?,
        source_url: row.get(7)?,
        entry_count: row.get::<_, i64>(8)? as u64,
        installed_at_epoch_ms: row.get::<_, i64>(9)? as u64,
        active: row.get(10)?,
    })
}

fn normalize(value: &str) -> String {
    clean(value).to_lowercase()
}

fn clean(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn normalized_zh_cn(
    converter: &OpenCC,
    translated_text: &str,
) -> Result<(String, Vec<String>, Vec<String>), AppError> {
    let mut originals = Vec::new();
    let mut aliases = Vec::new();
    for original in translated_text.split('|').map(clean) {
        if original.is_empty() {
            continue;
        }
        if !originals.contains(&original) {
            originals.push(original.clone());
        }
        let simplified = clean(&converter.convert_with_config(&original, OpenccConfig::T2s, false));
        for alias in [original, simplified] {
            if !alias.is_empty() && !aliases.contains(&alias) {
                aliases.push(alias);
            }
        }
    }
    let display = originals
        .first()
        .map(|value| clean(&converter.convert_with_config(value, OpenccConfig::T2s, false)))
        .filter(|value| !value.is_empty())
        .ok_or_else(invalid_package)?;
    Ok((display, originals, aliases))
}

fn append_unique(target: &mut Vec<String>, values: Vec<String>) {
    for value in values {
        if !target.contains(&value) {
            target.push(value);
        }
    }
}

fn escape_like(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

fn i64_from_u64(value: u64) -> Result<i64, AppError> {
    i64::try_from(value).map_err(|_| internal("Textbook numeric value is outside storage range"))
}

fn invalid_package() -> AppError {
    AppError::new(
        AppErrorCode::Internal,
        "The textbook package failed validation",
        false,
    )
}

fn download_error(error: reqwest::Error) -> AppError {
    if error.is_timeout() {
        AppError::new(
            AppErrorCode::Timeout,
            "The textbook download timed out",
            true,
        )
    } else {
        download_error_message()
    }
}

fn download_error_message() -> AppError {
    AppError::new(
        AppErrorCode::Offline,
        "The textbook could not be downloaded",
        true,
    )
}

fn storage_error(_: rusqlite::Error) -> AppError {
    internal("The local textbook database could not complete the request")
}

fn internal(message: &'static str) -> AppError {
    AppError::new(AppErrorCode::Internal, message, false)
}

#[cfg(test)]
mod tests {
    use rusqlite::{params, Connection};
    use sha2::{Digest, Sha256};
    use tempfile::NamedTempFile;

    use super::*;

    fn wikdict_fixture(entries: &[(&str, &str)]) -> NamedTempFile {
        let file = NamedTempFile::new().expect("fixture");
        let connection = Connection::open(file.path()).expect("open fixture");
        connection
            .execute_batch(
                "CREATE TABLE simple_translation (
                   written_rep TEXT NOT NULL,
                   trans_list TEXT NOT NULL,
                   max_score REAL,
                   rel_importance REAL
                 );",
            )
            .expect("schema");
        for (source, translation) in entries {
            connection
                .execute(
                    "INSERT INTO simple_translation (written_rep, trans_list) VALUES (?1, ?2)",
                    params![source, translation],
                )
                .expect("entry");
        }
        drop(connection);
        file
    }

    fn test_catalog(path: &std::path::Path, id: &str, version: &str) -> TextbookCatalogItem {
        let bytes = std::fs::read(path).expect("fixture bytes");
        TextbookCatalogItem {
            id: id.into(),
            title: "Test WikDict".into(),
            source_language: "en".into(),
            target_language: "zh-CN".into(),
            version: version.into(),
            download_url: "https://download.wikdict.com/test.sqlite3".into(),
            expected_bytes: bytes.len() as u64,
            sha256: format!("{:x}", Sha256::digest(&bytes)),
            license: "CC BY-SA 4.0".into(),
            attribution: "WikDict contributors".into(),
            source_url: "https://www.wikdict.com/page/download".into(),
        }
    }

    #[test]
    fn curated_catalog_pins_the_verified_wikdict_artifact() {
        let catalog = curated_catalog();
        assert_eq!(catalog.len(), 1);
        assert_eq!(catalog[0].expected_bytes, 5_169_152);
        assert_eq!(
            catalog[0].sha256,
            "16cf69dc8037a8d4dc6bde260142bf0181f9ff0a008d457f26452f1d80ca5ecd"
        );
        assert!(catalog[0]
            .download_url
            .starts_with("https://download.wikdict.com/"));
    }

    #[test]
    fn install_validates_then_atomically_replaces_and_preserves_activation() {
        let app_db = NamedTempFile::new().expect("app db");
        let store = TextbookStore::open(app_db.path()).expect("store");
        let first = wikdict_fixture(&[("ephemeral", "蜉蝣"), ("supersede", "代替")]);
        let first_catalog = test_catalog(first.path(), "test-en-zh", "1");
        store
            .install_sqlite(&first_catalog, first.path(), 10)
            .expect("install");
        store.set_active(Some("test-en-zh")).expect("activate");

        let broken = wikdict_fixture(&[("replacement", "替代")]);
        let mut broken_catalog = test_catalog(broken.path(), "test-en-zh", "2");
        broken_catalog.sha256 = "0".repeat(64);
        assert!(store
            .install_sqlite(&broken_catalog, broken.path(), 20)
            .is_err());
        let installed = store.list_installed().expect("installed");
        assert_eq!(installed[0].version, "1");
        assert!(installed[0].active);

        let second_catalog = test_catalog(broken.path(), "test-en-zh", "2");
        store
            .install_sqlite(&second_catalog, broken.path(), 30)
            .expect("update");
        let installed = store.list_installed().expect("updated");
        assert_eq!(installed[0].version, "2");
        assert!(installed[0].active);
        assert_eq!(installed[0].entry_count, 1);
    }

    #[test]
    fn activation_is_singular_and_browsing_is_bounded() {
        let app_db = NamedTempFile::new().expect("app db");
        let store = TextbookStore::open(app_db.path()).expect("store");
        for id in ["one", "two"] {
            let fixture = wikdict_fixture(&[("ephemeral", "蜉蝣"), ("supersede", "代替")]);
            let catalog = test_catalog(fixture.path(), id, "1");
            store
                .install_sqlite(&catalog, fixture.path(), 10)
                .expect("install");
        }
        store.set_active(Some("one")).expect("one active");
        store.set_active(Some("two")).expect("two active");
        let installed = store.list_installed().expect("installed");
        assert_eq!(installed.iter().filter(|book| book.active).count(), 1);
        assert!(
            installed
                .iter()
                .find(|book| book.id == "two")
                .unwrap()
                .active
        );

        let page = store
            .list_entries("two", Some("super"), 0, 50)
            .expect("page");
        assert_eq!(page.total, 1);
        assert_eq!(page.entries[0].source_text, "supersede");
        assert!(store.list_entries("two", None, 0, 501).is_err());
    }

    #[test]
    fn rejects_duplicates_and_unsafe_catalog_hosts_before_replacement() {
        let app_db = NamedTempFile::new().expect("app db");
        let store = TextbookStore::open(app_db.path()).expect("store");
        let fixture = wikdict_fixture(&[("ephemeral", "蜉蝣"), (" ephemeral ", "短暂的")]);
        let mut catalog = test_catalog(fixture.path(), "duplicate", "1");
        assert!(store.install_sqlite(&catalog, fixture.path(), 10).is_err());
        assert!(store.list_installed().expect("installed").is_empty());

        catalog.download_url = "https://example.com/deck.sqlite3".into();
        assert!(store.install_sqlite(&catalog, fixture.path(), 10).is_err());
    }

    #[test]
    fn normalized_case_variants_merge_without_ambiguous_entries() {
        let app_db = NamedTempFile::new().expect("app db");
        let store = TextbookStore::open(app_db.path()).expect("store");
        let fixture = wikdict_fixture(&[("Aborigine", "土著"), ("aborigine", "土着")]);
        let catalog = test_catalog(fixture.path(), "case-variants", "1");
        store
            .install_sqlite(&catalog, fixture.path(), 10)
            .expect("case variants merge");
        let page = store
            .list_entries("case-variants", Some("aborigine"), 0, 50)
            .expect("page");
        assert_eq!(page.total, 1);
        assert_eq!(page.entries[0].source_text, "aborigine");
        assert_eq!(page.entries[0].translated_text, "土著");
        let variant = store
            .list_entries("case-variants", Some("土着"), 0, 50)
            .expect("variant search");
        assert_eq!(variant.total, 1);
        assert_eq!(variant.entries[0].id, page.entries[0].id);
    }

    #[test]
    fn zh_cn_import_uses_one_simplified_display_and_searches_original_variants() {
        let app_db = NamedTempFile::new().expect("app db");
        let vocabulary = crate::services::vocabulary::VocabularyStore::open(app_db.path())
            .expect("vocabulary migrations");
        let store = TextbookStore::open(app_db.path()).expect("store");
        let fixture = wikdict_fixture(&[("abacus", "算盘 | 算盤"), ("abalone", "鮑魚")]);
        let catalog = test_catalog(fixture.path(), "mixed-script", "1");
        store
            .install_sqlite(&catalog, fixture.path(), 10)
            .expect("install");

        let page = store
            .list_entries("mixed-script", None, 0, 50)
            .expect("browse");
        assert_eq!(page.entries[0].translated_text, "算盘");
        assert_eq!(page.entries[1].translated_text, "鲍鱼");
        for search in ["算盘", "算盤", "鲍鱼", "鮑魚"] {
            assert_eq!(
                store
                    .list_entries("mixed-script", Some(search), 0, 50)
                    .expect("search")
                    .total,
                1,
                "search alias {search}"
            );
        }

        let promoted = page
            .entries
            .iter()
            .map(|entry| store.promote_entry(entry.id, 20).expect("promote"))
            .collect::<Vec<_>>();
        let connection = Connection::open(app_db.path()).expect("inspect promotion");
        let personal = promoted
            .iter()
            .map(|result| {
                connection
                    .query_row(
                        "SELECT translated_text FROM vocabulary_entries WHERE id = ?1",
                        params![result.vocabulary_entry_id],
                        |row| row.get::<_, String>(0),
                    )
                    .expect("personal translation")
            })
            .collect::<Vec<_>>();
        assert_eq!(personal, vec!["算盘", "鲍鱼"]);
        let question = vocabulary
            .practice_question(30)
            .expect("practice")
            .expect("question");
        assert!(question
            .choices
            .iter()
            .all(|choice| !choice.contains('盤') && !choice.contains('鮑')));
        assert!(question.choices.iter().any(|choice| choice == "算盘"));
        assert!(question.choices.iter().any(|choice| choice == "鲍鱼"));

        store.remove("mixed-script").expect("remove textbook");
        let retained = connection
            .prepare(
                "SELECT translated_text, original_translations
                 FROM vocabulary_textbook_provenance ORDER BY translated_text",
            )
            .expect("retained statement")
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .expect("retained rows")
            .collect::<Result<Vec<_>, _>>()
            .expect("retained provenance");
        assert_eq!(retained.len(), 2);
        assert!(retained.contains(&("算盘".into(), "算盘 | 算盤".into())));
        assert!(retained.contains(&("鲍鱼".into(), "鮑魚".into())));
    }

    #[test]
    fn v1_rows_migrate_to_simplified_display_without_redownload() {
        let app_db = NamedTempFile::new().expect("app db");
        crate::services::vocabulary::VocabularyStore::open(app_db.path())
            .expect("vocabulary migrations");
        let connection = Connection::open(app_db.path()).expect("seed v1");
        connection
            .execute_batch(
                "CREATE TABLE textbook_schema_migrations (
                   version INTEGER PRIMARY KEY,
                   applied_at_epoch_ms INTEGER NOT NULL DEFAULT 0
                 );
                 INSERT INTO textbook_schema_migrations(version) VALUES (1);
                 CREATE TABLE textbooks (
                   id TEXT PRIMARY KEY, title TEXT NOT NULL, source_language TEXT NOT NULL,
                   target_language TEXT NOT NULL, version TEXT NOT NULL, download_url TEXT NOT NULL,
                   expected_bytes INTEGER NOT NULL, sha256 TEXT NOT NULL, license TEXT NOT NULL,
                   attribution TEXT NOT NULL, source_url TEXT NOT NULL,
                   installed_at_epoch_ms INTEGER NOT NULL, active INTEGER NOT NULL DEFAULT 0,
                   entry_count INTEGER NOT NULL
                 );
                 CREATE TABLE textbook_entries (
                   id INTEGER PRIMARY KEY,
                   textbook_id TEXT NOT NULL REFERENCES textbooks(id) ON DELETE CASCADE,
                   normalized_source TEXT NOT NULL, source_text TEXT NOT NULL,
                   translated_text TEXT NOT NULL, phonetic_symbols TEXT,
                   source_language TEXT NOT NULL, target_language TEXT NOT NULL,
                   UNIQUE(textbook_id, normalized_source)
                 );
                 CREATE TABLE vocabulary_textbook_provenance (
                   id INTEGER PRIMARY KEY,
                   vocabulary_entry_id INTEGER NOT NULL REFERENCES vocabulary_entries(id) ON DELETE CASCADE,
                   textbook_id TEXT NOT NULL, textbook_title TEXT NOT NULL,
                   textbook_version TEXT NOT NULL, license TEXT NOT NULL,
                   attribution TEXT NOT NULL, source_url TEXT NOT NULL,
                   source_text TEXT NOT NULL, translated_text TEXT NOT NULL,
                   promoted_at_epoch_ms INTEGER NOT NULL,
                   UNIQUE(vocabulary_entry_id, textbook_id, source_text, translated_text)
                 );
                 INSERT INTO textbooks VALUES (
                   'legacy', 'Legacy', 'en', 'zh-CN', '1', 'https://download.wikdict.com/legacy',
                   1, 'digest', 'CC BY-SA 4.0', 'WikDict', 'https://www.wikdict.com', 1, 1, 2
                 );
                 INSERT INTO textbook_entries (
                   textbook_id, normalized_source, source_text, translated_text,
                   source_language, target_language
                 ) VALUES
                   ('legacy', 'abacus', 'abacus', '算盘 | 算盤', 'en', 'zh-CN'),
                   ('legacy', 'abalone', 'abalone', '鮑魚', 'en', 'zh-CN');
                 INSERT INTO vocabulary_entries (
                   id, normalized_text, source_text, requested_source_language, target_language,
                   translated_text, effective_source_language, first_seen_epoch_ms, last_seen_epoch_ms
                 ) VALUES (1, 'abacus', 'abacus', 'en', 'zh-CN', '算盘', 'en', 1, 1);
                 INSERT INTO vocabulary_textbook_provenance (
                   vocabulary_entry_id, textbook_id, textbook_title, textbook_version,
                   license, attribution, source_url, source_text, translated_text,
                   promoted_at_epoch_ms
                 ) VALUES
                   (1, 'legacy', 'Legacy', '1', 'CC BY-SA 4.0', 'WikDict',
                    'https://www.wikdict.com', 'abacus', '算盘', 2),
                   (1, 'legacy', 'Legacy', '2', 'CC BY-SA 4.0', 'WikDict',
                    'https://www.wikdict.com', 'abacus', '算盤', 3);",
            )
            .expect("v1 schema and rows");
        drop(connection);

        let store = TextbookStore::open(app_db.path()).expect("migrate v1");
        let page = store.list_entries("legacy", None, 0, 50).expect("browse");
        assert_eq!(page.entries[0].translated_text, "算盘");
        assert_eq!(page.entries[1].translated_text, "鲍鱼");
        assert_eq!(
            store
                .list_entries("legacy", Some("鮑魚"), 0, 50)
                .expect("traditional alias")
                .total,
            1
        );
        let connection = Connection::open(app_db.path()).expect("inspect migration");
        let provenance = connection
            .prepare(
                "SELECT textbook_version, translated_text, original_translations
                 FROM vocabulary_textbook_provenance ORDER BY textbook_version",
            )
            .expect("provenance statement")
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .expect("provenance rows")
            .collect::<Result<Vec<_>, _>>()
            .expect("provenance collect");
        assert_eq!(
            provenance,
            vec![
                ("1".into(), "算盘".into(), "算盘".into()),
                ("2".into(), "算盘".into(), "算盤".into()),
            ]
        );
        drop(connection);
        drop(store);
        TextbookStore::open(app_db.path()).expect("reopen migrated database");
    }

    #[tokio::test]
    async fn downloader_accepts_only_compiled_catalog_ids() {
        let app_db = NamedTempFile::new().expect("app db");
        let staging = tempfile::tempdir().expect("staging");
        let store = TextbookStore::open(app_db.path()).expect("store");
        assert!(store
            .download_and_install("renderer-supplied-url", staging.path(), 10)
            .await
            .is_err());
        assert_eq!(
            std::fs::read_dir(staging.path())
                .expect("staging dir")
                .count(),
            0
        );
    }

    #[test]
    fn promotion_is_idempotent_never_overwrites_personal_data_and_keeps_provenance() {
        let app_db = NamedTempFile::new().expect("app db");
        crate::services::vocabulary::VocabularyStore::open(app_db.path())
            .expect("vocabulary migrations");
        let store = TextbookStore::open(app_db.path()).expect("store");
        let fixture = wikdict_fixture(&[("ephemeral", "蜉蝣"), ("supersede", "代替")]);
        let catalog = test_catalog(fixture.path(), "test-en-zh", "1");
        store
            .install_sqlite(&catalog, fixture.path(), 10)
            .expect("install");
        let entries = store
            .list_entries("test-en-zh", None, 0, 50)
            .expect("entries");
        let ephemeral = entries
            .entries
            .iter()
            .find(|entry| entry.source_text == "ephemeral")
            .expect("ephemeral");
        let promoted = store.promote_entry(ephemeral.id, 20).expect("promote once");
        assert!(promoted.inserted);
        let promoted_again = store
            .promote_entry(ephemeral.id, 30)
            .expect("promote twice");
        assert!(!promoted_again.inserted);

        let connection = Connection::open(app_db.path()).expect("inspect");
        connection
            .execute(
                "UPDATE vocabulary_entries SET translated_text = '短暂的（个人）' WHERE id = ?1",
                params![promoted.vocabulary_entry_id],
            )
            .expect("personal edit");
        assert!(
            !store
                .promote_entry(ephemeral.id, 40)
                .expect("promote existing")
                .inserted
        );
        let (translation, provenance_count): (String, i64) = connection
            .query_row(
                "SELECT v.translated_text,
                        (SELECT count(*) FROM vocabulary_textbook_provenance p WHERE p.vocabulary_entry_id = v.id)
                 FROM vocabulary_entries v WHERE v.id = ?1",
                params![promoted.vocabulary_entry_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("personal row");
        assert_eq!(translation, "短暂的（个人）");
        assert_eq!(provenance_count, 1);

        connection
            .execute(
                "INSERT INTO vocabulary_entries (
                   normalized_text, source_text, requested_source_language, target_language,
                   translated_text, detected_source_language, effective_source_language,
                   first_seen_epoch_ms, last_seen_epoch_ms
                 ) VALUES ('supersede', 'supersede', 'auto', 'zh-CN', '个人译文', 'en', 'en', 1, 1)",
                [],
            )
            .expect("existing personal row");
        let supersede = entries
            .entries
            .iter()
            .find(|entry| entry.source_text == "supersede")
            .expect("supersede");
        let existing = store
            .promote_entry(supersede.id, 50)
            .expect("promote existing personal");
        assert!(!existing.inserted);
        let (translation, provenance_count): (String, i64) = connection
            .query_row(
                "SELECT v.translated_text,
                        (SELECT count(*) FROM vocabulary_textbook_provenance p WHERE p.vocabulary_entry_id = v.id)
                 FROM vocabulary_entries v WHERE v.id = ?1",
                params![existing.vocabulary_entry_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("existing personal provenance");
        assert_eq!(translation, "个人译文");
        assert_eq!(provenance_count, 1);
        store.remove("test-en-zh").expect("remove");
        let surviving: i64 = connection
            .query_row(
                "SELECT count(*) FROM vocabulary_textbook_provenance",
                [],
                |row| row.get(0),
            )
            .expect("surviving provenance");
        assert_eq!(surviving, 2);
    }
}
