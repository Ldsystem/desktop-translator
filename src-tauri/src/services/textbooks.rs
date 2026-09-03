//! Native-only textbook catalog, validation, and local storage boundary.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    io::Write,
    path::Path,
    sync::Mutex,
    time::Duration,
};

use opencc_fmmseg::{OpenCC, OpenccConfig};
use reqwest::Url;
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::contracts::{
    AppError, AppErrorCode, FilteredRelatedWord, InstalledTextbook, PartOfSpeech, RelatedFilter,
    RelatedOrigin, RelatedWordPage, TextbookCatalogItem, TextbookEntry, TextbookEntryPage,
    TextbookPromotionResult, TranslationSense, ValidateContract, VocabularyMorpheme,
};

const MAX_ARTIFACT_BYTES: u64 = 64 * 1024 * 1024;
const MAX_ENTRIES: u64 = 500_000;
const MAX_SOURCE_BYTES: usize = 2_048;
const MAX_TRANSLATION_BYTES: usize = 8_192;
const MAX_PAGE_SIZE: u64 = 500;
const WIKDICT_HOST: &str = "download.wikdict.com";
const SCOPE_HOST: &str = "static1.squarespace.com";
const MAX_SCOPE_BYTES: u64 = 512 * 1024;
const CURRENT_IMPORT_REVISION: i64 = 1;
const WIKDICT_URL: &str =
    "https://download.wikdict.com/dictionaries/sqlite/2_2026-06/en-zh.sqlite3";
const WIKDICT_SHA256: &str = "16cf69dc8037a8d4dc6bde260142bf0181f9ff0a008d457f26452f1d80ca5ecd";
const BUNDLED_STARTER_ID: &str = "starter-en-zh-cn-1";
const BUNDLED_STARTER_BYTES: &[u8] =
    include_bytes!("../../resources/textbooks/starter-en-zh.sqlite3");
const MORPHEME_BYTES: &[u8] = include_bytes!("../../resources/lexicon/morphemes-v1.json");
const MORPHEME_MANIFEST: &str = include_str!("../../resources/lexicon/morphemes-v1.manifest.json");

fn bundled_starter_item() -> TextbookCatalogItem {
    TextbookCatalogItem {
        id: BUNDLED_STARTER_ID.into(),
        title: "Everyday English Starter · 日常英语入门".into(),
        source_language: "en".into(),
        target_language: "zh-CN".into(),
        version: "1.0 bundled".into(),
        download_url: "https://download.wikdict.com/bundled/starter-en-zh.sqlite3".into(),
        expected_bytes: 12_288,
        sha256: "5c99f10fb3b4b1eea5212a81cc20e87b391eac2cd60cc2277fc1795feab0acd9".into(),
        license: "CC0-1.0".into(),
        attribution: "Desktop Translator contributors".into(),
        source_url: "https://github.com/Ldsystem/desktop-translator".into(),
    }
}

#[derive(Clone, Copy)]
enum ScopeFormat {
    TeachingCsv,
    DescriptionList,
    NumberedList,
}

#[derive(Clone, Copy)]
struct ScopeArtifact {
    url: &'static str,
    expected_bytes: u64,
    sha256: &'static str,
    format: ScopeFormat,
    expected_entries: usize,
}

#[derive(Clone, Copy)]
enum CatalogScope {
    All,
    Words(ScopeArtifact),
}

#[derive(Clone)]
struct CatalogDefinition {
    item: TextbookCatalogItem,
    scope: CatalogScope,
}

enum DownloadScope {
    Catalog(CatalogScope),
    // Existing rows are the already-verified scope. Refresh must not download
    // a mutable catalog's newer word list or silently expand the installed book.
    Installed(HashSet<String>),
}

struct DownloadPlan {
    catalog: TextbookCatalogItem,
    scope: DownloadScope,
}

fn catalog_item(
    id: &str,
    title: &str,
    version: &str,
    attribution: &str,
    source_url: &str,
) -> TextbookCatalogItem {
    TextbookCatalogItem {
        id: id.into(),
        title: title.into(),
        source_language: "en".into(),
        target_language: "zh-CN".into(),
        version: version.into(),
        download_url: WIKDICT_URL.into(),
        expected_bytes: 5_169_152,
        sha256: WIKDICT_SHA256.into(),
        license: "CC BY-SA 4.0".into(),
        attribution: attribution.into(),
        source_url: source_url.into(),
    }
}

fn catalog_definitions() -> Vec<CatalogDefinition> {
    let wikdict = "WikDict, Wiktionary and DBnary contributors";
    let ngsl_source = "https://www.newgeneralservicelist.com/word-lists";
    vec![
        CatalogDefinition {
            item: bundled_starter_item(),
            scope: CatalogScope::All,
        },
        CatalogDefinition {
            item: catalog_item(
                "wikdict-en-zh-2026-06",
                "General English Dictionary",
                "WikDict 2026.06",
                wikdict,
                "https://www.wikdict.com/page/download",
            ),
            scope: CatalogScope::All,
        },
        CatalogDefinition {
            item: catalog_item(
                "ngsl-en-zh-1-2",
                "Everyday English",
                "NGSL 1.2 · WikDict 2026.06",
                "NGSL Project; WikDict, Wiktionary and DBnary contributors",
                ngsl_source,
            ),
            scope: CatalogScope::Words(ScopeArtifact {
                url: "https://static1.squarespace.com/static/64336926d7c6bb38965fdf3b/t/66e83ec996b2ac4b2637bff9/1726496458266/NGSL_1.2_lemmatized_for_teaching.csv",
                expected_bytes: 73_146,
                sha256: "b54e297244988237457e04f823aa8dca68e3d646938dc76d383e099f04cb7666",
                format: ScopeFormat::TeachingCsv,
                expected_entries: 2_809,
            }),
        },
        CatalogDefinition {
            item: catalog_item(
                "nawl-en-zh-1-2",
                "Academic English",
                "NAWL 1.2 · WikDict 2026.06",
                "NGSL Project; WikDict, Wiktionary and DBnary contributors",
                ngsl_source,
            ),
            scope: CatalogScope::Words(ScopeArtifact {
                url: "https://static1.squarespace.com/static/64336926d7c6bb38965fdf3b/t/644e0e936f1c072f5a0503f8/1682837139514/NAWL_1.2_alphabetized_description.txt",
                expected_bytes: 10_340,
                sha256: "88a99099e10010ea40992cca4a5119102d05a0f2888ee5d43d4c8b4afd597fef",
                format: ScopeFormat::DescriptionList,
                expected_entries: 957,
            }),
        },
        CatalogDefinition {
            item: catalog_item(
                "tsl-en-zh-1-2",
                "TOEIC English",
                "TSL 1.2 · WikDict 2026.06",
                "NGSL Project; WikDict, Wiktionary and DBnary contributors",
                ngsl_source,
            ),
            scope: CatalogScope::Words(ScopeArtifact {
                url: "https://static1.squarespace.com/static/64336926d7c6bb38965fdf3b/t/644e131e4a9322006209c72a/1682838302275/TSL_1.2_alphabetized_description.txt",
                expected_bytes: 20_990,
                sha256: "935b89bdad27755b91f5b1b49b755af3dd7ee60c50d7b899f36520691c88eea8",
                format: ScopeFormat::NumberedList,
                expected_entries: 1_250,
            }),
        },
        CatalogDefinition {
            item: catalog_item(
                "bsl-en-zh-1-20",
                "Business English",
                "BSL 1.20 · WikDict 2026.06",
                "NGSL Project; WikDict, Wiktionary and DBnary contributors",
                ngsl_source,
            ),
            scope: CatalogScope::Words(ScopeArtifact {
                url: "https://static1.squarespace.com/static/64336926d7c6bb38965fdf3b/t/6445196c96519e3f970be88a/1682250092935/BSL_1.20_alphabetized_description.txt",
                expected_bytes: 18_215,
                sha256: "8a17b77465ecb382b33af4567ab1427c0950e2c0db8490cb83abf286bf9379ac",
                format: ScopeFormat::DescriptionList,
                expected_entries: 1_744,
            }),
        },
    ]
}

/// Returns the deliberately small, pinned catalog compiled into the application.
pub fn curated_catalog() -> Vec<TextbookCatalogItem> {
    catalog_definitions()
        .into_iter()
        .map(|definition| definition.item)
        .collect()
}

/// Versioned application-local textbook database. Source artifact paths never cross IPC.
pub struct TextbookStore {
    connection: Mutex<Connection>,
}

struct RawRelatedRow {
    normalized: String,
    source_text: String,
    translated_text: String,
    source_language: String,
    target_language: String,
    part_of_speech: Option<String>,
    textbook_id: String,
    textbook_title: String,
    saved_vocabulary_entry_id: Option<i64>,
}

fn raw_related_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawRelatedRow> {
    Ok(RawRelatedRow {
        normalized: row.get(0)?,
        source_text: row.get(1)?,
        translated_text: row.get(2)?,
        source_language: row.get(3)?,
        target_language: row.get(4)?,
        part_of_speech: row.get(5)?,
        textbook_id: row.get(6)?,
        textbook_title: row.get(7)?,
        saved_vocabulary_entry_id: row.get(8)?,
    })
}

