//! Application-local vocabulary cache, recall model, and practice selection.

use std::{
    collections::HashSet,
    path::Path,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::{
    contracts::{
        AppError, AppErrorCode, PracticeDirection, PracticeOutcome, PracticeQuestion,
        RelatedVocabulary, StudyPracticeOutcome, TranslationRequest, TranslationResult,
        VocabularyEntry, VocabularyProvenance,
    },
    services::{textbooks::TextbookStore, TranslationProvider},
};

pub struct VocabularyStore {
    connection: Mutex<Connection>,
}

impl VocabularyStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, AppError> {
        Self::from_connection(Connection::open(path).map_err(storage_error)?)
    }

    #[cfg(test)]
    fn in_memory() -> Result<Self, AppError> {
        Self::from_connection(Connection::open_in_memory().map_err(storage_error)?)
    }

    fn from_connection(connection: Connection) -> Result<Self, AppError> {
        connection
            .execute_batch(
                "PRAGMA foreign_keys = ON;
             PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS vocabulary_entries (
               id INTEGER PRIMARY KEY,
               normalized_text TEXT NOT NULL,
               source_text TEXT NOT NULL,
               requested_source_language TEXT NOT NULL,
               target_language TEXT NOT NULL,
               translated_text TEXT NOT NULL,
               detected_source_language TEXT,
               effective_source_language TEXT NOT NULL,
               lookup_count INTEGER NOT NULL DEFAULT 1,
               first_seen_epoch_ms INTEGER NOT NULL,
               last_seen_epoch_ms INTEGER NOT NULL,
               recall_score REAL NOT NULL DEFAULT 20,
               review_count INTEGER NOT NULL DEFAULT 0,
               correct_count INTEGER NOT NULL DEFAULT 0,
               wrong_count INTEGER NOT NULL DEFAULT 0,
               correct_streak INTEGER NOT NULL DEFAULT 0,
               wrong_streak INTEGER NOT NULL DEFAULT 0,
               last_reviewed_epoch_ms INTEGER,
               last_correct_epoch_ms INTEGER,
               last_wrong_epoch_ms INTEGER,
               UNIQUE(normalized_text, requested_source_language, target_language)
             );
             CREATE TABLE IF NOT EXISTS vocabulary_events (
               id INTEGER PRIMARY KEY,
               entry_id INTEGER NOT NULL REFERENCES vocabulary_entries(id) ON DELETE CASCADE,
               kind TEXT NOT NULL,
               created_at_epoch_ms INTEGER NOT NULL,
               score_before REAL,
               score_after REAL
             );
             CREATE INDEX IF NOT EXISTS vocabulary_events_entry
               ON vocabulary_events(entry_id, created_at_epoch_ms);",
            )
            .map_err(storage_error)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    fn lookup_and_touch(
        &self,
        request: &TranslationRequest,
        now_ms: u64,
    ) -> Result<Option<TranslationResult>, AppError> {
        let normalized = normalize_text(&request.text);
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| internal("Vocabulary database lock failed"))?;
        let transaction = connection.transaction().map_err(storage_error)?;
        let cached = transaction.query_row(
            "SELECT id, translated_text, detected_source_language, effective_source_language, target_language
             FROM vocabulary_entries
             WHERE normalized_text = ?1 AND target_language = ?3
               AND (
                 requested_source_language = ?2 COLLATE NOCASE
                 OR (lower(?2) <> 'auto' AND requested_source_language = 'auto'
                     AND effective_source_language = ?2 COLLATE NOCASE)
               )
             ORDER BY CASE WHEN requested_source_language = ?2 COLLATE NOCASE THEN 0 ELSE 1 END, id
             LIMIT 1",
            params![normalized, request.source_language.trim(), request.target_language.trim()],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, Option<String>>(2)?, row.get::<_, String>(3)?, row.get::<_, String>(4)?)),
        ).optional().map_err(storage_error)?;
        let Some((
            entry_id,
            translated_text,
            detected_source_language,
            effective_source_language,
            target_language,
        )) = cached
        else {
            return Ok(None);
        };
        transaction.execute(
            "UPDATE vocabulary_entries SET lookup_count = lookup_count + 1, last_seen_epoch_ms = ?1 WHERE id = ?2",
            params![to_i64(now_ms), entry_id],
        ).map_err(storage_error)?;
        insert_event(&transaction, entry_id, "lookup-hit", now_ms, None, None)?;
        transaction.commit().map_err(storage_error)?;
        Ok(Some(TranslationResult {
            selection_id: request.selection_id,
            translated_text,
            detected_source_language,
            effective_source_language,
            target_language,
        }))
    }

    fn insert_miss(
        &self,
        request: &TranslationRequest,
        result: &TranslationResult,
        now_ms: u64,
    ) -> Result<(), AppError> {
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| internal("Vocabulary database lock failed"))?;
        let transaction = connection.transaction().map_err(storage_error)?;
        transaction.execute(
            "INSERT INTO vocabulary_entries (
               normalized_text, source_text, requested_source_language, target_language,
               translated_text, detected_source_language, effective_source_language,
               first_seen_epoch_ms, last_seen_epoch_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)
             ON CONFLICT(normalized_text, requested_source_language, target_language)
             DO UPDATE SET lookup_count = lookup_count + 1, last_seen_epoch_ms = excluded.last_seen_epoch_ms",
            params![normalize_text(&request.text), request.text.trim(), request.source_language.trim(), request.target_language.trim(), result.translated_text, result.detected_source_language, result.effective_source_language, to_i64(now_ms)],
        ).map_err(storage_error)?;
        let entry_id = transaction.query_row(
            "SELECT id FROM vocabulary_entries WHERE normalized_text = ?1 AND requested_source_language = ?2 AND target_language = ?3",
            params![normalize_text(&request.text), request.source_language.trim(), request.target_language.trim()],
            |row| row.get::<_, i64>(0),
        ).map_err(storage_error)?;
        insert_event(&transaction, entry_id, "lookup-miss", now_ms, None, None)?;
        transaction.commit().map_err(storage_error)
    }

    pub fn list(
        &self,
        search: Option<&str>,
        now_ms: u64,
    ) -> Result<Vec<VocabularyEntry>, AppError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| internal("Vocabulary database lock failed"))?;
        let query = search
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| format!("%{}%", normalize_text(value)));
        let mut statement = connection.prepare(
            "SELECT id, source_text, translated_text, requested_source_language, effective_source_language,
                    target_language, lookup_count, recall_score, review_count, correct_count, wrong_count,
                    correct_streak, wrong_streak, last_seen_epoch_ms, last_reviewed_epoch_ms
             FROM vocabulary_entries
             WHERE ?1 IS NULL OR normalized_text LIKE ?1 OR lower(translated_text) LIKE ?1
             ORDER BY last_seen_epoch_ms DESC, id DESC",
        ).map_err(storage_error)?;
        let rows = statement
            .query_map(params![query], |row| row_to_entry(row, now_ms))
            .map_err(storage_error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(storage_error)
    }

    pub fn provenance(&self, entry_id: i64) -> Result<Vec<VocabularyProvenance>, AppError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| internal("Vocabulary database lock failed"))?;
        let mut statement = connection
            .prepare(
                "SELECT textbook_id, textbook_title, textbook_version, license, attribution,
                        source_url, source_text, translated_text, promoted_at_epoch_ms
                 FROM vocabulary_textbook_provenance WHERE vocabulary_entry_id = ?1
                 ORDER BY promoted_at_epoch_ms, id",
            )
            .map_err(storage_error)?;
        let rows = statement
            .query_map(params![entry_id], |row| {
                Ok(VocabularyProvenance {
                    textbook_id: row.get(0)?,
                    textbook_title: row.get(1)?,
                    textbook_version: row.get(2)?,
                    license: row.get(3)?,
                    attribution: row.get(4)?,
                    source_url: row.get(5)?,
                    source_text: row.get(6)?,
                    translated_text: row.get(7)?,
                    promoted_at_epoch_ms: row.get::<_, i64>(8)? as u64,
                })
            })
            .map_err(storage_error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(storage_error)
    }

    pub fn related(&self, entry_id: i64, now_ms: u64) -> Result<Vec<RelatedVocabulary>, AppError> {
        let entries = self.list(None, now_ms)?;
        let source = entries
            .iter()
            .find(|entry| entry.id == entry_id)
            .ok_or_else(|| internal("Vocabulary entry was not found"))?;
        let source_root = conservative_root(&source.source_text);
        let source_meaning = meaning_terms(&source.translated_text);
        Ok(entries
            .into_iter()
            .filter(|entry| entry.id != entry_id)
            .filter_map(|entry| {
                let root_match =
                    source_root.is_some() && conservative_root(&entry.source_text) == source_root;
                let meaning_match =
                    !source_meaning.is_disjoint(&meaning_terms(&entry.translated_text));
                let reason = if root_match {
                    Some("root")
                } else if meaning_match {
                    Some("meaning")
                } else {
                    None
                }?;
                Some(RelatedVocabulary {
                    entry,
                    reason: reason.into(),
                })
            })
            .take(12)
            .collect())
    }

    pub fn practice_question(&self, now_ms: u64) -> Result<Option<PracticeQuestion>, AppError> {
        let mut entries = self.list(None, now_ms)?;
        if entries.len() < 2 {
            return Ok(None);
        }
        entries.sort_by(|left, right| {
            left.effective_recall
                .total_cmp(&right.effective_recall)
                .then_with(|| right.lookup_count.cmp(&left.lookup_count))
                .then_with(|| {
                    left.last_reviewed_epoch_ms
                        .cmp(&right.last_reviewed_epoch_ms)
                })
                .then_with(|| left.id.cmp(&right.id))
        });
        let Some(candidate_index) = entries.iter().position(|candidate| {
            entries.iter().any(|entry| {
                entry.id != candidate.id
                    && entry.target_language == candidate.target_language
                    && translation_key(&entry.translated_text)
                        != translation_key(&candidate.translated_text)
            })
        }) else {
            return Ok(None);
        };
        let candidate = entries.remove(candidate_index);
        let mut choice_keys = HashSet::from([translation_key(&candidate.translated_text)]);
        let mut choices = vec![display_translation(&candidate.translated_text)];
        for entry in entries {
            if entry.target_language == candidate.target_language {
                let key = translation_key(&entry.translated_text);
                if !choice_keys.insert(key) {
                    continue;
                }
                choices.push(display_translation(&entry.translated_text));
                if choices.len() == 4 {
                    break;
                }
            }
        }
        let rotation = candidate.id.unsigned_abs() as usize % choices.len();
        choices.rotate_left(rotation);
        Ok(Some(PracticeQuestion {
            entry_id: candidate.id,
            source_text: candidate.source_text,
            effective_source_language: candidate.effective_source_language,
            target_language: candidate.target_language,
            choices,
        }))
    }

    pub fn submit_answer(
        &self,
        entry_id: i64,
        selected_translation: &str,
        now_ms: u64,
    ) -> Result<PracticeOutcome, AppError> {
        let outcome = self.submit_answer_direction(
            entry_id,
            PracticeDirection::SourceToTarget,
            selected_translation,
            now_ms,
        )?;
        Ok(PracticeOutcome {
            correct: outcome.correct,
            correct_translation: outcome.correct_answer,
            entry: outcome.entry,
        })
    }

    pub fn submit_answer_direction(
        &self,
        entry_id: i64,
        direction: PracticeDirection,
        selected_answer: &str,
        now_ms: u64,
    ) -> Result<StudyPracticeOutcome, AppError> {
        if direction == PracticeDirection::Random {
            return Err(internal("A resolved practice direction is required"));
        }
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| internal("Vocabulary database lock failed"))?;
        let transaction = connection.transaction().map_err(storage_error)?;
        let stored = transaction.query_row(
            "SELECT translated_text, source_text, recall_score, review_count, correct_count, wrong_count,
                    correct_streak, wrong_streak, last_reviewed_epoch_ms FROM vocabulary_entries WHERE id = ?1",
            params![entry_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, f64>(2)?, row.get::<_, i64>(3)? as u64, row.get::<_, i64>(4)? as u64, row.get::<_, i64>(5)? as u64, row.get::<_, i64>(6)? as u64, row.get::<_, i64>(7)? as u64, row.get::<_, Option<i64>>(8)?.map(|value| value as u64))),
        ).optional().map_err(storage_error)?.ok_or_else(|| internal("Vocabulary entry was not found"))?;
        let (
            translated_text,
            source_text,
            score,
            review_count,
            correct_count,
            wrong_count,
            correct_streak,
            wrong_streak,
            last_reviewed,
        ) = stored;
        let correct_answer = match direction {
            PracticeDirection::SourceToTarget => translated_text,
            PracticeDirection::TargetToSource => source_text,
            PracticeDirection::Random => unreachable!(),
        };
        let correct = translation_key(selected_answer) == translation_key(&correct_answer);
        let effective_before = effective_recall(score, last_reviewed, now_ms);
        let next_score = if correct {
            let gain = (8.0
                + 2.0 * correct_streak.min(3) as f64
                + if review_count < 3 { 4.0 } else { 0.0 })
            .min(18.0);
            (score.max(35.0) + gain).min(100.0)
        } else {
            let loss = (12.0_f64
                + if familiarity_level(effective_before) >= 3 {
                    4.0
                } else {
                    0.0
                }
                + if wrong_streak > 0 { 2.0 } else { 0.0 })
            .min(20.0);
            (score - loss).max(0.0)
        };
        transaction.execute(
            if correct {
                "UPDATE vocabulary_entries SET recall_score = ?1, review_count = review_count + 1,
                 correct_count = correct_count + 1, correct_streak = correct_streak + 1, wrong_streak = 0,
                 last_reviewed_epoch_ms = ?2, last_correct_epoch_ms = ?2 WHERE id = ?3"
            } else {
                "UPDATE vocabulary_entries SET recall_score = ?1, review_count = review_count + 1,
                 wrong_count = wrong_count + 1, wrong_streak = wrong_streak + 1, correct_streak = 0,
                 last_reviewed_epoch_ms = ?2, last_wrong_epoch_ms = ?2 WHERE id = ?3"
            },
            params![next_score, to_i64(now_ms), entry_id],
        ).map_err(storage_error)?;
        insert_event(
            &transaction,
            entry_id,
            match (direction, correct) {
                (PracticeDirection::SourceToTarget, true) => "practice-source-to-target-correct",
                (PracticeDirection::SourceToTarget, false) => "practice-source-to-target-wrong",
                (PracticeDirection::TargetToSource, true) => "practice-target-to-source-correct",
                (PracticeDirection::TargetToSource, false) => "practice-target-to-source-wrong",
                (PracticeDirection::Random, _) => unreachable!(),
            },
            now_ms,
            Some(score),
            Some(next_score),
        )?;
        transaction.commit().map_err(storage_error)?;
        drop(connection);
        let entry = self
            .list(None, now_ms)?
            .into_iter()
            .find(|entry| entry.id == entry_id)
            .ok_or_else(|| internal("Vocabulary entry was not found"))?;
        debug_assert_eq!(entry.review_count, review_count + 1);
        debug_assert_eq!(entry.correct_count, correct_count + u64::from(correct));
        debug_assert_eq!(entry.wrong_count, wrong_count + u64::from(!correct));
        Ok(StudyPracticeOutcome {
            correct,
            correct_answer: display_translation(&correct_answer),
            direction,
            entry,
        })
    }

    pub fn list_current(&self, search: Option<&str>) -> Result<Vec<VocabularyEntry>, AppError> {
        self.list(search, now_epoch_ms())
    }

    pub fn related_current(&self, entry_id: i64) -> Result<Vec<RelatedVocabulary>, AppError> {
        self.related(entry_id, now_epoch_ms())
    }

    pub fn practice_question_current(&self) -> Result<Option<PracticeQuestion>, AppError> {
        self.practice_question(now_epoch_ms())
    }

    pub fn submit_answer_current(
        &self,
        entry_id: i64,
        selected_translation: &str,
    ) -> Result<PracticeOutcome, AppError> {
        self.submit_answer(entry_id, selected_translation, now_epoch_ms())
    }
}

