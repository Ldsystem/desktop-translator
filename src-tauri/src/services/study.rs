//! Cross-store related-word and bidirectional-practice behavior.

use std::{
    collections::HashSet,
    path::Path,
    sync::{Arc, Mutex},
};

use rusqlite::{params, Connection};

use crate::{
    contracts::{
        AppError, AppErrorCode, PracticeDirection, PracticePreferences, RelatedSource, RelatedWord,
        StudyPracticeOutcome, StudyPracticeQuestion,
    },
    services::{textbooks::TextbookStore, vocabulary::VocabularyStore},
};

pub struct StudyService {
    preferences: Mutex<Connection>,
    vocabulary: Arc<VocabularyStore>,
    textbooks: Arc<TextbookStore>,
}

impl StudyService {
    pub fn open(
        path: impl AsRef<Path>,
        vocabulary: Arc<VocabularyStore>,
        textbooks: Arc<TextbookStore>,
    ) -> Result<Self, AppError> {
        let connection = Connection::open(path).map_err(storage_error)?;
        connection
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS study_preferences (
                   singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
                   direction TEXT NOT NULL CHECK(direction IN ('random', 'source-to-target', 'target-to-source'))
                 );
                 INSERT OR IGNORE INTO study_preferences(singleton, direction) VALUES (1, 'random');",
            )
            .map_err(storage_error)?;
        Ok(Self {
            preferences: Mutex::new(connection),
            vocabulary,
            textbooks,
        })
    }

    #[cfg(test)]
    fn in_memory() -> Result<Self, AppError> {
        let file = tempfile::NamedTempFile::new()
            .map_err(|_| internal("Temporary study database could not be created"))?;
        let vocabulary = Arc::new(VocabularyStore::open(file.path())?);
        let textbooks = Arc::new(TextbookStore::open(file.path())?);
        Self::open(file.path(), vocabulary, textbooks)
    }

    pub fn preferences(&self) -> Result<PracticePreferences, AppError> {
        let connection = self
            .preferences
            .lock()
            .map_err(|_| internal("Study preferences lock failed"))?;
        let direction = connection
            .query_row(
                "SELECT direction FROM study_preferences WHERE singleton = 1",
                [],
                |row| row.get::<_, String>(0),
            )
            .map_err(storage_error)?;
        Ok(PracticePreferences {
            direction: parse_direction(&direction)?,
        })
    }

    pub fn save_preferences(&self, value: PracticePreferences) -> Result<(), AppError> {
        let direction = direction_label(value.direction);
        self.preferences
            .lock()
            .map_err(|_| internal("Study preferences lock failed"))?
            .execute(
                "UPDATE study_preferences SET direction = ?1 WHERE singleton = 1",
                params![direction],
            )
            .map_err(storage_error)?;
        Ok(())
    }

    pub fn related(
        &self,
        entry_id: i64,
        source: RelatedSource,
        now_ms: u64,
    ) -> Result<Vec<RelatedWord>, AppError> {
        match source {
            RelatedSource::Personal => self.vocabulary.related(entry_id, now_ms).map(|items| {
                items
                    .into_iter()
                    .map(|item| RelatedWord {
                        kind: "personal".into(),
                        vocabulary_entry_id: Some(item.entry.id),
                        textbook_entry_id: None,
                        textbook_id: None,
                        source_text: item.entry.source_text,
                        translated_text: item.entry.translated_text,
                        source_language: item.entry.effective_source_language,
                        target_language: item.entry.target_language,
                        reason: item.reason,
                        promoted: true,
                    })
                    .collect()
            }),
            RelatedSource::Textbook { textbook_id } => {
                let active = self
                    .textbooks
                    .list_installed()?
                    .into_iter()
                    .find(|book| book.active && book.id == textbook_id)
                    .ok_or_else(|| internal("The selected textbook is not active"))?;
                let entries = self.vocabulary.list(None, now_ms)?;
                let anchor = entries
                    .iter()
                    .find(|entry| entry.id == entry_id)
                    .ok_or_else(|| internal("Vocabulary entry was not found"))?;
                let root = conservative_root(&anchor.source_text);
                let page = self.textbooks.list_entries(
                    &active.id,
                    root.as_deref().or(Some(anchor.source_text.as_str())),
                    0,
                    100,
                )?;
                let personal = entries
                    .iter()
                    .map(|entry| (key(&entry.source_text), entry.target_language.as_str()))
                    .collect::<HashSet<_>>();
                Ok(page
                    .entries
                    .into_iter()
                    .filter(|entry| key(&entry.source_text) != key(&anchor.source_text))
                    .take(12)
                    .map(|entry| {
                        let promoted = personal
                            .contains(&(key(&entry.source_text), entry.target_language.as_str()));
                        RelatedWord {
                            kind: "textbook".into(),
                            vocabulary_entry_id: None,
                            textbook_entry_id: Some(entry.id),
                            textbook_id: Some(entry.textbook_id),
                            source_text: entry.source_text,
                            translated_text: entry.translated_text,
                            source_language: entry.source_language,
                            target_language: entry.target_language,
                            reason: "root".into(),
                            promoted,
                        }
                    })
                    .collect())
            }
        }
    }

    pub fn question(
        &self,
        now_ms: u64,
        seed: u64,
    ) -> Result<Option<StudyPracticeQuestion>, AppError> {
        let preference = self.preferences()?.direction;
        let direction = match preference {
            PracticeDirection::Random if seed.is_multiple_of(2) => {
                PracticeDirection::SourceToTarget
            }
            PracticeDirection::Random => PracticeDirection::TargetToSource,
            fixed => fixed,
        };
        let mut entries = self.vocabulary.list(None, now_ms)?;
        if entries.is_empty() {
            return Ok(None);
        }
        entries.sort_by(|left, right| {
            left.effective_recall
                .total_cmp(&right.effective_recall)
                .then_with(|| right.lookup_count.cmp(&left.lookup_count))
                .then_with(|| left.id.cmp(&right.id))
        });
        let active_textbook = self
            .textbooks
            .list_installed()?
            .into_iter()
            .find(|book| book.active);
        let active_entries = if let Some(active) = &active_textbook {
            self.textbooks
                .list_entries(&active.id, None, 0, 500)?
                .entries
        } else {
            Vec::new()
        };

        for candidate in &entries {
            let (prompt, prompt_language, answer_language, correct_answer) = match direction {
                PracticeDirection::SourceToTarget => (
                    candidate.source_text.clone(),
                    candidate.effective_source_language.clone(),
                    candidate.target_language.clone(),
                    candidate.translated_text.clone(),
                ),
                PracticeDirection::TargetToSource => (
                    candidate.translated_text.clone(),
                    candidate.target_language.clone(),
                    candidate.effective_source_language.clone(),
                    candidate.source_text.clone(),
                ),
                PracticeDirection::Random => unreachable!(),
            };
            let mut choices = vec![display(&correct_answer)];
            let mut choice_keys = HashSet::from([key(&correct_answer)]);
            if active_textbook.as_ref().is_some_and(|active| {
                active.source_language == candidate.effective_source_language
                    && active.target_language == candidate.target_language
            }) {
                for textbook_entry in &active_entries {
                    let answer = match direction {
                        PracticeDirection::SourceToTarget => &textbook_entry.translated_text,
                        PracticeDirection::TargetToSource => &textbook_entry.source_text,
                        PracticeDirection::Random => unreachable!(),
                    };
                    if choice_keys.insert(key(answer)) {
                        choices.push(display(answer));
                    }
                    if choices.len() == 4 {
                        break;
                    }
                }
            }
            for entry in &entries {
                if choices.len() >= 4 {
                    break;
                }
                if entry.id == candidate.id
                    || entry.effective_source_language != candidate.effective_source_language
                    || entry.target_language != candidate.target_language
                {
                    continue;
                }
                let answer = match direction {
                    PracticeDirection::SourceToTarget => &entry.translated_text,
                    PracticeDirection::TargetToSource => &entry.source_text,
                    PracticeDirection::Random => unreachable!(),
                };
                if choice_keys.insert(key(answer)) {
                    choices.push(display(answer));
                }
            }
            if choices.len() < 2 {
                continue;
            }
            let rotation = seed as usize % choices.len();
            choices.rotate_left(rotation);
            return Ok(Some(StudyPracticeQuestion {
                entry_id: candidate.id,
                direction,
                prompt,
                prompt_language,
                answer_language,
                choices,
            }));
        }
        Ok(None)
    }

    pub fn submit(
        &self,
        entry_id: i64,
        direction: PracticeDirection,
        selected_answer: &str,
        now_ms: u64,
    ) -> Result<StudyPracticeOutcome, AppError> {
        if direction == PracticeDirection::Random {
            return Err(internal("A resolved practice direction is required"));
        }
        self.vocabulary
            .submit_answer_direction(entry_id, direction, selected_answer, now_ms)
    }
}

