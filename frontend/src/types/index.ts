// ═══════════════════════════════════════════════
// TypeScript Type Exports
// ═══════════════════════════════════════════════

export type ViewType = "home" | "search" | "recommended" | "downloads" | "liked" | "liked-authors" | "settings";

export type ThemeMode = "light" | "dark" | "auto";

export type DownloadStatus = "pending" | "downloading" | "completed" | "error" | "paused";

export interface AppState {
  currentView: ViewType;
  setView: (view: ViewType) => void;
  theme: ThemeMode;
  setTheme: (theme: ThemeMode) => void;
  sidebarCollapsed: boolean;
  toggleSidebar: () => void;
  bottomBarExpanded: boolean;
  toggleBottomBar: () => void;
  settingsOpen: boolean;
  setSettingsOpen: (open: boolean) => void;
  commandOpen: boolean;
  setCommandOpen: (open: boolean) => void;
  commandMode: "search" | "link";
  setCommandMode: (mode: "search" | "link") => void;
}

export interface DownloadTask {
  id: string;
  filename: string;
  progress: number;
  speed: number;
  status: DownloadStatus;
  totalBytes?: number;
  downloadedBytes?: number;
  startTime?: number;
}

export interface LogEntry {
  id: number;
  message: string;
  type: "info" | "success" | "error" | "warning";
  timestamp: number;
}
