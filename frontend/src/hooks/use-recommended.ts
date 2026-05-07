import { useState, useCallback, useRef } from "react";
import type { VideoItem } from "@/lib/tauri";
import { useLogStore } from "@/stores/app-store";

// ═══════════════════════════════════════════════
// Recommended Feed Hook
// ═══════════════════════════════════════════════

interface RecommendedState {
  videos: VideoItem[];
  loading: boolean;
  loadingMore: boolean;
  cursor: string;
  hasMore: boolean;
  error: string | null;
}

export function useRecommended() {
  const [state, setState] = useState<RecommendedState>({
    videos: [],
    loading: false,
    loadingMore: false,
    cursor: "",
    hasMore: true,
    error: null,
  });
  const addLog = useLogStore((s) => s.addLog);
  const abortRef = useRef(false);

  const loadFeed = useCallback(
    async (count = 20) => {
      setState((s) => ({ ...s, loading: true, error: null, videos: [] }));
      addLog("加载推荐视频...", "info");

      try {
        // const result = await invoke("get_recommended_feed", { count, cursor: "" })
        await new Promise((r) => setTimeout(r, 800));
        if (abortRef.current) return;

        setState({
          videos: [],
          loading: false,
          loadingMore: false,
          cursor: "",
          hasMore: true,
          error: null,
        });
        addLog("推荐视频加载完成", "success");
      } catch (e) {
        if (abortRef.current) return;
        const msg = e instanceof Error ? e.message : "加载失败";
        setState((s) => ({ ...s, loading: false, error: msg }));
        addLog(msg, "error");
      }
    },
    [addLog]
  );

  const loadMore = useCallback(async () => {
    if (state.loadingMore || !state.hasMore) return;

    setState((s) => ({ ...s, loadingMore: true }));

    try {
      // const result = await invoke("get_recommended_feed", { count: 20, cursor: state.cursor })
      await new Promise((r) => setTimeout(r, 500));
      if (abortRef.current) return;

      setState((s) => ({
        ...s,
        loadingMore: false,
        // videos: [...s.videos, ...result.videos],
        // cursor: result.cursor,
        // hasMore: result.videos.length > 0,
      }));
    } catch {
      setState((s) => ({ ...s, loadingMore: false }));
    }
  }, [state.loadingMore, state.hasMore, state.cursor]);

  const refresh = useCallback(() => {
    abortRef.current = false;
    loadFeed();
  }, [loadFeed]);

  return { ...state, loadFeed, loadMore, refresh };
}
