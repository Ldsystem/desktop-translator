import { useEffect, useMemo, useRef, useState } from "react";

import type {
  InstalledTextbook,
  TextbookCatalogItem,
  TextbookEntry,
  UiLocale,
} from "../../contracts/ipc";
import type { StudyApi } from "./VocabularyWindow";
import { PartOfSpeechBadge } from "./PracticeView";

type ShelfTab = "discover" | "downloaded";
const PAGE_SIZE = 40;

const catalogPresentation: Record<string, { description: string; descriptionZh: string; scope: string; scopeZh: string; count: number }> = {
  "wikdict-en-zh-2026-06": { description: "A broad English reference for lookup, discovery, and uncommon words.", descriptionZh: "适合查询、拓展和学习非常用词的综合英语词典。", scope: "General reference", scopeZh: "综合参考", count: 30_518 },
  "ngsl-en-zh-1-2": { description: "High-frequency vocabulary for daily reading and conversation.", descriptionZh: "覆盖日常阅读和交流中的高频词汇。", scope: "Everyday · NGSL", scopeZh: "日常 · NGSL", count: 2_809 },
  "nawl-en-zh-1-2": { description: "Vocabulary with high coverage across general academic texts.", descriptionZh: "覆盖一般学术文本中的常用词汇。", scope: "Academic · NAWL", scopeZh: "学术 · NAWL", count: 957 },
  "tsl-en-zh-1-2": { description: "A focused service list for TOEIC listening and reading preparation.", descriptionZh: "面向 TOEIC 听力和阅读备考的精简词表。", scope: "TOEIC · TSL", scopeZh: "TOEIC · TSL", count: 1_250 },
  "bsl-en-zh-1-20": { description: "High-frequency vocabulary for workplace and business communication.", descriptionZh: "覆盖职场和商务交流中的高频词汇。", scope: "Business · BSL", scopeZh: "商务 · BSL", count: 1_744 },
};

function presentation(item: TextbookCatalogItem, zh: boolean) {
  const curated = catalogPresentation[item.id];
  return {
    description: zh ? curated?.descriptionZh ?? item.description ?? "精选英语词汇参考。" : item.description ?? curated?.description ?? "A curated English vocabulary reference.",
    scope: zh ? curated?.scopeZh ?? item.scope ?? "精选词汇" : item.scope ?? curated?.scope ?? "Curated vocabulary",
    count: item.estimatedEntryCount ?? curated?.count,
    script: zh ? "简体中文" : item.script ?? "Simplified Chinese",
  };
}

interface TextbooksViewProps {
  api: StudyApi;
  locale?: UiLocale;
}

function message(error: unknown, fallback: string) {
  if (typeof error === "object" && error !== null && "message" in error) {
    return String(error.message);
  }
  return fallback;
}

