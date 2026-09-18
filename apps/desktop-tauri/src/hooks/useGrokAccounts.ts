import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import type { GrokAccount, GrokAccountUsage } from "../types/bridge";
import {
  grokAccountFetch,
  grokAccountsList,
} from "../lib/tauri";

export function useGrokAccounts({ reloadOnFocus = false }: { reloadOnFocus?: boolean } = {}) {
  const [accounts, setAccounts] = useState<GrokAccount[]>([]);
  const [usage, setUsage] = useState<Record<string, GrokAccountUsage>>({});
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const mounted = useRef(false);

  const reportError = useCallback((value: unknown) => {
    if (mounted.current) setError(String(value));
  }, []);

  const reload = useCallback(async () => {
    const next = await grokAccountsList();
    const snapshots: Record<string, GrokAccountUsage> = {};
    await Promise.all(
      next.map(async (account) => {
        try {
          snapshots[account.id] = await grokAccountFetch(account.id);
        } catch {
          // Keep the account row even if that login's usage fetch fails.
        }
      }),
    );
    if (mounted.current) {
      setAccounts(next);
      setUsage(snapshots);
      setError(null);
    }
  }, []);

  useEffect(() => {
    mounted.current = true;
    const refresh = () => void reload().catch(reportError);
    refresh();
    const unlisten = listen("grok-accounts-updated", refresh);
    if (reloadOnFocus) window.addEventListener("focus", refresh);
    return () => {
      mounted.current = false;
      if (reloadOnFocus) window.removeEventListener("focus", refresh);
      void unlisten.then((dispose) => dispose()).catch(() => {});
    };
  }, [reload, reloadOnFocus, reportError]);

  const run = useCallback(
    async (
      operation: () => Promise<void>,
      onSuccess?: () => void,
      onFinally?: () => void,
    ) => {
      setBusy(true);
      setError(null);
      try {
        await operation();
        await reload();
        if (mounted.current) onSuccess?.();
        return true;
      } catch (value) {
        reportError(value);
        return false;
      } finally {
        if (mounted.current) {
          setBusy(false);
          onFinally?.();
        }
      }
    },
    [reload, reportError],
  );

  return { accounts, usage, busy, error, reportError, reload, run };
}
