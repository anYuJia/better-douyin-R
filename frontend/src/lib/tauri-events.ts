// ═══════════════════════════════════════════════
// Tauri / Browser event listener
// ═══════════════════════════════════════════════

import { listen as tauriListen } from "@tauri-apps/api/event";
import type { VideoInfo } from "./contracts";
import { normalizeVideo } from "./normalizers";
import { isTauriRuntime, getBrowserSocket, toFiniteNumber } from "./tauri-core";

type TauriUnlisten = () => void;
type EventHandler<T> = (payload: T) => void;
type BrowserSocketListener = (payload: unknown) => void;

function normalizeProgress(value: unknown, processed?: number, total?: number, currentProgress?: unknown) {
  const explicit = toFiniteNumber(value);
  if (explicit !== undefined) return Math.max(0, Math.min(100, explicit));
  const current = toFiniteNumber(currentProgress);
  if (total !== undefined && total > 0 && processed !== undefined) {
    const currentWeight = current !== undefined ? Math.max(0, Math.min(100, current)) / 100 : 0;
    return Math.max(0, Math.min(100, ((processed + currentWeight) / total) * 100));
  }
  return current !== undefined ? Math.max(0, Math.min(100, current)) : 0;
}

export function normalizeBrowserTask(value: unknown) {
  if (!value || typeof value !== "object") return null;
  const task = value as Record<string, unknown>;
  const id = String(task.id || task.task_id || "").trim();
  if (!id) return null;

  const status = String(task.status || "pending").trim().toLowerCase();
  const mappedStatus =
    status === "completed" ? "completed"
      : status === "downloading" ? "downloading"
      : status === "paused" ? "paused"
      : status === "cancelled" || status === "canceled" ? "cancelled"
      : status === "error" || status === "failed" ? "error"
      : "pending";
  const total = toFiniteNumber(task.total_videos ?? task.file_total ?? task.fileTotal ?? task.total_files);
  const processed = toFiniteNumber(task.processed ?? task.current_downloaded ?? task.file_index ?? task.fileIndex ?? task.completed_files);

  return {
    id,
    filename: String(task.filename || task.display_name || task.desc || id).trim(),
    progress: normalizeProgress(task.overall_progress, processed, total, task.progress),
    speed: Number(task.speed ?? task.speed_bps ?? 0) || 0,
    status: mappedStatus,
    isBatch: Boolean(task.isBatch ?? task.total_videos ?? task.fileTotal ?? task.total_files ?? false),
    awemeId: String(task.aweme_id || task.awemeId || "").trim() || undefined,
    currentAwemeId: String(task.current_aweme_id || task.currentAwemeId || "").trim() || undefined,
    currentName: String(task.current_name || task.currentName || "").trim() || undefined,
    savePath: String(task.save_path || task.savePath || "").trim() || undefined,
    filePath: String(task.file_path || task.filePath || "").trim() || undefined,
    mediaType: String(task.media_type || task.mediaType || "").trim() || undefined,
    mediaCount: toFiniteNumber(task.media_count ?? task.mediaCount ?? total),
    fileIndex: processed,
    fileTotal: total,
    fileProgress: Number(task.file_progress ?? task.fileProgress ?? 0) || undefined,
    completedCount: Number(task.completed_count ?? task.completedCount ?? 0) || undefined,
    skippedCount: Number(task.skipped_count ?? task.skippedCount ?? 0) || undefined,
    failedCount: Number(task.failed_count ?? task.failedCount ?? 0) || undefined,
    etaSeconds: Number(task.eta_seconds ?? task.etaSeconds ?? 0) || undefined,
    totalBytes: Number(task.total_bytes ?? task.totalBytes ?? 0) || undefined,
    downloadedBytes: Number(task.downloaded_bytes ?? task.downloadedBytes ?? 0) || undefined,
    startTime: Number(task.start_time ?? task.startTime ?? 0) || undefined,
    finishedTime: Number(task.finished_time ?? task.finishedTime ?? 0) || undefined,
    errorMessage: String(task.error_message || task.errorMessage || "").trim() || undefined,
  };
}

