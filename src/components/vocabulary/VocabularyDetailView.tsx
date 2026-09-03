import { useEffect, useRef, useState } from "react";

import type { RelatedFilter, UiLocale, VocabularyDetail, VocabularyEntry } from "../../contracts/ipc";
import { PartOfSpeechBadge } from "./PracticeView";
import type { StudyApi } from "./VocabularyWindow";

interface VocabularyDetailViewProps {
  entry: VocabularyEntry;
  api: StudyApi;
  revision: number;
  locale?: UiLocale;
  speechAvailable?: boolean;
  onPronounce: (text: string, language: VocabularyEntry["effectiveSourceLanguage"]) => void;
  restoreFocusKey?: string;
  onBack: () => void;
  onOpenRelated: (filter: RelatedFilter, focusKey: string) => void;
}

function SpeakerIcon() {
  return <svg viewBox="0 0 20 20" width="16" height="16" aria-hidden="true"><path d="M3.5 8v4h3l4 3.25V4.75L6.5 8h-3ZM13.4 7a4 4 0 0 1 0 6M15.8 4.8a7 7 0 0 1 0 10.4" /></svg>;
}

export function VocabularyDetailView({ entry, api, revision, locale = "en", speechAvailable, onPronounce, restoreFocusKey, onBack, onOpenRelated }: VocabularyDetailViewProps) {
  const zh = locale === "zh-CN";
  const [detail, setDetail] = useState<VocabularyDetail>();
  const [error, setError] = useState<string>();
  const [refreshing, setRefreshing] = useState(false);
  const request = useRef(0);
  const section = useRef<HTMLElement>(null);

  useEffect(() => {
    const current = ++request.current;
    setDetail(undefined);
    setError(undefined);
    void api.getVocabularyDetail(entry.id)
      .then((next) => { if (current === request.current) setDetail(next); })
      .catch(() => { if (current === request.current) setError(zh ? "无法加载词汇详情。" : "Word details could not be loaded."); });
  }, [api, entry.id, revision, zh]);

  useEffect(() => {
    if (!detail || !restoreFocusKey) return;
    window.requestAnimationFrame(() => section.current?.querySelector<HTMLButtonElement>(`[data-relation-focus="${restoreFocusKey}"]`)?.focus());
  }, [detail, restoreFocusKey]);

  return <section ref={section} className="vocabulary-detail" aria-labelledby="vocabulary-detail-title">
    <header className="study-header vocabulary-detail__header">
      <div>
        <button className="text-button textbook-back" type="button" onClick={onBack}>← {zh ? "返回我的词汇本" : "Back to My wordbook"}</button>
        <p className="eyebrow">{zh ? "词汇详情" : "Word detail"}</p>
        <div className="vocabulary-detail__word"><button className="icon-button vocabulary-card__speak" type="button" aria-label={zh ? `朗读 ${entry.sourceText}` : `Pronounce ${entry.sourceText}`} disabled={speechAvailable !== true} onClick={() => onPronounce(entry.sourceText, entry.effectiveSourceLanguage)}><SpeakerIcon /></button><h2 id="vocabulary-detail-title" className="lexical-text">{entry.sourceText}</h2></div>
        {entry.exampleSentence && <p className="vocabulary-detail__example">{entry.exampleSentence}</p>}
      </div>
    </header>
    {error && <div className="study-notice study-notice--error" role="alert">{error}</div>}
    {!detail && !error ? <div className="study-empty" role="status"><strong>{zh ? "正在整理词义…" : "Gathering word details…"}</strong></div> : detail && <>
      <section className="detail-section" aria-labelledby="morpheme-title">
        <div className="detail-section__heading"><div><p className="eyebrow">{zh ? "构词" : "Word parts"}</p><h3 id="morpheme-title">{zh ? "组成词根" : "Composing roots"}</h3></div><span>{detail.morphemes.length}</span></div>
        {detail.morphemes.length === 0 ? <p className="detail-section__empty">{zh ? "暂无已验证的构词结构。" : "No verified word structure yet."}</p> : <div className="morpheme-list">{detail.morphemes.map((morpheme) => <button key={morpheme.id} data-relation-focus={`morpheme:${morpheme.id}`} className="morpheme-chip" type="button" aria-label={morpheme.accessibleLabel} onClick={() => onOpenRelated({ kind: "morpheme", morphemeId: morpheme.id }, `morpheme:${morpheme.id}`)}><strong>{morpheme.display}</strong><span>{zh ? `${morpheme.textbookWordCount} 个相似词` : `${morpheme.textbookWordCount} similar ${morpheme.textbookWordCount === 1 ? "word" : "words"}`}</span></button>)}</div>}
      </section>
      <section className="detail-section" aria-labelledby="sense-title">
        <div className="detail-section__heading"><div><p className="eyebrow">{zh ? "释义" : "Translations"}</p><h3 id="sense-title">{zh ? "所有译义" : "All meanings"}</h3></div><span>{detail.senses.length}</span></div>
        {detail.meaningRefresh.status === "available" && <button className="button button--secondary detail-refresh" type="button" disabled={refreshing} onClick={() => { setRefreshing(true); setError(undefined); void api.refreshVocabularyMeanings(entry.id).then(setDetail).catch(() => setError(zh ? "释义刷新失败，已有内容已保留。" : "Meanings could not be refreshed; saved content was preserved.")).finally(() => setRefreshing(false)); }}>{refreshing ? (zh ? "正在刷新…" : "Refreshing…") : (zh ? "刷新释义" : "Refresh meanings")}</button>}
        <div className="detail-sense-list">{detail.senses.map((sense) => <button key={sense.id} data-relation-focus={`translation:${sense.id}`} className="detail-sense" type="button" aria-label={zh ? `显示与${sense.text}译义相近的 ${sense.textbookWordCount} 个词` : `Show ${sense.textbookWordCount} textbook words with the translation ${sense.text}${sense.partOfSpeech ? `, ${sense.partOfSpeech}` : ""}`} onClick={() => onOpenRelated({ kind: "translation", vocabularySenseId: sense.id }, `translation:${sense.id}`)}><span className="detail-sense__copy"><strong className="lexical-text">{sense.text}</strong><PartOfSpeechBadge value={sense.partOfSpeech} /></span><span className="detail-sense__count">{zh ? `${sense.textbookWordCount} 个相近译义` : `${sense.textbookWordCount} with similar meaning`} <span aria-hidden="true">→</span></span></button>)}</div>
        {detail.meaningRefresh.status !== "available" && <p className="detail-section__note">{detail.meaningRefresh.status === "failed-retryable" ? (zh ? "刷新未完成，已有释义保持不变。" : "Refresh did not complete; saved meanings are unchanged.") : (zh ? "这个词的释义来自本地记录；当前语言方向暂不支持在线补全。" : "These meanings come from the local record; online enrichment is not available for this language direction.")}</p>}
      </section>
    </>}
  </section>;
}