impl TextbookStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, AppError> {
        let connection = Connection::open(path).map_err(storage_error)?;
        connection
            .execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")
            .map_err(storage_error)?;
        migrate(&connection)?;
        install_morpheme_catalog(&connection)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    /// Installs the small offline starter on first run and makes it active when no book is active.
    pub fn ensure_bundled_starter(&self, now_ms: u64) -> Result<(), AppError> {
        if self.get_installed(BUNDLED_STARTER_ID)?.is_none() {
            self.install_verified_bytes(&bundled_starter_item(), BUNDLED_STARTER_BYTES, now_ms)?;
        }
        if !self.list_installed()?.iter().any(|book| book.active) {
            self.set_active(Some(BUNDLED_STARTER_ID))?;
        }
        Ok(())
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

    fn download_plan(&self, catalog_id: &str) -> Result<DownloadPlan, AppError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| internal("Textbook database lock failed"))?;
        let stored = connection
            .query_row(
                "SELECT id, title, source_language, target_language, version, download_url,
                    expected_bytes, sha256, license, attribution, source_url
             FROM textbooks WHERE id = ?1",
                params![catalog_id],
                |row| {
                    Ok(TextbookCatalogItem {
                        id: row.get(0)?,
                        title: row.get(1)?,
                        source_language: row.get(2)?,
                        target_language: row.get(3)?,
                        version: row.get(4)?,
                        download_url: row.get(5)?,
                        expected_bytes: u64::try_from(row.get::<_, i64>(6)?).unwrap_or(0),
                        sha256: row.get(7)?,
                        license: row.get(8)?,
                        attribution: row.get(9)?,
                        source_url: row.get(10)?,
                    })
                },
            )
            .optional()
            .map_err(storage_error)?;
        if let Some(catalog) = stored {
            validate_catalog(&catalog)?;
            let words = connection
                .prepare("SELECT normalized_source FROM textbook_entries WHERE textbook_id = ?1")
                .map_err(storage_error)?
                .query_map(params![catalog_id], |row| row.get::<_, String>(0))
                .map_err(storage_error)?
                .collect::<Result<HashSet<_>, _>>()
                .map_err(storage_error)?;
            if words.is_empty() {
                return Err(invalid_package());
            }
            return Ok(DownloadPlan {
                catalog,
                scope: DownloadScope::Installed(words),
            });
        }
        let definition = catalog_definitions()
            .into_iter()
            .find(|definition| definition.item.id == catalog_id)
            .ok_or_else(|| internal("Textbook catalog item was not found"))?;
        validate_catalog(&definition.item)?;
        Ok(DownloadPlan {
            catalog: definition.item,
            scope: DownloadScope::Catalog(definition.scope),
        })
    }

    /// New installs use the compiled catalog; refreshes use the exact stored pin.
    pub async fn download_and_install(
        &self,
        catalog_id: &str,
        staging_directory: &Path,
        now_ms: u64,
    ) -> Result<InstalledTextbook, AppError> {
        let plan = self.download_plan(catalog_id)?;
        let catalog = plan.catalog;
        if catalog.id == BUNDLED_STARTER_ID
            && catalog.expected_bytes == BUNDLED_STARTER_BYTES.len() as u64
            && catalog
                .sha256
                .eq_ignore_ascii_case(&format!("{:x}", Sha256::digest(BUNDLED_STARTER_BYTES)))
        {
            let first_install = matches!(&plan.scope, DownloadScope::Catalog(_));
            let scope = match &plan.scope {
                DownloadScope::Installed(words) => Some(words),
                _ => None,
            };
            self.install_verified_bytes_scoped(&catalog, BUNDLED_STARTER_BYTES, scope, now_ms)?;
            if first_install {
                self.ensure_bundled_starter(now_ms)?;
            }
            return self.get_installed(&catalog.id)?.ok_or_else(invalid_package);
        }
        validate_catalog(&catalog)?;
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(download_error)?;
        fs::create_dir_all(staging_directory).map_err(|_| download_error_message())?;
        let bytes = download_exact(
            &client,
            &catalog.download_url,
            catalog.expected_bytes,
            MAX_ARTIFACT_BYTES,
            staging_directory,
        )
        .await?;
        let scope = match plan.scope {
            DownloadScope::Installed(words) => Some(words),
            DownloadScope::Catalog(CatalogScope::All) => None,
            DownloadScope::Catalog(CatalogScope::Words(artifact)) => {
                validate_scope_artifact(&artifact)?;
                let scope_bytes = download_exact(
                    &client,
                    artifact.url,
                    artifact.expected_bytes,
                    MAX_SCOPE_BYTES,
                    staging_directory,
                )
                .await?;
                Some(verified_scope_words(&scope_bytes, &artifact)?)
            }
        };
        self.install_verified_bytes_scoped(&catalog, &bytes, scope.as_ref(), now_ms)
    }

    fn install_verified_bytes(
        &self,
        catalog: &TextbookCatalogItem,
        bytes: &[u8],
        now_ms: u64,
    ) -> Result<InstalledTextbook, AppError> {
        self.install_verified_bytes_scoped(catalog, bytes, None, now_ms)
    }

    fn install_verified_bytes_scoped(
        &self,
        catalog: &TextbookCatalogItem,
        bytes: &[u8],
        scope: Option<&HashSet<String>>,
        now_ms: u64,
    ) -> Result<InstalledTextbook, AppError> {
        if bytes.len() as u64 != catalog.expected_bytes || bytes.len() as u64 > MAX_ARTIFACT_BYTES {
            return Err(invalid_package());
        }
        let digest = format!("{:x}", Sha256::digest(bytes));
        if !digest.eq_ignore_ascii_case(&catalog.sha256) {
            return Err(invalid_package());
        }
        let mut private_artifact = tempfile::NamedTempFile::new().map_err(|_| invalid_package())?;
        private_artifact
            .write_all(bytes)
            .map_err(|_| invalid_package())?;
        private_artifact.flush().map_err(|_| invalid_package())?;
        let mut imported = read_wikdict(private_artifact.path())?;
        if let Some(scope) = scope {
            imported.retain(|entry| scope.contains(&entry.normalized_source));
            if imported.is_empty() {
                return Err(invalid_package());
            }
        }
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
               installed_at_epoch_ms, active, entry_count, import_revision,
               catalog_revision, lexical_refresh_status
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, 'enriched')
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
               entry_count = excluded.entry_count,
               import_revision = excluded.import_revision,
               catalog_revision = CASE
                 WHEN textbooks.download_url = excluded.download_url
                  AND textbooks.expected_bytes = excluded.expected_bytes
                  AND lower(textbooks.sha256) = lower(excluded.sha256)
                 THEN textbooks.catalog_revision ELSE excluded.catalog_revision END,
               lexical_refresh_status = excluded.lexical_refresh_status",
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
                CURRENT_IMPORT_REVISION,
                catalog.version,
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
                       original_translations, source_language, target_language, part_of_speech
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                )
                .map_err(storage_error)?;
            let mut insert_alias = tx
                .prepare(
                    "INSERT OR IGNORE INTO textbook_entry_aliases (
                       textbook_entry_id, alias, normalized_alias
                     ) VALUES (?1, ?2, ?3)",
                )
                .map_err(storage_error)?;
            let mut insert_sense = tx
                .prepare(
                    "INSERT INTO textbook_entry_senses (
                       textbook_entry_id, display_text, normalized_text, pos_key, rank, is_primary
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
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
                        entry.part_of_speech.map(PartOfSpeech::as_str),
                    ])
                    .map_err(storage_error)?;
                let textbook_entry_id = tx.last_insert_rowid();
                if entry.senses.len() > 32 {
                    eprintln!(
                        "lexical sense overflow: provider=textbook source={} retained=32 discarded={}",
                        entry.normalized_source,
                        entry.senses.len() - 32
                    );
                }
                for (rank, (sense, part_of_speech)) in entry.senses.iter().take(32).enumerate() {
                    insert_sense
                        .execute(params![
                            textbook_entry_id,
                            sense,
                            normalize(sense),
                            part_of_speech.map(PartOfSpeech::as_str).unwrap_or(""),
                            rank as i64,
                            i64::from(rank == 0)
                        ])
                        .map_err(storage_error)?;
                }
                for alias in entry.aliases {
                    insert_alias
                        .execute(params![textbook_entry_id, alias, normalize(&alias)])
                        .map_err(storage_error)?;
                }
            }
        }
        if table_has_column(&tx, "vocabulary_entries", "part_of_speech")? {
            let candidates = {
                let mut statement = tx
                    .prepare(
                        "SELECT v.id, v.source_text, v.translated_text,
                                v.effective_source_language, v.target_language,
                                e.source_text, e.translated_text, e.source_language,
                                e.target_language, e.part_of_speech
                         FROM vocabulary_entries v
                         JOIN vocabulary_textbook_provenance p
                           ON p.vocabulary_entry_id = v.id
                         JOIN textbook_entries e
                           ON e.textbook_id = p.textbook_id
                          AND e.source_text = p.source_text
                          AND e.translated_text = p.translated_text
                         WHERE v.part_of_speech IS NULL
                           AND p.textbook_id = ?1
                           AND e.part_of_speech IS NOT NULL",
                    )
                    .map_err(storage_error)?;
                let rows = statement
                    .query_map(params![catalog.id], |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, String>(4)?,
                            row.get::<_, String>(5)?,
                            row.get::<_, String>(6)?,
                            row.get::<_, String>(7)?,
                            row.get::<_, String>(8)?,
                            row.get::<_, String>(9)?,
                        ))
                    })
                    .map_err(storage_error)?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(storage_error)?;
                rows
            };
            let mut verified = HashMap::<i64, HashSet<PartOfSpeech>>::new();
            for (
                vocabulary_entry_id,
                personal_source,
                personal_translation,
                personal_source_language,
                personal_target_language,
                textbook_source,
                textbook_translation,
                textbook_source_language,
                textbook_target_language,
                stored_category,
            ) in candidates
            {
                let category = PartOfSpeech::from_normalized(&stored_category);
                if normalize(&personal_source) == normalize(&textbook_source)
                    && normalize(&personal_translation) == normalize(&textbook_translation)
                    && personal_source_language.eq_ignore_ascii_case(&textbook_source_language)
                    && personal_target_language.eq_ignore_ascii_case(&textbook_target_language)
                {
                    if let Some(category) = category {
                        verified
                            .entry(vocabulary_entry_id)
                            .or_default()
                            .insert(category);
                    }
                }
            }
            for (vocabulary_entry_id, categories) in verified {
                if categories.len() == 1 {
                    tx.execute(
                        "UPDATE vocabulary_entries SET part_of_speech = ?1
                         WHERE id = ?2 AND part_of_speech IS NULL",
                        params![
                            categories.iter().next().expect("one category").as_str(),
                            vocabulary_entry_id
                        ],
                    )
                    .map_err(storage_error)?;
                }
            }
        }
        if table_has_column(&tx, "vocabulary_senses", "normalized_text")? {
            tx.execute(
                "INSERT OR IGNORE INTO vocabulary_senses (
                   vocabulary_entry_id, display_text, normalized_text, pos_key, rank,
                   is_primary, confidence, created_at_epoch_ms, updated_at_epoch_ms
                 )
                 SELECT v.id, s.display_text, s.normalized_text, s.pos_key, s.rank + 1,
                        0, NULL, ?2, ?2
                 FROM vocabulary_entries v
                 JOIN textbook_entries e
                   ON e.textbook_id = ?1
                  AND e.normalized_source = v.normalized_text
                  AND lower(e.source_language) = lower(v.effective_source_language)
                  AND lower(e.target_language) = lower(v.target_language)
                 JOIN textbook_entry_senses s ON s.textbook_entry_id = e.id",
                params![catalog.id, i64_from_u64(now_ms)?],
            )
            .map_err(storage_error)?;
            tx.execute(
                "INSERT OR IGNORE INTO vocabulary_sense_origins (
                   vocabulary_sense_id, origin_kind, source_id, source_version, evidence_key
                 )
                 SELECT vs.id, 'textbook', ?1, ?2, 'exact-source'
                 FROM vocabulary_senses vs
                 JOIN vocabulary_entries v ON v.id = vs.vocabulary_entry_id
                 JOIN textbook_entries e
                   ON e.textbook_id = ?1
                  AND e.normalized_source = v.normalized_text
                  AND lower(e.source_language) = lower(v.effective_source_language)
                  AND lower(e.target_language) = lower(v.target_language)
                 JOIN textbook_entry_senses s
                   ON s.textbook_entry_id = e.id
                  AND s.normalized_text = vs.normalized_text
                  AND s.pos_key = vs.pos_key",
                params![catalog.id, catalog.version],
            )
            .map_err(storage_error)?;
            normalize_vocabulary_sense_limits(&tx)?;
        }
        tx.commit().map_err(storage_error)?;
        install_morpheme_catalog(&connection)?;
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
                        attribution, source_url, entry_count, installed_at_epoch_ms, active,
                        import_revision < ?1, lexical_refresh_status
                 FROM textbooks ORDER BY title COLLATE NOCASE, id",
            )
            .map_err(storage_error)?;
        let rows = statement
            .query_map(params![CURRENT_IMPORT_REVISION], row_to_installed)
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
                        attribution, source_url, entry_count, installed_at_epoch_ms, active,
                        import_revision < ?2, lexical_refresh_status
                 FROM textbooks WHERE id = ?1",
                params![id, CURRENT_IMPORT_REVISION],
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

    pub fn morphemes_for_word(
        &self,
        source_text: &str,
        source_language: &str,
        target_language: &str,
    ) -> Result<Vec<VocabularyMorpheme>, AppError> {
        if !source_language.eq_ignore_ascii_case("en") {
            return Ok(Vec::new());
        }
        let artifact: MorphemeArtifact =
            serde_json::from_slice(MORPHEME_BYTES).map_err(|_| invalid_package())?;
        let Some(headword) = artifact
            .headwords
            .iter()
            .find(|item| item.normalized_headword == normalize(source_text))
        else {
            return Ok(Vec::new());
        };
        let records = artifact
            .morphemes
            .iter()
            .map(|item| (item.id.as_str(), item))
            .collect::<HashMap<_, _>>();
        let connection = self
            .connection
            .lock()
            .map_err(|_| internal("Textbook database lock failed"))?;
        let mut result = Vec::new();
        for component in headword.components.iter().take(12) {
            let Some(record) = records.get(component.morpheme_id.as_str()) else {
                continue;
            };
            let count = connection
                .query_row(
                    "SELECT count(DISTINCT te.normalized_source)
                     FROM textbook_entry_morphemes tem
                     JOIN textbook_entries te ON te.id = tem.textbook_entry_id
                     JOIN textbooks t ON t.id = te.textbook_id
                     WHERE tem.morpheme_id = ?1 AND te.normalized_source <> ?2
                       AND lower(te.source_language) = lower(?3)
                       AND lower(te.target_language) = lower(?4)",
                    params![
                        component.morpheme_id,
                        normalize(source_text),
                        source_language,
                        target_language
                    ],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(storage_error)? as u64;
            result.push(VocabularyMorpheme {
                id: record.id.clone(),
                display: record.display.clone(),
                kind: record.kind.clone(),
                accessible_label: format!("{} {}", record.kind, record.canonical_form),
                textbook_word_count: count,
            });
        }
        Ok(result)
    }

    pub fn senses_for_word(
        &self,
        source_text: &str,
        source_language: &str,
        target_language: &str,
    ) -> Result<Vec<TranslationSense>, AppError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| internal("Textbook database lock failed"))?;
        let mut statement = connection
            .prepare(
                "SELECT s.display_text, s.pos_key, min(s.rank)
                 FROM textbook_entry_senses s
                 JOIN textbook_entries e ON e.id = s.textbook_entry_id
                 JOIN textbooks t ON t.id = e.textbook_id
                 WHERE e.normalized_source = ?1
                   AND lower(e.source_language) = lower(?2)
                   AND lower(e.target_language) = lower(?3)
                 GROUP BY s.normalized_text, s.pos_key
                 ORDER BY min(s.rank), s.normalized_text, s.pos_key
                 LIMIT 32",
            )
            .map_err(storage_error)?;
        let rows = statement
            .query_map(
                params![normalize(source_text), source_language, target_language],
                |row| {
                    let pos_key: String = row.get(1)?;
                    Ok(TranslationSense {
                        text: row.get(0)?,
                        part_of_speech: PartOfSpeech::from_normalized(&pos_key),
                        rank: row.get::<_, i64>(2)?.clamp(0, 31) as u8,
                        is_primary: false,
                        confidence: None,
                    })
                },
            )
            .map_err(storage_error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(storage_error)
    }

    pub fn filtered_related(
        &self,
        anchor_source: &str,
        source_language: &str,
        target_language: &str,
        filter: &RelatedFilter,
        offset: u64,
        limit: u64,
    ) -> Result<RelatedWordPage, AppError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| internal("Textbook database lock failed"))?;
        let (sql, value, label) = match filter {
            RelatedFilter::Morpheme { morpheme_id } => (
                "SELECT te.normalized_source, te.source_text, te.translated_text,
                        te.source_language, te.target_language, te.part_of_speech,
                        t.id, t.title,
                        (SELECT v.id FROM vocabulary_entries v
                         WHERE v.normalized_text = te.normalized_source
                           AND lower(v.effective_source_language) = lower(te.source_language)
                           AND lower(v.target_language) = lower(te.target_language)
                         ORDER BY v.id LIMIT 1)
                 FROM textbook_entry_morphemes relation
                 JOIN textbook_entries te ON te.id = relation.textbook_entry_id
                 JOIN textbooks t ON t.id = te.textbook_id
                 WHERE relation.morpheme_id = ?1 AND te.normalized_source <> ?2
                   AND lower(te.source_language) = lower(?3)
                   AND lower(te.target_language) = lower(?4)
                 ORDER BY te.normalized_source, t.id",
                morpheme_id.clone(),
                morpheme_id.clone(),
            ),
            RelatedFilter::Translation {
                vocabulary_sense_id,
            } => {
                let label = connection
                    .query_row(
                        "SELECT display_text FROM vocabulary_senses WHERE id = ?1",
                        params![vocabulary_sense_id],
                        |row| row.get::<_, String>(0),
                    )
                    .map_err(storage_error)?;
                (
                    "SELECT te.normalized_source, te.source_text, te.translated_text,
                        te.source_language, te.target_language, te.part_of_speech,
                        t.id, t.title,
                        (SELECT v.id FROM vocabulary_entries v
                         WHERE v.normalized_text = te.normalized_source
                           AND lower(v.effective_source_language) = lower(te.source_language)
                           AND lower(v.target_language) = lower(te.target_language)
                         ORDER BY v.id LIMIT 1)
                 FROM textbook_entry_senses relation
                 JOIN textbook_entries te ON te.id = relation.textbook_entry_id
                 JOIN textbooks t ON t.id = te.textbook_id
                 JOIN vocabulary_senses vs ON vs.id = ?1
                 WHERE relation.normalized_text = vs.normalized_text
                   AND (vs.pos_key = '' OR relation.pos_key = '' OR relation.pos_key = vs.pos_key)
                   AND te.normalized_source <> ?2
                   AND lower(te.source_language) = lower(?3)
                   AND lower(te.target_language) = lower(?4)
                 ORDER BY te.normalized_source, t.id",
                    vocabulary_sense_id.to_string(),
                    label,
                )
            }
        };
        let mut statement = connection.prepare(sql).map_err(storage_error)?;
        let rows = match filter {
            RelatedFilter::Morpheme { .. } => statement.query_map(
                params![
                    value,
                    normalize(anchor_source),
                    source_language,
                    target_language
                ],
                raw_related_row,
            ),
            RelatedFilter::Translation {
                vocabulary_sense_id,
            } => statement.query_map(
                params![
                    vocabulary_sense_id,
                    normalize(anchor_source),
                    source_language,
                    target_language
                ],
                raw_related_row,
            ),
        }
        .map_err(storage_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage_error)?;
        let mut grouped = BTreeMap::<String, FilteredRelatedWord>::new();
        for row in rows {
            let item =
                grouped
                    .entry(row.normalized.clone())
                    .or_insert_with(|| FilteredRelatedWord {
                        key: format!(
                            "{}:{}:{}",
                            row.source_language, row.target_language, row.normalized
                        ),
                        source_text: row.source_text.clone(),
                        translated_text: row.translated_text.clone(),
                        source_language: row.source_language.clone(),
                        target_language: row.target_language.clone(),
                        part_of_speech: row
                            .part_of_speech
                            .as_deref()
                            .and_then(PartOfSpeech::from_normalized),
                        saved_vocabulary_entry_id: row.saved_vocabulary_entry_id,
                        origins: Vec::new(),
                    });
            if !item
                .origins
                .iter()
                .any(|origin| origin.textbook_id.as_deref() == Some(row.textbook_id.as_str()))
            {
                item.origins.push(RelatedOrigin {
                    kind: "textbook".into(),
                    textbook_id: Some(row.textbook_id),
                    textbook_title: Some(row.textbook_title),
                });
            }
        }
        let total = grouped.len() as u64;
        let items = grouped
            .into_values()
            .skip(offset.min(usize::MAX as u64) as usize)
            .take(limit.clamp(1, 100) as usize)
            .collect();
        Ok(RelatedWordPage {
            items,
            total,
            offset,
            limit: limit.clamp(1, 100),
            relation_label: label,
        })
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
                        source_language, target_language, part_of_speech
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
                        part_of_speech: row
                            .get::<_, Option<String>>(7)?
                            .and_then(|value| PartOfSpeech::from_normalized(&value)),
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
                        e.original_translations, e.source_language, e.target_language, e.part_of_speech, t.title,
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
                        row.get::<_, Option<String>>(7)?
                            .and_then(|value| PartOfSpeech::from_normalized(&value)),
                        row.get::<_, String>(8)?,
                        row.get::<_, String>(9)?,
                        row.get::<_, String>(10)?,
                        row.get::<_, String>(11)?,
                        row.get::<_, String>(12)?,
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
            part_of_speech,
            title,
            version,
            license,
            attribution,
            source_url,
        ) = entry;
        if normalize(&source_text) == normalize(&translated_text) {
            return Err(AppError::new(
                AppErrorCode::InvalidLanguagePair,
                "Source-equals-translation entries cannot be added to personal vocabulary",
                false,
            ));
        }
        let existing = tx
            .query_row(
                "SELECT id, translated_text, effective_source_language, target_language
                 FROM vocabulary_entries
             WHERE normalized_text = ?1 AND target_language = ?2
               AND requested_source_language IN ('auto', ?3)
             ORDER BY CASE requested_source_language WHEN 'auto' THEN 0 ELSE 1 END, id
             LIMIT 1",
                params![normalized_source, target_language, source_language],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                },
            )
            .optional()
            .map_err(storage_error)?;
        let changed = if existing.is_none() {
            tx.execute(
                "INSERT INTO vocabulary_entries (
               normalized_text, source_text, requested_source_language, target_language,
               translated_text, detected_source_language, effective_source_language,
               lookup_count, first_seen_epoch_ms, last_seen_epoch_ms, part_of_speech
             ) VALUES (?1, ?2, 'auto', ?3, ?4, ?5, ?5, 1, ?6, ?6, ?7)
             ON CONFLICT(normalized_text, requested_source_language, target_language) DO NOTHING",
                params![
                    normalized_source,
                    source_text,
                    target_language,
                    translated_text,
                    source_language,
                    i64_from_u64(now_ms)?,
                    part_of_speech.map(PartOfSpeech::as_str),
                ],
            )
            .map_err(storage_error)?
        } else {
            0
        };
        let vocabulary_entry_id = if let Some((id, _, _, _)) = &existing {
            *id
        } else {
            tx.query_row(
                "SELECT id FROM vocabulary_entries
                 WHERE normalized_text = ?1 AND requested_source_language = 'auto' AND target_language = ?2",
                params![normalized_source, target_language],
                |row| row.get::<_, i64>(0),
            ).map_err(storage_error)?
        };
        let exact_existing_pair = existing.as_ref().is_some_and(
            |(_, personal_translation, personal_source_language, personal_target_language)| {
                normalize(personal_translation) == normalize(&translated_text)
                    && personal_source_language.eq_ignore_ascii_case(&source_language)
                    && personal_target_language.eq_ignore_ascii_case(&target_language)
            },
        );
        if exact_existing_pair {
            if let Some(part_of_speech) = part_of_speech {
                tx.execute(
                    "UPDATE vocabulary_entries
                     SET part_of_speech = coalesce(part_of_speech, ?1)
                     WHERE id = ?2",
                    params![part_of_speech.as_str(), vocabulary_entry_id],
                )
                .map_err(storage_error)?;
            }
        }
        tx.execute(
            "INSERT OR IGNORE INTO vocabulary_senses (
               vocabulary_entry_id, display_text, normalized_text, pos_key, rank,
               is_primary, confidence, created_at_epoch_ms, updated_at_epoch_ms
             )
             SELECT id, translated_text, lower(trim(translated_text)),
                    coalesce(part_of_speech, ''), 0, 1, NULL, ?2, ?2
             FROM vocabulary_entries WHERE id = ?1",
            params![vocabulary_entry_id, i64_from_u64(now_ms)?],
        )
        .map_err(storage_error)?;
        tx.execute(
            "INSERT OR IGNORE INTO vocabulary_senses (
               vocabulary_entry_id, display_text, normalized_text, pos_key, rank,
               is_primary, confidence, created_at_epoch_ms, updated_at_epoch_ms
             )
             SELECT ?1, s.display_text, s.normalized_text, s.pos_key, s.rank + 1,
                    0, NULL, ?3, ?3
             FROM textbook_entry_senses s WHERE s.textbook_entry_id = ?2",
            params![
                vocabulary_entry_id,
                textbook_entry_id,
                i64_from_u64(now_ms)?
            ],
        )
        .map_err(storage_error)?;
        tx.execute(
            "INSERT OR IGNORE INTO vocabulary_sense_origins (
               vocabulary_sense_id, origin_kind, source_id, source_version, evidence_key
             )
             SELECT vs.id, 'textbook', ?2, ?3, 'promoted-entry'
             FROM vocabulary_senses vs
             JOIN textbook_entry_senses ts
               ON ts.textbook_entry_id = ?4
              AND ts.normalized_text = vs.normalized_text
              AND ts.pos_key = vs.pos_key
             WHERE vs.vocabulary_entry_id = ?1",
            params![vocabulary_entry_id, book_id, version, textbook_entry_id],
        )
        .map_err(storage_error)?;
        normalize_vocabulary_sense_limits(&tx)?;
        tx.execute(
            "INSERT INTO vocabulary_textbook_provenance (
                   vocabulary_entry_id, textbook_id, textbook_title, textbook_version,
                   license, attribution, source_url, source_text, translated_text,
                   original_translations, promoted_at_epoch_ms
                 )
                 SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11
                 WHERE NOT EXISTS (
                   SELECT 1 FROM vocabulary_textbook_provenance
                   WHERE vocabulary_entry_id = ?1 AND textbook_id = ?2
                     AND textbook_title = ?3 AND textbook_version = ?4
                     AND license = ?5 AND attribution = ?6 AND source_url = ?7
                     AND source_text = ?8 AND translated_text = ?9
                     AND original_translations = ?10
                 )",
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
    senses: Vec<(String, Option<PartOfSpeech>)>,
    part_of_speech: Option<PartOfSpeech>,
    part_of_speech_conflicted: bool,
}