pub struct VocabularyTranslationProvider {
    upstream: Arc<dyn TranslationProvider>,
    store: Arc<VocabularyStore>,
}

/// Consults only the singular active textbook before delegating to the online provider.
pub struct TextbookTranslationProvider {
    upstream: Arc<dyn TranslationProvider>,
    textbooks: Arc<TextbookStore>,
}

impl TextbookTranslationProvider {
    pub fn new(upstream: Arc<dyn TranslationProvider>, textbooks: Arc<TextbookStore>) -> Self {
        Self {
            upstream,
            textbooks,
        }
    }
}

#[async_trait]
impl TranslationProvider for TextbookTranslationProvider {
    async fn translate(&self, request: &TranslationRequest) -> Result<TranslationResult, AppError> {
        if is_vocabulary_eligible(&request.text) {
            if let Some(book) = self
                .textbooks
                .list_installed()?
                .into_iter()
                .find(|book| book.active)
            {
                let source_compatible = request.source_language.eq_ignore_ascii_case("auto")
                    || request
                        .source_language
                        .eq_ignore_ascii_case(&book.source_language);
                if source_compatible
                    && request
                        .target_language
                        .eq_ignore_ascii_case(&book.target_language)
                {
                    let normalized = normalize_text(&request.text);
                    let entry = self
                        .textbooks
                        .list_entries(&book.id, Some(&request.text), 0, 50)?
                        .entries
                        .into_iter()
                        .find(|entry| normalize_text(&entry.source_text) == normalized);
                    if let Some(entry) = entry {
                        self.textbooks.promote_entry(entry.id, now_epoch_ms())?;
                        return Ok(TranslationResult {
                            selection_id: request.selection_id,
                            translated_text: display_translation(&entry.translated_text),
                            detected_source_language: Some(entry.source_language.clone()),
                            effective_source_language: entry.source_language,
                            target_language: entry.target_language,
                        });
                    }
                }
            }
        }
        self.upstream.translate(request).await
    }

