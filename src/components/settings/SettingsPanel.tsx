import { useEffect, useState, type FormEvent } from "react";

import type { TranslationProviderId, UserSettings } from "../../contracts/ipc";
import { messages } from "../../i18n/catalog";
import { defaultLanguages } from "../context/ContextualOverlay";

type CredentialStatus = "missing" | "ready" | "invalid" | "testing";
type PermissionStatus = "unknown" | "granted" | "denied";

interface SettingsPanelProps {
  settings: UserSettings;
  credentialStatus: CredentialStatus;
  permissionStatus: PermissionStatus;
  onSave: (settings: UserSettings) => void;
  onSaveCredential: (provider: TranslationProviderId, field: "api-key" | "app-id") => void;
  onProviderChange?: (provider: TranslationProviderId) => void;
  onTestCredential: (provider: TranslationProviderId) => void;
  onRemoveCredential: (provider: TranslationProviderId) => void;
  onOpenSystemSettings: () => void;
  onQuit: () => void;
}

export function SettingsPanel({
  settings,
  credentialStatus,
  permissionStatus,
  onSave,
  onSaveCredential,
  onProviderChange = () => undefined,
  onTestCredential,
  onRemoveCredential,
  onOpenSystemSettings,
  onQuit,
}: SettingsPanelProps) {
  const [draft, setDraft] = useState(settings);
  const [confirmRemoval, setConfirmRemoval] = useState(false);
  const monitoringEnabled = permissionStatus !== "denied" && draft.enabled;
  const copy = messages(draft.uiLocale);
  const credentialLabel = {
    missing: copy.notConfigured,
    ready: copy.storedSecurely,
    invalid: copy.needsAttention,
    testing: copy.testing,
  }[credentialStatus];

  useEffect(() => setDraft(settings), [settings]);

  const persist = (next: UserSettings) => {
    onSave({ ...next, enabled: permissionStatus !== "denied" && next.enabled });
  };

  const update = <Key extends keyof UserSettings>(key: Key, value: UserSettings[Key]) => {
    setDraft((current) => ({ ...current, [key]: value }));
  };

  const updateLanguage = (key: "sourceLanguage" | "targetLanguage", value: string) => {
    const next = { ...draft, [key]: value };
    setDraft(next);
    persist(next);
  };

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    persist(draft);
  };

  return (
    <main className="settings-shell">
      <header className="settings-header">
        <div className="brand-lockup">
          <span className="brand-mark" aria-hidden="true">译</span>
          <div>
            <p className="eyebrow">Desktop Translator</p>
            <h1>{copy.settings}</h1>
          </div>
        </div>
        <span className={`status-chip status-chip--${monitoringEnabled ? "active" : "quiet"}`}>
          <span aria-hidden="true" />
          {monitoringEnabled ? copy.monitoringOn : copy.monitoringOff}
        </span>
      </header>

      {permissionStatus === "denied" && (
        <section className="permission-banner" role="alert" aria-labelledby="permission-title">
          <div>
            <h2 id="permission-title">{copy.permissionTitle}</h2>
            <p>{copy.permissionBody}</p>
          </div>
          <button className="button button--secondary" type="button" onClick={onOpenSystemSettings}>
            {copy.openSystemSettings}
          </button>
        </section>
      )}

      <form onSubmit={submit}>
        <section className="settings-section" aria-labelledby="general-heading">
          <div className="section-heading">
            <span className="section-heading__index" aria-hidden="true">01</span>
            <div>
              <h2 id="general-heading">{copy.general}</h2>
              <p>{copy.generalHint}</p>
            </div>
          </div>
          <div className="settings-card">
            <label className="setting-row setting-row--toggle">
              <span>
                <strong>{copy.enableSelection}</strong>
                <small>{copy.enableSelectionHint}</small>
              </span>
              <input
                type="checkbox"
                name="enabled"
                checked={monitoringEnabled}
                onChange={(event) => update("enabled", event.currentTarget.checked)}
                disabled={permissionStatus === "denied"}
              />
              <span className="toggle" aria-hidden="true" />
            </label>
            <label className="setting-row setting-row--toggle">
              <span>
                <strong>{copy.startAtLogin}</strong>
                <small>{copy.startAtLoginHint}</small>
              </span>
              <input
                type="checkbox"
                name="startAtLogin"
                checked={draft.startAtLogin}
                onChange={(event) => update("startAtLogin", event.currentTarget.checked)}
              />
              <span className="toggle" aria-hidden="true" />
            </label>
          </div>
        </section>

        <section className="settings-section" aria-labelledby="languages-heading">
          <div className="section-heading">
            <span className="section-heading__index" aria-hidden="true">02</span>
            <div>
              <h2 id="languages-heading">{copy.languages}</h2>
              <p>{copy.languagesHint}</p>
            </div>
          </div>
          <div className="settings-card settings-card--fields">
            <label className="field">
              <span>{copy.sourceLanguage}</span>
              <select
                id="source-language"
                name="sourceLanguage"
                value={draft.sourceLanguage}
                onChange={(event) => updateLanguage("sourceLanguage", event.currentTarget.value)}
              >
                <option value="auto">{copy.detectAutomatically}</option>
                {defaultLanguages.map((language) => (
                  <option key={language.code} value={language.code}>{language.label}</option>
                ))}
              </select>
            </label>
            <span className="language-arrow" aria-hidden="true">→</span>
            <label className="field">
              <span>{copy.targetLanguage}</span>
              <select
                id="target-language"
                name="targetLanguage"
                value={draft.targetLanguage}
                onChange={(event) => updateLanguage("targetLanguage", event.currentTarget.value)}
              >
                {defaultLanguages.map((language) => (
                  <option key={language.code} value={language.code}>{language.label}</option>
                ))}
              </select>
            </label>
          </div>
        </section>

        <section className="settings-section" aria-labelledby="appearance-heading">
          <div className="section-heading">
            <span className="section-heading__index" aria-hidden="true">03</span>
            <div>
              <h2 id="appearance-heading">{copy.appearance}</h2>
              <p>{copy.appearanceHint}</p>
            </div>
          </div>
          <div className="settings-card settings-card--fields settings-card--appearance">
            <label className="field">
              <span>{copy.interfaceLanguage}</span>
              <select value={draft.uiLocale} onChange={(event) => update("uiLocale", event.currentTarget.value as UserSettings["uiLocale"])}>
                <option value="en">English</option>
                <option value="zh-CN">简体中文</option>
              </select>
            </label>
            <fieldset className="theme-picker">
              <legend>{copy.theme}</legend>
              {(["system", "light", "dark"] as const).map((theme) => (
                <label key={theme}>
                  <input
                    type="radio"
                    name="theme"
                    value={theme}
                    checked={draft.theme === theme}
                    onChange={() => update("theme", theme)}
                  />
                  <span className={`theme-preview theme-preview--${theme}`} aria-hidden="true">
                    <span />
                  </span>
                  <span>{theme === "system" ? copy.system : theme === "light" ? copy.light : copy.dark}</span>
                </label>
              ))}
            </fieldset>
          </div>
        </section>

        <section className="settings-section" aria-labelledby="access-heading">
          <div className="section-heading">
            <span className="section-heading__index" aria-hidden="true">04</span>
            <div>
              <h2 id="access-heading">{copy.serviceAccess}</h2>
              <p>{copy.serviceHint}</p>
            </div>
          </div>
          <div className="settings-card provider-card">
            <label className="field">
              <span>{copy.provider}</span>
              <select value={draft.translationProvider} onChange={(event) => { const provider = event.currentTarget.value as TranslationProviderId; update("translationProvider", provider); onProviderChange(provider); }}>
                <option value="google">{copy.google}</option>
                <option value="baidu">{copy.baidu}</option>
                <option value="microsoft">{copy.microsoft}</option>
              </select>
            </label>
            {draft.translationProvider === "microsoft" && (
              <>
                <label className="field">
                  <span>{copy.microsoftCloud}</span>
                  <select value={draft.microsoftCloud} onChange={(event) => update("microsoftCloud", event.currentTarget.value as UserSettings["microsoftCloud"])}>
                    <option value="global">{copy.globalCloud}</option>
                    <option value="china">{copy.chinaCloud}</option>
                  </select>
                </label>
                <label className="field"><span>{copy.region}</span><input value={draft.microsoftRegion ?? ""} onChange={(event) => update("microsoftRegion", event.currentTarget.value || undefined)} placeholder="chinaeast2" /></label>
              </>
            )}
          </div>
          <div className="settings-card credential-card">
            <div>
              <span className={`credential-dot credential-dot--${credentialStatus}`} aria-hidden="true" />
              <strong>{draft.translationProvider === "baidu" ? `${copy.appId} + ${copy.secretKey}` : copy.apiKey}</strong>
              <small aria-live="polite">{credentialLabel}</small>
            </div>
            <div className="button-group">
              {draft.translationProvider === "baidu" && <button className="button button--secondary" type="button" onClick={() => onSaveCredential("baidu", "app-id")}>{copy.configure} {copy.appId}</button>}
              <button className="button button--secondary" type="button" onClick={() => onSaveCredential(draft.translationProvider, "api-key")}>
                {copy.configure} {draft.translationProvider === "baidu" ? copy.secretKey : copy.apiKey}
              </button>
              <button
                className="button button--secondary"
                type="button"
                onClick={() => onTestCredential(draft.translationProvider)}
                disabled={credentialStatus === "missing" || credentialStatus === "testing"}
              >
                {credentialStatus === "testing" ? copy.testing : copy.test}
              </button>
              {confirmRemoval ? (
                <>
                  <button
                    className="button button--danger"
                    type="button"
                    onClick={() => {
                      onRemoveCredential(draft.translationProvider);
                      setConfirmRemoval(false);
                    }}
                  >
                    {copy.confirmRemoval}
                  </button>
                  <button
                    className="button button--secondary"
                    type="button"
                    onClick={() => setConfirmRemoval(false)}
                  >
                    {copy.cancel}
                  </button>
                </>
              ) : (
                <button
                  className="button button--danger"
                  type="button"
                  onClick={() => setConfirmRemoval(true)}
                  disabled={credentialStatus === "missing"}
                >
                  {copy.remove}
                </button>
              )}
            </div>
          </div>
        </section>

        <section className="settings-section" aria-labelledby="privacy-heading">
          <div className="section-heading">
            <span className="section-heading__index" aria-hidden="true">05</span>
            <div>
              <h2 id="privacy-heading">{copy.privacy}</h2>
              <p>{copy.privacyHint}</p>
            </div>
          </div>
          <div className="settings-card guidance-card">
            <ul>
              <li>{copy.privacyOne}</li>
              <li>{copy.privacyTwo}</li>
              <li>{copy.privacyThree}</li>
            </ul>
          </div>
        </section>

        <footer className="settings-footer">
          <button className="text-button text-button--danger" type="button" onClick={onQuit}>
            {copy.quit}
          </button>
          <button className="button button--primary" type="submit">{copy.save}</button>
        </footer>
      </form>
    </main>
  );
}
