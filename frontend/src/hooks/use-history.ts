import { useState, useCallback, useEffect } from "react";
import type { HistoryItem } from "@/lib/tauri";
import { useLogStore } from "@/stores/app-store";

// ═══════════════════════════════════════════════
// History Hook
// ═══════════════════════════════════════════════

interface HistoryState {
  items: HistoryItem[];
  loading: boolean;
  error: string | null;
}

export function useHistory() {
  const [state, setState] = useState<HistoryState>({
    items: [],
    loading: false,
    error: null,
  });
  const addLog = useLogStore((s) => s.addLog);

  const loadHistory = useCallback(async () => {
    setState((s) => ({ ...s, loading: true }));
    try {
      // const items = await invoke("get_history")
      setState((s) => ({ ...s, loading: false, items: [] }));
    } catch (e) {
      const msg = e instanceof Error ? e.message : "加载历史失败";
      setState((s) => ({ ...s, loading: false, error: msg }));
      addLog(msg, "error");
    }
  }, [addLog]);

  const clearHistory = useCallback(async () => {
    try {
      // await invoke("clear_history")
      setState((s) => ({ ...s, items: [] }));
      addLog("下载历史已清空", "info");
    } catch (e) {
      const msg = e instanceof Error ? e.message : "清空失败";
      addLog(msg, "error");
    }
  }, [addLog]);

  useEffect(() => {
    loadHistory();
  }, [loadHistory]);

  return { ...state, loadHistory, clearHistory };
}