#[derive(Debug)]
struct LexicalCandidate {
    part_of_speech: PartOfSpeech,
    score: f64,
    is_good: i64,
    importance: f64,
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

async fn download_exact(
    client: &reqwest::Client,
    url: &str,
    expected_bytes: u64,
    maximum_bytes: u64,
    staging_directory: &Path,
) -> Result<Vec<u8>, AppError> {
    if expected_bytes == 0 || expected_bytes > maximum_bytes {
        return Err(invalid_package());
    }
    let mut response = client.get(url).send().await.map_err(download_error)?;
    if !response.status().is_success()
        || response
            .content_length()
            .is_some_and(|length| length > expected_bytes)
    {
        return Err(download_error_message());
    }
    let mut staged =
        tempfile::NamedTempFile::new_in(staging_directory).map_err(|_| download_error_message())?;
    let mut received = 0_u64;
    while let Some(chunk) = response.chunk().await.map_err(download_error)? {
        received = received
            .checked_add(chunk.len() as u64)
            .ok_or_else(download_error_message)?;
        if received > expected_bytes || received > maximum_bytes {
            return Err(invalid_package());
        }
        staged
            .write_all(&chunk)
            .map_err(|_| download_error_message())?;
    }
    if received != expected_bytes {
        return Err(invalid_package());
    }
    staged.flush().map_err(|_| download_error_message())?;
    staged
        .as_file()
        .sync_all()
        .map_err(|_| download_error_message())?;
    fs::read(staged.path()).map_err(|_| download_error_message())
}

fn validate_scope_artifact(artifact: &ScopeArtifact) -> Result<(), AppError> {
    let url = Url::parse(artifact.url).map_err(|_| invalid_package())?;
    if url.scheme() != "https"
        || url.host_str() != Some(SCOPE_HOST)
        || url.username() != ""
        || url.password().is_some()
        || url.port().is_some()
        || artifact.expected_bytes == 0
        || artifact.expected_bytes > MAX_SCOPE_BYTES
        || artifact.sha256.len() != 64
        || !artifact.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        || artifact.expected_entries == 0
        || artifact.expected_entries as u64 > MAX_ENTRIES
    {
        return Err(invalid_package());
    }
    Ok(())
}

fn parse_scope_words(
    bytes: &[u8],
    format: ScopeFormat,
    expected_entries: usize,
) -> Result<HashSet<String>, AppError> {
    let text = std::str::from_utf8(bytes).map_err(|_| invalid_package())?;
    let mut words = HashSet::with_capacity(expected_entries);
    let mut after_references = false;
    let mut collecting_description_list = false;
    for raw_line in text.lines() {
        let line = raw_line.trim().trim_start_matches('\u{feff}');
        let word = match format {
            ScopeFormat::TeachingCsv => {
                if line.is_empty() || line.starts_with('#') {
                    None
                } else {
                    line.split(',').next()
                }
            }
            ScopeFormat::DescriptionList => {
                if collecting_description_list {
                    (!line.is_empty()).then_some(line)
                } else {
                    if line == "References" {
                        after_references = true;
                    } else if after_references && line.is_empty() {
                        collecting_description_list = true;
                    }
                    None
                }
            }
            ScopeFormat::NumberedList => line.split_once('.').and_then(|(number, word)| {
                number
                    .bytes()
                    .all(|byte| byte.is_ascii_digit())
                    .then_some(word.trim())
            }),
        };
        if let Some(word) = word {
            let normalized = normalize(word);
            if normalized.is_empty()
                || normalized.len() > MAX_SOURCE_BYTES
                || !normalized.chars().all(|character| {
                    character.is_alphabetic()
                        || character == '-'
                        || character == '\''
                        || character == ' '
                })
                || !words.insert(normalized)
            {
                return Err(invalid_package());
            }
        }
    }
    if words.len() != expected_entries {
        return Err(invalid_package());
    }
    Ok(words)
}

fn verified_scope_words(
    bytes: &[u8],
    artifact: &ScopeArtifact,
) -> Result<HashSet<String>, AppError> {
    validate_scope_artifact(artifact)?;
    if bytes.len() as u64 != artifact.expected_bytes
        || format!("{:x}", Sha256::digest(bytes)) != artifact.sha256
    {
        return Err(invalid_package());
    }
    parse_scope_words(bytes, artifact.format, artifact.expected_entries)
}

fn validate_catalog(catalog: &TextbookCatalogItem) -> Result<(), AppError> {
    // The renderer catalog contract canonicalizes SHA to lowercase, but stored
    // legacy pins may use either hexadecimal case. Validate without rewriting it.
    let mut canonical = catalog.clone();
    canonical.sha256.make_ascii_lowercase();
    canonical.validate()?;
    if !valid_package_pin(
        &catalog.download_url,
        catalog.expected_bytes,
        &catalog.sha256,
    ) {
        return Err(invalid_package());
    }
    Ok(())
}

fn valid_package_pin(download_url: &str, expected_bytes: u64, sha256: &str) -> bool {
    expected_bytes > 0
        && expected_bytes <= MAX_ARTIFACT_BYTES
        && sha256.len() == 64
        && sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        && Url::parse(download_url).is_ok_and(|url| {
            url.scheme() == "https"
                && url.host_str() == Some(WIKDICT_HOST)
                && url.username().is_empty()
                && url.password().is_none()
                && url.port().is_none()
                && url.fragment().is_none()
        })
}

fn normalize_vocabulary_sense_limits(connection: &Connection) -> Result<(), AppError> {
    let ids = connection
        .prepare("SELECT DISTINCT vocabulary_entry_id FROM vocabulary_senses WHERE pos_key = ''")
        .map_err(storage_error)?
        .query_map([], |row| row.get::<_, i64>(0))
        .map_err(storage_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage_error)?;
    for id in ids {
        super::vocabulary::coalesce_unknown_senses(connection, id)?;
    }
    let overflow = {
        let mut statement = connection
            .prepare(
                "SELECT vocabulary_entry_id, count(*)
                 FROM vocabulary_senses
                 GROUP BY vocabulary_entry_id
                 HAVING count(*) > 32",
            )
            .map_err(storage_error)?;
        let rows = statement
            .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))
            .map_err(storage_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage_error)?;
        rows
    };
    for (entry_id, count) in overflow {
        eprintln!(
            "lexical sense overflow: provider=textbook vocabulary_entry_id={entry_id} retained=32 discarded={}",
            count - 32
        );
    }
    connection
        .execute(
            "DELETE FROM vocabulary_senses
             WHERE id IN (
               SELECT id FROM (
                 SELECT id,
                        row_number() OVER (
                          PARTITION BY vocabulary_entry_id
                          ORDER BY is_primary DESC, rank, id
                        ) AS ordinal
                 FROM vocabulary_senses
               ) WHERE ordinal > 32
             )",
            [],
        )
        .map_err(storage_error)?;
    connection
        .execute(
            "UPDATE vocabulary_senses AS current
             SET rank = (
               SELECT count(*) FROM vocabulary_senses AS prior
               WHERE prior.vocabulary_entry_id = current.vocabulary_entry_id
                 AND (
                   prior.is_primary > current.is_primary
                   OR (prior.is_primary = current.is_primary AND prior.rank < current.rank)
                   OR (prior.is_primary = current.is_primary AND prior.rank = current.rank AND prior.id < current.id)
                 )
             )",
            [],
        )
        .map_err(storage_error)?;
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
    let lexical_parts = read_lexical_parts(&source)?;
    let mut statement = source
        .prepare("SELECT written_rep, trans_list FROM simple_translation ORDER BY written_rep")
        .map_err(|_| invalid_package())?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|_| invalid_package())?;
    let mut distinct_pairs = HashSet::with_capacity(count as usize);
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
            || !distinct_pairs.insert((source_text.clone(), translated_text.clone()))
        {
            return Err(invalid_package());
        }
        let (display, originals, aliases) = normalized_zh_cn(&converter, &translated_text)?;
        let parts = lexical_parts
            .get(&(source_text.clone(), translated_text.clone()))
            .cloned()
            .unwrap_or_default();
        let part_of_speech = parts.first().copied();
        let row_senses = normalized_senses(&converter, &translated_text)
            .into_iter()
            .flat_map(|text| {
                if parts.is_empty() {
                    vec![(text, None)]
                } else {
                    parts
                        .iter()
                        .map(|part| (text.clone(), Some(*part)))
                        .collect()
                }
            })
            .collect::<Vec<_>>();
        if let Some(existing) = imported.get_mut(&key) {
            append_unique(&mut existing.original_translations, originals);
            append_unique(&mut existing.aliases, aliases);
            for sense in row_senses {
                if !existing.senses.contains(&sense) {
                    existing.senses.push(sense);
                }
            }
            if source_text == key && existing.source_text != key {
                existing.source_text = source_text;
            }
            merge_part_of_speech(existing, part_of_speech);
        } else {
            imported.insert(
                key.clone(),
                ImportedEntry {
                    normalized_source: key,
                    source_text,
                    translated_text: display,
                    original_translations: originals,
                    aliases,
                    senses: row_senses,
                    part_of_speech,
                    part_of_speech_conflicted: false,
                },
            );
        }
    }
    Ok(imported.into_values().collect())
}