export function TextbooksView({ api, locale = "en" }: TextbooksViewProps) {
  const zh = locale === "zh-CN";
  const tr = (english: string, chinese: string) => zh ? chinese : english;
  const refreshUnavailableMessage = tr("This book remains usable. Its verified source package is unavailable for refresh.", "此词书仍可使用，但缺少可验证的原始安装包，暂不能刷新。");
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
    setDownloadedError(undefined);
    if (openBook?.id === item.id) setEntryError(undefined);
    try {
      await api.downloadTextbook(item.id);
      const next = await api.listDownloaded();
      setDownloaded(next);
      if (openBook?.id === item.id) {
        const refreshed = next.find((book) => book.id === item.id);
        setOpenBook(refreshed);
        if (refreshed) loadEntries(refreshed, entrySearch, entryOffset);
      }
    } catch (error) {
      const failure = message(error, "This textbook could not be downloaded.");
      setCatalogError(failure);
      setDownloadedError(failure);
      if (openBook?.id === item.id) setEntryError(failure);
    } finally {
      setBusyBook(undefined);
    }
  };

  if (openBook) {
    const refreshUnavailable = openBook.lexicalRefreshStatus === "unavailable-legacy";
    const refreshItem = !refreshUnavailable && (openBook.metadataRefreshAvailable || openBook.lexicalRefreshStatus === "exact")
      ? catalog.find((item) => item.id === openBook.id)
      : undefined;
    const pageStart = entryTotal === 0 ? 0 : entryOffset + 1;
    const pageEnd = Math.min(entryTotal, entryOffset + entries.length);
    return (
      <section className="textbook-detail" aria-labelledby="textbook-detail-title">
        <header className="study-header textbook-detail__header">
          <div>
            <button className="text-button textbook-back" type="button" onClick={() => setOpenBook(undefined)}>← {tr("Back to shelf", "返回词书架")}</button>
            <p className="eyebrow">{tr("Downloaded textbook", "已下载词书")}</p>
            <h2 id="textbook-detail-title">{openBook.title}</h2>
            <p>{openBook.sourceLanguage.toUpperCase()} → {openBook.targetLanguage.toUpperCase()} · {openBook.entryCount.toLocaleString()} words · {openBook.license}</p>
          </div>
          <form className="study-search" onSubmit={(event) => { event.preventDefault(); loadEntries(openBook, entrySearch, 0); }}>
            <label>
              <span className="sr-only">{tr("Search this textbook", "搜索此词书")}</span>
              <input ref={searchInput} value={entrySearch} type="search" placeholder={tr("Find a word in this textbook", "在此词书中查找单词")} onChange={(event) => setEntrySearch(event.target.value)} />
            </label>
          </form>
        </header>
        {refreshItem && <div className="study-notice" role="status"><span>{tr("This download can be refreshed with richer meanings and parts of speech.", "此词书可刷新，以补充更多释义和词性。")}</span><button className="button button--secondary" type="button" disabled={busyBook === openBook.id} onClick={() => install(refreshItem)}>{busyBook === openBook.id ? tr("Refreshing…", "更新中…") : tr("Refresh word details", "刷新词汇详情")}</button></div>}
        {refreshUnavailable && <div className="study-notice" role="status"><span>{refreshUnavailableMessage}</span><button className="button button--secondary" type="button" disabled>{tr("Refresh word details", "刷新词汇详情")}</button></div>}
        {entryError && <div className="study-notice study-notice--error" role="alert">{entryError} <button className="text-button" type="button" onClick={() => loadEntries(openBook, entrySearch, entryOffset)}>{tr("Try again", "重试")}</button></div>}
        {entryLoading ? (
          <div className="study-empty" role="status"><strong>{tr("Opening this textbook…", "正在打开词书…")}</strong><span>{tr("Reading its local index.", "正在读取本地索引。")}</span></div>
        ) : entries.length === 0 ? (
          <div className="study-empty"><strong>{tr("No matching words.", "没有匹配的词汇。")}</strong><span>{tr("Try a shorter spelling or clear the search.", "可缩短拼写或清空搜索条件。")}</span></div>
        ) : (
          <>
            <div className="textbook-entry-list">
              {entries.map((entry) => {
                const isAdded = added.has(entry.id);
                return (
                  <article className="textbook-entry" key={entry.id}>
                    <div><span className="textbook-entry__lexeme"><strong className="lexical-text">{entry.sourceText}</strong><PartOfSpeechBadge value={entry.partOfSpeech} /></span>{entry.phoneticSymbols && <small>{entry.phoneticSymbols}</small>}</div>
                    <span className="lexical-text">{entry.translatedText}</span>
                    <button className="button button--secondary" type="button" aria-disabled={isAdded} disabled={adding === entry.id} onClick={() => {
                      if (isAdded) return;
                      setAdding(entry.id);
                      setEntryError(undefined);
                      void api.addTextbookEntry(entry.id).then(() => {
                        setAdded((current) => new Set(current).add(entry.id));
                        api.refreshPersonal();
                      }).catch((error) => setEntryError(message(error, "This word could not be added."))).finally(() => setAdding(undefined));
                    }}>{isAdded ? tr("Added", "已添加") : adding === entry.id ? tr("Adding…", "添加中…") : tr("Add to my wordbook", "添加到我的词汇本")}</button>
                  </article>
                );
              })}
            </div>
            <div className="textbook-pagination" aria-label="Textbook pages">
              <span>{pageStart.toLocaleString()}–{pageEnd.toLocaleString()} of {entryTotal.toLocaleString()}</span>
              <div>
                <button className="button button--secondary" type="button" disabled={entryOffset === 0} onClick={() => loadEntries(openBook, entrySearch, Math.max(0, entryOffset - PAGE_SIZE))}>{tr("Previous", "上一页")}</button>
                <button className="button button--secondary" type="button" disabled={entryOffset + entries.length >= entryTotal} onClick={() => loadEntries(openBook, entrySearch, entryOffset + PAGE_SIZE)}>{tr("Next", "下一页")}</button>
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
        <div><p className="eyebrow">{tr("Textbooks", "词书")}</p><h2 id="textbook-shelf-title">{tr("Textbook shelf", "词书架")}</h2><p>{tr("Choose a learning path with clear Simplified Chinese meanings, then make one book active.", "选择带有清晰简体中文释义的学习路径，并将其中一本设为当前词书。")}</p></div>
        <div className="shelf-tabs" aria-label="Textbook shelf sections">
          <button type="button" aria-pressed={tab === "discover"} className={tab === "discover" ? "is-active" : ""} onClick={() => setTab("discover")}>{tr("Discover", "发现")}</button>
          <button type="button" aria-pressed={tab === "downloaded"} className={tab === "downloaded" ? "is-active" : ""} onClick={() => setTab("downloaded")}>{tr("Downloaded", "已下载")}</button>
        </div>
      </header>

      {tab === "discover" ? (
        <div className="textbook-surface">
          {catalogError && <div className="study-notice study-notice--error" role="alert">{catalogError} <button className="text-button" type="button" onClick={loadShelf}>Try again</button></div>}
          {catalogLoading ? <div className="study-empty" role="status"><strong>Opening the catalog…</strong></div> : catalog.length === 0 ? <div className="study-empty"><strong>No textbooks are available right now.</strong><span>Your personal wordbook and practice remain available.</span></div> : (
            <div className="textbook-grid">{catalog.map((item) => {
              const installed = installedById.get(item.id);
              const update = installed && installed.version !== item.version;
              const refreshUnavailable = installed?.lexicalRefreshStatus === "unavailable-legacy";
              const metadataRefresh = !refreshUnavailable && (installed?.metadataRefreshAvailable === true || installed?.lexicalRefreshStatus === "exact");
              const details = presentation(item, zh);
              return <article className="textbook-volume" key={item.id}>
                <span className="textbook-volume__spine" aria-hidden="true">{details.scope}</span>
                <div className="textbook-volume__copy">
                  <p className="eyebrow">{item.sourceLanguage.toUpperCase()} → 简体中文</p>
                  <h3>{item.title}</h3>
                  <p className="textbook-volume__description">{details.description}</p>
                  <div className="textbook-volume__facts" aria-label="Textbook details">
                    <span>{details.scope}</span>
                    {details.count && <span>{details.count.toLocaleString()} {tr("words", "词")}</span>}
                    <span>{details.script}</span>
                  </div>
                  <p className="textbook-volume__credit">{item.attribution}</p>
                  <div className="textbook-volume__source"><small>{item.license} · {item.version}</small><a href={item.sourceUrl} target="_blank" rel="noreferrer">{tr("View source", "查看来源")}</a></div>
                  {refreshUnavailable && <p className="textbook-volume__description">{refreshUnavailableMessage}</p>}
                </div>
                <button className="button button--primary" type="button" disabled={refreshUnavailable || busyBook === item.id || Boolean(installed && !update && !metadataRefresh)} onClick={() => install(item)}>{busyBook === item.id ? tr("Downloading…", "下载中…") : metadataRefresh || refreshUnavailable ? tr("Refresh word details", "刷新词汇详情") : update ? tr("Update", "更新") : installed ? tr("Downloaded", "已下载") : tr("Download", "下载")}</button>
              </article>;
            })}</div>
          )}
        </div>
      ) : (
        <div className="textbook-surface">
          {downloadedError && <div className="study-notice study-notice--error" role="alert">{downloadedError}</div>}
          {downloadedLoading ? <div className="study-empty" role="status"><strong>{tr("Reading your shelf…", "正在读取词书架…")}</strong></div> : downloaded.length === 0 ? <div className="study-empty"><strong>{tr("No downloaded textbooks yet.", "尚未下载词书。")}</strong><span>{tr("Open Discover to add a curated local reference.", "前往“发现”添加精选本地词书。")}</span><button className="button button--secondary study-empty__action" type="button" onClick={() => setTab("discover")}>{tr("Browse Discover", "浏览发现")}</button></div> : (
            <div className="downloaded-list">{downloaded.map((book) => {
              const refreshUnavailable = book.lexicalRefreshStatus === "unavailable-legacy";
              const refreshItem = !refreshUnavailable && (book.metadataRefreshAvailable || book.lexicalRefreshStatus === "exact") ? catalog.find((item) => item.id === book.id) : undefined;
              return <article className={book.active ? "downloaded-book is-active" : "downloaded-book"} key={book.id} aria-label={book.title}>
              <div className="downloaded-book__identity"><span aria-hidden="true">Aa</span><div><small>{book.active ? tr("Active textbook", "当前词书") : tr("Downloaded", "已下载")}</small><h3>{book.title}</h3><p>{book.sourceLanguage.toUpperCase()} → {book.targetLanguage.toUpperCase()} · {book.entryCount.toLocaleString()} {tr("words", "词")}</p></div></div>
              <div className="downloaded-book__footer">
              <div className="downloaded-book__actions" role="group" aria-label={tr(`Study ${book.title}`, `学习《${book.title}》`)}>
                <button className="button button--secondary" type="button" onClick={() => { setEntrySearch(""); loadEntries(book, "", 0); }}>{tr("Browse words", "浏览词汇")}</button>
                <button className="button button--secondary" type="button" disabled={busyBook === book.id} onClick={() => updateShelf(() => api.setActiveTextbook(book.active ? undefined : book.id), book.id)}>{book.active ? tr("Deactivate", "停用") : tr("Make active", "设为当前")}</button>
              </div>
              <div className="downloaded-book__remove">
                {confirmRemove === book.id ? <span className="inline-confirm"><span>{tr("Remove local copy?", "移除本地副本？")}</span><button className="text-button is-danger" type="button" onClick={() => updateShelf(() => api.removeTextbook(book.id), book.id).then((removed) => { if (removed) setConfirmRemove(undefined); })}>{tr("Remove", "移除")}</button><button className="text-button" type="button" onClick={() => setConfirmRemove(undefined)}>{tr("Cancel", "取消")}</button></span> : <button className="text-button is-danger" type="button" onClick={() => setConfirmRemove(book.id)}>{tr("Remove", "移除")}</button>}
              </div>
              </div>
              {(refreshItem || refreshUnavailable) && <div className="downloaded-book__maintenance"><span>{refreshUnavailable ? refreshUnavailableMessage : tr("Richer meanings and parts of speech available", "可补充更多释义和词性")}</span><button className="text-button" type="button" disabled={refreshUnavailable || busyBook === book.id} onClick={() => { if (refreshItem) void install(refreshItem); }}>{busyBook === book.id ? tr("Refreshing…", "更新中…") : tr("Refresh word details", "刷新词汇详情")}</button></div>}
            </article>})}</div>
          )}
        </div>
      )}
    </section>
  );
}
