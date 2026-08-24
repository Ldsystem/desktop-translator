import { useEffect, useRef, useState } from "react";

import type {
  InstalledTextbook,
  PracticePreferences,
  PracticeOutcome,
  PracticeQuestion,
  RelatedWord,
  RelatedVocabulary,
  StudyPracticeOutcome,
  StudyPracticeQuestion,
  TextbookCatalogItem,
  TextbookEntryPage,
  TextbookPromotionResult,
  VocabularyEntry,
  VocabularyProvenance,
} from "../../contracts/ipc";
import { PartOfSpeechBadge, PracticeView } from "./PracticeView";
import { RelatedWordsView } from "./RelatedWordsView";
import { TextbooksView } from "./TextbooksView";

type StudyView = "library" | "related" | "practice" | "textbooks";

interface VocabularyWindowProps {
  entries: readonly VocabularyEntry[];
  loading: boolean;
  error?: string;
  revision?: number;
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
  listVocabularyProvenance: (entryId: number) => Promise<VocabularyProvenance[]>;
  listRelated: (entryId: number, seed?: number) => Promise<RelatedWord[]>;
  deleteVocabularyEntry: (entryId: number) => Promise<void>;
  correctVocabularySourceLanguage: (entryId: number, sourceLanguage: string) => Promise<VocabularyEntry>;
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
    <svg viewBox="0 0 20 20" width="16" height="16" aria-hidden="true">
      <path d="M3.5 8v4h3l4 3.25V4.75L6.5 8h-3ZM13.4 7a4 4 0 0 1 0 6M15.8 4.8a7 7 0 0 1 0 10.4" />
    </svg>
  );
}

function PencilIcon() {
  return <svg viewBox="0 0 20 20" width="17" height="17" aria-hidden="true"><path d="m4 14.7.6-3.1L13 3.2a1.4 1.4 0 0 1 2 0l1.8 1.8a1.4 1.4 0 0 1 0 2l-8.4 8.4-3.1.6L4 14.7Z" /><path d="m11.8 4.4 3.8 3.8M4.7 11.7l3.6 3.6" /></svg>;
}

function BranchIcon() {
  return <svg viewBox="0 0 20 20" width="17" height="17" aria-hidden="true"><circle cx="5" cy="5" r="2" /><circle cx="15" cy="5" r="2" /><circle cx="15" cy="15" r="2" /><path d="M7 5h2a4 4 0 0 1 4 4v4M9 5h4" /></svg>;
}

function lexicalTextClass(value: string) {
  return value.length >= 11 ? "lexical-text lexical-text--long" : "lexical-text";
}

function EntryCard({
  entry,
  speechAvailability,
  onOpen,
  onPronounce,
  onManage,
  managed,
}: {
  entry: VocabularyEntry;
  speechAvailability: Readonly<Record<string, boolean>>;
  onOpen: () => void;
  onPronounce: (text: string, language: VocabularyEntry["effectiveSourceLanguage"]) => void;
  onManage: (trigger: HTMLButtonElement) => void;
  managed: boolean;
}) {
  const speak = (text: string, language: string) => {
    const availability = speechAvailability[language];
    const title = availability === true
    ? `Pronounce ${text}`
    : availability === false
      ? "No installed voice supports this language"
      : "Checking installed voice availability";
    return <span className="vocabulary-card__speak-state" title={title} tabIndex={availability === true ? undefined : 0} aria-label={availability === true ? undefined : `${title} for ${text}`}><button className="icon-button vocabulary-card__speak" type="button" aria-label={`Pronounce ${text}`} title={title} disabled={availability !== true} onClick={() => onPronounce(text, language)}><SpeakerIcon /></button></span>;
  };

  return (
    <article className="vocabulary-card">
      <RecallRuler entry={entry} />
      <div className="vocabulary-card__copy">
        <span className="vocabulary-card__word-row">{speak(entry.sourceText, entry.effectiveSourceLanguage)}<span className="vocabulary-card__lexeme"><strong className={lexicalTextClass(entry.sourceText)}>{entry.sourceText}</strong><PartOfSpeechBadge value={entry.partOfSpeech} /></span></span>
        <span className="vocabulary-card__word-row">{speak(entry.translatedText, entry.targetLanguage)}<span className={lexicalTextClass(entry.translatedText)}>{entry.translatedText}</span></span>
        <small>{entry.effectiveSourceLanguage} → {entry.targetLanguage}</small>
      </div>
      <div className="vocabulary-card__meta">
        <span className="vocabulary-card__signals">
          <span>{entry.lookupCount} {entry.lookupCount === 1 ? "lookup" : "lookups"}</span>
          <span>{familiarityNames[entry.familiarityLevel] ?? "New"}</span>
        </span>
        <span className="vocabulary-card__actions">
          <button className="icon-button vocabulary-card__action" type="button" aria-label={`Manage ${entry.sourceText}`} title="Manage word" aria-controls={`word-manage-${entry.id}`} aria-expanded={managed} onClick={(event) => onManage(event.currentTarget)}><PencilIcon /></button>
          <button className="icon-button vocabulary-card__action" type="button" aria-label={`Open related words for ${entry.sourceText}`} title="Find related words" onClick={onOpen}><BranchIcon /></button>
        </span>
      </div>
    </article>
  );
}