fn read_lexical_parts(
    source: &Connection,
) -> Result<HashMap<(String, String), Vec<PartOfSpeech>>, AppError> {
    let columns = source
        .prepare("PRAGMA table_info(translation)")
        .and_then(|mut statement| {
            statement
                .query_map([], |row| row.get::<_, String>(1))?
                .collect::<Result<HashSet<_>, _>>()
        })
        .map_err(|_| invalid_package())?;
    if !["written_rep", "trans_list", "lexentry"]
        .iter()
        .all(|required| columns.contains(*required))
    {
        return Ok(HashMap::new());
    }
    let score = if columns.contains("score") {
        "CAST(coalesce(score, 0) AS REAL)"
    } else {
        "0"
    };
    let is_good = if columns.contains("is_good") {
        "CAST(coalesce(is_good, 0) AS INTEGER)"
    } else {
        "0"
    };
    let importance = if columns.contains("importance") {
        "CAST(coalesce(importance, 0) AS REAL)"
    } else {
        "0"
    };
    let sql = format!(
        "SELECT written_rep, trans_list, lexentry, {score}, {is_good}, {importance}
         FROM translation
         WHERE typeof(written_rep) = 'text'
           AND typeof(trans_list) = 'text'
           AND typeof(lexentry) = 'text'"
    );
    let mut statement = source.prepare(&sql).map_err(|_| invalid_package())?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, f64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, f64>(5)?,
            ))
        })
        .map_err(|_| invalid_package())?;
    let mut ranked = HashMap::<(String, String), Vec<LexicalCandidate>>::new();
    for row in rows {
        let (source_text, translated_text, lexentry, score, is_good, importance) =
            row.map_err(|_| invalid_package())?;
        let Some(part_of_speech) = parse_part_of_speech(&lexentry) else {
            continue;
        };
        let candidate = LexicalCandidate {
            part_of_speech,
            score,
            is_good,
            importance,
        };
        let key = (clean(&source_text), clean(&translated_text));
        let parts = ranked.entry(key).or_default();
        if let Some(current) = parts
            .iter_mut()
            .find(|current| current.part_of_speech == part_of_speech)
        {
            if lexical_candidate_precedes(&candidate, current) {
                *current = candidate;
            }
        } else {
            parts.push(candidate);
        }
    }
    Ok(ranked
        .into_iter()
        .map(|(key, mut parts)| {
            parts.sort_by(|left, right| {
                if lexical_candidate_precedes(left, right) {
                    std::cmp::Ordering::Less
                } else if lexical_candidate_precedes(right, left) {
                    std::cmp::Ordering::Greater
                } else {
                    std::cmp::Ordering::Equal
                }
            });
            (
                key,
                parts
                    .into_iter()
                    .map(|candidate| candidate.part_of_speech)
                    .collect(),
            )
        })
        .collect())
}

