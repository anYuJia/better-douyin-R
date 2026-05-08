// ═══════════════════════════════════════════════
// History Refresh Event Bus
// ═══════════════════════════════════════════════
// Allows socket event handlers to trigger a history refresh
// when the downloads view is visible.

type RefreshCallback = () => void;

let currentCallback: RefreshCallback | null = null;

export function setHistoryRefreshCallback(cb: RefreshCallback | null) {
  currentCallback = cb;
}

export function triggerHistoryRefresh() {
  if (currentCallback) {
    currentCallback();
  }
}
