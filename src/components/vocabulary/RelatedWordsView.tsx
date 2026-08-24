import { useEffect, useRef, useState } from "react";

import type { RelatedWord, VocabularyEntry, VocabularyProvenance } from "../../contracts/ipc";
import type { StudyApi } from "./VocabularyWindow";

interface RelatedWordsViewProps {
  anchor?: VocabularyEntry;
  api: StudyApi;
  revision: number;
}

export function RelatedWordsView({ anchor, api, revision }: RelatedWordsViewProps) {
  const [items, setItems] = useState<RelatedWord[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string>();
  const [adding, setAdding] = useState<number>();
  const [provenance, setProvenance] = useState<VocabularyProvenance[]>([]);
  const relatedRequest = useRef(0);

  useEffect(() => {
    let current = true;
    setProvenance([]);
    if (!anchor) return () => { current = false; };
    void api.listVocabularyProvenance(anchor.id)
      .then((items) => { if (current) setProvenance(items); })
      .catch(() => undefined);
    return () => { current = false; };
  }, [anchor, api]);

  useEffect(() => {
    const request = ++relatedRequest.current;
    if (!anchor) {
      setItems([]);
      return;
    }
    setLoading(true);
    setError(undefined);
    void api.listRelated(anchor.id)
      .then((next) => { if (request === relatedRequest.current) setItems(next); })
      .catch(() => { if (request === relatedRequest.current) setError("Related words could not be loaded."); })
      .finally(() => { if (request === relatedRequest.current) setLoading(false); });
  }, [anchor, api, revision]);

  return <section aria-labelledby="related-title">
    <header className="study-header"><div><p className="eyebrow">Related words</p><h2 id="related-title">Connections for {anchor?.sourceText ?? "your next word"}</h2><p>Connections combine your wordbook and every compatible downloaded textbook.</p></div></header>
    {provenance.length > 0 && <aside className="word-provenance" aria-label="Textbook provenance">
      <p className="eyebrow">Added from a textbook</p>
      {provenance.map((item) => <div key={`${item.textbookId}-${item.sourceText}-${item.translatedText}`}>
        <strong>{item.textbookTitle}</strong>
        <span>Version {item.textbookVersion} · {item.license}</span>
        <span>{item.attribution}</span>
        <a href={item.sourceUrl} target="_blank" rel="noreferrer">View source</a>
      </div>)}
    </aside>}
    {error && <div className="study-notice study-notice--error" role="alert">{error}</div>}
    {!anchor ? <div className="study-empty"><strong>Choose a word first.</strong><span>Open a card in My wordbook to make it the connection anchor.</span></div> : loading ? <div className="study-empty" role="status"><strong>Tracing connections…</strong></div> : items.length === 0 ? <div className="study-empty"><strong>No compatible connections yet.</strong><span>Try another word as your local collection grows.</span></div> : <div className="relation-list">{items.map((item) => <article key={`${item.kind}-${item.vocabularyEntryId ?? item.textbookEntryId}-${item.reason}`}>
      <span className={`relation-badge relation-badge--${item.reason}`}>{item.reason === "root" ? "shared root" : "shared meaning"}</span><strong>{item.sourceText}</strong><span>{item.translatedText}</span>
      <span className="relation-tail"><span className="relation-origins">{item.origins.map((origin) => <span className="relation-origin" key={`${origin.kind}-${origin.textbookId ?? "personal"}`}>{origin.kind === "personal" ? "Personal" : origin.textbookTitle}</span>)}</span>
      {item.kind === "textbook" && <button className="button button--secondary relation-add" type="button" disabled={adding === item.textbookEntryId} onClick={() => {
        if (item.promoted || !item.textbookEntryId) return;
        setAdding(item.textbookEntryId);
        setError(undefined);
        void api.addTextbookEntry(item.textbookEntryId).then(() => {
          setItems((current) => current.map((candidate) => candidate.textbookEntryId === item.textbookEntryId ? { ...candidate, promoted: true } : candidate));
          api.refreshPersonal();
        }).catch(() => setError("This related word could not be added.")).finally(() => setAdding(undefined));
      }}>{item.promoted ? "Added" : adding === item.textbookEntryId ? "Adding…" : "Add"}</button>}</span>
    </article>)}</div>}
  </section>;
}