export function VocabularyWindow({
  entries,
  loading,
  error,
  revision = 0,
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
  const [managedEntry, setManagedEntry] = useState<VocabularyEntry>();
  const [correction, setCorrection] = useState("");
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [manageError, setManageError] = useState<string>();
  const manageTrigger = useRef<HTMLButtonElement | null>(null);
  const manageCloseButton = useRef<HTMLButtonElement>(null);
  const manageDialog = useRef<HTMLElement>(null);
  const studyRail = useRef<HTMLElement>(null);
  const studyScroller = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (view === "library") searchInput.current?.focus();
  }, [view]);

  useEffect(() => setSelectedChoice(undefined), [question?.entryId]);

  useEffect(() => {
    if (outcome) nextWordButton.current?.focus();
  }, [outcome]);

  useEffect(() => {
    if (!managedEntry || !studyApi) return;

    const dialog = manageDialog.current;
    const background = [studyRail.current, studyScroller.current].filter((element): element is HTMLElement => element !== null);
    const priorInert = background.map((element) => element.hasAttribute("inert"));
    background.forEach((element) => element.setAttribute("inert", ""));

    const focusableElements = () => dialog
      ? [...dialog.querySelectorAll<HTMLElement>('button:not([disabled]), select:not([disabled]), input:not([disabled]), a[href], [tabindex]:not([tabindex="-1"])')]
      : [];
    const handleDialogKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        setManagedEntry(undefined);
        setConfirmDelete(false);
        return;
      }
      if (event.key !== "Tab") return;

      const focusable = focusableElements();
      if (focusable.length === 0) {
        event.preventDefault();
        dialog?.focus();
        return;
      }
      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      const active = document.activeElement;
      if (event.shiftKey && (active === first || !dialog?.contains(active))) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && (active === last || !dialog?.contains(active))) {
        event.preventDefault();
        first.focus();
      }
    };

    document.addEventListener("keydown", handleDialogKeyDown);
    manageCloseButton.current?.focus();
    const trigger = manageTrigger.current;
    return () => {
      document.removeEventListener("keydown", handleDialogKeyDown);
      background.forEach((element, index) => {
        if (priorInert[index]) element.setAttribute("inert", "");
        else element.removeAttribute("inert");
      });
      if (trigger?.isConnected) trigger.focus();
    };
  }, [managedEntry, studyApi]);

  const closeManage = () => {
    setManagedEntry(undefined);
    setConfirmDelete(false);
  };

  const navigate = (next: StudyView) => {
    setView(next);
    if (next === "practice" && !studyApi) onStartPractice();
  };

  return (
    <main className="study-window">
      <aside ref={studyRail} className="study-rail" aria-label="Vocabulary study navigation">
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

      <section className="study-content"><div ref={studyScroller} className="study-scroll-region" data-scroll-owner="study-content">
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
              <div className="vocabulary-grid">{entries.map((entry) => <EntryCard key={entry.id} entry={entry} speechAvailability={speechAvailability} onPronounce={onPronounce} managed={managedEntry?.id === entry.id} onManage={(trigger) => { manageTrigger.current = trigger; setManagedEntry(entry); setCorrection(entry.effectiveSourceLanguage); setConfirmDelete(false); setManageError(undefined); }} onOpen={() => { setRelatedAnchor(entry); onSelectEntry(entry.id); setView("related"); }} />)}</div>
            )}
          </>
        )}

        {view === "related" && studyApi && <RelatedWordsView anchor={relatedAnchor} api={studyApi} revision={revision} />}

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

        {view === "practice" && studyApi && <PracticeView api={studyApi} revision={revision} />}

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
      </div>
      {managedEntry && studyApi && <section ref={manageDialog} id={`word-manage-${managedEntry.id}`} className="word-manage word-manage--drawer" role="dialog" aria-modal="true" tabIndex={-1} data-placement="drawer" aria-labelledby={`word-manage-title-${managedEntry.id}`}><header className="word-manage__header"><strong id={`word-manage-title-${managedEntry.id}`}>Manage {managedEntry.sourceText}</strong><button ref={manageCloseButton} className="text-button" type="button" onClick={closeManage}>Close</button></header><div className="word-manage__body">{manageError && <p className="study-notice study-notice--error" role="alert">{manageError}</p>}<label className="field">Source language<select value={correction} onChange={(event) => setCorrection(event.target.value)}>{["en", "zh-CN", "zh-TW", "ja", "ko", "ru", "fr", "de", "es"].map((language) => <option key={language} value={language}>{language.toUpperCase()}</option>)}</select></label></div><footer className="word-manage__footer"><button className="button button--secondary" type="button" onClick={() => { void studyApi.correctVocabularySourceLanguage(managedEntry.id, correction).then(() => { studyApi.refreshPersonal(); closeManage(); }).catch(() => setManageError("The language correction could not be saved.")); }}>Save language</button><span className="word-manage__delete-slot">{confirmDelete ? <span className="word-manage__confirm" role="status"><span>Delete this word?</span><button className="button button--danger" type="button" onClick={() => { void studyApi.deleteVocabularyEntry(managedEntry.id).then(() => { studyApi.refreshPersonal(); closeManage(); }).catch(() => setManageError("This word could not be deleted.")); }}>Confirm</button><button className="text-button" type="button" onClick={() => setConfirmDelete(false)}>Cancel</button></span> : <button className="text-button is-danger" type="button" aria-label={`Delete ${managedEntry.sourceText}`} onClick={() => setConfirmDelete(true)}>Delete word</button>}</span></footer></section>}
      </section>
    </main>
  );
}
