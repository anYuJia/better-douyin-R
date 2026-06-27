// ═══════════════════════════════════════════════
// Download tasks, history, file operations, clipboard
// ═══════════════════════════════════════════════

import type {
  ApiResponse,
  DownloadFilesResult,
  HistoryItem,
  VideoInfo,
} from "./contracts";
import { normalizeHistoryItem } from "./normalizers";
import { invoke, shouldUseBrowserBridge, requestJson, isTauriRuntime, writeTextWithBrowserClipboard } from "./tauri-core";
import { getDownloadPayload } from "./tauri-events";

export async function downloadVideo(video: VideoInfo): Promise<ApiResponse & { task_id?: string }> {
  const payload = getDownloadPayload(video);
  if (shouldUseBrowserBridge()) {
    return requestJson("/api/download_single_video", {
      method: "POST",
      body: JSON.stringify(payload),
    });
  }
  return invoke("download_video", { video: payload });
}

export async function downloadUserVideos(
  secUid: string,
  nickname: string,
  awemeCount: number
): Promise<ApiResponse & { task_id?: string; total_videos?: number; nickname?: string }> {
  if (shouldUseBrowserBridge()) {
    return requestJson("/api/download_user_video", {
      method: "POST",
      body: JSON.stringify({
        sec_uid: secUid,
        nickname,
        aweme_count: awemeCount,
      }),
    });
  }
  return invoke("download_user_videos", {
    secUid,
    sec_uid: secUid,
    nickname,
    awemeCount,
    aweme_count: awemeCount,
  });
}

export async function downloadLikedVideos(count: number): Promise<{ success: boolean; message: string }> {
  if (shouldUseBrowserBridge()) {
    return requestJson("/api/download_liked", {
      method: "POST",
      body: JSON.stringify({ count }),
    });
  }
  return invoke("download_liked_videos", { count });
}

export async function downloadLikedAuthors(count: number): Promise<{ success: boolean; message: string }> {
  if (shouldUseBrowserBridge()) {
    return requestJson("/api/download_liked_authors", {
      method: "POST",
      body: JSON.stringify({ count }),
    });
  }
  return invoke("download_liked_authors", { count });
}

export async function addDownloadTask(video: VideoInfo, savePath?: string): Promise<string> {
  const payload = getDownloadPayload(video);
  if (shouldUseBrowserBridge()) {
    const result = await requestJson<ApiResponse & { task_id?: string }>("/api/download_single_video", {
      method: "POST",
      body: JSON.stringify({
        ...payload,
        save_path: savePath,
      }),
    });
    return result.task_id || video.aweme_id;
  }
  return invoke("add_download_task", { video: payload, savePath, save_path: savePath });
}

export async function startDownload(taskId: string): Promise<void> {
  if (shouldUseBrowserBridge()) return;
  return invoke("start_download", { taskId, task_id: taskId });
}

export async function getDownloadTasks(): Promise<unknown[]> {
  if (shouldUseBrowserBridge()) {
    const result = await requestJson<{ success: boolean; tasks?: unknown }>(
      "/api/tasks"
    );
    const tasks = result.tasks;
    if (Array.isArray(tasks)) return tasks;
    if (tasks && typeof tasks === "object") {
      return Object.values(tasks as Record<string, unknown>);
    }
    return [];
  }
  const result = await invoke<{ success: boolean; tasks?: unknown[] }>("get_download_tasks");
  return result.tasks || [];
}

export async function cancelDownloadTask(taskId: string): Promise<ApiResponse> {
  if (shouldUseBrowserBridge()) {
    return requestJson("/api/cancel_download", {
      method: "POST",
      body: JSON.stringify({ task_id: taskId }),
    });
  }
  return invoke("cancel_download_task", { taskId, task_id: taskId });
}

export async function removeDownloadTask(taskId: string): Promise<void> {
  if (shouldUseBrowserBridge()) return;
  return invoke("remove_download_task", { taskId, task_id: taskId });
}

export async function pauseDownload(taskId: string): Promise<ApiResponse> {
  if (shouldUseBrowserBridge()) {
    return requestJson("/api/pause_download", {
      method: "POST",
      body: JSON.stringify({ task_id: taskId }),
    });
  }
  return invoke("pause_download", { taskId, task_id: taskId });
}

export async function resumeDownload(taskId: string): Promise<ApiResponse> {
  if (shouldUseBrowserBridge()) {
    return requestJson("/api/resume_download", {
      method: "POST",
      body: JSON.stringify({ task_id: taskId }),
    });
  }
  return invoke("resume_download", { taskId, task_id: taskId });
}

export async function getHistory(): Promise<HistoryItem[]> {
  if (shouldUseBrowserBridge()) {
    const result = await requestJson<{ success: boolean; items?: unknown[] }>("/api/download_history");
    return (result.items || []).map(normalizeHistoryItem).filter(Boolean) as HistoryItem[];
  }
  const result = await invoke<{ success: boolean; items?: unknown[] }>("get_history");
  return (result.items || []).map(normalizeHistoryItem).filter(Boolean) as HistoryItem[];
}

function buildDownloadHistoryParams(
  options: {
    offset?: number;
    limit?: number;
    forceRefresh?: boolean;
    query?: string;
    mediaType?: string;
    sortBy?: string;
  } = {},
  forceRefresh = false
): URLSearchParams {
  const params = new URLSearchParams();
  if (forceRefresh || options.forceRefresh) params.set("refresh", "1");
  if (options.offset !== undefined) params.set("offset", String(options.offset));
  if (options.limit !== undefined) params.set("limit", String(options.limit));
  if (options.query?.trim()) params.set("query", options.query.trim());
  if (options.mediaType) params.set("media_type", options.mediaType);
  if (options.sortBy) params.set("sort_by", options.sortBy);
  return params;
}

