import { useEffect, useRef, useState } from "react";

import type {
  InstalledTextbook,
  PracticePreferences,
  PracticeOutcome,
  PracticeQuestion,
  RelatedSource,
  RelatedWord,
  RelatedVocabulary,
  StudyPracticeOutcome,
  StudyPracticeQuestion,
  TextbookCatalogItem,
  TextbookEntryPage,
  TextbookPromotionResult,
  VocabularyEntry,
} from "../../contracts/ipc";
import { PracticeView } from "./PracticeView";
import { RelatedWordsView } from "./RelatedWordsView";
import { TextbooksView } from "./TextbooksView";

type StudyView = "library" | "related" | "practice" | "textbooks";

interface VocabularyWindowProps {
  entries: readonly VocabularyEntry[];
  loading: boolean;
  error?: string;
  related: readonly RelatedVocabulary[];
  question: PracticeQuestion | null | undefined;
  outcome?: PracticeOutcome;
  speechAvailability?: Readonly<Record<string, boolean>>;
  onPronounce?: (text: string, language: VocabularyEntry["effectiveSourceLanguage"]) => void;
  onSearch: (search: string) => void;
  onSelectEntry: (entryId: number) => void;
  onStartPractice: () => void;
  onSubmitAnswer: (entryId: number, selectedTranslation: string) => void;
  studyApi?: StudyApi;
}

export interface StudyApi {
  listCatalog: () => Promise<TextbookCatalogItem[]>;
  listDownloaded: () => Promise<InstalledTextbook[]>;
  downloadTextbook: (textbookId: string) => Promise<InstalledTextbook>;
  setActiveTextbook: (textbookId?: string) => Promise<void>;
  removeTextbook: (textbookId: string) => Promise<void>;
  listTextbookEntries: (textbookId: string, search: string, offset: number, limit: number) => Promise<TextbookEntryPage>;
  addTextbookEntry: (textbookEntryId: number) => Promise<TextbookPromotionResult>;
  listRelated: (entryId: number, source: RelatedSource) => Promise<RelatedWord[]>;
  getPracticePreferences: () => Promise<PracticePreferences>;
  savePracticePreferences: (preferences: PracticePreferences) => Promise<void>;
  getPracticeQuestion: () => Promise<StudyPracticeQuestion | null>;
  submitPracticeAnswer: (entryId: number, direction: StudyPracticeQuestion["direction"], selectedAnswer: string) => Promise<StudyPracticeOutcome>;
  refreshPersonal: () => void;
}

const familiarityNames = ["New", "Fragile", "Forming", "Steady", "Strong", "Fluent"];

function RecallRuler({ entry }: { entry: VocabularyEntry }) {
  const recall = Math.round(entry.effectiveRecall);

  return (
    <div className="recall-ruler">
      <span className="sr-only">Recall {recall} out of 100</span>
      <span className="recall-ruler__track" aria-hidden="true">
        <span style={{ height: `${entry.effectiveRecall}%` }} />
      </span>
      <span className="recall-ruler__value" aria-hidden="true">{recall}</span>
    </div>
  );
}

function SpeakerIcon() {
  return (
    <svg viewBox="0 0 24 24" width="18" height="18" aria-hidden="true">
      <path d="M4 9v6h4l5 4V5L8 9H4Zm12.5-.7a5 5 0 0 1 0 7.4M19 5a9 9 0 0 1 0 14" />
    </svg>
  );
}

