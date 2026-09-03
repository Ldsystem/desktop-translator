import { useEffect, useRef, useState } from "react";

import type { FilteredRelatedWord, RelatedFilter, RelatedWord, UiLocale, VocabularyEntry } from "../../contracts/ipc";
import { PartOfSpeechBadge } from "./PracticeView";
import type { StudyApi } from "./VocabularyWindow";

interface RelatedWordsViewProps {
  anchor?: VocabularyEntry;
  api: StudyApi;
  revision: number;
  locale?: UiLocale;
  filter?: RelatedFilter;
  onBack: () => void;
}

type DisplayRelatedWord = RelatedWord | (FilteredRelatedWord & { kind: "filtered"; reason: "root" | "meaning"; promoted: boolean });

export function RelatedWordsView({ anchor, api, revision, locale = "en", filter, onBack }: RelatedWordsViewProps) {
  const zh = locale === "zh-CN";
  const [items, setItems] = useState<DisplayRelatedWord[]>([]);
  const [relationLabel, setRelationLabel] = useState<string>();
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string>();
  const [loadError, setLoadError] = useState(false);
  const [retry, setRetry] = useState(0);
  const [adding, setAdding] = useState<number>();
  const relatedRequest = useRef(0);

  useEffect(() => {
    const request = ++relatedRequest.current;
    if (!anchor) {
      setItems([]);
      return;
    }
    setRelationLabel(undefined);
    setItems([]);
    setLoading(true);
    setLoadError(false);
    setError(undefined);
    const pending = filter
      ? api.listFilteredRelated(anchor.id, filter, 0, 100).then((page) => {
          if (request === relatedRequest.current) setRelationLabel(page.relationLabel);
          return page.items.map((item): DisplayRelatedWord => ({ ...item, kind: "filtered", reason: filter.kind === "morpheme" ? "root" : "meaning", promoted: item.savedVocabularyEntryId !== undefined }));
        })
      : api.listRelated(anchor.id);
    void pending
      .then((next) => { if (request === relatedRequest.current) setItems(next); })
      .catch(() => { if (request === relatedRequest.current) { setLoadError(true); setError(zh ? "无法加载相关词。" : "Related words could not be loaded."); } })
      .finally(() => { if (request === relatedRequest.current) setLoading(false); });
    return () => { relatedRequest.current += 1; };
  }, [anchor, api, filter, revision, zh, retry]);

  const subtitle = relationLabel ?? anchor?.exampleSentence
    ?? (zh ? "关联来自你的词汇本和已下载词书。" : "Connections use your wordbook and downloaded textbooks.");

  return <section className="related-view" aria-labelledby="related-title">
    <header className="study-header"><div><button className="text-button textbook-back" type="button" onClick={onBack}>← {filter ? (zh ? "返回词汇详情" : "Back to word details") : (zh ? "返回我的词汇本" : "Back to My wordbook")}</button><p className="eyebrow">{zh ? "相关词" : "Related words"}</p><h2 id="related-title">{zh ? `${anchor?.sourceText ?? "下一个词"} 的关联` : `Connections for ${anchor?.sourceText ?? "your next word"}`}</h2><p className={anchor?.exampleSentence ? "related-example is-saved" : "related-example"}>{subtitle}</p></div></header>
    {error && <div className="study-notice study-notice--error" role="alert">{error}{loadError && <button className="text-button" type="button" onClick={() => setRetry((value) => value + 1)}>{zh ? "重试" : "Try again"}</button>}</div>}
    {!anchor ? <div className="study-empty"><strong>{zh ? "请先选择一个词。" : "Choose a word first."}</strong><span>{zh ? "从“我的词汇本”打开一张卡片作为关联词。" : "Open a card in My wordbook to make it the connection anchor."}</span></div> : loading ? <div className="study-empty" role="status"><strong>{zh ? "正在查找关联…" : "Tracing connections…"}</strong></div> : loadError ? null : items.length === 0 ? <div className="study-empty"><strong>{zh ? "暂未找到兼容的关联。" : "No compatible connections yet."}</strong><span>{zh ? "随着本地词汇积累，可以尝试其他词。" : "Try another word as your local collection grows."}</span></div> : <div className="relation-list">{items.map((item) => <article key={item.kind === "filtered" ? item.key : `${item.kind}-${item.vocabularyEntryId ?? ("textbookEntryId" in item ? item.textbookEntryId : "unknown")}-${item.reason}`}>
      <span className={`relation-badge relation-badge--${item.reason}`}>{item.reason === "root" ? (zh ? "同词根" : "shared root") : (zh ? "同义项" : "shared meaning")}</span><span className="relation-lexeme"><strong className="lexical-text">{item.sourceText}</strong><PartOfSpeechBadge value={item.partOfSpeech} /></span><span className="lexical-text">{item.translatedText}</span>
      <span className="relation-tail"><span className="relation-origins">{item.origins.map((origin) => <span className="relation-origin" key={`${origin.kind}-${origin.textbookId ?? "personal"}`}>{origin.kind === "personal" ? (zh ? "个人词汇本" : "Personal") : origin.textbookTitle}</span>)}</span>
      {item.kind === "textbook" && <button className="button button--secondary relation-add" type="button" disabled={adding === item.textbookEntryId} onClick={() => {
        if (item.promoted || !item.textbookEntryId) return;
        setAdding(item.textbookEntryId);
        setError(undefined);
        void api.addTextbookEntry(item.textbookEntryId).then(() => {
          setItems((current) => current.map((candidate) => candidate.kind === "textbook" && candidate.textbookEntryId === item.textbookEntryId ? { ...candidate, promoted: true } : candidate));
          api.refreshPersonal();
        }).catch(() => setError(zh ? "无法添加这个相关词。" : "This related word could not be added.")).finally(() => setAdding(undefined));
      }}>{item.promoted ? (zh ? "已添加" : "Added") : adding === item.textbookEntryId ? (zh ? "正在添加…" : "Adding…") : (zh ? "添加" : "Add")}</button>}</span>
    </article>)}</div>}
  </section>;
}