function normalizeBrowserDownloadProgress(payload: Record<string, unknown>) {
  const currentVideo = payload.current_video && typeof payload.current_video === "object"
    ? (payload.current_video as Record<string, unknown>)
    : {};
  const total = toFiniteNumber(payload.total_videos ?? payload.total);
  const processed = toFiniteNumber(payload.processed ?? payload.current_downloaded ?? payload.completed);

  return {
    task_id: String(payload.task_id || ""),
    progress: normalizeProgress(payload.overall_progress, processed, total, payload.progress ?? currentVideo.progress),
    overall_progress: normalizeProgress(payload.overall_progress, processed, total, payload.progress ?? currentVideo.progress),
    completed: Number(payload.current_downloaded ?? payload.completed ?? 0) || 0,
    current_downloaded: processed,
    total: Number(payload.total_videos ?? payload.total ?? 0) || 0,
    total_videos: total,
    processed,
    skipped: Number(payload.skipped ?? 0) || undefined,
    failed: Number(payload.failed ?? 0) || undefined,
    status: String(payload.status || "downloading"),
    desc: String(payload.desc || ""),
    display_name: String(payload.display_name || payload.desc || ""),
    file_index: Number(currentVideo.file_index ?? payload.file_index ?? 0) || undefined,
    file_total: Number(currentVideo.file_total ?? payload.file_total ?? 0) || undefined,
    file_progress: Number(currentVideo.progress ?? payload.file_progress ?? 0) || undefined,
    bytes_downloaded: Number(currentVideo.bytes_downloaded ?? payload.bytes_downloaded ?? 0) || undefined,
    bytes_total: Number(currentVideo.bytes_total ?? payload.bytes_total ?? 0) || undefined,
    speed_bps: Number(currentVideo.speed_bps ?? payload.speed_bps ?? 0) || undefined,
    eta_seconds: Number(payload.eta_seconds ?? currentVideo.eta_seconds ?? 0) || undefined,
    message: String(payload.message || currentVideo.message || ""),
  };
}

function normalizeDownloadInfoPayload(payload: Record<string, unknown>) {
  const total = toFiniteNumber(payload.total_videos);
  const processed = toFiniteNumber(payload.processed ?? payload.current_downloaded);
  return {
    task_id: String(payload.task_id || ""),
    progress: normalizeProgress(payload.overall_progress, processed, total),
    overall_progress: normalizeProgress(payload.overall_progress, processed, total),
    completed: Number(payload.current_downloaded ?? 0) || 0,
    current_downloaded: processed,
    total: Number(payload.total_videos ?? 0) || 0,
    total_videos: total,
    processed,
    skipped: Number(payload.skipped ?? 0) || undefined,
    failed: Number(payload.failed ?? 0) || undefined,
    status: "downloading",
    desc: String(payload.desc || ""),
    display_name: String(payload.display_name || payload.desc || ""),
    message: String(payload.message || ""),
  };
}

export function getDownloadPayload(video: VideoInfo) {
  const normalized = normalizeVideo(video) || video;
  const authorName = normalized.author?.nickname || "未知作者";
  const mediaUrls = normalized.media_urls && normalized.media_urls.length > 0
    ? normalized.media_urls
    : [];
  return {
    aweme_id: normalized.aweme_id,
    desc: normalized.desc || "",
    create_time: normalized.create_time || 0,
    author: normalized.author,
    video: normalized.video,
    cover_url: normalized.cover_url || normalized.video?.cover || "",
    media_type: normalized.media_type ?? "video",
    media_urls: mediaUrls,
    raw_media_type: normalized.raw_media_type ?? normalized.media_type ?? "video",
    author_name: authorName,
  };
}