function EntryCard({
  entry,
  speechAvailability,
  onOpen,
  onPronounce,
}: {
  entry: VocabularyEntry;
  speechAvailability: Readonly<Record<string, boolean>>;
  onOpen: () => void;
  onPronounce: (text: string, language: VocabularyEntry["effectiveSourceLanguage"]) => void;
}) {
  const availability = speechAvailability[entry.effectiveSourceLanguage];
  const pronunciationTitle = availability === true
    ? `Pronounce ${entry.sourceText}`
    : availability === false
      ? "No installed voice supports this language"
      : "Checking installed voice availability";

  return (
    <article className="vocabulary-card">
      <RecallRuler entry={entry} />
      <button className="vocabulary-card__open" type="button" aria-label={`Open related words for ${entry.sourceText}`} onClick={onOpen}>
        <span className="vocabulary-card__copy">
          <strong>{entry.sourceText}</strong>
          <span>{entry.translatedText}</span>
          <small>{entry.effectiveSourceLanguage} → {entry.targetLanguage}</small>
        </span>
      </button>
      <div className="vocabulary-card__meta">
        <span className="vocabulary-card__signals">
          <span>{entry.lookupCount} {entry.lookupCount === 1 ? "lookup" : "lookups"}</span>
          <span>{familiarityNames[entry.familiarityLevel] ?? "New"}</span>
        </span>
        <span className="vocabulary-card__speak-state" title={pronunciationTitle} tabIndex={availability === true ? undefined : 0} aria-label={availability === true ? undefined : `${pronunciationTitle} for ${entry.sourceText}`}>
          <button className="icon-button vocabulary-card__speak" type="button" aria-label={`Pronounce ${entry.sourceText}`} title={pronunciationTitle} disabled={availability !== true} onClick={() => onPronounce(entry.sourceText, entry.effectiveSourceLanguage)}>
            <SpeakerIcon />
          </button>
        </span>
      </div>
    </article>
  );
}

