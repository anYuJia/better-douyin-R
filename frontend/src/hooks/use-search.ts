import { useState, useCallback } from "react";
import type { VideoItem, UserInfo } from "@/lib/tauri";
import { useLogStore, useAppStore } from "@/stores/app-store";

// ═══════════════════════════════════════════════
// Search Hook
// ═══════════════════════════════════════════════

interface SearchState {
  loading: boolean;
  users: UserInfo[];
  currentUser: UserInfo | null;
  videos: VideoItem[];
  error: string | null;
}

export function useSearch() {
  const [state, setState] = useState<SearchState>({
    loading: false,
    users: [],
    currentUser: null,
    videos: [],
    error: null,
  });
  const addLog = useLogStore((s) => s.addLog);
  const setView = useAppStore((s) => s.setView);

  const search = useCallback(async (keyword: string) => {
    setState((s) => ({ ...s, loading: true, error: null }));
    addLog(`搜索用户: ${keyword}`, "info");

    try {
      // In real app: const result = await invoke("search_user", { keyword })
      // For now, mock:
      await new Promise((r) => setTimeout(r, 500));
      addLog("搜索完成", "success");
      setState((s) => ({ ...s, loading: false }));
      setView("search");
    } catch (e) {
      const msg = e instanceof Error ? e.message : "搜索失败";
      setState((s) => ({ ...s, loading: false, error: msg }));
      addLog(msg, "error");
    }
  }, [addLog, setView]);

  const selectUser = useCallback(async (user: UserInfo) => {
    setState((s) => ({ ...s, currentUser: user, loading: true }));
    addLog(`加载用户: ${user.nickname}`, "info");

    try {
      // const videos = await invoke("get_user_videos", { uid: user.uid, count: 20, cursor: 0 })
      await new Promise((r) => setTimeout(r, 500));
      setState((s) => ({ ...s, loading: false, videos: [] }));
      addLog("作品列表已加载", "success");
    } catch (e) {
      const msg = e instanceof Error ? e.message : "加载失败";
      setState((s) => ({ ...s, loading: false, error: msg }));
      addLog(msg, "error");
    }
  }, [addLog]);

  const clear = useCallback(() => {
    setState({ loading: false, users: [], currentUser: null, videos: [], error: null });
  }, []);

  return { ...state, search, selectUser, clear };
}
