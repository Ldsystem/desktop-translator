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
  UiLocale,
} from "../../contracts/ipc";
import { PartOfSpeechBadge, PracticeView } from "./PracticeView";
import { RelatedWordsView } from "./RelatedWordsView";
import { TextbooksView } from "./TextbooksView";

type StudyView = "library" | "related" | "practice" | "textbooks";

interface VocabularyWindowProps {
  locale?: UiLocale;
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

const familiarityNames = {
  en: ["New", "Fragile", "Forming", "Steady", "Strong", "Fluent"],
  "zh-CN": ["新词", "生疏", "正在形成", "稳定", "熟练", "流利"],
} as const;

function RecallRuler({ entry, locale }: { entry: VocabularyEntry; locale: UiLocale }) {
  const recall = Math.round(entry.effectiveRecall);

  return (
    <div className="recall-ruler">
      <span className="sr-only">{locale === "zh-CN" ? `记忆度 ${recall}/100` : `Recall ${recall} out of 100`}</span>
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
  locale,
  speechAvailability,
  onOpen,
  onPronounce,
  onManage,
  managed,
}: {
  entry: VocabularyEntry;
  locale: UiLocale;
  speechAvailability: Readonly<Record<string, boolean>>;
  onOpen: () => void;
  onPronounce: (text: string, language: VocabularyEntry["effectiveSourceLanguage"]) => void;
  onManage: (trigger: HTMLButtonElement) => void;
  managed: boolean;
}) {
  const zh = locale === "zh-CN";
  const speak = (text: string, language: string) => {
    const availability = speechAvailability[language];
    const title = availability === true
    ? (zh ? `朗读 ${text}` : `Pronounce ${text}`)
    : availability === false
      ? (zh ? "没有已安装的语音支持此语言" : "No installed voice supports this language")
      : (zh ? "正在检查已安装语音" : "Checking installed voice availability");
    const unavailableLabel = zh ? `${text}：${title}` : `${title} for ${text}`;
    return <span className="vocabulary-card__speak-state" title={title} tabIndex={availability === true ? undefined : 0} aria-label={availability === true ? undefined : unavailableLabel}><button className="icon-button vocabulary-card__speak" type="button" aria-label={zh ? `朗读 ${text}` : `Pronounce ${text}`} title={title} disabled={availability !== true} onClick={() => onPronounce(text, language)}><SpeakerIcon /></button></span>;
  };

  return (
    <article className="vocabulary-card">
      <RecallRuler entry={entry} locale={locale} />
      <div className="vocabulary-card__copy">
        <span className="vocabulary-card__word-row">{speak(entry.sourceText, entry.effectiveSourceLanguage)}<span className="vocabulary-card__lexeme"><strong className={lexicalTextClass(entry.sourceText)}>{entry.sourceText}</strong><PartOfSpeechBadge value={entry.partOfSpeech} /></span></span>
        <span className="vocabulary-card__word-row">{speak(entry.translatedText, entry.targetLanguage)}<span className={lexicalTextClass(entry.translatedText)}>{entry.translatedText}</span></span>
        <small>{entry.effectiveSourceLanguage} → {entry.targetLanguage}</small>
      </div>
      <div className="vocabulary-card__meta">
        <span className="vocabulary-card__signals">
          <span>{zh ? `${entry.lookupCount} 次查词` : `${entry.lookupCount} ${entry.lookupCount === 1 ? "lookup" : "lookups"}`}</span>
          <span>{familiarityNames[locale][entry.familiarityLevel] ?? familiarityNames[locale][0]}</span>
        </span>
        <span className="vocabulary-card__actions">
          <button className="icon-button vocabulary-card__action" type="button" aria-label={zh ? `管理 ${entry.sourceText}` : `Manage ${entry.sourceText}`} title={zh ? "管理词汇" : "Manage word"} aria-controls={`word-manage-${entry.id}`} aria-expanded={managed} onClick={(event) => onManage(event.currentTarget)}><PencilIcon /></button>
          <button className="icon-button vocabulary-card__action" type="button" aria-label={zh ? `查看 ${entry.sourceText} 的相关词` : `Open related words for ${entry.sourceText}`} title={zh ? "查找相关词" : "Find related words"} onClick={onOpen}><BranchIcon /></button>
        </span>
      </div>
    </article>
  );
}

export function VocabularyWindow({
  locale = "en",
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
  const zh = locale === "zh-CN";
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
      <aside ref={studyRail} className="study-rail" aria-label={zh ? "词汇学习导航" : "Vocabulary study navigation"}>
        <div className="study-mark" aria-hidden="true">Aa</div>
        <div>
          <p className="eyebrow">{zh ? "个人词库" : "Personal lexicon"}</p>
          <h1>{zh ? "词汇本" : "Wordbook"}</h1>
          <p>{zh ? "从你翻译的词汇中安静积累。" : "Built quietly from the words you translate."}</p>
        </div>
        <nav>
          {(["library", "practice", "textbooks"] as const).map((item) => (
            <button key={item} className={view === item ? "study-nav is-active" : "study-nav"} type="button" onClick={() => navigate(item)}>
              {item === "library" ? (zh ? "我的词汇本" : "My wordbook") : item === "practice" ? (zh ? "练习" : "Practice") : (zh ? "词书" : "Textbooks")}
            </button>
          ))}
        </nav>
        <p className="study-rail__privacy">{zh ? "词汇本和练习记录仅保存在本机。" : "Your wordbook and practice activity stay on this device."}</p>
      </aside>

      <section className="study-content"><div ref={studyScroller} className="study-scroll-region" data-scroll-owner="study-content">
        {view === "library" && (
          <>
            <header className="study-header">
              <div><p className="eyebrow">{zh ? "词汇本" : "Textbook"}</p><h2>{zh ? "正在学习的词汇" : "Your working vocabulary"}</h2></div>
              <label className="study-search">
                <span className="sr-only">{zh ? "搜索词汇" : "Search vocabulary"}</span>
                <input ref={searchInput} value={search} type="search" placeholder={zh ? "查找单词或译文" : "Find a word or translation"} onChange={(event) => { setSearch(event.target.value); onSearch(event.target.value); }} />
              </label>
            </header>
            {error && <div className="study-notice study-notice--error" role="alert">{error}</div>}
            {loading ? (
              <div className="study-empty" role="status"><strong>{zh ? "正在打开词汇本…" : "Opening your wordbook…"}</strong><span>{zh ? "正在读取本地学习记录。" : "Reading local study history."}</span></div>
            ) : entries.length === 0 ? (
              <div className="study-empty"><strong>{zh ? "翻译一个单词即可开始。" : "Translate a word to begin."}</strong><span>{zh ? "符合条件的单词和短语会自动出现在这里。" : "Eligible words and short phrases will appear here automatically."}</span></div>
            ) : (
              <div className="vocabulary-grid">{entries.map((entry) => <EntryCard key={entry.id} entry={entry} locale={locale} speechAvailability={speechAvailability} onPronounce={onPronounce} managed={managedEntry?.id === entry.id} onManage={(trigger) => { manageTrigger.current = trigger; setManagedEntry(entry); setCorrection(entry.effectiveSourceLanguage); setConfirmDelete(false); setManageError(undefined); }} onOpen={() => { setRelatedAnchor(entry); onSelectEntry(entry.id); setView("related"); }} />)}</div>
            )}
          </>
        )}

        {view === "related" && studyApi && <RelatedWordsView anchor={relatedAnchor} api={studyApi} revision={revision} locale={locale} onBack={() => navigate("library")} />}

        {view === "related" && !studyApi && (
          <>
            <header className="study-header"><div><button className="text-button textbook-back" type="button" onClick={() => navigate("library")}>← {zh ? "返回我的词汇本" : "Back to My wordbook"}</button><p className="eyebrow">{zh ? "相关词" : "Related words"}</p><h2>{zh ? "词汇本中的关联" : "Connections in your wordbook"}</h2><p>{zh ? "词根采用保守的拉丁后缀规则；释义关联来自已保存的翻译。" : "Roots use a conservative Latin suffix rule. Meanings share words in stored translations."}</p></div></header>
            {related.length === 0 ? (
              <div className="study-empty"><strong>{zh ? "暂未找到本地关联。" : "No local connections yet."}</strong><span>{zh ? "随着词汇积累，可从词汇卡片打开相关词。" : "Open a word from the textbook as your collection grows."}</span></div>
            ) : (
              <div className="relation-list">{related.map(({ entry, reason }) => <article key={`${entry.id}-${reason}`}><span className={`relation-badge relation-badge--${reason}`}>{reason === "root" ? "shared root" : "shared meaning"}</span><strong>{entry.sourceText}</strong><span>{entry.translatedText}</span></article>)}</div>
            )}
          </>
        )}

        {view === "practice" && studyApi && <PracticeView api={studyApi} revision={revision} locale={locale} />}

        {view === "practice" && !studyApi && (
          <>
            <header className="study-header"><div><p className="eyebrow">{zh ? "练习" : "Practice"}</p><h2>{zh ? "选择正确译文" : "Choose the translation"}</h2><p>{zh ? "提交答案后才会更新 Recall。" : "Recall changes only after you check an answer."}</p></div></header>
            {question === undefined ? (
              <div className="study-empty" role="status"><strong>{zh ? "正在挑选需要复习的词汇…" : "Choosing what needs attention…"}</strong></div>
            ) : question === null ? (
              <div className="study-empty"><strong>{zh ? "请至少添加两个释义不同的词汇。" : "Add at least two distinct translations."}</strong><span>{zh ? "练习题仅从你的本地词汇本中生成。" : "Practice questions are assembled only from your local wordbook."}</span></div>
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
                        <span className="practice-feedback__copy"><strong>{outcome.correct ? (zh ? "正确" : "Correct") : (zh ? "再记一遍" : "Keep this one close")}</strong><span>{outcome.correct ? (zh ? `Recall 已更新为 ${Math.round(outcome.entry.effectiveRecall)}。` : `Recall is now ${Math.round(outcome.entry.effectiveRecall)}.`) : (zh ? `正确译文是“${outcome.correctTranslation}”。` : `The translation is “${outcome.correctTranslation}”.`)}</span></span>
                      </div>
                      <button ref={nextWordButton} className="button button--primary practice-next" type="button" onClick={onStartPractice}>{zh ? "下一个词" : "Next word"}</button>
                    </>
                  ) : (
                    <button className="button button--primary practice-submit" type="button" disabled={!selectedChoice} onClick={() => selectedChoice && onSubmitAnswer(question.entryId, selectedChoice)}>{zh ? "检查答案" : "Check answer"}</button>
                  )}
                </div>
              </section>
            )}
          </>
        )}

