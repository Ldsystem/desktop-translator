import { useEffect, useRef, useState } from "react";

import type { PracticeDirection, StudyPracticeOutcome, StudyPracticeQuestion } from "../../contracts/ipc";
import type { StudyApi } from "./VocabularyWindow";

interface PracticeViewProps {
  api: StudyApi;
  revision: number;
}

const directions: { value: PracticeDirection; label: string; note: string }[] = [
  { value: "random", label: "Mix", note: "Both directions" },
  { value: "source-to-target", label: "Word → meaning", note: "Recognition" },
  { value: "target-to-source", label: "Meaning → word", note: "Production" },
];

export function PracticeView({ api, revision }: PracticeViewProps) {
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
      setError("A practice question could not be prepared.");
    });
  };

  useEffect(() => {
    let current = true;
    void api.getPracticePreferences().then((preferences) => {
      if (current) setDirection(preferences.direction);
    }).catch(() => setError("Your practice preference could not be opened."));
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
      setError("Your practice direction could not be saved.");
    }).finally(() => setSaving(false));
  };

  const submit = (questionValue: StudyPracticeQuestion, answer: string) => {
    if (submittingRef.current) return;
    submittingRef.current = true;
    setSubmitting(true);
    setError(undefined);
    void api.submitPracticeAnswer(questionValue.entryId, questionValue.direction, answer)
      .then(setOutcome)
      .catch(() => setError("Your answer could not be saved."))
      .finally(() => {
        submittingRef.current = false;
        setSubmitting(false);
      });
  };

  return <section aria-labelledby="practice-title">
    <header className="study-header"><div><p className="eyebrow">Practice</p><h2 id="practice-title">Choose the answer</h2><p>Questions come from your wordbook. Related words and downloaded textbooks make the choices more challenging.</p></div></header>
    <fieldset className="direction-selector" disabled={saving || submitting}><legend>Practice direction</legend>{directions.map((item) => <label key={item.value} className={direction === item.value ? "is-active" : ""}><input type="radio" name="practice-direction" value={item.value} checked={direction === item.value} onChange={() => chooseDirection(item.value)} /><span><strong>{item.label}</strong><small>{item.note}</small></span></label>)}</fieldset>
    {error && <div className="study-notice study-notice--error" role="alert">{error} <button className="text-button" type="button" onClick={() => failedDirection ? chooseDirection(failedDirection) : loadQuestion()}>{failedDirection ? "Try saving again" : "Try again"}</button></div>}
    {question === undefined ? <div className="study-empty" role="status"><strong>Choosing what needs attention…</strong></div> : question === null ? <div className="study-empty study-empty--complete"><strong>You have practised every available word.</strong><span>Come back after recall has faded, or add another word to continue.</span></div> : <section className="practice-card">
      <div className="practice-prompt"><span>{question.promptLanguage.toUpperCase()} → {question.answerLanguage.toUpperCase()}</span><strong>{question.prompt}</strong></div>
      <div className="practice-choices" role="radiogroup" aria-label="Answer choices">{question.choices.map((choice) => <button key={choice} className={selected === choice ? "practice-choice is-selected" : "practice-choice"} type="button" role="radio" aria-checked={selected === choice} disabled={Boolean(outcome) || submitting} onClick={() => setSelected(choice)}>{choice}</button>)}</div>
      <div className="practice-actions">{outcome ? <><div className={outcome.correct ? "practice-feedback is-correct" : "practice-feedback is-wrong"} role="status"><span className="practice-feedback__mark" aria-hidden="true">{outcome.correct ? "✓" : "↺"}</span><span className="practice-feedback__copy"><strong>{outcome.correct ? "Correct" : "Review this answer"}</strong><span>{outcome.correct ? `Recall is now ${Math.round(outcome.entry.effectiveRecall)}.` : `The answer is “${outcome.correctAnswer}”.`}</span></span></div><button ref={nextButton} className="button button--primary practice-next" type="button" onClick={loadQuestion}>Next word</button></> : <button className="button button--primary practice-submit" type="button" disabled={!selected || submitting} onClick={() => selected && submit(question, selected)}>{submitting ? "Checking…" : "Check answer"}</button>}</div>
    </section>}
  </section>;
}