export function VocabularyWindow({
  entries,
  loading,
  error,
  related,
  question,
  outcome,
  speechAvailability = {},
  onPronounce = () => undefined,
  onSearch,
  onSelectEntry,
  onStartPractice,
  onSubmitAnswer,
  studyApi,
}: VocabularyWindowProps) {
  const [view, setView] = useState<StudyView>(question !== undefined ? "practice" : "library");
  const [search, setSearch] = useState("");
  const [selectedChoice, setSelectedChoice] = useState<string>();
  const searchInput = useRef<HTMLInputElement>(null);
  const nextWordButton = useRef<HTMLButtonElement>(null);
  const [relatedAnchor, setRelatedAnchor] = useState<VocabularyEntry>();

  useEffect(() => {
    if (view === "library") searchInput.current?.focus();
  }, [view]);

  useEffect(() => setSelectedChoice(undefined), [question?.entryId]);

  useEffect(() => {
    if (outcome) nextWordButton.current?.focus();
  }, [outcome]);

  const navigate = (next: StudyView) => {
    setView(next);
    if (next === "practice" && !studyApi) onStartPractice();
  };

  return (
    <main className="study-window">
      <aside className="study-rail" aria-label="Vocabulary study navigation">
        <div className="study-mark" aria-hidden="true">Aa</div>
        <div>
          <p className="eyebrow">Personal lexicon</p>
          <h1>Wordbook</h1>
          <p>Built quietly from the words you translate.</p>
        </div>
        <nav>
          {(["library", "related", "practice", "textbooks"] as const).map((item) => (
            <button key={item} className={view === item ? "study-nav is-active" : "study-nav"} type="button" onClick={() => navigate(item)}>
              {item === "library" ? "My wordbook" : item === "related" ? "Related words" : item === "practice" ? "Practice" : "Textbooks"}
            </button>
          ))}
        </nav>
        <p className="study-rail__privacy">Your wordbook and practice activity stay on this device.</p>
      </aside>

      <section className="study-content">
        {view === "library" && (
          <>
            <header className="study-header">
              <div><p className="eyebrow">Textbook</p><h2>Your working vocabulary</h2></div>
              <label className="study-search">
                <span className="sr-only">Search vocabulary</span>
                <input ref={searchInput} value={search} type="search" placeholder="Find a word or translation" onChange={(event) => { setSearch(event.target.value); onSearch(event.target.value); }} />
              </label>
            </header>
            {error && <div className="study-notice study-notice--error" role="alert">{error}</div>}
            {loading ? (
              <div className="study-empty" role="status"><strong>Opening your wordbook…</strong><span>Reading local study history.</span></div>
            ) : entries.length === 0 ? (
              <div className="study-empty"><strong>Translate a word to begin.</strong><span>Eligible words and short phrases will appear here automatically.</span></div>
            ) : (
              <div className="vocabulary-grid">{entries.map((entry) => <EntryCard key={entry.id} entry={entry} speechAvailability={speechAvailability} onPronounce={onPronounce} onOpen={() => { setRelatedAnchor(entry); onSelectEntry(entry.id); setView("related"); }} />)}</div>
            )}
          </>
        )}

        {view === "related" && studyApi && <RelatedWordsView anchor={relatedAnchor} api={studyApi} />}

        {view === "related" && !studyApi && (
          <>
            <header className="study-header"><div><p className="eyebrow">Related words</p><h2>Connections in your wordbook</h2><p>Roots use a conservative Latin suffix rule. Meanings share words in stored translations.</p></div></header>
            {related.length === 0 ? (
              <div className="study-empty"><strong>No local connections yet.</strong><span>Open a word from the textbook as your collection grows.</span></div>
            ) : (
              <div className="relation-list">{related.map(({ entry, reason }) => <article key={`${entry.id}-${reason}`}><span className={`relation-badge relation-badge--${reason}`}>{reason === "root" ? "shared root" : "shared meaning"}</span><strong>{entry.sourceText}</strong><span>{entry.translatedText}</span></article>)}</div>
            )}
          </>
        )}

        {view === "practice" && studyApi && <PracticeView api={studyApi} />}

        {view === "practice" && !studyApi && (
          <>
            <header className="study-header"><div><p className="eyebrow">Practice</p><h2>Choose the translation</h2><p>Recall changes only after you check an answer.</p></div></header>
            {question === undefined ? (
              <div className="study-empty" role="status"><strong>Choosing what needs attention…</strong></div>
            ) : question === null ? (
              <div className="study-empty"><strong>Add at least two distinct translations.</strong><span>Practice questions are assembled only from your local wordbook.</span></div>
            ) : (
              <section className="practice-card">
                <div className="practice-prompt"><span>{question.effectiveSourceLanguage} → {question.targetLanguage}</span><strong>{question.sourceText}</strong></div>
                <div className="practice-choices" role="radiogroup" aria-label="Translation choices">
                  {question.choices.map((choice) => <button key={choice} type="button" role="radio" aria-checked={selectedChoice === choice} className={selectedChoice === choice ? "practice-choice is-selected" : "practice-choice"} disabled={Boolean(outcome)} onClick={() => setSelectedChoice(choice)}>{choice}</button>)}
                </div>
                <div className="practice-actions">
                  {outcome ? (
                    <>
                      <div className={outcome.correct ? "practice-feedback is-correct" : "practice-feedback is-wrong"} role="status">
                        <span className="practice-feedback__mark" aria-hidden="true">{outcome.correct ? "✓" : "↺"}</span>
                        <span className="practice-feedback__copy"><strong>{outcome.correct ? "Correct" : "Keep this one close"}</strong><span>{outcome.correct ? `Recall is now ${Math.round(outcome.entry.effectiveRecall)}.` : `The translation is “${outcome.correctTranslation}”.`}</span></span>
                      </div>
                      <button ref={nextWordButton} className="button button--primary practice-next" type="button" onClick={onStartPractice}>Next word</button>
                    </>
                  ) : (
                    <button className="button button--primary practice-submit" type="button" disabled={!selectedChoice} onClick={() => selectedChoice && onSubmitAnswer(question.entryId, selectedChoice)}>Check answer</button>
                  )}
                </div>
              </section>
            )}
          </>
        )}

        {view === "textbooks" && (studyApi ? <TextbooksView api={studyApi} /> : <div className="study-empty"><strong>Textbooks are unavailable.</strong><span>Restart the desktop app to reconnect the local textbook service.</span></div>)}
      </section>
    </main>
  );
}
