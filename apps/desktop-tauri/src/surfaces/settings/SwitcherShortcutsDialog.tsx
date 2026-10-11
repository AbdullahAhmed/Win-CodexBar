import { useEffect, useId, useRef } from "react";
import { useLocale } from "../../hooks/useLocale";
import SwitcherShortcutsSection from "./SwitcherShortcutsSection";
import type { TabProps } from "./settingsTabs";

/**
 * In-window sheet for the provider-switcher keys, opened from
 * Settings > General > Portable preferences. Escape or Done closes it and
 * focus returns to the button that opened it. Every change saves at once,
 * like the rest of Settings.
 */
export default function SwitcherShortcutsDialog({
  settings,
  set,
  saving,
  onClose,
}: TabProps & { onClose: () => void }) {
  const { t } = useLocale();
  const titleId = useId();
  const dialogRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const opener = document.activeElement as HTMLElement | null;
    dialogRef.current?.focus();
    return () => opener?.focus?.();
  }, []);

  return (
    <div
      className="settings-dialog-backdrop"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <div
        ref={dialogRef}
        className="settings-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        tabIndex={-1}
        onKeyDown={(event) => {
          if (event.key === "Escape") {
            event.stopPropagation();
            onClose();
          }
        }}
      >
        <div className="settings-dialog__header">
          <h2 id={titleId} className="settings-dialog__title">
            {t("SwitcherShortcutsTitle")}
          </h2>
        </div>
        <div className="settings-dialog__body">
          <SwitcherShortcutsSection settings={settings} set={set} saving={saving} />
        </div>
        <div className="settings-dialog__actions">
          <button
            type="button"
            className="credential-btn credential-btn--primary"
            onClick={onClose}
          >
            {t("DialogDone")}
          </button>
        </div>
      </div>
    </div>
  );
}
