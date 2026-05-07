// ═══════════════════════════════════════════════
// Tauri IPC Wrappers
// ═══════════════════════════════════════════════

type TauriInvoke = <T>(command: string, args?: Record<string, unknown>) => Promise<T>;

function invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  const tauriInvoke = (window as Window & {
    __TAURI__?: {
      core?: {
        invoke?: TauriInvoke;
      };
    };
  }).__TAURI__?.core?.invoke;

  if (!tauriInvoke) {
    return Promise.reject(new Error("Tauri API unavailable"));
  }

  return tauriInvoke<T>(command, args);
}

export interface Config {
  cookie: string;
  download_dir: string;
  max_concurrent: number;
  download_quality: string;
}

export interface UserInfo {
  uid: string;
  nickname: string;
  unique_id: string;
  signature: string;
  avatar_url: string;
  aweme_count: number;
  follower_count: number;
  following_count: number;
  total_favorited: number;
}

export interface VideoItem {
  aweme_id: string;
  desc: string;
  author_nickname: string;
  author_uid: string;
  author_avatar: string;
  cover_url: string;
  video_url?: string;
  image_urls?: string[];
  duration?: number;
  create_time: number;
  digg_count: number;
  comment_count: number;
  share_count: number;
  aweme_type: number;
  music_title?: string;
  music_author?: string;
  music_url?: string;
}

export interface SearchResult {
  users: UserInfo[];
}

export interface DownloadProgress {
  task_id: string;
  filename: string;
  progress: number;
  speed: number;
  status: string;
  total_bytes?: number;
  downloaded_bytes?: number;
  elapsed_seconds?: number;
}

export interface HistoryItem {
  id: string;
  filename: string;
  path: string;
  author: string;
  desc: string;
  size: number;
  timestamp: number;
  file_type: string;
}

// ── Tauri invoke wrappers ──

export async function searchUser(keyword: string): Promise<SearchResult> {
  return invoke("search_user", { keyword });
}

export async function getUserInfo(uid: string): Promise<UserInfo> {
  return invoke("get_user_info", { uid });
}

export async function getUserVideos(uid: string, count: number, cursor: number): Promise<{ videos: VideoItem[]; has_more: boolean }> {
  return invoke("get_user_videos", { uid, count, cursor });
}

export async function parseLink(url: string): Promise<VideoItem[]> {
  return invoke("parse_link", { url });
}

export async function downloadVideo(awemeId: string, videoUrl?: string): Promise<string> {
  return invoke("download_video", { awemeId, videoUrl });
}

export async function downloadBatch(items: { aweme_id: string; url?: string }[]): Promise<string> {
  return invoke("download_batch", { items });
}

export async function getRecommendedFeed(count: number, cursor?: string): Promise<{ videos: VideoItem[]; cursor: string }> {
  return invoke("get_recommended_feed", { count, cursor });
}

export async function getLikedVideos(count: number): Promise<VideoItem[]> {
  return invoke("get_liked_videos", { count });
}

export async function getLikedAuthors(count: number): Promise<UserInfo[]> {
  return invoke("get_liked_authors", { count });
}

export async function loadConfig(): Promise<Config> {
  return invoke("load_config");
}

export async function saveConfig(config: Config): Promise<void> {
  return invoke("save_config", { config });
}

export async function openPath(path: string): Promise<void> {
  return invoke("open_path", { path });
}

export async function selectDirectory(): Promise<string | null> {
  return invoke("select_directory");
}

export async function getHistory(): Promise<HistoryItem[]> {
  return invoke("get_history");
}

export async function clearHistory(): Promise<void> {
  return invoke("clear_history");
}

export async function getVersion(): Promise<string> {
  return invoke("get_version");
}

export async function validateCookie(cookie: string): Promise<{ valid: boolean; missing: string[] }> {
  return invoke("validate_cookie", { cookie });
}

export async function checkUpdate(): Promise<{ current: string; latest: string; notes: string; url: string } | null> {
  return invoke("check_update");
}

export async function listDownloadFiles(dir: string): Promise<{ files: HistoryItem[]; total_size: number }> {
  return invoke("list_download_files", { dir });
}

export async function deleteFile(path: string): Promise<void> {
  return invoke("delete_file", { path });
}

export async function startBrowserLogin(browserType: string): Promise<string> {
  return invoke("start_browser_login", { browserType });
}

export async function cancelBrowserLogin(): Promise<void> {
  return invoke("cancel_browser_login");
}

export async function getStorageStats(): Promise<{ video_count: number; total_size: number; author_count: number; oldest: string }> {
  return invoke("get_storage_stats");
}