fn direction_label(direction: PracticeDirection) -> &'static str {
    match direction {
        PracticeDirection::Random => "random",
        PracticeDirection::SourceToTarget => "source-to-target",
        PracticeDirection::TargetToSource => "target-to-source",
    }
}

fn parse_direction(value: &str) -> Result<PracticeDirection, AppError> {
    match value {
        "random" => Ok(PracticeDirection::Random),
        "source-to-target" => Ok(PracticeDirection::SourceToTarget),
        "target-to-source" => Ok(PracticeDirection::TargetToSource),
        _ => Err(internal("Stored practice direction is invalid")),
    }
}

fn display(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}
fn key(value: &str) -> String {
    display(value).to_lowercase()
}

fn conservative_root(value: &str) -> Option<String> {
    let normalized = key(value);
    if normalized.contains(' ') || !normalized.chars().all(|c| c.is_ascii_alphabetic()) {
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

fn storage_error(_: rusqlite::Error) -> AppError {
    internal("The local study database could not complete the request")
}
fn internal(message: &'static str) -> AppError {
    AppError::new(AppErrorCode::Internal, message, false)
}

#[cfg(test)]
mod tests {
    use sha2::{Digest, Sha256};
    use tempfile::NamedTempFile;

    use super::*;
    use crate::contracts::TextbookCatalogItem;

    fn seeded_service() -> (NamedTempFile, StudyService) {
        let file = NamedTempFile::new().expect("app db");
        let vocabulary = Arc::new(VocabularyStore::open(file.path()).expect("vocabulary"));
        let textbooks = Arc::new(TextbookStore::open(file.path()).expect("textbooks"));
        let service = StudyService::open(file.path(), vocabulary, textbooks).expect("study");
        {
            let connection = service.preferences.lock().expect("connection");
            for (source, translation, seen) in [
                ("ephemeral", "短暂的", 30),
                ("supersede", "取代", 20),
                ("predeclare", "预先申报", 10),
            ] {
                connection.execute(
                    "INSERT INTO vocabulary_entries (normalized_text, source_text, requested_source_language, target_language, translated_text, detected_source_language, effective_source_language, first_seen_epoch_ms, last_seen_epoch_ms) VALUES (?1, ?1, 'auto', 'zh-CN', ?2, 'en', 'en', ?3, ?3)",
                    params![source, translation, seen],
                ).expect("personal entry");
            }
        }
        (file, service)
    }

    fn install_book(service: &StudyService) {
        let fixture = NamedTempFile::new().expect("fixture");
        let connection = Connection::open(fixture.path()).expect("fixture db");
        connection.execute_batch("CREATE TABLE simple_translation (written_rep TEXT NOT NULL, trans_list TEXT NOT NULL);").expect("schema");
        for (source, translation) in [
            ("ephemeral", "短暂的"),
            ("replacement", "替代"),
            ("temporary", "临时的"),
            ("lasting", "持久的"),
        ] {
            connection
                .execute(
                    "INSERT INTO simple_translation VALUES (?1, ?2)",
                    params![source, translation],
                )
                .expect("fixture entry");
        }
        drop(connection);
        let bytes = std::fs::read(fixture.path()).expect("bytes");
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
        service
            .textbooks
            .install_sqlite(&catalog, fixture.path(), 1)
            .expect("install");
        service
            .textbooks
            .set_active(Some("test-en-zh"))
            .expect("activate");
    }

    #[test]
    fn preferences_default_to_random_and_persist() {
        let service = StudyService::in_memory().expect("study service");
        assert_eq!(
            service.preferences().expect("preferences").direction,
            PracticeDirection::Random
        );
        service
            .save_preferences(PracticePreferences {
                direction: PracticeDirection::TargetToSource,
            })
            .expect("save preferences");
        assert_eq!(
            service.preferences().expect("preferences").direction,
            PracticeDirection::TargetToSource
        );
    }

    #[test]
    fn forced_directions_are_neutral_and_question_creation_does_not_mutate_recall() {
        let (_file, service) = seeded_service();
        let before = service.vocabulary.list(None, 100).expect("before");
        service
            .save_preferences(PracticePreferences {
                direction: PracticeDirection::SourceToTarget,
            })
            .expect("forward");
        let forward = service
            .question(100, 0)
            .expect("question")
            .expect("forward question");
        assert_eq!(forward.direction, PracticeDirection::SourceToTarget);
        assert_eq!(forward.prompt, "ephemeral");
        assert!(forward.choices.contains(&"短暂的".to_owned()));

        service
            .save_preferences(PracticePreferences {
                direction: PracticeDirection::TargetToSource,
            })
            .expect("reverse");
        let reverse = service
            .question(100, 0)
            .expect("question")
            .expect("reverse question");
        assert_eq!(reverse.direction, PracticeDirection::TargetToSource);
        assert_eq!(reverse.prompt, "短暂的");
        assert!(reverse.choices.contains(&"ephemeral".to_owned()));
        assert_eq!(before, service.vocabulary.list(None, 100).expect("after"));
    }

    #[test]
    fn random_direction_is_seeded_and_textbook_choices_precede_personal_fallback() {
        let (_file, service) = seeded_service();
        install_book(&service);
        service
            .save_preferences(PracticePreferences {
                direction: PracticeDirection::Random,
            })
            .expect("random");
        let forward = service
            .question(100, 2)
            .expect("question")
            .expect("forward");
        let reverse = service
            .question(100, 3)
            .expect("question")
            .expect("reverse");
        assert_eq!(forward.direction, PracticeDirection::SourceToTarget);
        assert_eq!(reverse.direction, PracticeDirection::TargetToSource);
        assert_eq!(forward.choices.len(), 4);
        assert!(forward.choices.contains(&"替代".to_owned()));
        assert!(forward.choices.contains(&"临时的".to_owned()));
        assert!(forward.choices.contains(&"持久的".to_owned()));
    }

    #[test]
    fn submission_scores_the_resolved_direction_once_and_records_it() {
        let (_file, service) = seeded_service();
        service
            .save_preferences(PracticePreferences {
                direction: PracticeDirection::TargetToSource,
            })
            .expect("reverse");
        let question = service
            .question(100, 0)
            .expect("question")
            .expect("reverse");
        let outcome = service
            .submit(question.entry_id, question.direction, "ephemeral", 200)
            .expect("submit");
        assert!(outcome.correct);
        assert_eq!(outcome.direction, PracticeDirection::TargetToSource);
        assert_eq!(outcome.entry.review_count, 1);
        let connection = service.preferences.lock().expect("connection");
        let count: i64 = connection.query_row(
            "SELECT count(*) FROM vocabulary_events WHERE entry_id = ?1 AND kind = 'practice-target-to-source-correct'",
            params![question.entry_id], |row| row.get(0),
        ).expect("event");
        assert_eq!(count, 1);
    }

    #[test]
    fn related_source_returns_only_the_selected_corpus() {
        let (_file, service) = seeded_service();
        install_book(&service);
        let anchor = service
            .vocabulary
            .list(None, 100)
            .expect("entries")
            .into_iter()
            .find(|entry| entry.source_text == "ephemeral")
            .expect("anchor");
        let personal = service
            .related(anchor.id, RelatedSource::Personal, 100)
            .expect("personal");
        assert!(personal.iter().all(|item| item.kind == "personal"));
        let textbook = service
            .related(
                anchor.id,
                RelatedSource::Textbook {
                    textbook_id: "test-en-zh".into(),
                },
                100,
            )
            .expect("textbook");
        assert!(textbook.iter().all(|item| item.kind == "textbook"));
        service.textbooks.set_active(None).expect("deactivate");
        assert!(service
            .related(
                anchor.id,
                RelatedSource::Textbook {
                    textbook_id: "test-en-zh".into(),
                },
                100,
            )
            .is_err());
    }

    #[test]
    fn skips_an_incompatible_top_ranked_candidate_for_a_later_practicable_pair() {
        let (_file, service) = seeded_service();
        {
            let connection = service.preferences.lock().expect("connection");
            connection
                .execute(
                    "UPDATE vocabulary_entries SET target_language = 'es', recall_score = 0 WHERE source_text = 'ephemeral'",
                    [],
                )
                .expect("make top candidate incompatible");
        }
        service
            .save_preferences(PracticePreferences {
                direction: PracticeDirection::SourceToTarget,
            })
            .expect("forward");
        let question = service
            .question(100, 0)
            .expect("question")
            .expect("later compatible candidate");
        assert_ne!(question.prompt, "ephemeral");
        assert!(question.choices.len() >= 2);
    }
}