export async function listenEvent<T>(event: string, handler: EventHandler<T>): Promise<TauriUnlisten> {
  const listenFn = window.__TAURI__?.event?.listen || tauriListen;
  if (isTauriRuntime()) {
    return listenFn<T>(event, (ev) => handler(ev.payload as T));
  }

  const socket = getBrowserSocket();
  if (!socket) return () => {};

  const bindings: Array<{ event: string; listener: BrowserSocketListener }> = [];
  const bind = (socketEvent: string, transform: (payload: unknown) => T | null) => {
    const listener: BrowserSocketListener = (payload) => {
      const mapped = transform(payload);
      if (mapped !== null) handler(mapped);
    };
    socket.on(socketEvent, listener);
    bindings.push({ event: socketEvent, listener });
  };

  switch (event) {
    case "download-started":
      bind("download_started", (payload) => {
        const data = payload && typeof payload === "object" ? (payload as Record<string, unknown>) : {};
        if (String(data.type || "") === "single_video") {
          return {
            task_id: String(data.task_id || ""),
            desc: String(data.desc || ""),
            display_name: String(data.display_name || data.desc || ""),
            type: String(data.type || ""),
            aweme_id: String(data.aweme_id || ""),
            media_type: String(data.media_type || ""),
            media_count: Number(data.media_count || 0) || 0,
          } as T;
        }
        return null;
      });
      break;
    case "batch-download-started":
      bind("download_started", (payload) => {
        const data = payload && typeof payload === "object" ? (payload as Record<string, unknown>) : {};
        if (String(data.type || "") === "single_video") return null;
        return {
          task_id: String(data.task_id || ""),
          nickname: String(data.user || data.nickname || ""),
          total_videos: Number(data.total_videos || 0) || undefined,
          message: String(data.message || ""),
        } as T;
      });
      break;
    case "download-progress":
      bind("download_progress", (payload) => payload as T);
      bind("user_video_download_progress", (payload) => {
        const data = payload && typeof payload === "object" ? (payload as Record<string, unknown>) : {};
        return normalizeBrowserDownloadProgress(data) as T;
      });
      bind("download_info", (payload) => {
        const data = payload && typeof payload === "object" ? (payload as Record<string, unknown>) : {};
        return normalizeDownloadInfoPayload(data) as T;
      });
      break;
    case "download-log":
      bind("download_log", (payload) => {
        const data = payload && typeof payload === "object" ? (payload as Record<string, unknown>) : {};
        return {
          task_id: String(data.task_id || ""),
          display_name: String(data.display_name || data.desc || ""),
          message: String(data.message || ""),
          timestamp: String(data.timestamp || ""),
        } as T;
      });
      break;
    case "download-failed":
      bind("download_failed", (payload) => {
        const data = payload && typeof payload === "object" ? (payload as Record<string, unknown>) : {};
        return {
          task_id: String(data.task_id || ""),
          error: String(data.error || data.message || ""),
        } as T;
      });
      break;
    case "download-error":
      bind("download_error", (payload) => {
        const data = payload && typeof payload === "object" ? (payload as Record<string, unknown>) : {};
        return {
          task_id: String(data.task_id || ""),
          message: String(data.message || data.error || ""),
        } as T;
      });
      break;
    case "download-cancelled":
      bind("download_cancelled", (payload) => payload as T);
      break;
    case "download-completed":
      bind("download_completed", (payload) => {
        const data = payload && typeof payload === "object" ? (payload as Record<string, unknown>) : {};
        if (data.total_videos !== undefined && data.aweme_id === undefined) return null;
        return {
          task_id: String(data.task_id || ""),
          display_name: String(data.display_name || data.message || ""),
          message: String(data.message || ""),
          files: Array.isArray(data.files) ? data.files.map((item) => String(item)) : undefined,
          file_path: String(data.file_path || ""),
          save_path: String(data.save_path || ""),
          total_size: Number(data.total_size || 0) || undefined,
        } as T;
      });
      break;
    case "batch-download-completed":
      bind("download_completed", (payload) => {
        const data = payload && typeof payload === "object" ? (payload as Record<string, unknown>) : {};
        if (data.total_videos === undefined && data.aweme_id !== undefined) return null;
        return {
          task_id: String(data.task_id || ""),
          total_videos: Number(data.total_videos || 0) || undefined,
          completed: Number(data.current_downloaded ?? data.completed ?? 0) || undefined,
          succeeded: Number(data.succeeded ?? 0) || undefined,
          skipped: Number(data.skipped ?? 0) || undefined,
          failed: Number(data.failed ?? 0) || undefined,
          processed: Number(data.processed ?? data.current_downloaded ?? data.completed ?? 0) || undefined,
          message: String(data.message || ""),
        } as T;
      });
      break;
    case "batch-download-cancelled":
      bind("download_cancelled", (payload) => {
        const data = payload && typeof payload === "object" ? (payload as Record<string, unknown>) : {};
        return {
          task_id: String(data.task_id || ""),
          message: String(data.message || ""),
        } as T;
      });
      break;
    case "current-video-progress":
      bind("user_video_download_progress", (payload) => {
        const data = payload && typeof payload === "object" ? (payload as Record<string, unknown>) : {};
        const currentVideo = data.current_video && typeof data.current_video === "object"
          ? (data.current_video as Record<string, unknown>)
          : {};
        return {
          task_id: String(data.task_id || ""),
          aweme_id: String(currentVideo.aweme_id || ""),
          name: String(currentVideo.desc || data.message || ""),
          progress: Number(currentVideo.progress ?? 0) || 0,
          speed_bps: Number(currentVideo.speed_bps ?? 0) || undefined,
          speed_mbps: Number(currentVideo.speed_mbps ?? 0) || undefined,
        } as T;
      });
      break;
    case "cookie-login-status":
      bind("cookie_login_status", (payload) => payload as T);
      break;
    default: {
      const fallback = event.replace(/-/g, "_");
      bind(fallback, (payload) => payload as T);
      break;
    }
  }

  return () => {
    bindings.forEach(({ event: socketEvent, listener }) => socket.off(socketEvent, listener));
  };
}