    async fn supported_languages(&self) -> Result<Vec<String>, AppError> {
        self.upstream.supported_languages().await
    }
}

impl VocabularyTranslationProvider {
    pub fn new(upstream: Arc<dyn TranslationProvider>, store: Arc<VocabularyStore>) -> Self {
        Self { upstream, store }
    }
}

#[async_trait]
impl TranslationProvider for VocabularyTranslationProvider {
    async fn translate(&self, request: &TranslationRequest) -> Result<TranslationResult, AppError> {
        if !is_vocabulary_eligible(&request.text) {
            return self.upstream.translate(request).await;
        }
        let now_ms = now_epoch_ms();
        if let Some(result) = self.store.lookup_and_touch(request, now_ms)? {
            return Ok(result);
        }
        let result = self.upstream.translate(request).await?;
        self.store.insert_miss(request, &result, now_ms)?;
        Ok(result)
    }

    async fn supported_languages(&self) -> Result<Vec<String>, AppError> {
        self.upstream.supported_languages().await
    }
}

pub fn is_vocabulary_eligible(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() || text.contains(['\n', '\r']) {
        return false;
    }
    let tokens = trimmed.split_whitespace().collect::<Vec<_>>();
    if tokens.is_empty() || tokens.len() > 5 {
        return false;
    }
    tokens.into_iter().all(|token| {
        let characters = token.chars().collect::<Vec<_>>();
        characters.iter().any(|character| character.is_alphabetic())
            && characters.iter().enumerate().all(|(index, character)| {
                character.is_alphabetic()
                    || (matches!(character, '\'' | '’' | '-')
                        && index > 0
                        && index + 1 < characters.len()
                        && characters[index - 1].is_alphabetic()
                        && characters[index + 1].is_alphabetic())
            })
    })
}