        {view === "textbooks" && (studyApi ? <TextbooksView api={studyApi} locale={locale} /> : <div className="study-empty"><strong>{zh ? "词书暂不可用。" : "Textbooks are unavailable."}</strong><span>{zh ? "请重启桌面翻译以重新连接本地词书服务。" : "Restart the desktop app to reconnect the local textbook service."}</span></div>)}
      </div>
      {managedEntry && studyApi && <section ref={manageDialog} id={`word-manage-${managedEntry.id}`} className="word-manage word-manage--drawer" role="dialog" aria-modal="true" tabIndex={-1} data-placement="drawer" aria-labelledby={`word-manage-title-${managedEntry.id}`}><header className="word-manage__header"><strong id={`word-manage-title-${managedEntry.id}`}>{zh ? "管理" : "Manage"} {managedEntry.sourceText}</strong><button ref={manageCloseButton} className="text-button" type="button" onClick={closeManage}>{zh ? "关闭" : "Close"}</button></header><div className="word-manage__body">{manageError && <p className="study-notice study-notice--error" role="alert">{manageError}</p>}<label className="field">{zh ? "源语言" : "Source language"}<select value={correction} onChange={(event) => setCorrection(event.target.value)}>{["en", "zh-CN", "zh-TW", "ja", "ko", "ru", "fr", "de", "es"].map((language) => <option key={language} value={language}>{language.toUpperCase()}</option>)}</select></label></div><footer className="word-manage__footer"><button className="button button--secondary" type="button" onClick={() => { void studyApi.correctVocabularySourceLanguage(managedEntry.id, correction).then(() => { studyApi.refreshPersonal(); closeManage(); }).catch(() => setManageError(zh ? "无法保存语言更正。" : "The language correction could not be saved.")); }}>{zh ? "保存语言" : "Save language"}</button><span className="word-manage__delete-slot">{confirmDelete ? <span className="word-manage__confirm" role="status"><span>{zh ? "删除此词？" : "Delete this word?"}</span><button className="button button--danger" type="button" onClick={() => { void studyApi.deleteVocabularyEntry(managedEntry.id).then(() => { studyApi.refreshPersonal(); closeManage(); }).catch(() => setManageError(zh ? "无法删除此词。" : "This word could not be deleted.")); }}>{zh ? "确认" : "Confirm"}</button><button className="text-button" type="button" onClick={() => setConfirmDelete(false)}>{zh ? "取消" : "Cancel"}</button></span> : <button className="text-button is-danger" type="button" aria-label={`Delete ${managedEntry.sourceText}`} onClick={() => setConfirmDelete(true)}>{zh ? "删除词汇" : "Delete word"}</button>}</span></footer></section>}
      </section>
    </main>
  );
}
