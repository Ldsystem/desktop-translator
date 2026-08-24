import type { LanguageCode } from "../contracts/ipc";

interface PronouncerDependencies {
  stop: () => Promise<unknown>;
  speak: (text: string, language: LanguageCode) => Promise<unknown>;
  onError: () => void;
}

/** Coordinates native speech so rapid card clicks pronounce only the newest word. */
export function createPronouncer({ stop, speak, onError }: PronouncerDependencies) {
  let generation = 0;
  let stopBarrier: Promise<unknown> = Promise.resolve();

  return (text: string, language: LanguageCode) => {
    const request = ++generation;
    stopBarrier = stopBarrier.then(stop, stop).catch(() => undefined);
    void stopBarrier
      .then(() => {
        if (request !== generation) return undefined;
        return speak(text, language);
      })
      .catch(() => {
        if (request === generation) onError();
      });
  };
}