fn lexical_candidate_precedes(left: &LexicalCandidate, right: &LexicalCandidate) -> bool {
    left.score
        .total_cmp(&right.score)
        .then_with(|| left.is_good.cmp(&right.is_good))
        .then_with(|| left.importance.total_cmp(&right.importance))
        .then_with(|| right.part_of_speech.cmp(&left.part_of_speech))
        .is_gt()
}

fn parse_part_of_speech(lexentry: &str) -> Option<PartOfSpeech> {
    let (head, sense) = lexentry.rsplit_once("__")?;
    if sense.is_empty() || !sense.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let (_, category) = head.rsplit_once("__")?;
    canonical_part_of_speech(category)
}

fn canonical_part_of_speech(category: &str) -> Option<PartOfSpeech> {
    PartOfSpeech::from_normalized(category)
}

fn merge_part_of_speech(entry: &mut ImportedEntry, candidate: Option<PartOfSpeech>) {
    if entry.part_of_speech_conflicted {
        return;
    }
    match (&entry.part_of_speech, candidate) {
        (None, Some(value)) => entry.part_of_speech = Some(value),
        (Some(current), Some(value)) if current != &value => {
            entry.part_of_speech = None;
            entry.part_of_speech_conflicted = true;
        }
        _ => {}
    }
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
               promoted_at_epoch_ms INTEGER NOT NULL
             );
             CREATE INDEX vocabulary_textbook_provenance_identity
               ON vocabulary_textbook_provenance(
                 vocabulary_entry_id, textbook_id, textbook_version, source_text
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
    if version < 3 {
        let tx = connection.unchecked_transaction().map_err(storage_error)?;
        tx.execute_batch(
            "ALTER TABLE textbook_entries ADD COLUMN part_of_speech TEXT;
             INSERT INTO textbook_schema_migrations(version) VALUES (3);",
        )
        .map_err(storage_error)?;
        tx.commit().map_err(storage_error)?;
    }
    if version < 4 {
        let tx = connection.unchecked_transaction().map_err(storage_error)?;
        tx.execute_batch(
            "ALTER TABLE textbooks ADD COLUMN import_revision INTEGER NOT NULL DEFAULT 0;
             INSERT INTO textbook_schema_migrations(version) VALUES (4);",
        )
        .map_err(storage_error)?;
        tx.commit().map_err(storage_error)?;
    }
    if version < 5 {
        let tx = connection.unchecked_transaction().map_err(storage_error)?;
        tx.execute_batch(
            "ALTER TABLE textbooks ADD COLUMN catalog_revision TEXT NOT NULL DEFAULT '';
             ALTER TABLE textbooks ADD COLUMN lexical_refresh_status TEXT NOT NULL DEFAULT 'legacy'
               CHECK(lexical_refresh_status IN ('legacy', 'exact', 'enriched', 'unavailable-legacy'));
             CREATE TABLE textbook_entry_senses (
               id INTEGER PRIMARY KEY,
               textbook_entry_id INTEGER NOT NULL REFERENCES textbook_entries(id) ON DELETE CASCADE,
               display_text TEXT NOT NULL,
               normalized_text TEXT NOT NULL,
               pos_key TEXT NOT NULL DEFAULT '',
               rank INTEGER NOT NULL,
               is_primary INTEGER NOT NULL CHECK(is_primary IN (0, 1)),
               UNIQUE(textbook_entry_id, normalized_text, pos_key)
             );
             CREATE UNIQUE INDEX textbook_entry_senses_one_primary
               ON textbook_entry_senses(textbook_entry_id) WHERE is_primary = 1;
             CREATE INDEX textbook_entry_senses_relation
               ON textbook_entry_senses(normalized_text, pos_key, textbook_entry_id);
             INSERT INTO textbook_entry_senses (
               textbook_entry_id, display_text, normalized_text, pos_key, rank, is_primary
             ) SELECT id, translated_text, lower(trim(translated_text)),
                      coalesce(part_of_speech, ''), 0, 1 FROM textbook_entries;
             CREATE TABLE morphemes (
               id TEXT PRIMARY KEY,
               language TEXT NOT NULL,
               canonical_form TEXT NOT NULL,
               display TEXT NOT NULL,
               kind TEXT NOT NULL CHECK(kind IN ('prefix', 'root', 'suffix')),
               artifact_version TEXT NOT NULL
             );
             CREATE TABLE textbook_entry_morphemes (
               textbook_entry_id INTEGER NOT NULL REFERENCES textbook_entries(id) ON DELETE CASCADE,
               morpheme_id TEXT NOT NULL REFERENCES morphemes(id),
               ordinal INTEGER NOT NULL,
               PRIMARY KEY(textbook_entry_id, morpheme_id, ordinal)
             );
             CREATE INDEX textbook_entry_morphemes_relation
               ON textbook_entry_morphemes(morpheme_id, textbook_entry_id);
             INSERT INTO textbook_schema_migrations(version) VALUES (5);",
        )
        .map_err(storage_error)?;
        tx.commit().map_err(storage_error)?;
    }
    if version < 6 {
        let tx = connection.unchecked_transaction().map_err(storage_error)?;
        // Replace only the status column's constraint. Entry IDs, foreign keys,
        // pinned metadata, and existing status values remain intact.
        tx.execute_batch(
            "ALTER TABLE textbooks RENAME COLUMN lexical_refresh_status TO lexical_refresh_status_v5;
             ALTER TABLE textbooks ADD COLUMN lexical_refresh_status TEXT NOT NULL DEFAULT 'legacy'
               CHECK(lexical_refresh_status IN ('legacy', 'exact', 'enriched', 'unavailable-legacy'));
             UPDATE textbooks SET lexical_refresh_status = lexical_refresh_status_v5;
             ALTER TABLE textbooks DROP COLUMN lexical_refresh_status_v5;
             INSERT INTO textbook_schema_migrations(version) VALUES (6);"
        ).map_err(storage_error)?;
        tx.commit().map_err(storage_error)?;
    }
    // Reconcile older prerelease v5 databases as well as newly migrated v4 rows.
    // No networking and no changes to entries, pins, active state, or provenance.
    let pins = connection
        .prepare("SELECT id, download_url, expected_bytes, sha256 FROM textbooks")
        .map_err(storage_error)?
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(storage_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage_error)?;
    for (id, url, bytes, sha) in pins {
        if !valid_package_pin(&url, u64::try_from(bytes).unwrap_or(0), &sha) {
            connection.execute("UPDATE textbooks SET lexical_refresh_status = 'unavailable-legacy' WHERE id = ?1", params![id]).map_err(storage_error)?;
        } else {
            connection.execute("UPDATE textbooks SET lexical_refresh_status = 'exact' WHERE id = ?1 AND lexical_refresh_status = 'legacy'", params![id]).map_err(storage_error)?;
        }
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MorphemeManifest {
    schema_version: u8,
    dataset_version: String,
    license: String,
    attribution: String,
    source_url: String,
    expected_bytes: usize,
    sha256: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MorphemeArtifact {
    schema_version: u8,
    dataset_version: String,
    license: String,
    attribution: String,
    morphemes: Vec<MorphemeRecord>,
    headwords: Vec<MorphemeHeadword>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MorphemeRecord {
    id: String,
    language: String,
    canonical_form: String,
    display: String,
    kind: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MorphemeHeadword {
    normalized_headword: String,
    source_url: String,
    source_revision: String,
    reviewed_at: String,
    components: Vec<MorphemeComponent>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MorphemeComponent {
    morpheme_id: String,
    surface: String,
    ordinal: usize,
}

fn install_morpheme_catalog(connection: &Connection) -> Result<(), AppError> {
    let manifest: MorphemeManifest =
        serde_json::from_str(MORPHEME_MANIFEST).map_err(|_| invalid_package())?;
    if manifest.schema_version != 1
        || manifest.expected_bytes != MORPHEME_BYTES.len()
        || format!("{:x}", Sha256::digest(MORPHEME_BYTES)) != manifest.sha256
        || manifest.license.trim().is_empty()
        || manifest.attribution.trim().is_empty()
        || !manifest.source_url.starts_with("https://")
    {
        return Err(invalid_package());
    }
    let artifact: MorphemeArtifact =
        serde_json::from_slice(MORPHEME_BYTES).map_err(|_| invalid_package())?;
    if artifact.schema_version != manifest.schema_version
        || artifact.dataset_version != manifest.dataset_version
        || artifact.license != manifest.license
        || artifact.attribution.trim().is_empty()
        || artifact.morphemes.len() > 4_096
        || artifact.headwords.len() > 100_000
    {
        return Err(invalid_package());
    }
    let mut ids = HashSet::new();
    for morpheme in &artifact.morphemes {
        if !ids.insert(morpheme.id.as_str())
            || morpheme.language != "en"
            || morpheme.canonical_form.trim().is_empty()
            || morpheme.display.trim().is_empty()
            || !matches!(morpheme.kind.as_str(), "prefix" | "root" | "suffix")
        {
            return Err(invalid_package());
        }
    }
    let mut headwords = HashSet::new();
    for headword in &artifact.headwords {
        let mut components = headword.components.iter().collect::<Vec<_>>();
        components.sort_by_key(|component| component.ordinal);
        let reconstructed = components
            .iter()
            .map(|item| item.surface.as_str())
            .collect::<String>();
        if !headwords.insert(headword.normalized_headword.as_str())
            || reconstructed != headword.normalized_headword
            || components.len() > 12
            || components.iter().enumerate().any(|(ordinal, component)| {
                component.ordinal != ordinal || !ids.contains(component.morpheme_id.as_str())
            })
            || !headword
                .source_url
                .starts_with("https://en.wiktionary.org/")
            || headword.source_revision.trim().is_empty()
            || headword.reviewed_at.trim().is_empty()
        {
            return Err(invalid_package());
        }
    }
    let tx = connection.unchecked_transaction().map_err(storage_error)?;
    tx.execute("DELETE FROM textbook_entry_morphemes", [])
        .map_err(storage_error)?;
    tx.execute("DELETE FROM morphemes", [])
        .map_err(storage_error)?;
    for morpheme in artifact.morphemes {
        tx.execute(
            "INSERT INTO morphemes(id, language, canonical_form, display, kind, artifact_version)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                morpheme.id,
                morpheme.language,
                morpheme.canonical_form,
                morpheme.display,
                morpheme.kind,
                manifest.dataset_version
            ],
        )
        .map_err(storage_error)?;
    }
    for headword in artifact.headwords {
        for component in headword.components {
            tx.execute(
                "INSERT OR IGNORE INTO textbook_entry_morphemes(textbook_entry_id, morpheme_id, ordinal)
                 SELECT id, ?1, ?2 FROM textbook_entries WHERE normalized_source = ?3",
                params![component.morpheme_id, component.ordinal as i64, headword.normalized_headword],
            )
            .map_err(storage_error)?;
        }
    }
    tx.commit().map_err(storage_error)
}

fn table_has_column(connection: &Connection, table: &str, column: &str) -> Result<bool, AppError> {
    let sql = format!("PRAGMA table_info({table})");
    let mut statement = connection.prepare(&sql).map_err(storage_error)?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(storage_error)?;
    for current in columns {
        if current.map_err(storage_error)? == column {
            return Ok(true);
        }
    }
    Ok(false)
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
        metadata_refresh_available: row.get(11)?,
        lexical_refresh_status: row.get(12)?,
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

fn normalized_senses(converter: &OpenCC, translated_text: &str) -> Vec<String> {
    let mut senses = Vec::new();
    for original in translated_text.split('|').map(clean) {
        let simplified = clean(&converter.convert_with_config(&original, OpenccConfig::T2s, false));
        if !simplified.is_empty() && !senses.contains(&simplified) {
            senses.push(simplified);
        }
    }
    senses
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

    fn wikdict_fixture_with_lexical_rows(
        entries: &[(&str, &str)],
        lexical_rows: &[(&str, &str, &str, f64, i64, f64)],
    ) -> NamedTempFile {
        let file = wikdict_fixture(entries);
        let connection = Connection::open(file.path()).expect("open lexical fixture");
        connection
            .execute_batch(
                "CREATE TABLE translation (
                   lexentry TEXT,
                   written_rep TEXT NOT NULL,
                   trans_list TEXT NOT NULL,
                   score REAL,
                   is_good INTEGER,
                   importance REAL
                 );",
            )
            .expect("lexical schema");
        for (source, translation, lexentry, score, is_good, importance) in lexical_rows {
            connection
                .execute(
                    "INSERT INTO translation (written_rep, trans_list, lexentry, score, is_good, importance)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![source, translation, lexentry, score, is_good, importance],
                )
                .expect("lexical row");
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
    fn curated_catalog_offers_bundled_and_five_pinned_simplified_chinese_choices() {
        let catalog = curated_catalog();
        assert_eq!(catalog.len(), 6);
        assert_eq!(catalog[0].id, BUNDLED_STARTER_ID);
        assert_eq!(
            catalog[0].expected_bytes,
            BUNDLED_STARTER_BYTES.len() as u64
        );
        assert!(catalog.iter().any(|item| item.id.starts_with("ngsl-")));
        assert!(catalog.iter().any(|item| item.id.starts_with("nawl-")));
        assert!(catalog.iter().any(|item| item.id.starts_with("tsl-")));
        assert!(catalog.iter().any(|item| item.id.starts_with("bsl-")));
        assert_eq!(catalog[1].expected_bytes, 5_169_152);
        assert_eq!(
            catalog[1].sha256,
            "16cf69dc8037a8d4dc6bde260142bf0181f9ff0a008d457f26452f1d80ca5ecd"
        );
        assert!(catalog[1]
            .download_url
            .starts_with("https://download.wikdict.com/"));
    }

    #[test]
    fn bundled_starter_installs_offline_and_becomes_active() {
        let app_db = NamedTempFile::new().expect("app db");
        let store = TextbookStore::open(app_db.path()).expect("store");
        store.ensure_bundled_starter(10).expect("bundled install");
        let installed = store
            .get_installed(BUNDLED_STARTER_ID)
            .expect("query")
            .expect("installed");
        assert!(installed.active);
        assert_eq!(installed.target_language, "zh-CN");
        assert!(installed.entry_count >= 75);
    }

    #[test]
    fn parses_each_official_learning_scope_format() {
        let ngsl = parse_scope_words(
            b"## notes\na,an\nabandon,abandons,abandoned\n",
            ScopeFormat::TeachingCsv,
            2,
        )
        .expect("NGSL");
        assert_eq!(ngsl, HashSet::from(["a".into(), "abandon".into()]));

        let plain = parse_scope_words(
            b"Description\n\nReferences\nCitation\n\nabdominal\nabsorb\n",
            ScopeFormat::DescriptionList,
            2,
        )
        .expect("NAWL/BSL");
        assert_eq!(plain, HashSet::from(["abdominal".into(), "absorb".into()]));

        let numbered = parse_scope_words(
            b"TOEIC Service List\n\n1.  abide\n2.  aboard\n",
            ScopeFormat::NumberedList,
            2,
        )
        .expect("TSL");
        assert_eq!(numbered, HashSet::from(["abide".into(), "aboard".into()]));
    }

    #[test]
    fn focused_install_keeps_only_declared_scope_words() {
        let app_db = NamedTempFile::new().expect("app db");
        let store = TextbookStore::open(app_db.path()).expect("store");
        let fixture = wikdict_fixture(&[
            ("abandon", "放弃"),
            ("academic", "学术的"),
            ("zoo", "动物园"),
        ]);
        let catalog = test_catalog(fixture.path(), "focused", "1");
        let bytes = std::fs::read(fixture.path()).expect("fixture bytes");
        let scope = HashSet::from(["abandon".into(), "academic".into()]);

        let installed = store
            .install_verified_bytes_scoped(&catalog, &bytes, Some(&scope), 10)
            .expect("focused install");
        assert_eq!(installed.entry_count, 2);
        assert_eq!(
            store
                .list_entries("focused", None, 0, 50)
                .expect("page")
                .total,
            2
        );
    }

    #[test]
    fn wikdict_import_uses_pair_specific_ranked_canonical_part_of_speech() {
        let app_db = NamedTempFile::new().expect("app db");
        let store = TextbookStore::open(app_db.path()).expect("store");
        let fixture = wikdict_fixture_with_lexical_rows(
            &[("essential", "本质"), ("fallback", "后备")],
            &[
                ("essential", "本质", "eng/essential__Noun__1", 1.0, 1, 1.0),
                (
                    "essential",
                    "本质",
                    "eng/essential__Adjective__1",
                    2.0,
                    1,
                    1.0,
                ),
                (
                    "essential",
                    "other meaning",
                    "eng/essential__Verb__1",
                    100.0,
                    1,
                    100.0,
                ),
                ("fallback", "后备", "malformed", 10.0, 1, 10.0),
            ],
        );
        let catalog = test_catalog(fixture.path(), "pos", "1");

        store
            .install_sqlite(&catalog, fixture.path(), 10)
            .expect("install");
        let entries = store
            .list_entries("pos", None, 0, 50)
            .expect("entries")
            .entries;

        let essential = entries
            .iter()
            .find(|entry| entry.source_text == "essential")
            .unwrap();
        let connection = store.connection.lock().unwrap();
        let parts = connection.prepare("SELECT pos_key FROM textbook_entry_senses WHERE textbook_entry_id = ?1 ORDER BY rank").unwrap().query_map(params![essential.id], |row| row.get::<_, String>(0)).unwrap().collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(parts, vec!["adjective", "noun"]);
        drop(connection);

        assert_eq!(
            entries
                .iter()
                .find(|entry| entry.source_text == "essential")
                .and_then(|entry| entry.part_of_speech),
            Some(PartOfSpeech::Adjective)
        );
        assert_eq!(
            entries
                .iter()
                .find(|entry| entry.source_text == "fallback")
                .and_then(|entry| entry.part_of_speech),
            None
        );
    }

    #[tokio::test]
    async fn legacy_refresh_rejects_non_hex_pins_on_reopen() {
        let db = NamedTempFile::new().unwrap();
        let fixture = wikdict_fixture(&[("book", "书")]);
        let catalog = test_catalog(fixture.path(), "legacy", "old");
        let store = TextbookStore::open(db.path()).unwrap();
        store.install_sqlite(&catalog, fixture.path(), 1).unwrap();
        store.connection.lock().unwrap().execute("UPDATE textbooks SET sha256 = ?1, lexical_refresh_status = 'exact' WHERE id = 'legacy'", params!["z".repeat(64)]).unwrap();
        drop(store);
        let reopened = TextbookStore::open(db.path()).unwrap();
        assert_eq!(
            reopened
                .get_installed("legacy")
                .unwrap()
                .unwrap()
                .lexical_refresh_status,
            "unavailable-legacy"
        );
        assert_eq!(
            reopened.list_entries("legacy", None, 0, 10).unwrap().total,
            1
        );
        let staging = tempfile::tempdir().unwrap();
        assert!(reopened
            .download_and_install("legacy", staging.path(), 3)
            .await
            .is_err());
        assert_eq!(fs::read_dir(staging.path()).unwrap().count(), 0);
        assert_eq!(
            reopened
                .get_installed("legacy")
                .unwrap()
                .unwrap()
                .installed_at_epoch_ms,
            1
        );
    }

    #[test]
    fn refresh_plan_uses_stored_pin_and_membership_not_current_catalog() {
        let db = NamedTempFile::new().unwrap();
        let store = TextbookStore::open(db.path()).unwrap();
        let fixture = wikdict_fixture_with_lexical_rows(
            &[("record", "记录"), ("book", "书")],
            &[("record", "记录", "eng/record__Verb__1", 1.0, 1, 1.0)],
        );
        // Deliberately share an ID with today's catalog, but not its pin/version.
        let catalog = test_catalog(fixture.path(), "nawl-en-zh-1-2", "legacy-pin");
        let bytes = fs::read(fixture.path()).unwrap();
        let words = HashSet::from(["record".into()]);
        store
            .install_verified_bytes_scoped(&catalog, &bytes, Some(&words), 1)
            .unwrap();
        store.set_active(Some(&catalog.id)).unwrap();
        let plan = store.download_plan(&catalog.id).unwrap();
        assert_eq!(plan.catalog.download_url, catalog.download_url);
        assert_eq!(plan.catalog.expected_bytes, catalog.expected_bytes);
        assert_eq!(plan.catalog.sha256, catalog.sha256);
        assert_eq!(plan.catalog.version, "legacy-pin");
        let DownloadScope::Installed(scope) = plan.scope else {
            panic!("refresh substituted catalog scope")
        };
        assert_eq!(scope, words);
        assert!(store
            .install_verified_bytes_scoped(&plan.catalog, b"wrong bytes", Some(&scope), 2)
            .is_err());
        let unchanged = store.get_installed(&catalog.id).unwrap().unwrap();
        assert!(unchanged.active);
        assert_eq!(unchanged.version, "legacy-pin");
        assert_eq!(unchanged.installed_at_epoch_ms, 1);
        let refreshed = store
            .install_verified_bytes_scoped(&plan.catalog, &bytes, Some(&scope), 3)
            .unwrap();
        assert!(refreshed.active);
        assert_eq!(refreshed.entry_count, 1);
        assert_eq!(
            store
                .list_entries(&catalog.id, None, 0, 10)
                .unwrap()
                .entries[0]
                .source_text,
            "record"
        );
    }

    #[test]
    fn refresh_pin_validation_covers_url_size_and_hex_boundaries() {
        let url = "https://download.wikdict.com/old.sqlite3";
        let sha = "A0".repeat(32);
        assert!(valid_package_pin(url, 1, &sha));
        for bad in [
            "https://",
            "https://evil.example/old.sqlite3",
            "http://download.wikdict.com/old.sqlite3",
            "https://user@download.wikdict.com/old.sqlite3",
            "https://download.wikdict.com:444/old.sqlite3",
        ] {
            assert!(!valid_package_pin(bad, 1, &sha));
        }
        assert!(!valid_package_pin(url, 0, &sha));
        assert!(!valid_package_pin(url, MAX_ARTIFACT_BYTES + 1, &sha));
        assert!(!valid_package_pin(url, 1, &"z".repeat(64)));
        assert!(!valid_package_pin(url, 1, "abc"));
    }

    #[test]
    fn uppercase_stored_sha_is_refreshable() {
        let db = NamedTempFile::new().unwrap();
        let store = TextbookStore::open(db.path()).unwrap();
        let fixture = wikdict_fixture(&[("record", "记录")]);
        let catalog = test_catalog(fixture.path(), "upper", "1");
        store.install_sqlite(&catalog, fixture.path(), 1).unwrap();
        store
            .connection
            .lock()
            .unwrap()
            .execute(
                "UPDATE textbooks SET sha256 = upper(sha256) WHERE id = 'upper'",
                [],
            )
            .unwrap();
        let plan = store
            .download_plan("upper")
            .expect("valid uppercase stored SHA");
        let DownloadScope::Installed(words) = plan.scope else {
            panic!("stored scope")
        };
        store
            .install_verified_bytes_scoped(
                &plan.catalog,
                &fs::read(fixture.path()).unwrap(),
                Some(&words),
                2,
            )
            .unwrap();
    }

    #[test]
    fn v5_refresh_status_upgrade_retains_rows_and_allows_legacy_default() {
        let db = NamedTempFile::new().unwrap();
        let store = TextbookStore::open(db.path()).unwrap();
        let fixture = wikdict_fixture(&[("record", "记录")]);
        let catalog = test_catalog(fixture.path(), "v5", "1");
        store.install_sqlite(&catalog, fixture.path(), 1).unwrap();
        store.set_active(Some("v5")).unwrap();
        // Model the already-installed prerelease schema, not only fresh DDL.
        store
            .connection
            .lock()
            .unwrap()
            .execute_batch(
                "ALTER TABLE textbooks DROP COLUMN lexical_refresh_status;
             ALTER TABLE textbooks ADD COLUMN lexical_refresh_status TEXT NOT NULL DEFAULT 'exact'
               CHECK(lexical_refresh_status IN ('exact', 'enriched', 'unavailable-legacy'));
             DELETE FROM textbook_schema_migrations WHERE version > 5;",
            )
            .unwrap();
        drop(store);
        let reopened = TextbookStore::open(db.path()).unwrap();
        assert!(reopened.get_installed("v5").unwrap().unwrap().active);
        assert_eq!(reopened.list_entries("v5", None, 0, 10).unwrap().total, 1);
        let connection = reopened.connection.lock().unwrap();
        connection
            .execute(
                "UPDATE textbooks SET lexical_refresh_status = 'legacy' WHERE id = 'v5'",
                [],
            )
            .expect("legacy is a supported state");
        let default: String = connection.query_row("SELECT dflt_value FROM pragma_table_info('textbooks') WHERE name = 'lexical_refresh_status'", [], |row| row.get(0)).unwrap();
        assert_eq!(default, "'legacy'");
        let violations: i64 = connection
            .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(violations, 0);
    }

    #[test]
    fn invalid_scope_artifact_cannot_replace_an_installed_version() {
        let app_db = NamedTempFile::new().expect("app db");
        let store = TextbookStore::open(app_db.path()).expect("store");
        let fixture = wikdict_fixture(&[("abandon", "放弃"), ("academic", "学术的")]);
        let catalog = test_catalog(fixture.path(), "focused", "1");
        store
            .install_sqlite(&catalog, fixture.path(), 10)
            .expect("install v1");

        let valid = b"## notes\nabandon,abandoned\n";
        let artifact = ScopeArtifact {
            url: "https://static1.squarespace.com/scope.csv",
            expected_bytes: valid.len() as u64,
            sha256: "be08b297c68ef379557fb26ff08a376986e9eec37a5bb6fb4f128719508a9bed",
            format: ScopeFormat::TeachingCsv,
            expected_entries: 1,
        };
        assert!(verified_scope_words(b"tampered", &artifact).is_err());

        let installed = store
            .get_installed("focused")
            .expect("query")
            .expect("v1 remains");
        assert_eq!(installed.version, "1");
        assert_eq!(installed.entry_count, 2);
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
        assert!(!installed[0].metadata_refresh_available);

        let second_catalog = test_catalog(broken.path(), "test-en-zh", "2");
        store
            .install_sqlite(&second_catalog, broken.path(), 30)
            .expect("update");
        let installed = store.list_installed().expect("updated");
        assert_eq!(installed[0].version, "2");
        assert!(installed[0].active);
        assert_eq!(installed[0].entry_count, 1);
        assert!(!installed[0].metadata_refresh_available);
    }

    #[test]
    fn installed_entries_are_indexed_for_curated_morpheme_navigation() {
        let file = NamedTempFile::new().expect("app db");
        let _vocabulary = crate::services::vocabulary::VocabularyStore::open(file.path())
            .expect("vocabulary store");
        let store = TextbookStore::open(file.path()).expect("textbook store");
        let fixture = wikdict_fixture(&[("produce", "生产"), ("reduce", "减少")]);
        let catalog = test_catalog(fixture.path(), "morpheme-en-zh", "1");
        store
            .install_sqlite(&catalog, fixture.path(), 10)
            .expect("install fixture");

        let parts = store
            .morphemes_for_word("produce", "en", "zh-CN")
            .expect("morphemes");
        let root = parts
            .iter()
            .find(|part| part.display == "+duce")
            .expect("duce root");
        assert_eq!(root.textbook_word_count, 1);

        let page = store
            .filtered_related(
                "produce",
                "en",
                "zh-CN",
                &RelatedFilter::Morpheme {
                    morpheme_id: root.id.clone(),
                },
                0,
                20,
            )
            .expect("related by root");
        assert_eq!(page.total, 1);
        assert_eq!(page.items[0].source_text, "reduce");
    }

    #[test]
    fn same_version_verified_reinstall_clears_legacy_metadata_refresh() {
        let app_db = NamedTempFile::new().expect("app db");
        let store = TextbookStore::open(app_db.path()).expect("store");
        let fixture = wikdict_fixture_with_lexical_rows(
            &[("supersede", "代替")],
            &[("supersede", "代替", "eng/supersede__Verb__1", 1.0, 1, 1.0)],
        );
        let catalog = test_catalog(fixture.path(), "test-en-zh", "1");
        store
            .install_sqlite(&catalog, fixture.path(), 10)
            .expect("install");
        {
            let connection = store.connection.lock().expect("database lock");
            connection
                .execute(
                    "UPDATE textbooks SET import_revision = 0 WHERE id = ?1",
                    params![catalog.id],
                )
                .expect("mark legacy import");
        }
        assert!(
            store
                .get_installed(&catalog.id)
                .expect("query")
                .expect("installed")
                .metadata_refresh_available
        );

        store
            .install_sqlite(&catalog, fixture.path(), 20)
            .expect("verified refresh");
        let refreshed = store
            .get_installed(&catalog.id)
            .expect("query")
            .expect("installed");
        assert!(!refreshed.metadata_refresh_available);
        assert_eq!(
            store
                .list_entries(&catalog.id, None, 0, 50)
                .expect("entries")
                .entries[0]
                .part_of_speech,
            Some(PartOfSpeech::Verb)
        );
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
    fn merges_distinct_senses_and_rejects_unsafe_catalog_hosts_before_replacement() {
        let app_db = NamedTempFile::new().expect("app db");
        let store = TextbookStore::open(app_db.path()).expect("store");
        let fixture = wikdict_fixture(&[("ephemeral", "蜉蝣"), (" ephemeral ", "短暂的")]);
        let mut catalog = test_catalog(fixture.path(), "duplicate", "1");
        store
            .install_sqlite(&catalog, fixture.path(), 10)
            .expect("distinct senses install");
        let senses = store
            .senses_for_word("ephemeral", "en", "zh-CN")
            .expect("textbook senses");
        assert_eq!(senses.len(), 2);

        catalog.download_url = "https://example.com/deck.sqlite3".into();
        assert!(store.install_sqlite(&catalog, fixture.path(), 10).is_err());
        assert_eq!(store.list_installed().expect("installed").len(), 1);
    }

    #[test]
    fn caps_large_textbook_sense_sets_at_the_native_boundary() {
        let app_db = NamedTempFile::new().expect("app db");
        let store = TextbookStore::open(app_db.path()).expect("store");
        let rows = (0..35)
            .map(|index| ("polysemous".to_owned(), format!("义项{index}")))
            .collect::<Vec<_>>();
        let borrowed = rows
            .iter()
            .map(|(source, translation)| (source.as_str(), translation.as_str()))
            .collect::<Vec<_>>();
        let fixture = wikdict_fixture(&borrowed);
        let catalog = test_catalog(fixture.path(), "bounded-senses", "1");
        store
            .install_sqlite(&catalog, fixture.path(), 10)
            .expect("bounded install");
        let senses = store
            .senses_for_word("polysemous", "en", "zh-CN")
            .expect("textbook senses");
        assert_eq!(senses.len(), 32);
        assert_eq!(senses.first().map(|sense| sense.rank), Some(0));
        assert_eq!(senses.last().map(|sense| sense.rank), Some(31));
    }

    #[test]
    fn promotion_caps_a_distinct_personal_primary_plus_textbook_senses() {
        let app_db = NamedTempFile::new().expect("app db");
        crate::services::vocabulary::VocabularyStore::open(app_db.path())
            .expect("vocabulary migrations");
        let store = TextbookStore::open(app_db.path()).expect("store");
        let rows = (0..32)
            .map(|index| ("polysemous".to_owned(), format!("义项{index}")))
            .collect::<Vec<_>>();
        let borrowed = rows
            .iter()
            .map(|(source, translation)| (source.as_str(), translation.as_str()))
            .collect::<Vec<_>>();
        let fixture = wikdict_fixture(&borrowed);
        let catalog = test_catalog(fixture.path(), "promotion-cap", "1");
        store
            .install_sqlite(&catalog, fixture.path(), 10)
            .expect("install");
        let connection = Connection::open(app_db.path()).expect("inspect");
        connection
            .execute(
                "INSERT INTO vocabulary_entries (
                   normalized_text, source_text, requested_source_language, target_language,
                   translated_text, detected_source_language, effective_source_language,
                   first_seen_epoch_ms, last_seen_epoch_ms
                 ) VALUES ('polysemous', 'polysemous', 'auto', 'zh-CN', '个人主译', 'en', 'en', 1, 1)",
                [],
            )
            .expect("personal entry");
        let entry = store
            .list_entries("promotion-cap", Some("polysemous"), 0, 10)
            .expect("entry")
            .entries
            .into_iter()
            .next()
            .expect("textbook entry");
        let promoted = store.promote_entry(entry.id, 20).expect("promote");
        let (count, primary_count, min_rank, max_rank): (i64, i64, i64, i64) = connection
            .query_row(
                "SELECT count(*), sum(is_primary), min(rank), max(rank)
                 FROM vocabulary_senses WHERE vocabulary_entry_id = ?1",
                params![promoted.vocabulary_entry_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("bounded senses");
        assert_eq!((count, primary_count, min_rank, max_rank), (32, 1, 0, 31));
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
                    'https://www.wikdict.com', 'abacus', '算盘|算盤', 2),
                   (1, 'legacy', 'Legacy', '2', 'CC BY-SA 4.0', 'WikDict',
                    'https://www.wikdict.com', 'abacus', '算盘 | 算盤', 3);",
            )
            .expect("v1 schema and rows");
        drop(connection);

        let store = TextbookStore::open(app_db.path()).expect("migrate v1");
        assert!(
            store
                .get_installed("legacy")
                .expect("query legacy")
                .expect("legacy textbook")
                .metadata_refresh_available
        );
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
                ("1".into(), "算盘".into(), "算盘 | 算盤".into()),
                ("2".into(), "算盘".into(), "算盘 | 算盤".into()),
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
        let fixture = wikdict_fixture_with_lexical_rows(
            &[("ephemeral", "蜉蝣"), ("supersede", "代替")],
            &[("supersede", "代替", "eng/supersede__Verb__1", 1.0, 1, 1.0)],
        );
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
        let (translation, provenance_count, sense_count): (String, i64, i64) = connection
            .query_row(
                "SELECT v.translated_text,
                        (SELECT count(*) FROM vocabulary_textbook_provenance p WHERE p.vocabulary_entry_id = v.id),
                        (SELECT count(*) FROM vocabulary_senses s WHERE s.vocabulary_entry_id = v.id)
                 FROM vocabulary_entries v WHERE v.id = ?1",
                params![promoted.vocabulary_entry_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("personal row");
        assert_eq!(translation, "短暂的（个人）");
        assert_eq!(provenance_count, 1);
        assert_eq!(sense_count, 1);

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
        let (translation, part_of_speech, provenance_count): (String, Option<String>, i64) = connection
            .query_row(
                "SELECT v.translated_text, v.part_of_speech,
                        (SELECT count(*) FROM vocabulary_textbook_provenance p WHERE p.vocabulary_entry_id = v.id)
                 FROM vocabulary_entries v WHERE v.id = ?1",
                params![existing.vocabulary_entry_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("existing personal provenance");
        assert_eq!(translation, "个人译文");
        assert_eq!(part_of_speech, None);
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

    #[test]
    fn reinstall_enriches_matching_promoted_rows_without_guessing() {
        let app_db = NamedTempFile::new().expect("app db");
        crate::services::vocabulary::VocabularyStore::open(app_db.path())
            .expect("vocabulary migrations");
        let store = TextbookStore::open(app_db.path()).expect("store");
        let original = wikdict_fixture(&[("essential", "本质"), ("core", "核心")]);
        store
            .install_sqlite(
                &test_catalog(original.path(), "test-en-zh", "1"),
                original.path(),
                10,
            )
            .expect("install metadata-free version");
        let entries = store
            .list_entries("test-en-zh", None, 0, 10)
            .expect("entries")
            .entries;
        let essential = entries
            .iter()
            .find(|entry| entry.source_text == "essential")
            .expect("essential");
        let core = entries
            .iter()
            .find(|entry| entry.source_text == "core")
            .expect("core");
        let promoted = store.promote_entry(essential.id, 20).expect("promote");
        let promoted_core = store.promote_entry(core.id, 20).expect("promote core");
        let connection = Connection::open(app_db.path()).expect("inspect");
        let before: Option<String> = connection
            .query_row(
                "SELECT part_of_speech FROM vocabulary_entries WHERE id = ?1",
                params![promoted.vocabulary_entry_id],
                |row| row.get(0),
            )
            .expect("before reinstall");
        assert_eq!(before, None);
        connection.execute_batch(
            "INSERT INTO vocabulary_entries (
               id, normalized_text, source_text, requested_source_language, target_language,
               translated_text, detected_source_language, effective_source_language,
               first_seen_epoch_ms, last_seen_epoch_ms
             ) VALUES
               (100, 'essential-personal', 'essential', 'auto', 'zh-CN', '个人译文', 'en', 'en', 1, 1),
               (101, 'essential-fr', 'essential', 'fr', 'zh-CN', '本质', 'fr', 'fr', 1, 1),
               (102, 'essential-es', 'essential', 'en', 'es', '本质', 'en', 'en', 1, 1),
               (103, 'essentials', 'essentials', 'en', 'zh-CN', '本质', 'en', 'en', 1, 1);
             INSERT INTO vocabulary_textbook_provenance (
               vocabulary_entry_id, textbook_id, textbook_title, textbook_version,
               license, attribution, source_url, source_text, translated_text,
               original_translations, promoted_at_epoch_ms
             ) VALUES
               (100, 'test-en-zh', 'Test WikDict', '1', 'CC BY-SA 4.0', 'WikDict contributors', 'https://www.wikdict.com/page/download', 'essential', '本质', '本质', 1),
               (101, 'test-en-zh', 'Test WikDict', '1', 'CC BY-SA 4.0', 'WikDict contributors', 'https://www.wikdict.com/page/download', 'essential', '本质', '本质', 1),
               (102, 'test-en-zh', 'Test WikDict', '1', 'CC BY-SA 4.0', 'WikDict contributors', 'https://www.wikdict.com/page/download', 'essential', '本质', '本质', 1),
               (103, 'test-en-zh', 'Test WikDict', '1', 'CC BY-SA 4.0', 'WikDict contributors', 'https://www.wikdict.com/page/download', 'essential', '本质', '本质', 1);"
        ).expect("mismatched personal rows");

        let enriched = wikdict_fixture_with_lexical_rows(
            &[("essential", "本质"), ("core", "核心")],
            &[
                (
                    "essential",
                    "本质",
                    "eng/essential__Adjective__1",
                    1.0,
                    1,
                    1.0,
                ),
                ("core", "核心", "eng/core__Noun__1", 1.0, 1, 1.0),
            ],
        );
        store
            .install_sqlite(
                &test_catalog(enriched.path(), "test-en-zh", "2"),
                enriched.path(),
                30,
            )
            .expect("reinstall enriched version");

        let after: Option<String> = connection
            .query_row(
                "SELECT part_of_speech FROM vocabulary_entries WHERE id = ?1",
                params![promoted.vocabulary_entry_id],
                |row| row.get(0),
            )
            .expect("after reinstall");
        assert_eq!(after.as_deref(), Some("adjective"));
        let core_after: Option<String> = connection
            .query_row(
                "SELECT part_of_speech FROM vocabulary_entries WHERE id = ?1",
                params![promoted_core.vocabulary_entry_id],
                |row| row.get(0),
            )
            .expect("core after reinstall");
        assert_eq!(core_after.as_deref(), Some("noun"));
        let mismatched_non_null: i64 = connection
            .query_row(
                "SELECT count(*) FROM vocabulary_entries
                 WHERE id IN (100, 101, 102, 103) AND part_of_speech IS NOT NULL",
                [],
                |row| row.get(0),
            )
            .expect("mismatched rows remain unknown");
        assert_eq!(mismatched_non_null, 0);
    }

    #[test]
    fn promotion_rejects_source_equal_translation_without_persisting_junk() {
        let app_db = NamedTempFile::new().expect("app db");
        crate::services::vocabulary::VocabularyStore::open(app_db.path()).expect("vocabulary");
        let store = TextbookStore::open(app_db.path()).expect("store");
        let fixture = wikdict_fixture(&[("same", "same")]);
        let catalog = test_catalog(fixture.path(), "same-pair", "1");
        store
            .install_sqlite(&catalog, fixture.path(), 10)
            .expect("install");
        let entry = store
            .list_entries("same-pair", None, 0, 50)
            .expect("entries")
            .entries
            .remove(0);

        assert!(store.promote_entry(entry.id, 20).is_err());
        let count: i64 = Connection::open(app_db.path())
            .expect("inspect")
            .query_row("SELECT count(*) FROM vocabulary_entries", [], |row| {
                row.get(0)
            })
            .expect("count");
        assert_eq!(count, 0);
    }
}
