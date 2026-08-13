import { useEffect, useState, type FormEvent } from "react";

import type { UserSettings } from "../../contracts/ipc";
import { defaultLanguages } from "../context/ContextualOverlay";

type CredentialStatus = "missing" | "ready" | "invalid" | "testing";
type PermissionStatus = "unknown" | "granted" | "denied";

interface SettingsPanelProps {
  settings: UserSettings;
  credentialStatus: CredentialStatus;
  permissionStatus: PermissionStatus;
  onSave: (settings: UserSettings) => void;
  onSaveCredential: () => void;
  onTestCredential: () => void;
  onRemoveCredential: () => void;
  onOpenSystemSettings: () => void;
  onQuit: () => void;
}

const credentialLabels: Record<CredentialStatus, string> = {
  missing: "Not Configured",
  ready: "Stored Securely",
  invalid: "Needs Attention",
  testing: "Testing…",
};

export function SettingsPanel({
  settings,
  credentialStatus,
  permissionStatus,
  onSave,
  onSaveCredential,
  onTestCredential,
  onRemoveCredential,
  onOpenSystemSettings,
  onQuit,
}: SettingsPanelProps) {
  const [draft, setDraft] = useState(settings);
  const [confirmRemoval, setConfirmRemoval] = useState(false);
  const monitoringEnabled = permissionStatus !== "denied" && draft.enabled;

  useEffect(() => setDraft(settings), [settings]);

  const update = <Key extends keyof UserSettings>(key: Key, value: UserSettings[Key]) => {
    setDraft((current) => ({ ...current, [key]: value }));
  };

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    onSave({ ...draft, enabled: monitoringEnabled });
  };

  return (
    <main className="settings-shell">
      <header className="settings-header">
        <div className="brand-lockup">
          <span className="brand-mark" aria-hidden="true">译</span>
          <div>
            <p className="eyebrow">Desktop Translator</p>
            <h1>Settings</h1>
          </div>
        </div>
        <span className={`status-chip status-chip--${monitoringEnabled ? "active" : "quiet"}`}>
          <span aria-hidden="true" />
          {monitoringEnabled ? "Monitoring On" : "Monitoring Off"}
        </span>
      </header>

      {permissionStatus === "denied" && (
        <section className="permission-banner" role="alert" aria-labelledby="permission-title">
          <div>
            <h2 id="permission-title">Accessibility Permission Required</h2>
            <p>
              Monitoring is off. Allow access in System Settings, then return here to enable it.
            </p>
          </div>
          <button className="button button--secondary" type="button" onClick={onOpenSystemSettings}>
            Open System Settings
          </button>
        </section>
      )}

      <form onSubmit={submit}>
        <section className="settings-section" aria-labelledby="general-heading">
          <div className="section-heading">
            <span className="section-heading__index" aria-hidden="true">01</span>
            <div>
              <h2 id="general-heading">General</h2>
              <p>Choose when translation is available.</p>
            </div>
          </div>
          <div className="settings-card">
            <label className="setting-row setting-row--toggle">
              <span>
                <strong>Enable Selection Translation</strong>
                <small>Show the translate control when text is selected.</small>
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
                <strong>Start at Login</strong>
                <small>Keep translation ready after you sign in.</small>
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
              <h2 id="languages-heading">Languages</h2>
              <p>Set the default direction for new selections.</p>
            </div>
          </div>
          <div className="settings-card settings-card--fields">
            <label className="field">
              <span>Source Language</span>
              <select
                id="source-language"
                name="sourceLanguage"
                value={draft.sourceLanguage}
                onChange={(event) => update("sourceLanguage", event.currentTarget.value)}
              >
                <option value="auto">Detect Automatically</option>
                {defaultLanguages.map((language) => (
                  <option key={language.code} value={language.code}>{language.label}</option>
                ))}
              </select>
            </label>
            <span className="language-arrow" aria-hidden="true">→</span>
            <label className="field">
              <span>Target Language</span>
              <select
                id="target-language"
                name="targetLanguage"
                value={draft.targetLanguage}
                onChange={(event) => update("targetLanguage", event.currentTarget.value)}
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
              <h2 id="appearance-heading">Appearance</h2>
              <p>Match the overlay to your workspace.</p>
            </div>
          </div>
          <fieldset className="theme-picker">
            <legend>Theme</legend>
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
                <span>{theme === "system" ? "System" : theme === "light" ? "Light" : "Dark"}</span>
              </label>
            ))}
          </fieldset>
        </section>

        <section className="settings-section" aria-labelledby="access-heading">
          <div className="section-heading">
            <span className="section-heading__index" aria-hidden="true">04</span>
            <div>
              <h2 id="access-heading">Service Access</h2>
              <p>Manage the key stored by the secure native service.</p>
            </div>
          </div>
          <div className="settings-card credential-card">
            <div>
              <span className={`credential-dot credential-dot--${credentialStatus}`} aria-hidden="true" />
              <strong>API Key</strong>
              <small aria-live="polite">{credentialLabels[credentialStatus]}</small>
            </div>
            <div className="button-group">
              <button className="button button--secondary" type="button" onClick={onSaveCredential}>
                Save API Key
              </button>
              <button
                className="button button--secondary"
                type="button"
                onClick={onTestCredential}
                disabled={credentialStatus === "missing" || credentialStatus === "testing"}
              >
                {credentialStatus === "testing" ? "Testing…" : "Test API Key"}
              </button>
              {confirmRemoval ? (
                <>
                  <button
                    className="button button--danger"
                    type="button"
                    onClick={() => {
                      onRemoveCredential();
                      setConfirmRemoval(false);
                    }}
                  >
                    Confirm Removal
                  </button>
                  <button
                    className="button button--secondary"
                    type="button"
                    onClick={() => setConfirmRemoval(false)}
                  >
                    Cancel
                  </button>
                </>
              ) : (
                <button
                  className="button button--danger"
                  type="button"
                  onClick={() => setConfirmRemoval(true)}
                  disabled={credentialStatus === "missing"}
                >
                  Remove API Key
                </button>
              )}
            </div>
          </div>
        </section>

        <section className="settings-section" aria-labelledby="privacy-heading">
          <div className="section-heading">
            <span className="section-heading__index" aria-hidden="true">05</span>
            <div>
              <h2 id="privacy-heading">Privacy & Usage</h2>
              <p>Know what leaves your Mac and when.</p>
            </div>
          </div>
          <div className="settings-card guidance-card">
            <ul>
              <li>Selected text is sent to Google only after you choose Translate.</li>
              <li>
                API usage may incur charges and is subject to billing, quota, and API key
                restrictions.
              </li>
              <li>No content history is stored.</li>
            </ul>
          </div>
        </section>

        <footer className="settings-footer">
          <button className="text-button text-button--danger" type="button" onClick={onQuit}>
            Quit Desktop Translator
          </button>
          <button className="button button--primary" type="submit">Save Changes</button>
        </footer>
      </form>
    </main>
  );
}
