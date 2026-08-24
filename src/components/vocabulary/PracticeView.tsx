import { useEffect, useRef, useState } from "react";

import type { PartOfSpeech, PracticeDirection, StudyPracticeOutcome, StudyPracticeQuestion, UiLocale } from "../../contracts/ipc";
import type { StudyApi } from "./VocabularyWindow";

interface PracticeViewProps {
  api: StudyApi;
  revision: number;
  locale?: UiLocale;
}

function directions(zh: boolean): { value: PracticeDirection; label: string; note: string }[] {
  return zh ? [
    { value: "random", label: "双向混合", note: "随机方向" },
    { value: "source-to-target", label: "单词 → 释义", note: "识别" },
    { value: "target-to-source", label: "释义 → 单词", note: "回忆" },
  ] : [
    { value: "random", label: "Mix", note: "Both directions" },
    { value: "source-to-target", label: "Word → meaning", note: "Recognition" },
    { value: "target-to-source", label: "Meaning → word", note: "Production" },
  ];
}

const partOfSpeechLabels: Record<PartOfSpeech, string> = {
  adjective: "adj.", adverb: "adv.", article: "art.", conjunction: "conj.", determiner: "det.",
  interjection: "interj.", noun: "n.", number: "num.", numeral: "num.", particle: "part.",
  phrase: "phr.", postposition: "postp.", prefix: "pref.", preposition: "prep.",
  "prepositional phrase": "prep. phr.", pronoun: "pron.", "proper noun": "prop. n.",
  proverb: "prov.", suffix: "suff.", symbol: "sym.", verb: "v.",
};

export function PartOfSpeechBadge({ value }: { value?: PartOfSpeech }) {
  return value ? <span className="part-of-speech" title={value}>{partOfSpeechLabels[value]}</span> : null;
}

function lexicalTextClass(value: string) {
  return value.length >= 11 ? "lexical-text lexical-text--long" : "lexical-text";
}