pub fn effective_recall(score: f64, last_reviewed_ms: Option<u64>, now_ms: u64) -> f64 {
    let Some(last_reviewed_ms) = last_reviewed_ms else {
        return score.clamp(0.0, 100.0);
    };
    let days = now_ms.saturating_sub(last_reviewed_ms) as f64 / 86_400_000.0;
    let penalty = if days <= 3.0 {
        0.0
    } else if days <= 14.0 {
        days - 3.0
    } else {
        (11.0 + 1.5 * (days - 14.0)).min(35.0)
    };
    (score - penalty).clamp(0.0, 100.0)
}

pub fn familiarity_level(score: f64) -> u8 {
    match score {
        value if value < 20.0 => 0,
        value if value < 35.0 => 1,
        value if value < 50.0 => 2,
        value if value < 65.0 => 3,
        value if value < 80.0 => 4,
        _ => 5,
    }
}

fn normalize_text(text: &str) -> String {
    text.split_whitespace()
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join(" ")
}

fn translation_key(text: &str) -> String {
    normalize_text(text)
}

fn display_translation(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn now_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}
fn to_i64(value: u64) -> i64 {
    value.min(i64::MAX as u64) as i64
}

fn row_to_entry(row: &Row<'_>, now_ms: u64) -> rusqlite::Result<VocabularyEntry> {
    let recall_score = row.get::<_, f64>(7)?;
    let last_reviewed_epoch_ms = row.get::<_, Option<i64>>(14)?.map(|value| value as u64);
    let effective_recall = effective_recall(recall_score, last_reviewed_epoch_ms, now_ms);
    Ok(VocabularyEntry {
        id: row.get(0)?,
        source_text: row.get(1)?,
        translated_text: row.get(2)?,
        requested_source_language: row.get(3)?,
        effective_source_language: row.get(4)?,
        target_language: row.get(5)?,
        lookup_count: row.get::<_, i64>(6)? as u64,
        recall_score,
        effective_recall,
        familiarity_level: familiarity_level(effective_recall),
        review_count: row.get::<_, i64>(8)? as u64,
        correct_count: row.get::<_, i64>(9)? as u64,
        wrong_count: row.get::<_, i64>(10)? as u64,
        correct_streak: row.get::<_, i64>(11)? as u64,
        wrong_streak: row.get::<_, i64>(12)? as u64,
        last_seen_epoch_ms: row.get::<_, i64>(13)? as u64,
        last_reviewed_epoch_ms,
    })
}

