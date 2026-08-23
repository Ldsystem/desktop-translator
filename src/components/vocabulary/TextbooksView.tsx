import { useEffect, useMemo, useRef, useState } from "react";

import type {
  InstalledTextbook,
  TextbookCatalogItem,
  TextbookEntry,
} from "../../contracts/ipc";
import type { StudyApi } from "./VocabularyWindow";

type ShelfTab = "discover" | "downloaded";
const PAGE_SIZE = 40;

interface TextbooksViewProps {
  api: StudyApi;
}

function message(error: unknown, fallback: string) {
  if (typeof error === "object" && error !== null && "message" in error) {
    return String(error.message);
  }
  return fallback;
}

export function TextbooksView({ api }: TextbooksViewProps) {
  const [tab, setTab] = useState<ShelfTab>("discover");
  const [catalog, setCatalog] = useState<TextbookCatalogItem[]>([]);
  const [downloaded, setDownloaded] = useState<InstalledTextbook[]>([]);
  const [catalogLoading, setCatalogLoading] = useState(true);
  const [catalogError, setCatalogError] = useState<string>();
  const [downloadedLoading, setDownloadedLoading] = useState(true);
  const [downloadedError, setDownloadedError] = useState<string>();
  const [busyBook, setBusyBook] = useState<string>();
  const [confirmRemove, setConfirmRemove] = useState<string>();
  const [openBook, setOpenBook] = useState<InstalledTextbook>();
  const [entries, setEntries] = useState<TextbookEntry[]>([]);
  const [entryTotal, setEntryTotal] = useState(0);
  const [entryOffset, setEntryOffset] = useState(0);
  const [entrySearch, setEntrySearch] = useState("");
  const [entryLoading, setEntryLoading] = useState(false);
  const [entryError, setEntryError] = useState<string>();
  const [added, setAdded] = useState<Set<number>>(new Set());
  const [adding, setAdding] = useState<number>();
  const searchInput = useRef<HTMLInputElement>(null);
  const entryRequest = useRef(0);

  const loadShelf = () => {
    setCatalogLoading(true);
    setCatalogError(undefined);
    void api.listCatalog()
      .then(setCatalog)
      .catch((error) => setCatalogError(message(error, "The textbook catalog could not be opened.")))
      .finally(() => setCatalogLoading(false));
    setDownloadedLoading(true);
    setDownloadedError(undefined);
    void api.listDownloaded()
      .then(setDownloaded)
      .catch((error) => setDownloadedError(message(error, "Downloaded textbooks could not be opened.")))
      .finally(() => setDownloadedLoading(false));
  };

  useEffect(loadShelf, [api]);

  const installedById = useMemo(
    () => new Map(downloaded.map((book) => [book.id, book])),
    [downloaded],
  );

  const loadEntries = (book: InstalledTextbook, search = entrySearch, offset = 0) => {
    const request = ++entryRequest.current;
    setOpenBook(book);
    setEntryLoading(true);
    setEntryError(undefined);
    void api.listTextbookEntries(book.id, search, offset, PAGE_SIZE)
      .then((page) => {
        if (request !== entryRequest.current) return;
        setEntries(page.entries);
        setEntryTotal(page.total);
        setEntryOffset(page.offset);
      })
      .catch((error) => { if (request === entryRequest.current) setEntryError(message(error, "This textbook could not be browsed.")); })
      .finally(() => { if (request === entryRequest.current) setEntryLoading(false); });
  };

  const updateShelf = async (operation: () => Promise<unknown>, bookId: string) => {
    setBusyBook(bookId);
    setDownloadedError(undefined);
    try {
      await operation();
      const next = await api.listDownloaded();
      setDownloaded(next);
      if (openBook) setOpenBook(next.find((book) => book.id === openBook.id));
      return true;
    } catch (error) {
      setDownloadedError(message(error, "The textbook change could not be saved."));
      return false;
    } finally {
      setBusyBook(undefined);
    }
  };

  const install = async (item: TextbookCatalogItem) => {
    setBusyBook(item.id);
    setCatalogError(undefined);
    try {
      await api.downloadTextbook(item.id);
      setDownloaded(await api.listDownloaded());
    } catch (error) {
      setCatalogError(message(error, "This textbook could not be downloaded."));
    } finally {
      setBusyBook(undefined);
    }
  };

  if (openBook) {
    const pageStart = entryTotal === 0 ? 0 : entryOffset + 1;
    const pageEnd = Math.min(entryTotal, entryOffset + entries.length);
    return (
      <section className="textbook-detail" aria-labelledby="textbook-detail-title">
        <header className="study-header textbook-detail__header">
          <div>
            <button className="text-button textbook-back" type="button" onClick={() => setOpenBook(undefined)}>← Back to shelf</button>
            <p className="eyebrow">Downloaded textbook</p>
            <h2 id="textbook-detail-title">{openBook.title}</h2>
            <p>{openBook.sourceLanguage.toUpperCase()} → {openBook.targetLanguage.toUpperCase()} · {openBook.entryCount.toLocaleString()} words · {openBook.license}</p>
          </div>
          <form className="study-search" onSubmit={(event) => { event.preventDefault(); loadEntries(openBook, entrySearch, 0); }}>
            <label>
              <span className="sr-only">Search this textbook</span>
              <input ref={searchInput} value={entrySearch} type="search" placeholder="Find a word in this textbook" onChange={(event) => setEntrySearch(event.target.value)} />
            </label>
          </form>
        </header>
        {entryError && <div className="study-notice study-notice--error" role="alert">{entryError} <button className="text-button" type="button" onClick={() => loadEntries(openBook, entrySearch, entryOffset)}>Try again</button></div>}
        {entryLoading ? (
          <div className="study-empty" role="status"><strong>Opening this textbook…</strong><span>Reading its local index.</span></div>
        ) : entries.length === 0 ? (
          <div className="study-empty"><strong>No matching words.</strong><span>Try a shorter spelling or clear the search.</span></div>
        ) : (
          <>
            <div className="textbook-entry-list">
              {entries.map((entry) => {
                const isAdded = added.has(entry.id);
                return (
                  <article className="textbook-entry" key={entry.id}>
                    <div><strong>{entry.sourceText}</strong>{entry.phoneticSymbols && <small>{entry.phoneticSymbols}</small>}</div>
                    <span>{entry.translatedText}</span>
                    <button className="button button--secondary" type="button" aria-disabled={isAdded} disabled={adding === entry.id} onClick={() => {
                      if (isAdded) return;
                      setAdding(entry.id);
                      setEntryError(undefined);
                      void api.addTextbookEntry(entry.id).then(() => {
                        setAdded((current) => new Set(current).add(entry.id));
                        api.refreshPersonal();
                      }).catch((error) => setEntryError(message(error, "This word could not be added."))).finally(() => setAdding(undefined));
                    }}>{isAdded ? "Added" : adding === entry.id ? "Adding…" : "Add to my wordbook"}</button>
                  </article>
                );
              })}
            </div>
            <div className="textbook-pagination" aria-label="Textbook pages">
              <span>{pageStart.toLocaleString()}–{pageEnd.toLocaleString()} of {entryTotal.toLocaleString()}</span>
              <div>
                <button className="button button--secondary" type="button" disabled={entryOffset === 0} onClick={() => loadEntries(openBook, entrySearch, Math.max(0, entryOffset - PAGE_SIZE))}>Previous</button>
                <button className="button button--secondary" type="button" disabled={entryOffset + entries.length >= entryTotal} onClick={() => loadEntries(openBook, entrySearch, entryOffset + PAGE_SIZE)}>Next</button>
              </div>
            </div>
          </>
        )}
      </section>
    );
  }

  return (
    <section aria-labelledby="textbook-shelf-title">
      <header className="study-header textbook-shelf__header">
        <div><p className="eyebrow">Textbooks</p><h2 id="textbook-shelf-title">Textbook shelf</h2><p>Bring a curated dictionary into your local study desk, then choose one active source.</p></div>
        <div className="shelf-tabs" aria-label="Textbook shelf sections">
          <button type="button" aria-pressed={tab === "discover"} className={tab === "discover" ? "is-active" : ""} onClick={() => setTab("discover")}>Discover</button>
          <button type="button" aria-pressed={tab === "downloaded"} className={tab === "downloaded" ? "is-active" : ""} onClick={() => setTab("downloaded")}>Downloaded</button>
        </div>
      </header>

      {tab === "discover" ? (
        <div className="textbook-surface">
          {catalogError && <div className="study-notice study-notice--error" role="alert">{catalogError} <button className="text-button" type="button" onClick={loadShelf}>Try again</button></div>}
          {catalogLoading ? <div className="study-empty" role="status"><strong>Opening the catalog…</strong></div> : catalog.length === 0 ? <div className="study-empty"><strong>No textbooks are available right now.</strong><span>Your personal wordbook and practice remain available.</span></div> : (
            <div className="textbook-grid">{catalog.map((item) => {
              const installed = installedById.get(item.id);
              const update = installed && installed.version !== item.version;
              return <article className="textbook-volume" key={item.id}>
                <span className="textbook-volume__spine" aria-hidden="true">{item.sourceLanguage.toUpperCase()}<br />{item.targetLanguage.toUpperCase()}</span>
                <div className="textbook-volume__copy"><p className="eyebrow">{item.version}</p><h3>{item.title}</h3><p>{item.attribution}</p><small>{item.license}</small></div>
                <button className="button button--primary" type="button" disabled={busyBook === item.id || Boolean(installed && !update)} onClick={() => install(item)}>{busyBook === item.id ? "Downloading…" : update ? "Update" : installed ? "Downloaded" : "Download"}</button>
              </article>;
            })}</div>
          )}
        </div>
      ) : (
        <div className="textbook-surface">
          {downloadedError && <div className="study-notice study-notice--error" role="alert">{downloadedError}</div>}
          {downloadedLoading ? <div className="study-empty" role="status"><strong>Reading your shelf…</strong></div> : downloaded.length === 0 ? <div className="study-empty"><strong>No downloaded textbooks yet.</strong><span>Open Discover to add a curated local reference.</span><button className="button button--secondary study-empty__action" type="button" onClick={() => setTab("discover")}>Browse Discover</button></div> : (
            <div className="downloaded-list">{downloaded.map((book) => <article className={book.active ? "downloaded-book is-active" : "downloaded-book"} key={book.id}>
              <div className="downloaded-book__identity"><span aria-hidden="true">Aa</span><div><small>{book.active ? "Active textbook" : "Downloaded"}</small><h3>{book.title}</h3><p>{book.sourceLanguage.toUpperCase()} → {book.targetLanguage.toUpperCase()} · {book.entryCount.toLocaleString()} words</p></div></div>
              <div className="downloaded-book__actions">
                <button className="button button--secondary" type="button" onClick={() => { setEntrySearch(""); loadEntries(book, "", 0); }}>Browse words</button>
                <button className="button button--secondary" type="button" disabled={busyBook === book.id} onClick={() => updateShelf(() => api.setActiveTextbook(book.active ? undefined : book.id), book.id)}>{book.active ? "Deactivate" : "Make active"}</button>
                {confirmRemove === book.id ? <span className="inline-confirm"><span>Remove local copy?</span><button className="text-button is-danger" type="button" onClick={() => updateShelf(() => api.removeTextbook(book.id), book.id).then((removed) => { if (removed) setConfirmRemove(undefined); })}>Remove</button><button className="text-button" type="button" onClick={() => setConfirmRemove(undefined)}>Cancel</button></span> : <button className="text-button is-danger" type="button" onClick={() => setConfirmRemove(book.id)}>Remove</button>}
              </div>
            </article>)}</div>
          )}
        </div>
      )}
    </section>
  );
}