export function PracticeView({ api, revision, locale = "en" }: PracticeViewProps) {
  const zh = locale === "zh-CN";
  const [direction, setDirection] = useState<PracticeDirection>("random");
  const [question, setQuestion] = useState<StudyPracticeQuestion | null>();
  const [outcome, setOutcome] = useState<StudyPracticeOutcome>();
  const [selected, setSelected] = useState<string>();
  const [error, setError] = useState<string>();
  const [saving, setSaving] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [failedDirection, setFailedDirection] = useState<PracticeDirection>();
  const nextButton = useRef<HTMLButtonElement>(null);
  const questionRequest = useRef(0);
  const submittingRef = useRef(false);

  const loadQuestion = () => {
    const request = ++questionRequest.current;
    setQuestion(undefined);
    setOutcome(undefined);
    setSelected(undefined);
    setError(undefined);
    void api.getPracticeQuestion().then((next) => {
      if (request === questionRequest.current) setQuestion(next);
    }).catch(() => {
      if (request !== questionRequest.current) return;
      setQuestion(null);
      setError(zh ? "无法生成练习题。" : "A practice question could not be prepared.");
    });
  };

  useEffect(() => {
    let current = true;
    void api.getPracticePreferences().then((preferences) => {
      if (current) setDirection(preferences.direction);
    }).catch(() => setError(zh ? "无法读取练习偏好。" : "Your practice preference could not be opened."));
    loadQuestion();
    return () => { current = false; };
  }, [api]);

  useEffect(() => { if (outcome) nextButton.current?.focus(); }, [outcome]);
  useEffect(() => {
    if (revision > 0 && !submittingRef.current && !outcome) loadQuestion();
  }, [revision]);

  const chooseDirection = (next: PracticeDirection) => {
    if (submittingRef.current) return;
    setSaving(true);
    setError(undefined);
    setFailedDirection(undefined);
    void api.savePracticePreferences({ direction: next }).then(() => {
      setDirection(next);
      loadQuestion();
    }).catch(() => {
      setFailedDirection(next);
      setError(zh ? "无法保存练习方向。" : "Your practice direction could not be saved.");
    }).finally(() => setSaving(false));
  };

  const submit = (questionValue: StudyPracticeQuestion, answer: string) => {
    if (submittingRef.current) return;
    submittingRef.current = true;
    setSubmitting(true);
    setError(undefined);
    void api.submitPracticeAnswer(questionValue.entryId, questionValue.direction, answer)
      .then(setOutcome)
      .catch(() => setError(zh ? "无法保存答案。" : "Your answer could not be saved."))
      .finally(() => {
        submittingRef.current = false;
        setSubmitting(false);
      });
  };

  return <section className="practice-view" aria-labelledby="practice-title">
    <header className="study-header"><div><p className="eyebrow">{zh ? "练习" : "Practice"}</p><h2 id="practice-title">{zh ? "选择答案" : "Choose the answer"}</h2><p>{zh ? "题目来自你的词汇本；相关词和已下载词书会让选项更有挑战性。" : "Questions come from your wordbook. Related words and downloaded textbooks make the choices more challenging."}</p></div></header>
    <fieldset className="direction-selector" disabled={saving || submitting}><legend>{zh ? "练习方向" : "Practice direction"}</legend>{directions(zh).map((item) => <label key={item.value} className={direction === item.value ? "is-active" : ""}><input type="radio" name="practice-direction" value={item.value} checked={direction === item.value} onChange={() => chooseDirection(item.value)} /><span><strong>{item.label}</strong><small>{item.note}</small></span></label>)}</fieldset>
    {error && <div className="study-notice study-notice--error" role="alert">{error} <button className="text-button" type="button" onClick={() => failedDirection ? chooseDirection(failedDirection) : loadQuestion()}>{failedDirection ? (zh ? "重新保存" : "Try saving again") : (zh ? "重试" : "Try again")}</button></div>}
    {question === undefined ? <div className="study-empty" role="status"><strong>{zh ? "正在挑选需要复习的词汇…" : "Choosing what needs attention…"}</strong></div> : question === null ? <div className="study-empty study-empty--complete"><strong>{zh ? "当前词汇都已练习完毕。" : "You have practised every available word."}</strong><span>{zh ? "记忆分数回落后再来，或添加新词继续练习。" : "Come back after recall has faded, or add another word to continue."}</span></div> : <section className="practice-card">
      <div className="practice-prompt"><span>{question.promptLanguage.toUpperCase()} → {question.answerLanguage.toUpperCase()}</span><div className="practice-prompt__lexeme"><strong className={lexicalTextClass(question.prompt)}>{question.prompt}</strong><PartOfSpeechBadge value={question.promptPartOfSpeech} /></div></div>
      <div className="practice-choices" role="radiogroup" aria-label={zh ? "答案选项" : "Answer choices"}>{question.choices.map((choice) => <button key={choice.value} className={selected === choice.value ? "practice-choice is-selected" : "practice-choice"} type="button" role="radio" aria-checked={selected === choice.value} disabled={Boolean(outcome) || submitting} onClick={() => setSelected(choice.value)}><span className={lexicalTextClass(choice.value)}>{choice.value}</span><PartOfSpeechBadge value={choice.partOfSpeech} /></button>)}</div>
      <div className="practice-actions">{outcome ? <><div className={outcome.correct ? "practice-feedback is-correct" : "practice-feedback is-wrong"} role="status"><span className="practice-feedback__mark" aria-hidden="true">{outcome.correct ? "✓" : "↺"}</span><span className="practice-feedback__copy"><strong>{outcome.correct ? (zh ? "正确" : "Correct") : (zh ? "复习这个答案" : "Review this answer")}</strong><span>{outcome.correct ? (zh ? `记忆分数现为 ${Math.round(outcome.entry.effectiveRecall)}。` : `Recall is now ${Math.round(outcome.entry.effectiveRecall)}.`) : (zh ? `正确答案是“${outcome.correctAnswer}”。` : `The answer is “${outcome.correctAnswer}”.`)}</span></span></div><button ref={nextButton} className="button button--primary practice-next" type="button" onClick={loadQuestion}>{zh ? "下一个词" : "Next word"}</button></> : <button className="button button--primary practice-submit" type="button" disabled={!selected || submitting} onClick={() => selected && submit(question, selected)}>{submitting ? (zh ? "正在检查…" : "Checking…") : (zh ? "检查答案" : "Check answer")}</button>}</div>
    </section>}
  </section>;
}