fn conservative_root(text: &str) -> Option<String> {
    let normalized = normalize_text(text);
    if normalized.contains(' ')
        || !normalized.is_ascii()
        || !normalized
            .chars()
            .all(|character| character.is_ascii_alphabetic())
    {
        return None;
    }
    for suffix in [
        "ness", "ment", "tion", "ing", "ers", "er", "ed", "ly", "es", "s",
    ] {
        if normalized.len() >= suffix.len() + 3 && normalized.ends_with(suffix) {
            return Some(normalized[..normalized.len() - suffix.len()].into());
        }
    }
    Some(normalized)
}

fn meaning_terms(text: &str) -> HashSet<String> {
    text.split(|character: char| !character.is_alphabetic())
        .filter(|term| term.chars().count() >= 2)
        .map(|term| conservative_root(term).unwrap_or_else(|| normalize_text(term)))
        .collect()
}

fn insert_event(
    connection: &Connection,
    entry_id: i64,
    kind: &str,
    now_ms: u64,
    score_before: Option<f64>,
    score_after: Option<f64>,
) -> Result<(), AppError> {
    connection.execute(
        "INSERT INTO vocabulary_events (entry_id, kind, created_at_epoch_ms, score_before, score_after) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![entry_id, kind, to_i64(now_ms), score_before, score_after],
    ).map_err(storage_error)?;
    Ok(())
}

fn storage_error(_: rusqlite::Error) -> AppError {
    internal("The local vocabulary database could not complete the request")
}
fn internal(message: &'static str) -> AppError {
    AppError::new(AppErrorCode::Internal, message, false)
}

#[cfg(test)]
mod tests {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    use async_trait::async_trait;
    use sha2::{Digest, Sha256};
    use tempfile::NamedTempFile;

    use super::*;
    use crate::{
        contracts::{AppError, TextbookCatalogItem, TranslationRequest, TranslationResult},
        services::{textbooks::TextbookStore, TranslationProvider},
    };

    struct FakeProvider {
        calls: AtomicUsize,
    }

