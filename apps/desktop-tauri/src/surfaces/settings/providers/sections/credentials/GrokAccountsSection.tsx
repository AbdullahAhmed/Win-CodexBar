import { useState } from "react";
import type { GrokAccount, GrokAccountUsage } from "../../../../../types/bridge";
import type { LocaleKey } from "../../../../../i18n/keys";
import {
  grokAccountAdd,
  grokAccountCancelLogin,
  grokAccountSaveCurrent,
  grokAccountRemove,
  grokAccountSwitch,
} from "../../../../../lib/tauri";
import { useGrokAccounts } from "../../../../../hooks/useGrokAccounts";

export function GrokAccountsSection({ t }: { t: (key: LocaleKey) => string }) {
  const [loggingIn, setLoggingIn] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const { accounts, usage, busy, error, reportError, run } = useGrokAccounts();
  const runOperation = (operation: () => Promise<void>, success?: LocaleKey) =>
    void run(
      operation,
      success ? () => setMessage(t(success)) : undefined,
      () => setLoggingIn(false),
    );
  return (
    <section className="provider-detail-section codex-accounts">
      <h4>{t("GrokAccountsTitle")}</h4>
      <p className="settings-section__hint">{t("GrokAccountsHint")}</p>
      {error && (
        <div className="provider-detail-error" role="alert">
          {error}
        </div>
      )}
      {message && (
        <div className="provider-detail-note" role="status">
          {message}
        </div>
      )}
      {loggingIn && <p role="status">{t("GrokAccountsSigningIn")}</p>}
      {accounts.length === 0 && <p>{t("GrokAccountsEmpty")}</p>}
      <ul className="credential-list">
        {accounts.map((account) => (
          <li className="credential-card" key={account.id}>
            <div className="credential-card__header">
              <div className="credential-card__info">
                <strong>{account.email}</strong>
                <span className="credential-card__meta">
                  {usageLabel(account, usage[account.id])}
                </span>
                {account.isActive && (
                  <span className="credential-card__badge credential-card__badge--set">
                    {t("TokenAccountActive")}
                  </span>
                )}
              </div>
              <div className="credential-card__actions">
                {!account.isActive && account.isSaved && (
                  <button
                    className="credential-btn credential-btn--primary"
                    disabled={busy}
                    onClick={() =>
                      run(
                        () => grokAccountSwitch(account.id),
                        () => setMessage(t("GrokAccountsSwitched")),
                      )
                    }
                  >
                    {t("CodexAccountsSwitchButton")}
                  </button>
                )}
                {!account.isSaved && (
                  <button
                    className="credential-btn credential-btn--secondary"
                    disabled={busy}
                    onClick={() => runOperation(grokAccountSaveCurrent)}
                  >
                    {t("GrokAccountsSaveCurrent")}
                  </button>
                )}
                {account.isSaved && (
                  <button
                    className="credential-btn credential-btn--danger"
                    disabled={busy}
                    onClick={() => runOperation(() => grokAccountRemove(account.id))}
                  >
                    {t("CodexAccountsRemoveButton")}
                  </button>
                )}
              </div>
            </div>
          </li>
        ))}
      </ul>
      <button
        className="credential-btn credential-btn--primary"
        disabled={busy}
        onClick={() => {
          setLoggingIn(true);
          runOperation(grokAccountAdd, "GrokAccountsAdded");
        }}
      >
        {t("CodexAccountsAddButton")}
      </button>
      {loggingIn && (
        <button
          className="credential-btn credential-btn--secondary"
          onClick={() =>
            void grokAccountCancelLogin().catch(reportError)
          }
        >
          {t("GrokAccountsCancelLogin")}
        </button>
      )}
    </section>
  );
}

function usageLabel(account: GrokAccount, snapshot?: GrokAccountUsage): string {
  const plan = snapshot?.plan || account.plan || "";
  const percent =
    snapshot?.usageAvailable && snapshot.usedPercent != null
      ? `${Math.round(snapshot.usedPercent)}%`
      : null;
  return [plan, percent].filter(Boolean).join(" · ");
}
