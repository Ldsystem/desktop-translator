import { useEffect, useRef, useState } from "react";

import type { InstalledTextbook, RelatedSource, RelatedWord, VocabularyEntry, VocabularyProvenance } from "../../contracts/ipc";
import type { StudyApi } from "./VocabularyWindow";

interface RelatedWordsViewProps {
  anchor?: VocabularyEntry;
  api: StudyApi;
}

export function RelatedWordsView({ anchor, api }: RelatedWordsViewProps) {
  const [activeBook, setActiveBook] = useState<InstalledTextbook>();
  const [sourceKind, setSourceKind] = useState<"personal" | "textbook">("personal");
  const [items, setItems] = useState<RelatedWord[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string>();
  const [adding, setAdding] = useState<number>();
  const [provenance, setProvenance] = useState<VocabularyProvenance[]>([]);
  const [provenanceError, setProvenanceError] = useState<string>();
  const relatedRequest = useRef(0);

  useEffect(() => {
    void api.listDownloaded().then((books) => setActiveBook(books.find((book) => book.active))).catch(() => {
      setActiveBook(undefined);
      setError("The active textbook could not be checked.");
    });
  }, [api]);

  useEffect(() => {
    let current = true;
    setProvenance([]);
    setProvenanceError(undefined);
    if (!anchor) return () => { current = false; };
    void api.listVocabularyProvenance(anchor.id)
      .then((items) => { if (current) setProvenance(items); })
      .catch(() => { if (current) setProvenanceError("Textbook attribution could not be opened."); });
    return () => { current = false; };
  }, [anchor, api]);

  useEffect(() => {
    const request = ++relatedRequest.current;
    if (!anchor || (sourceKind === "textbook" && !activeBook)) {
      setItems([]);
      return;
    }
    const source: RelatedSource = sourceKind === "personal"
      ? { kind: "personal" }
      : { kind: "textbook", textbookId: activeBook!.id };
    setLoading(true);
    setError(undefined);
    void api.listRelated(anchor.id, source)
      .then((next) => { if (request === relatedRequest.current) setItems(next); })
      .catch(() => { if (request === relatedRequest.current) setError("Related words could not be loaded from this source."); })
      .finally(() => { if (request === relatedRequest.current) setLoading(false); });
  }, [activeBook, anchor, api, sourceKind]);

  return <section aria-labelledby="related-title">
    <header className="study-header"><div><p className="eyebrow">Related words</p><h2 id="related-title">Connections for {anchor?.sourceText ?? "your next word"}</h2><p>Choose whether connections come from your personal wordbook or the active textbook.</p></div></header>
    {provenanceError && <div className="study-notice study-notice--error" role="alert">{provenanceError}</div>}
    {provenance.length > 0 && <aside className="word-provenance" aria-label="Textbook provenance">
      <p className="eyebrow">Added from a textbook</p>
      {provenance.map((item) => <div key={`${item.textbookId}-${item.sourceText}-${item.translatedText}`}>
        <strong>{item.textbookTitle}</strong>
        <span>Version {item.textbookVersion} · {item.license}</span>
        <span>{item.attribution}</span>
        <a href={item.sourceUrl} target="_blank" rel="noreferrer">View source</a>
      </div>)}
    </aside>}
    <fieldset className="source-selector"><legend>Connection source</legend><label><input type="radio" name="related-source" value="personal" checked={sourceKind === "personal"} onChange={() => setSourceKind("personal")} /> My wordbook</label><label className={!activeBook ? "is-disabled" : ""}><input type="radio" name="related-source" value="textbook" checked={sourceKind === "textbook"} disabled={!activeBook} onChange={() => setSourceKind("textbook")} /> Active textbook{activeBook ? ` · ${activeBook.title}` : " · none selected"}</label></fieldset>
    {error && <div className="study-notice study-notice--error" role="alert">{error}</div>}
    {!anchor ? <div className="study-empty"><strong>Choose a word first.</strong><span>Open a card in My wordbook to make it the connection anchor.</span></div> : loading ? <div className="study-empty" role="status"><strong>Tracing connections…</strong></div> : items.length === 0 ? <div className="study-empty"><strong>No connections in this source yet.</strong><span>{sourceKind === "personal" ? "Try the active textbook for a wider local search." : "Try another word or switch to your wordbook."}</span></div> : <div className="relation-list">{items.map((item) => <article key={`${item.kind}-${item.vocabularyEntryId ?? item.textbookEntryId}-${item.reason}`}>
      <span className={`relation-badge relation-badge--${item.reason}`}>{item.reason === "root" ? "shared root" : "shared meaning"}</span><strong>{item.sourceText}</strong><span>{item.translatedText}</span>
      {item.kind === "textbook" && <button className="button button--secondary relation-add" type="button" disabled={adding === item.textbookEntryId} onClick={() => {
        if (item.promoted || !item.textbookEntryId) return;
        setAdding(item.textbookEntryId);
        setError(undefined);
        void api.addTextbookEntry(item.textbookEntryId).then(() => {
          setItems((current) => current.map((candidate) => candidate.textbookEntryId === item.textbookEntryId ? { ...candidate, promoted: true } : candidate));
          api.refreshPersonal();
        }).catch(() => setError("This related word could not be added.")).finally(() => setAdding(undefined));
      }}>{item.promoted ? "Added" : adding === item.textbookEntryId ? "Adding…" : "Add"}</button>}
    </article>)}</div>}
  </section>;
}