    #[async_trait]
    impl TranslationProvider for FakeProvider {
        async fn translate(
            &self,
            request: &TranslationRequest,
        ) -> Result<TranslationResult, AppError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(TranslationResult {
                selection_id: request.selection_id,
                translated_text: match request.text.as_str() {
                    "greeting" => "  HoLa \t",
                    "runner" => "a person who runs",
                    "running" | "quickly" => "run quickly",
                    "world" => "mundo",
                    _ => "hola",
                }
                .into(),
                detected_source_language: Some("en".into()),
                effective_source_language: "en".into(),
                target_language: request.target_language.clone(),
            })
        }

        async fn supported_languages(&self) -> Result<Vec<String>, AppError> {
            Ok(vec!["en".into(), "es".into()])
        }
    }

    fn request(id: u64, text: &str) -> TranslationRequest {
        request_to(id, text, "es")
    }

    fn request_to(id: u64, text: &str, target_language: &str) -> TranslationRequest {
        TranslationRequest {
            selection_id: id,
            text: text.into(),
            source_language: "auto".into(),
            target_language: target_language.into(),
        }
    }

    fn install_test_textbook(app_db: &std::path::Path) -> Arc<TextbookStore> {
        let fixture = NamedTempFile::new().expect("fixture");
        let connection = Connection::open(fixture.path()).expect("fixture db");
        connection
            .execute_batch("CREATE TABLE simple_translation (written_rep TEXT NOT NULL, trans_list TEXT NOT NULL);")
            .expect("fixture schema");
        connection
            .execute(
                "INSERT INTO simple_translation VALUES ('ephemeral', '短暂的')",
                [],
            )
            .expect("fixture entry");
        drop(connection);
        let bytes = std::fs::read(fixture.path()).expect("fixture bytes");
        let catalog = TextbookCatalogItem {
            id: "test-en-zh".into(),
            title: "Test".into(),
            source_language: "en".into(),
            target_language: "zh-CN".into(),
            version: "1".into(),
            download_url: "https://download.wikdict.com/test.sqlite3".into(),
            expected_bytes: bytes.len() as u64,
            sha256: format!("{:x}", Sha256::digest(&bytes)),
            license: "CC BY-SA 4.0".into(),
            attribution: "Test".into(),
            source_url: "https://www.wikdict.com/page/download".into(),
        };
        let store = Arc::new(TextbookStore::open(app_db).expect("textbook store"));
        store
            .install_sqlite(&catalog, fixture.path(), 1)
            .expect("install");
        store.set_active(Some("test-en-zh")).expect("activate");
        store
    }

    #[tokio::test]
    async fn provider_chain_prefers_personal_then_active_textbook_then_api() {
        let file = NamedTempFile::new().expect("app db");
        let personal = Arc::new(VocabularyStore::open(file.path()).expect("personal"));
        let textbooks = install_test_textbook(file.path());
        let api = Arc::new(FakeProvider {
            calls: AtomicUsize::new(0),
        });
        let textbook = Arc::new(TextbookTranslationProvider::new(
            api.clone(),
            textbooks.clone(),
        ));
        let provider = VocabularyTranslationProvider::new(textbook, personal.clone());

        let first = provider
            .translate(&request_to(1, "ephemeral", "zh-CN"))
            .await
            .expect("textbook hit");
        assert_eq!(first.translated_text, "短暂的");
        assert_eq!(api.calls.load(Ordering::SeqCst), 0);
        let promoted = personal.list(None, 2).expect("personal");
        assert_eq!(promoted.len(), 1);
        assert_eq!(
            personal
                .provenance(promoted[0].id)
                .expect("provenance")
                .len(),
            1
        );

        Connection::open(file.path())
            .expect("edit personal translation")
            .execute(
                "UPDATE vocabulary_entries SET translated_text = '短暂的（个人）' WHERE id = ?1",
                params![promoted[0].id],
            )
            .expect("preserve personal meaning");
        let explicit_source = TranslationRequest {
            selection_id: 2,
            text: "ephemeral".into(),
            source_language: "en".into(),
            target_language: "zh-CN".into(),
        };
        let explicit_hit = provider
            .translate(&explicit_source)
            .await
            .expect("explicit source personal hit");
        assert_eq!(explicit_hit.translated_text, "短暂的（个人）");
        assert_eq!(api.calls.load(Ordering::SeqCst), 0);

        provider
            .translate(&request_to(3, "ephemeral", "zh-CN"))
            .await
            .expect("personal hit");
        assert_eq!(api.calls.load(Ordering::SeqCst), 0);
        provider
            .translate(&request_to(4, "unknown", "zh-CN"))
            .await
            .expect("api miss");
        assert_eq!(api.calls.load(Ordering::SeqCst), 1);

        // Provenance is personal history and remains displayable after source removal.
        textbooks.remove("test-en-zh").expect("remove textbook");
        assert_eq!(
            personal
                .provenance(promoted[0].id)
                .expect("retained provenance")
                .len(),
            1
        );
    }

    #[test]
    fn lexical_eligibility_is_conservative_and_unicode_aware() {
        for accepted in ["hello", "mother-in-law", "l’esprit", "你好", "look up"] {
            assert!(
                is_vocabulary_eligible(accepted),
                "expected eligible: {accepted}"
            );
        }
        for rejected in [
            "",
            "hello.",
            "hello\n",
            "hello\rworld",
            "version 2",
            "six token phrases are too long",
            "one two three four five six",
            "hello_world",
            "-hello",
        ] {
            assert!(
                !is_vocabulary_eligible(rejected),
                "expected ineligible: {rejected}"
            );
        }
    }

    #[test]
    fn decay_and_levels_follow_the_legacy_schedule() {
        const DAY: u64 = 86_400_000;
        assert_eq!(effective_recall(60.0, Some(0), 3 * DAY), 60.0);
        assert_eq!(effective_recall(60.0, Some(0), 10 * DAY), 53.0);
        assert_eq!(effective_recall(60.0, Some(0), 20 * DAY), 40.0);
        assert_eq!(effective_recall(20.0, Some(0), 100 * DAY), 0.0);
        assert_eq!(
            [19.0, 34.0, 49.0, 64.0, 79.0, 80.0].map(familiarity_level),
            [0, 1, 2, 3, 4, 5]
        );
    }

    #[tokio::test]
    async fn cache_hit_preserves_selection_and_does_not_call_upstream() {
        let file = NamedTempFile::new().expect("temp db");
        let store = Arc::new(VocabularyStore::open(file.path()).expect("store"));
        let upstream = Arc::new(FakeProvider {
            calls: AtomicUsize::new(0),
        });
        let provider = VocabularyTranslationProvider::new(upstream.clone(), store.clone());

        provider
            .translate(&request(1, "hello"))
            .await
            .expect("miss");
        let hit = provider
            .translate(&request(9, "  HELLO "))
            .await
            .expect("hit");

        assert_eq!(upstream.calls.load(Ordering::SeqCst), 1);
        assert_eq!(hit.selection_id, 9);
        let entries = store.list(None, 1).expect("list");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].lookup_count, 2);
        assert_eq!(entries[0].recall_score, 20.0);
    }

    #[tokio::test]
    async fn sentence_like_input_bypasses_storage_every_time() {
        let store = Arc::new(VocabularyStore::in_memory().expect("store"));
        let upstream = Arc::new(FakeProvider {
            calls: AtomicUsize::new(0),
        });
        let provider = VocabularyTranslationProvider::new(upstream.clone(), store.clone());
        provider
            .translate(&request(1, "Hello world."))
            .await
            .expect("first");
        provider
            .translate(&request(2, "Hello world."))
            .await
            .expect("second");
        assert_eq!(upstream.calls.load(Ordering::SeqCst), 2);
        assert!(store.list(None, 1).expect("list").is_empty());
    }

    #[tokio::test]
    async fn practice_records_only_after_submission_and_updates_recall() {
        let store = Arc::new(VocabularyStore::in_memory().expect("store"));
        let upstream = Arc::new(FakeProvider {
            calls: AtomicUsize::new(0),
        });
        let provider = VocabularyTranslationProvider::new(upstream, store.clone());
        provider
            .translate(&request(1, "hello"))
            .await
            .expect("hello");
        provider
            .translate(&request(2, "world"))
            .await
            .expect("world");

        let before = store.list(None, 10).expect("before");
        let question = store
            .practice_question(10)
            .expect("question")
            .expect("candidate");
        let after_question = store.list(None, 10).expect("after question");
        assert_eq!(before, after_question);
        assert!((2..=4).contains(&question.choices.len()));

        let correct = store
            .submit_answer(question.entry_id, "hola", 20)
            .expect("answer");
        assert!(correct.correct);
        assert_eq!(correct.entry.recall_score, 47.0);
        assert_eq!(correct.entry.review_count, 1);
        assert_eq!(correct.entry.correct_count, 1);
        assert_eq!(correct.entry.correct_streak, 1);
        assert_eq!(correct.entry.wrong_count, 0);
        assert_eq!(correct.entry.wrong_streak, 0);
        assert_eq!(correct.entry.last_reviewed_epoch_ms, Some(20));

        let wrong = store
            .submit_answer(question.entry_id, "not it", 30)
            .expect("wrong");
        assert!(!wrong.correct);
        assert_eq!(wrong.entry.recall_score, 35.0);
        assert_eq!(wrong.entry.wrong_streak, 1);
        assert_eq!(wrong.entry.correct_streak, 0);
        assert_eq!(wrong.entry.review_count, 2);
        assert_eq!(wrong.entry.correct_count, 1);
        assert_eq!(wrong.entry.wrong_count, 1);
        assert_eq!(wrong.entry.last_reviewed_epoch_ms, Some(30));
        let practice_events: i64 = store
            .connection
            .lock()
            .expect("connection")
            .query_row(
                "SELECT count(*) FROM vocabulary_events WHERE entry_id = ?1 AND kind LIKE 'practice-%'",
                params![question.entry_id],
                |row| row.get(0),
            )
            .expect("event count");
        assert_eq!(practice_events, 2);
    }

    #[tokio::test]
    async fn practice_is_unavailable_for_mixed_target_entries() {
        let store = Arc::new(VocabularyStore::in_memory().expect("store"));
        let upstream = Arc::new(FakeProvider {
            calls: AtomicUsize::new(0),
        });
        let provider = VocabularyTranslationProvider::new(upstream, store.clone());
        provider
            .translate(&request_to(1, "hello", "es"))
            .await
            .expect("hello");
        provider
            .translate(&request_to(2, "world", "pt"))
            .await
            .expect("world");

        assert!(store.practice_question(10).expect("question").is_none());
    }

    #[tokio::test]
    async fn practice_is_unavailable_for_duplicate_translations() {
        let store = Arc::new(VocabularyStore::in_memory().expect("store"));
        let upstream = Arc::new(FakeProvider {
            calls: AtomicUsize::new(0),
        });
        let provider = VocabularyTranslationProvider::new(upstream, store.clone());
        provider
            .translate(&request(1, "hello"))
            .await
            .expect("hello");
        provider
            .translate(&request(2, "goodbye"))
            .await
            .expect("goodbye");

        assert!(store.practice_question(10).expect("question").is_none());
    }

    #[tokio::test]
    async fn practice_ranks_only_compatible_candidates_and_keeps_choices_consistent() {
        let store = Arc::new(VocabularyStore::in_memory().expect("store"));
        let upstream = Arc::new(FakeProvider {
            calls: AtomicUsize::new(0),
        });
        let provider = VocabularyTranslationProvider::new(upstream, store.clone());
        provider
            .translate(&request_to(1, "hello", "es"))
            .await
            .expect("hello");
        provider
            .translate(&request_to(2, "hello", "es"))
            .await
            .expect("hello hit");
        provider
            .translate(&request_to(3, "world", "pt"))
            .await
            .expect("world");
        provider
            .translate(&request_to(4, "runner", "pt"))
            .await
            .expect("runner");

        let question = store
            .practice_question(10)
            .expect("question")
            .expect("candidate");

        assert_eq!(question.source_text, "world");
        assert_eq!(question.target_language, "pt");
        assert_eq!(question.choices.len(), 2);
        assert!(question.choices.contains(&"mundo".to_owned()));
        assert!(question.choices.contains(&"a person who runs".to_owned()));
        assert_eq!(question.choices.iter().collect::<HashSet<_>>().len(), 2);
    }

    #[tokio::test]
    async fn practice_normalizes_translation_keys_for_choices_and_scoring() {
        let store = Arc::new(VocabularyStore::in_memory().expect("store"));
        let upstream = Arc::new(FakeProvider {
            calls: AtomicUsize::new(0),
        });
        let provider = VocabularyTranslationProvider::new(upstream, store.clone());
        provider
            .translate(&request(1, "greeting"))
            .await
            .expect("greeting");
        provider
            .translate(&request(2, "greeting"))
            .await
            .expect("greeting hit");
        provider
            .translate(&request(3, "hello"))
            .await
            .expect("hello");
        provider
            .translate(&request(4, "world"))
            .await
            .expect("world");

        let question = store
            .practice_question(10)
            .expect("question")
            .expect("candidate");

        assert_eq!(question.source_text, "greeting");
        assert_eq!(question.choices.len(), 2);
        assert_eq!(
            question
                .choices
                .iter()
                .filter(|choice| translation_key(choice) == translation_key("hola"))
                .count(),
            1
        );
        assert!(question
            .choices
            .iter()
            .all(|choice| { choice == &choice.split_whitespace().collect::<Vec<_>>().join(" ") }));

        let outcome = store
            .submit_answer(question.entry_id, " hola ", 20)
            .expect("answer");
        assert!(outcome.correct);
    }

    #[tokio::test]
    async fn related_items_report_local_root_and_meaning_reasons() {
        let store = Arc::new(VocabularyStore::in_memory().expect("store"));
        let upstream = Arc::new(FakeProvider {
            calls: AtomicUsize::new(0),
        });
        let provider = VocabularyTranslationProvider::new(upstream, store.clone());
        provider
            .translate(&request(1, "runner"))
            .await
            .expect("runner");
        provider
            .translate(&request(2, "running"))
            .await
            .expect("running");
        provider
            .translate(&request(3, "quickly"))
            .await
            .expect("quickly");
        let runner = store.list(Some("runner"), 1).expect("list").remove(0);
        let related = store.related(runner.id, 1).expect("related");
        assert!(related.iter().any(|item| item.reason == "root"));
        assert!(related.iter().any(|item| item.reason == "meaning"));
    }
}
