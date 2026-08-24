//! Cross-store related-word and bidirectional-practice behavior.

use std::{
    collections::{HashMap, HashSet},
    path::Path,
    sync::{Arc, Mutex},
};

use rusqlite::{params, Connection};

use crate::{
    contracts::{
        AppError, AppErrorCode, PracticeDirection, PracticePreferences, RelatedOrigin, RelatedWord,
        StudyPracticeOutcome, StudyPracticeQuestion, TextbookEntry,
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
        seed: u64,
        now_ms: u64,
    ) -> Result<Vec<RelatedWord>, AppError> {
        let personal_entries = self.vocabulary.list(None, now_ms)?;
        let anchor = personal_entries
            .iter()
            .find(|entry| entry.id == entry_id)
            .ok_or_else(|| internal("Vocabulary entry was not found"))?;
        let anchor_source = anchor.source_text.clone();
        let anchor_translation = anchor.translated_text.clone();
        let source_language = anchor.effective_source_language.clone();
        let target_language = anchor.target_language.clone();
        let mut combined = HashMap::<String, RelatedWord>::new();

        for item in self.vocabulary.related(entry_id, now_ms)? {
            if !item
                .entry
                .effective_source_language
                .eq_ignore_ascii_case(&source_language)
                || !item
                    .entry
                    .target_language
                    .eq_ignore_ascii_case(&target_language)
            {
                continue;
            }
            let pair = pair_key(
                &item.entry.source_text,
                &item.entry.translated_text,
                &item.entry.effective_source_language,
                &item.entry.target_language,
            );
            let mut origins = vec![RelatedOrigin {
                kind: "personal".into(),
                textbook_id: None,
                textbook_title: None,
            }];
            for provenance in self.vocabulary.provenance(item.entry.id)? {
                push_origin(
                    &mut origins,
                    RelatedOrigin {
                        kind: "textbook".into(),
                        textbook_id: Some(provenance.textbook_id),
                        textbook_title: Some(provenance.textbook_title),
                    },
                );
            }
            combined.insert(
                pair,
                RelatedWord {
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
                    origins,
                },
            );
        }

        let mut searches = Vec::new();
        if let Some(root) = conservative_root(&anchor_source) {
            searches.push(root);
        }
        searches.extend(meaning_terms(&anchor_translation));
        searches.sort();
        searches.dedup();

        for book in self.textbooks.list_installed()?.into_iter().filter(|book| {
            book.source_language.eq_ignore_ascii_case(&source_language)
                && book.target_language.eq_ignore_ascii_case(&target_language)
        }) {
            let mut seen = HashSet::new();
            for search in &searches {
                for entry in self
                    .textbooks
                    .list_entries(&book.id, Some(search), 0, 100)?
                    .entries
                {
                    if !seen.insert(entry.id)
                        || (key(&entry.source_text) == key(&anchor_source)
                            && key(&entry.translated_text) == key(&anchor_translation))
                    {
                        continue;
                    }
                    let Some(reason) = relation_reason(
                        &anchor_source,
                        &anchor_translation,
                        &entry.source_text,
                        &entry.translated_text,
                    ) else {
                        continue;
                    };
                    let pair = pair_key(
                        &entry.source_text,
                        &entry.translated_text,
                        &entry.source_language,
                        &entry.target_language,
                    );
                    let origin = RelatedOrigin {
                        kind: "textbook".into(),
                        textbook_id: Some(book.id.clone()),
                        textbook_title: Some(book.title.clone()),
                    };
                    if let Some(existing) = combined.get_mut(&pair) {
                        push_origin(&mut existing.origins, origin);
                        existing.textbook_entry_id.get_or_insert(entry.id);
                        existing.textbook_id.get_or_insert(book.id.clone());
                    } else {
                        combined.insert(
                            pair,
                            RelatedWord {
                                kind: "textbook".into(),
                                vocabulary_entry_id: None,
                                textbook_entry_id: Some(entry.id),
                                textbook_id: Some(book.id.clone()),
                                source_text: entry.source_text,
                                translated_text: entry.translated_text,
                                source_language: entry.source_language,
                                target_language: entry.target_language,
                                reason: reason.into(),
                                promoted: false,
                                origins: vec![origin],
                            },
                        );
                    }
                }
            }
        }

        let mut result = combined.into_values().collect::<Vec<_>>();
        result.sort_by_key(|item| {
            (
                if item.reason == "root" { 0 } else { 1 },
                seeded_hash(
                    seed,
                    &pair_key(
                        &item.source_text,
                        &item.translated_text,
                        &item.source_language,
                        &item.target_language,
                    ),
                ),
            )
        });
        result.truncate(48);
        Ok(result)
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
        let all_personal_entries = self.vocabulary.list(None, now_ms)?;
        let most_recent_reviewed = all_personal_entries
            .iter()
            .filter_map(|entry| {
                entry
                    .last_reviewed_epoch_ms
                    .map(|reviewed| (reviewed, entry.id))
            })
            .max()
            .map(|(_, id)| id);
        let mut entries = all_personal_entries.clone();
        entries.retain(|entry| entry.effective_recall < 100.0);
        if entries.is_empty() {
            return Ok(None);
        }
        entries.sort_by(|left, right| {
            left.effective_recall
                .total_cmp(&right.effective_recall)
                .then_with(|| right.lookup_count.cmp(&left.lookup_count))
                .then_with(|| left.id.cmp(&right.id))
        });
        if entries.len() > 1 {
            if let Some(most_recent) = most_recent_reviewed {
                entries.retain(|entry| entry.id != most_recent);
            }
        }
        if entries.is_empty() {
            return Ok(None);
        }
        let total_weight = entries
            .iter()
            .map(|entry| practice_weight(entry.effective_recall, entry.lookup_count))
            .sum::<u64>();
        let mut draw = seeded_hash(seed, "study-selection") % total_weight;
        let candidate_rotation = entries
            .iter()
            .position(|entry| {
                let weight = practice_weight(entry.effective_recall, entry.lookup_count);
                if draw < weight {
                    true
                } else {
                    draw -= weight;
                    false
                }
            })
            .unwrap_or_default();
        entries.rotate_left(candidate_rotation);
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

            for related in self
                .related(candidate.id, seed, now_ms)?
                .into_iter()
                .filter(|item| {
                    item.source_language
                        .eq_ignore_ascii_case(&candidate.effective_source_language)
                        && item
                            .target_language
                            .eq_ignore_ascii_case(&candidate.target_language)
                })
            {
                let answer = match direction {
                    PracticeDirection::SourceToTarget => related.translated_text,
                    PracticeDirection::TargetToSource => related.source_text,
                    PracticeDirection::Random => unreachable!(),
                };
                if choice_keys.insert(key(&answer)) {
                    choices.push(display(&answer));
                }
                if choices.len() == 4 {
                    break;
                }
            }

            let mut textbook_fallback = self.sample_textbook_fallback(
                &candidate.effective_source_language,
                &candidate.target_language,
                seed,
            )?;
            textbook_fallback.sort_by_key(|entry| {
                seeded_hash(
                    seed ^ 0x7478_7462,
                    &pair_key(
                        &entry.source_text,
                        &entry.translated_text,
                        &entry.source_language,
                        &entry.target_language,
                    ),
                )
            });
            for textbook_entry in textbook_fallback {
                if choices.len() >= 4 {
                    break;
                }
                let answer = match direction {
                    PracticeDirection::SourceToTarget => &textbook_entry.translated_text,
                    PracticeDirection::TargetToSource => &textbook_entry.source_text,
                    PracticeDirection::Random => unreachable!(),
                };
                if choice_keys.insert(key(answer)) {
                    choices.push(display(answer));
                }
            }

            let mut personal_fallback = all_personal_entries.iter().collect::<Vec<_>>();
            personal_fallback.sort_by_key(|entry| {
                seeded_hash(
                    seed ^ 0x7065_7273,
                    &pair_key(
                        &entry.source_text,
                        &entry.translated_text,
                        &entry.effective_source_language,
                        &entry.target_language,
                    ),
                )
            });
            for entry in personal_fallback {
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
            let rotation =
                seeded_hash(seed ^ 0x6368_6f69, &correct_answer) as usize % choices.len();
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

    fn sample_textbook_fallback(
        &self,
        source_language: &str,
        target_language: &str,
        seed: u64,
    ) -> Result<Vec<TextbookEntry>, AppError> {
        const SAMPLE_PAGE_SIZE: u64 = 8;
        const MAX_SAMPLED_ENTRIES: usize = 64;

        let mut sampled = Vec::<(u64, TextbookEntry)>::new();
        for book in self.textbooks.list_installed()?.into_iter().filter(|book| {
            book.entry_count > 0
                && book.source_language.eq_ignore_ascii_case(source_language)
                && book.target_language.eq_ignore_ascii_case(target_language)
        }) {
            let offset = seeded_hash(seed ^ 0x7478_7462, &book.id) % book.entry_count;
            let limit = SAMPLE_PAGE_SIZE.min(book.entry_count - offset);
            for entry in self
                .textbooks
                .list_entries(&book.id, None, offset, limit)?
                .entries
            {
                let rank = seeded_hash(
                    seed ^ 0x7361_6d70,
                    &pair_key(
                        &entry.source_text,
                        &entry.translated_text,
                        &entry.source_language,
                        &entry.target_language,
                    ),
                );
                sampled.push((rank, entry));
                sampled.sort_by_key(|(rank, _)| *rank);
                sampled.truncate(MAX_SAMPLED_ENTRIES);
            }
        }
        Ok(sampled.into_iter().map(|(_, entry)| entry).collect())
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

fn meaning_terms(value: &str) -> HashSet<String> {
    value
        .split(|character: char| !character.is_alphabetic())
        .filter(|term| term.chars().count() >= 2)
        .map(|term| conservative_root(term).unwrap_or_else(|| key(term)))
        .collect()
}

fn relation_reason(
    anchor_source: &str,
    anchor_translation: &str,
    source: &str,
    translation: &str,
) -> Option<&'static str> {
    let anchor_root = conservative_root(anchor_source);
    if anchor_root.is_some() && conservative_root(source) == anchor_root {
        return Some("root");
    }
    let anchor_meaning = meaning_terms(anchor_translation);
    (!anchor_meaning.is_disjoint(&meaning_terms(translation))).then_some("meaning")
}

fn pair_key(
    source: &str,
    translation: &str,
    source_language: &str,
    target_language: &str,
) -> String {
    format!(
        "{}\u{1f}{}\u{1f}{}\u{1f}{}",
        key(source),
        key(translation),
        source_language.to_lowercase(),
        target_language.to_lowercase()
    )
}

fn push_origin(origins: &mut Vec<RelatedOrigin>, origin: RelatedOrigin) {
    if !origins.contains(&origin) {
        origins.push(origin);
    }
}

fn seeded_hash(seed: u64, value: &str) -> u64 {
    value
        .bytes()
        .fold(seed ^ 0xcbf2_9ce4_8422_2325, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x1000_0000_01b3)
        })
}

fn practice_weight(effective_recall: f64, lookup_count: u64) -> u64 {
    let recall_need = (100.0 - effective_recall).clamp(0.0, 100.0).round() as u64;
    recall_need.saturating_add(lookup_count.min(100)).max(1)
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
            ("ephemerally", "短暂地"),
            ("ephemeralness", "短暂性"),
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
    fn random_direction_candidate_and_choices_are_seeded() {
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
        assert_eq!(
            forward,
            service.question(100, 2).expect("repeat").expect("forward")
        );
        assert_eq!(
            forward
                .choices
                .iter()
                .map(|choice| key(choice))
                .collect::<HashSet<_>>()
                .len(),
            forward.choices.len()
        );
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
    fn related_combines_personal_and_installed_textbooks_with_personal_precedence() {
        let (_file, service) = seeded_service();
        install_book(&service);
        service.preferences.lock().expect("connection").execute(
            "INSERT INTO vocabulary_entries (normalized_text, source_text, requested_source_language, target_language, translated_text, detected_source_language, effective_source_language, first_seen_epoch_ms, last_seen_epoch_ms) VALUES ('ephemerally', 'ephemerally', 'auto', 'zh-CN', '短暂地', 'en', 'en', 40, 40)",
            [],
        ).expect("personal duplicate");
        let anchor = service
            .vocabulary
            .list(None, 100)
            .expect("entries")
            .into_iter()
            .find(|entry| entry.source_text == "ephemeral")
            .expect("anchor");
        let related = service.related(anchor.id, 7, 100).expect("combined");
        let merged = related
            .iter()
            .find(|item| item.source_text == "ephemerally")
            .expect("merged pair");
        assert_eq!(merged.kind, "personal");
        assert!(merged.vocabulary_entry_id.is_some());
        assert!(merged
            .origins
            .iter()
            .any(|origin| origin.kind == "personal"));
        assert!(merged
            .origins
            .iter()
            .any(|origin| origin.kind == "textbook"));
        assert!(related
            .iter()
            .any(|item| item.source_text == "ephemeralness" && item.kind == "textbook"));
        service.textbooks.set_active(None).expect("deactivate");
        assert!(service
            .related(anchor.id, 7, 100)
            .expect("all installed")
            .iter()
            .any(|item| item.origins.iter().any(|origin| origin.kind == "textbook")));
    }

    #[test]
    fn related_excludes_personal_entries_with_an_incompatible_language_pair() {
        let (_file, service) = seeded_service();
        service.preferences.lock().expect("connection").execute_batch(
            "INSERT INTO vocabulary_entries (normalized_text, source_text, requested_source_language, target_language, translated_text, detected_source_language, effective_source_language, first_seen_epoch_ms, last_seen_epoch_ms) VALUES ('ephemerally-fr', 'ephemerally', 'auto', 'zh-CN', '短暂地', 'fr', 'fr', 40, 40);
             INSERT INTO vocabulary_entries (normalized_text, source_text, requested_source_language, target_language, translated_text, detected_source_language, effective_source_language, first_seen_epoch_ms, last_seen_epoch_ms) VALUES ('ephemeralness-es', 'ephemeralness', 'auto', 'es', 'brevedad', 'en', 'en', 41, 41);",
        ).expect("incompatible related entries");
        let anchor = service
            .vocabulary
            .list(None, 100)
            .expect("entries")
            .into_iter()
            .find(|entry| entry.source_text == "ephemeral")
            .expect("anchor");

        let related = service.related(anchor.id, 7, 100).expect("related");

        assert!(!related.iter().any(|item| item.source_text == "ephemerally"));
        assert!(!related
            .iter()
            .any(|item| item.source_text == "ephemeralness"));
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

    #[test]
    fn practice_uses_seeded_full_pool_excludes_mastered_and_avoids_immediate_repeat() {
        let (_file, service) = seeded_service();
        service
            .save_preferences(PracticePreferences {
                direction: PracticeDirection::SourceToTarget,
            })
            .expect("forward");
        {
            let connection = service.preferences.lock().expect("connection");
            connection.execute(
                "INSERT INTO vocabulary_entries (normalized_text, source_text, requested_source_language, target_language, translated_text, detected_source_language, effective_source_language, first_seen_epoch_ms, last_seen_epoch_ms) VALUES ('transient', 'transient', 'auto', 'zh-CN', '短期的', 'en', 'en', 40, 40)",
                [],
            ).expect("extra candidate");
            connection.execute(
                "UPDATE vocabulary_entries SET recall_score = 100, last_reviewed_epoch_ms = 90 WHERE source_text = 'predeclare'",
                [],
            ).expect("mastered");
            connection.execute(
                "UPDATE vocabulary_entries SET last_reviewed_epoch_ms = 95 WHERE source_text = 'ephemeral'",
                [],
            ).expect("recent");
        }

        let first = service
            .question(100, 0)
            .expect("question")
            .expect("candidate");
        let second = service
            .question(100, 1)
            .expect("question")
            .expect("candidate");
        for question in [&first, &second] {
            assert_ne!(question.prompt, "predeclare");
            assert_ne!(question.prompt, "ephemeral");
        }
        assert_ne!(
            first.entry_id, second.entry_id,
            "seeds should vary candidate selection when the pool permits"
        );
    }

    #[test]
    fn practice_candidate_frequencies_weight_recall_need_and_lookup_demand() {
        let (_file, service) = seeded_service();
        service
            .save_preferences(PracticePreferences {
                direction: PracticeDirection::SourceToTarget,
            })
            .expect("forward");
        service.preferences.lock().expect("connection").execute_batch(
            "UPDATE vocabulary_entries SET recall_score = 0, lookup_count = 1 WHERE source_text = 'ephemeral';
             UPDATE vocabulary_entries SET recall_score = 80, lookup_count = 1 WHERE source_text = 'supersede';
             UPDATE vocabulary_entries SET recall_score = 80, lookup_count = 50 WHERE source_text = 'predeclare';",
        ).expect("weighted candidates");

        let mut frequencies = HashMap::<String, usize>::new();
        for seed in 0..384 {
            let prompt = service
                .question(100, seed)
                .expect("question")
                .expect("candidate")
                .prompt;
            *frequencies.entry(prompt).or_default() += 1;
        }

        let low_weight = frequencies["supersede"];
        assert!(
            frequencies["ephemeral"] > low_weight * 2,
            "greater recall need should materially increase frequency: {frequencies:?}"
        );
        assert!(
            frequencies["predeclare"] > low_weight * 2,
            "greater lookup demand should materially increase frequency: {frequencies:?}"
        );
    }

    #[test]
    fn mastered_latest_review_does_not_exclude_the_next_eligible_entry() {
        let (_file, service) = seeded_service();
        service
            .save_preferences(PracticePreferences {
                direction: PracticeDirection::SourceToTarget,
            })
            .expect("forward");
        service.preferences.lock().expect("connection").execute_batch(
            "UPDATE vocabulary_entries SET recall_score = 0, last_reviewed_epoch_ms = 90 WHERE source_text = 'ephemeral';
             UPDATE vocabulary_entries SET recall_score = 10, last_reviewed_epoch_ms = NULL WHERE source_text = 'supersede';
             UPDATE vocabulary_entries SET recall_score = 100, last_reviewed_epoch_ms = 100 WHERE source_text = 'predeclare';",
        ).expect("review state");

        let question = service
            .question(100, 0)
            .expect("question")
            .expect("eligible candidate");
        assert_eq!(question.prompt, "ephemeral");
    }

    #[test]
    fn practice_uses_related_distractors_from_installed_books_before_fallbacks() {
        let (_file, service) = seeded_service();
        install_book(&service);
        service
            .textbooks
            .set_active(None)
            .expect("inactive but installed");
        service
            .save_preferences(PracticePreferences {
                direction: PracticeDirection::SourceToTarget,
            })
            .expect("forward");

        let question = service
            .question(100, 0)
            .expect("question")
            .expect("candidate");
        assert_eq!(question.prompt, "ephemeral");
        assert!(question.choices.contains(&"短暂地".to_owned()));
        assert!(question.choices.contains(&"短暂性".to_owned()));
        assert_eq!(
            question,
            service
                .question(100, 0)
                .expect("repeat")
                .expect("candidate")
        );
        assert_eq!(
            question
                .choices
                .iter()
                .map(|choice| key(choice))
                .collect::<HashSet<_>>()
                .len(),
            question.choices.len()
        );
        assert_eq!(
            question
                .choices
                .iter()
                .filter(|choice| key(choice) == key("短暂的"))
                .count(),
            1
        );
    }

    #[test]
    fn practice_textbook_fallback_can_sample_beyond_the_first_page() {
        let (_file, service) = seeded_service();
        let fixture = NamedTempFile::new().expect("fixture");
        let connection = Connection::open(fixture.path()).expect("fixture db");
        connection.execute_batch("CREATE TABLE simple_translation (written_rep TEXT NOT NULL, trans_list TEXT NOT NULL);").expect("schema");
        for index in 0..510 {
            let source = format!("word{index:04}");
            let translation = if index < 500 {
                "短暂的".to_owned()
            } else {
                format!("页外选项{index}")
            };
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
            id: "large-en-zh".into(),
            title: "Large test book".into(),
            source_language: "en".into(),
            target_language: "zh-CN".into(),
            version: "1".into(),
            download_url: "https://download.wikdict.com/large-test.sqlite3".into(),
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
        service.preferences.lock().expect("connection").execute_batch(
            "UPDATE vocabulary_entries SET recall_score = 100 WHERE source_text != 'ephemeral';",
        ).expect("single candidate");
        service
            .save_preferences(PracticePreferences {
                direction: PracticeDirection::SourceToTarget,
            })
            .expect("forward");
        let seed = (0..10_000)
            .find(|seed| seeded_hash(*seed ^ 0x7478_7462, "large-en-zh") % 510 >= 500)
            .expect("seed beyond first page");

        let question = service
            .question(100, seed)
            .expect("question")
            .expect("candidate");

        assert!(question
            .choices
            .iter()
            .any(|choice| choice.starts_with("页外选项")));
    }
}
