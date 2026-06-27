// ═══════════════════════════════════════════════
// Tauri IPC Wrappers — Facade
//
// Re-exports from thematic sub-modules so callers
// can keep importing from "@/lib/tauri".
// ═══════════════════════════════════════════════

export type * from "./contracts";

export {
  getErrorMessage,
  normalizeHistoryItem,
  normalizeLikedVideo,
  normalizeUser,
  normalizeVideo,
  normalizeVideos,
} from "./normalizers";

export * from "./tauri-media";
export * from "./tauri-events";
export * from "./tauri-config";
export * from "./tauri-content";
export * from "./tauri-download";
export * from "./tauri-social";
