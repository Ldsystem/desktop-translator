import { useEffect, useState } from "react";

import type { TranslationResult, UiLocale } from "../contracts/ipc";
import { PartOfSpeechBadge } from "./vocabulary/PracticeView";

export function TranslationSenses({ result, locale = "en" }: { result: TranslationResult; locale?: UiLocale }) {
  const [showAll, setShowAll] = useState(false);
  useEffect(() => setShowAll(false), [result.selectionId, result.translatedText]);
  const other = (result.senses ?? []).filter((sense) => !sense.isPrimary);
  if (other.length === 0) return null;
  const zh = locale === "zh-CN";
  const visible = other.length > 4 && !showAll ? other.slice(0, 3) : other;
  const headingId = `translation-senses-${result.selectionId}`;

  return <section className="translation-senses" aria-labelledby={headingId}>
    <div className="translation-senses__heading" id={headingId}>{zh ? `其他翻译（${other.length}）` : `Other translations (${other.length})`}</div>
    <div className="translation-senses__list">{visible.map((sense) => <div className="translation-senses__item" key={`${sense.rank}-${sense.text}-${sense.partOfSpeech ?? "unknown"}`}><PartOfSpeechBadge value={sense.partOfSpeech} /><span>{sense.text}</span></div>)}</div>
    {other.length > 4 && <button className="text-button translation-senses__toggle" type="button" aria-expanded={showAll} onClick={() => setShowAll((current) => !current)}>{showAll ? (zh ? "收起" : "Show less") : (zh ? "显示全部" : "Show all")}</button>}
  </section>;
}