export async function listDownloadFiles(options?: {
  offset?: number;
  limit?: number;
  forceRefresh?: boolean;
  query?: string;
  mediaType?: string;
  sortBy?: string;
}): Promise<HistoryItem[]> {
  if (shouldUseBrowserBridge()) {
    const params = buildDownloadHistoryParams(options, true);
    const result = await requestJson<{ success: boolean; items?: unknown[] }>(`/api/download_history?${params.toString()}`);
    return (result.items || []).map(normalizeHistoryItem).filter(Boolean) as HistoryItem[];
  }
  const result = await invoke<{ success: boolean; items?: unknown[] }>("list_download_files", {
    offset: options?.offset,
    limit: options?.limit,
    forceRefresh: options?.forceRefresh,
    query: options?.query,
    mediaType: options?.mediaType,
    media_type: options?.mediaType,
    sortBy: options?.sortBy,
    sort_by: options?.sortBy,
  });
  return (result.items || []).map(normalizeHistoryItem).filter(Boolean) as HistoryItem[];
}

export async function listDownloadFilesPage(options: {
  offset?: number;
  limit?: number;
  forceRefresh?: boolean;
  query?: string;
  mediaType?: string;
  sortBy?: string;
} = {}): Promise<DownloadFilesResult> {
  if (shouldUseBrowserBridge()) {
    const params = buildDownloadHistoryParams(options, true);
    const result = await requestJson<{ success: boolean; items?: unknown[]; total?: number; total_size?: number; latest?: unknown }>(
      `/api/download_history?${params.toString()}`
    );
    return {
      items: (result.items || []).map(normalizeHistoryItem).filter(Boolean) as HistoryItem[],
      total: Number(result.total ?? 0) || 0,
      totalSize: Number(result.total_size ?? 0) || 0,
      latest: normalizeHistoryItem(result.latest) as HistoryItem | null,
    };
  }
  const result = await invoke<{ success: boolean; items?: unknown[]; total?: number; total_size?: number; latest?: unknown }>(
    "list_download_files",
    {
      offset: options.offset,
      limit: options.limit,
      forceRefresh: options.forceRefresh,
      query: options.query,
      mediaType: options.mediaType,
      media_type: options.mediaType,
      sortBy: options.sortBy,
      sort_by: options.sortBy,
    }
  );
  return {
    items: (result.items || []).map(normalizeHistoryItem).filter(Boolean) as HistoryItem[],
    total: Number(result.total ?? 0) || 0,
    totalSize: Number(result.total_size ?? 0) || 0,
    latest: normalizeHistoryItem(result.latest) as HistoryItem | null,
  };
}

export async function clearHistory(): Promise<void> {
  if (shouldUseBrowserBridge()) {
    const history = await getHistory().catch(() => []);
    const paths = history.map((item) => item.path).filter(Boolean);
    if (paths.length > 0) {
      await requestJson("/api/download_history/delete", {
        method: "POST",
        body: JSON.stringify({ paths }),
      });
    }
    return;
  }
  return invoke("clear_history");
}

export async function deleteHistory(id: string): Promise<void> {
  if (shouldUseBrowserBridge()) {
    const history = await getHistory().catch(() => []);
    const target = history.find((item) => item.id === id || item.aweme_id === id || item.path === id);
    if (target?.path) {
      await deleteFile(target.path);
    }
    return;
  }
  return invoke("delete_history", { awemeId: id, aweme_id: id });
}

export async function addHistory(entry: Omit<HistoryItem, "id">): Promise<void> {
  if (shouldUseBrowserBridge()) return;
  return invoke("add_history", { entry });
}

export async function openFile(path: string): Promise<void> {
  if (shouldUseBrowserBridge()) {
    await requestJson("/api/download_history/open", {
      method: "POST",
      body: JSON.stringify({ path }),
    });
    return;
  }
  return invoke("open_file", { path });
}

export async function openDownloadDirectory(): Promise<void> {
  if (shouldUseBrowserBridge()) {
    await requestJson("/api/download_history/open_directory", { method: "POST" });
    return;
  }
  return invoke("open_download_directory");
}

export async function openFileLocation(path: string): Promise<void> {
  if (shouldUseBrowserBridge()) {
    await requestJson("/api/download_history/open_location", {
      method: "POST",
      body: JSON.stringify({ path }),
    });
    return;
  }
  return invoke("open_file_location", { path });
}

export async function openExternalUrl(url: string): Promise<void> {
  const target = String(url || "").trim();
  if (!target) return;

  if (isTauriRuntime()) {
    return invoke("open_external_url", { url: target });
  }

  window.open(target, "_blank", "noopener,noreferrer");
}

export async function deleteFile(path: string): Promise<void> {
  if (shouldUseBrowserBridge()) {
    await requestJson("/api/download_history/delete", {
      method: "POST",
      body: JSON.stringify({ paths: [path] }),
    });
    return;
  }
  return invoke("delete_file", { path });
}

export async function copyTextToClipboard(text: string): Promise<boolean> {
  const value = String(text || "");
  if (!value) return false;

  if (isTauriRuntime()) {
    try {
      await invoke("copy_text_to_clipboard", { text: value });
      return true;
    } catch {
      // Fall back to browser clipboard if the native bridge is unavailable.
    }
  }

  try {
    const result = await requestJson<{ success?: boolean }>("/api/clipboard/write", {
      method: "POST",
      body: JSON.stringify({ text: value }),
    });
    if (result.success !== false) return true;
  } catch {
    // Fall back to browser clipboard below.
  }

  return writeTextWithBrowserClipboard(value);
}
