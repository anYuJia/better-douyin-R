// ═══════════════════════════════════════════════
// Tauri IPC Wrappers
// ═══════════════════════════════════════════════

type TauriInvoke = <T>(command: string, args?: Record<string, unknown>) => Promise<T>;
const MEDIA_PROXY_BASE = "http://127.0.0.1:39143/api/media/proxy";

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

export function mediaProxyUrl(url: string | null | undefined, mediaType = "image"): string {
  const trimmed = (url || "").trim();
  if (!trimmed) return "";
  if (trimmed.startsWith("data:") || trimmed.startsWith("blob:")) return trimmed;
  if (trimmed.startsWith("/") || trimmed.includes("127.0.0.1:39143/api/media/proxy")) {
    return trimmed;
  }

  try {
    const parsed = new URL(trimmed);
    if (parsed.protocol !== "http:" && parsed.protocol !== "https:") return trimmed;
    return `${MEDIA_PROXY_BASE}?url=${encodeURIComponent(trimmed)}&media_type=${encodeURIComponent(mediaType)}`;
  } catch {
    return trimmed;
  }
}

// ── Types matching Rust backend structs ──

export interface AppConfig {
  download_path: string;
  download_dir?: string;
  filename_template: string;
  max_concurrent: number;
  download_quality: string;
  auto_create_folder: boolean;
  folder_name_template: string;
  save_metadata: boolean;
  proxy: string | null;
  cookie: string;
  theme: string;
  language: string;
  // Frontend-only computed field from get_config
  cookie_set?: boolean;
}

export interface AuthorInfo {
  uid: string;
  sec_uid: string;
  nickname: string;
  avatar_thumb: string;
  avatar_medium: string;
  signature: string;
  follower_count: number;
  following_count: number;
  aweme_count: number;
  favoriting_count: number;
  is_follow: boolean;
  verify_status: number;
  unique_id: string;
}

export interface VideoData {
  preview_addr: string | null;
  play_addr: string;
  play_addr_h264: string | null;
  play_addr_lowbr: string | null;
  download_addr: string | null;
  cover: string;
  dynamic_cover: string;
  origin_cover: string;
  width: number;
  height: number;
  duration: number;
  ratio: string;
}

export interface Statistics {
  play_count: number;
  digg_count: number;
  comment_count: number;
  share_count: number;
  collect_count: number;
  forward_count: number;
}

export interface MusicInfo {
  title: string;
  author: string;
  play_url: string;
  cover: string;
  duration: number;
}

export interface VideoMediaUrl {
  type?: string;
  url: string;
}

export interface VideoInfo {
  aweme_id: string;
  desc: string;
  create_time: number;
  author: AuthorInfo;
  video: VideoData;
  statistics: Statistics;
  media_urls?: VideoMediaUrl[] | null;
  image_urls: string[] | null;
  images?: string[] | null;
  live_photo_urls?: string[] | null;
  live_photos?: string[] | null;
  has_live_photo?: boolean;
  is_image: boolean;
  media_type: string;
  raw_media_type?: string | number | null;
  bgm_url?: string | null;
  cover_url?: string | null;
  music: MusicInfo | null;
}

// Alias for backward compat in components
export type VideoItem = VideoInfo;

export interface UserInfo {
  uid: string;
  nickname: string;
  avatar_thumb: string;
  avatar_medium: string;
  avatar_larger: string;
  signature: string;
  follower_count: number;
  following_count: number;
  total_favorited: number;
  aweme_count: number;
  favoriting_count: number;
  is_follow: boolean;
  sec_uid: string;
  unique_id: string;
  verify_status: number;
}

export interface SearchResult {
  users: UserInfo[];
}

export interface ApiResponse {
  success: boolean;
  message?: string;
}

export interface SearchUserResponse extends ApiResponse {
  type?: "single" | "multiple";
  user?: UserInfo;
  users?: UserInfo[];
  need_verify?: boolean;
  verify_url?: string;
}

export interface UserDetailResponse extends ApiResponse {
  user?: UserInfo;
}

export interface UserVideosResponse extends ApiResponse {
  videos?: VideoInfo[];
  has_more?: boolean;
  cursor?: number;
  total_count?: number;
}

export interface VideoDetailResponse extends ApiResponse {
  video?: VideoInfo;
}

export interface LinkParseResponse extends ApiResponse {
  type?: string;
  user?: UserInfo;
  video?: VideoInfo;
  videos?: VideoInfo[];
}

export interface RecommendedResponse extends ApiResponse {
  videos?: VideoInfo[];
  cursor?: number;
  has_more?: boolean;
}

export interface LikedVideosResponse extends ApiResponse {
  data?: VideoInfo[];
  count?: number;
}

export interface LikedAuthorsResponse extends ApiResponse {
  data?: UserInfo[];
  count?: number;
}

export interface DownloadProgress {
  task_id: string;
  desc?: string;
  display_name?: string;
  progress: number;
  completed?: number;
  total?: number;
  status: string;
  error?: string;
  message?: string;
}

export interface HistoryItem {
  id: string;
  aweme_id?: string;
  filename: string;
  title?: string;
  path: string;
  file_path?: string;
  author: string;
  desc: string;
  size: number;
  file_size?: number;
  timestamp: number;
  create_time?: number;
  file_type: string;
  media_type?: string;
  cover?: string;
  author_id?: string;
}

export interface CookieStatus {
  valid: boolean;
  user_name: string | null;
  user_id: string | null;
  expires_at: number | null;
  message: string;
}

type LikedVideoMediaUrl = VideoMediaUrl;

interface LikedVideoAuthorRaw {
  nickname?: string;
  sec_uid?: string;
  avatar_thumb?: string;
}

interface LikedVideoItemRaw {
  aweme_id?: string;
  desc?: string;
  create_time?: number;
  digg_count?: number;
  comment_count?: number;
  share_count?: number;
  cover_url?: string;
  media_type?: string;
  media_urls?: LikedVideoMediaUrl[];
  bgm_url?: string | null;
  author?: LikedVideoAuthorRaw;
}

function buildEmptyVideoData(): VideoData {
  return {
    preview_addr: null,
    play_addr: "",
    play_addr_h264: null,
    play_addr_lowbr: null,
    download_addr: null,
    cover: "",
    dynamic_cover: "",
    origin_cover: "",
    width: 0,
    height: 0,
    duration: 0,
    ratio: "",
  };
}

function buildEmptyStatistics(): Statistics {
  return {
    play_count: 0,
    digg_count: 0,
    comment_count: 0,
    share_count: 0,
    collect_count: 0,
    forward_count: 0,
  };
}

function extractUrl(value: unknown): string {
  if (!value) return "";
  if (typeof value === "string") return value.trim();
  if (Array.isArray(value)) {
    for (const item of value) {
      const url = extractUrl(item);
      if (url) return url;
    }
    return "";
  }
  if (typeof value === "object") {
    const record = value as Record<string, unknown>;
    return extractUrl(record.url || record.play_url || record.play_addr || record.url_list);
  }
  return "";
}

function normalizeMediaType(type: unknown, fallback = "video"): string {
  const normalized = String(type || fallback).trim().toLowerCase();
  if (normalized === "livephoto") return "live_photo";
  if (normalized === "live-photo") return "live_photo";
  if (normalized === "image" || normalized === "live_photo" || normalized === "video") {
    return normalized;
  }
  return fallback;
}

function normalizeMediaUrls(value: unknown): VideoMediaUrl[] {
  if (!Array.isArray(value)) return [];

  return value
    .map((item) => {
      if (!item) return null;
      if (typeof item === "string") {
        const url = extractUrl(item);
        return url ? { type: "video", url } : null;
      }
      if (typeof item !== "object") return null;
      const record = item as Record<string, unknown>;
      const url = extractUrl(record.url || record.play_url || record.play_addr || record.url_list);
      if (!url) return null;
      return {
        type: normalizeMediaType(record.type),
        url,
      };
    })
    .filter(Boolean) as VideoMediaUrl[];
}

function uniqueMediaUrls(urls: VideoMediaUrl[]): VideoMediaUrl[] {
  const seen = new Set<string>();
  const items: VideoMediaUrl[] = [];

  for (const item of urls) {
    const url = (item.url || "").trim();
    if (!url || seen.has(`${item.type || "video"}::${url}`)) continue;
    seen.add(`${item.type || "video"}::${url}`);
    items.push({
      type: normalizeMediaType(item.type),
      url,
    });
  }

  return items;
}

function normalizeMediaUrlsFromVideo(
  explicitMediaUrls: VideoMediaUrl[],
  livePhotoUrls: string[],
  imageUrls: string[],
  fallbackVideoUrl: string,
  mediaType: string
): VideoMediaUrl[] {
  if (explicitMediaUrls.length > 0) {
    return uniqueMediaUrls(explicitMediaUrls);
  }

  const items: VideoMediaUrl[] = [];

  for (const url of livePhotoUrls) {
    if (!url.trim()) continue;
    items.push({ type: "live_photo", url });
  }

  for (const url of imageUrls) {
    if (!url.trim()) continue;
    items.push({ type: "image", url });
  }

  if (items.length === 0 && fallbackVideoUrl.trim()) {
    items.push({
      type: normalizeMediaType(mediaType, "video"),
      url: fallbackVideoUrl.trim(),
    });
  }

  return uniqueMediaUrls(items);
}

export function normalizeLikedVideo(item: unknown): VideoInfo | null {
  if (!item || typeof item !== "object") return null;

  const candidate = item as Partial<VideoInfo> & LikedVideoItemRaw;
  if (candidate.aweme_id && candidate.video && candidate.statistics) {
    const normalized = normalizeVideo(candidate);
    return normalized || (candidate as VideoInfo);
  }

  const mediaUrls = uniqueMediaUrls(normalizeMediaUrls(candidate.media_urls));
  const imageUrls = mediaUrls.filter((media) => media.type === "image").map((media) => media.url);
  const livePhotoUrls = mediaUrls.filter((media) => media.type === "live_photo").map((media) => media.url);
  const primaryVideoUrl = mediaUrls.find((media) => media.type === "video")?.url || "";
  const cover = candidate.cover_url || imageUrls[0] || "";
  const mediaType = String(candidate.media_type || (imageUrls.length > 0 ? "image" : "video"));
  const isImage = mediaType === "image" || mediaType === "mixed" || mediaType === "live_photo";

  return {
    aweme_id: candidate.aweme_id || "",
    desc: candidate.desc || "",
    create_time: candidate.create_time || 0,
    author: {
      uid: "",
      sec_uid: candidate.author?.sec_uid || "",
      nickname: candidate.author?.nickname || "",
      avatar_thumb: candidate.author?.avatar_thumb || "",
      avatar_medium: candidate.author?.avatar_thumb || "",
      signature: "",
      follower_count: 0,
      following_count: 0,
      aweme_count: 0,
      favoriting_count: 0,
      is_follow: false,
      verify_status: 0,
      unique_id: "",
    },
    video: {
      ...buildEmptyVideoData(),
      play_addr: primaryVideoUrl || livePhotoUrls[0] || "",
      download_addr: primaryVideoUrl || livePhotoUrls[0] || null,
      cover,
      dynamic_cover: cover,
      origin_cover: cover,
    },
    statistics: {
      ...buildEmptyStatistics(),
      digg_count: candidate.digg_count || 0,
      comment_count: candidate.comment_count || 0,
      share_count: candidate.share_count || 0,
    },
    image_urls: imageUrls.length > 0 ? imageUrls : null,
    images: imageUrls.length > 0 ? imageUrls : null,
    live_photo_urls: livePhotoUrls.length > 0 ? livePhotoUrls : null,
    live_photos: livePhotoUrls.length > 0 ? livePhotoUrls : null,
    has_live_photo: livePhotoUrls.length > 0,
    is_image: isImage,
    media_type: mediaType,
    media_urls: mediaUrls.length > 0 ? mediaUrls : null,
    bgm_url: candidate.bgm_url || null,
    cover_url: cover || null,
    music: candidate.bgm_url
      ? {
          title: "抖音原声",
          author: candidate.author?.nickname || "",
          play_url: candidate.bgm_url,
          cover,
          duration: 0,
        }
      : null,
  };
}

function normalizeUser(user: unknown): UserInfo {
  const source = user && typeof user === "object" ? (user as Partial<UserInfo>) : {};
  return {
    uid: source.uid || "",
    sec_uid: source.sec_uid || "",
    nickname: source.nickname || "",
    avatar_thumb: source.avatar_thumb || source.avatar_medium || source.avatar_larger || "",
    avatar_medium: source.avatar_medium || source.avatar_thumb || source.avatar_larger || "",
    avatar_larger: source.avatar_larger || source.avatar_medium || source.avatar_thumb || "",
    signature: source.signature || "",
    follower_count: source.follower_count || 0,
    following_count: source.following_count || 0,
    total_favorited: source.total_favorited || 0,
    aweme_count: source.aweme_count || 0,
    favoriting_count: source.favoriting_count || 0,
    is_follow: source.is_follow || false,
    unique_id: source.unique_id || "",
    verify_status: source.verify_status || 0,
  };
}

export function normalizeVideo(video: unknown): VideoInfo | null {
  if (!video || typeof video !== "object") return null;

  const source = video as Record<string, unknown>;
  const author = normalizeUser(source.author || source.user || {});
  const stats = (source.statistics && typeof source.statistics === "object")
    ? (source.statistics as Partial<Statistics>)
    : {};
  const videoRecord = source.video && typeof source.video === "object" ? (source.video as Record<string, unknown>) : {};
  const topLevelMediaUrls = normalizeMediaUrls(source.media_urls);
  const nestedMediaUrls = normalizeMediaUrls(videoRecord.media_urls);
  const mediaUrls = uniqueMediaUrls(topLevelMediaUrls.length > 0 ? topLevelMediaUrls : nestedMediaUrls);
  const imageUrls = uniqueMediaUrls([
    ...(
      Array.isArray(source.image_urls)
        ? (source.image_urls as unknown[]).map((item) => extractUrl(item)).filter(Boolean)
        : Array.isArray(source.images)
          ? (source.images as unknown[]).map((item) => extractUrl(item)).filter(Boolean)
          : []
    ).map((url) => ({ type: "image", url })),
    ...mediaUrls.filter((item) => item.type === "image"),
  ]).map((item) => item.url);
  const livePhotoUrls = uniqueMediaUrls([
    ...(
      Array.isArray(source.live_photo_urls)
        ? (source.live_photo_urls as unknown[]).map((item) => extractUrl(item)).filter(Boolean)
        : Array.isArray(source.live_photos)
          ? (source.live_photos as unknown[]).map((item) => extractUrl(item)).filter(Boolean)
          : []
    ).map((url) => ({ type: "live_photo", url })),
    ...mediaUrls.filter((item) => item.type === "live_photo"),
  ]).map((item) => item.url);
  const primaryMediaUrl =
    mediaUrls.find((item) => item.type === "video")?.url ||
    mediaUrls.find((item) => item.type === "live_photo")?.url ||
    "";
  const cover = String(
    source.cover_url ||
      videoRecord.cover ||
      videoRecord.origin_cover ||
      videoRecord.dynamic_cover ||
      imageUrls[0] ||
      livePhotoUrls[0] ||
      ""
  );
  const playAddr = extractUrl(
    videoRecord.play_addr ||
    source.play_addr ||
    source.video_url ||
    source.url
  ) || primaryMediaUrl;
  const previewAddr = extractUrl(
    source.preview_addr ||
    source.play_addr_lowbr ||
    source.play_addr_h264 ||
      videoRecord.preview_addr ||
      videoRecord.play_addr_lowbr ||
      videoRecord.play_addr_h264
  );
  const duration = Number(source.duration || videoRecord.duration || 0);
  const musicSource = source.music && typeof source.music === "object" ? (source.music as Record<string, unknown>) : null;
  const musicPlayUrl = extractUrl(
    source.bgm_url ||
      source.music_url ||
      source.music_play_url ||
      source.music_play_addr ||
      musicSource?.play_url
  );
  const mediaType = String(source.media_type || source.raw_media_type || (imageUrls.length > 0 ? "image" : "video"));
  const isImage = Boolean(source.is_image || mediaType === "image" || mediaType === "mixed" || mediaType === "live_photo" || imageUrls.length > 0);
  const rawMediaType =
    typeof source.raw_media_type === "string" || typeof source.raw_media_type === "number"
      ? source.raw_media_type
      : null;
  const normalizedMediaUrls = normalizeMediaUrlsFromVideo(
    mediaUrls,
    livePhotoUrls,
    imageUrls,
    playAddr || previewAddr || livePhotoUrls[0] || "",
    mediaType
  );

  return {
    aweme_id: String(source.aweme_id || ""),
    desc: String(source.desc || ""),
    create_time: Number(source.create_time || 0),
    author,
    video: {
      preview_addr: previewAddr || null,
      play_addr: playAddr || previewAddr || livePhotoUrls[0] || "",
      play_addr_h264: null,
      play_addr_lowbr: null,
      download_addr: playAddr || previewAddr || livePhotoUrls[0] || null,
      cover,
      dynamic_cover: String(source.dynamic_cover || cover),
      origin_cover: String(source.origin_cover || cover),
      width: Number(videoRecord.width || source.width || 0),
      height: Number(videoRecord.height || source.height || 0),
      duration,
      ratio: String(videoRecord.ratio || source.ratio || ""),
    },
    statistics: {
      play_count: Number(stats.play_count || 0),
      digg_count: Number(source.digg_count || stats.digg_count || 0),
      comment_count: Number(source.comment_count || stats.comment_count || 0),
      share_count: Number(source.share_count || stats.share_count || 0),
      collect_count: Number(stats.collect_count || 0),
      forward_count: Number(stats.forward_count || 0),
    },
    image_urls: imageUrls.length > 0 ? imageUrls : null,
    images: imageUrls.length > 0 ? imageUrls : null,
    live_photo_urls: livePhotoUrls.length > 0 ? livePhotoUrls : null,
    live_photos: livePhotoUrls.length > 0 ? livePhotoUrls : null,
    has_live_photo: Boolean(source.has_live_photo || livePhotoUrls.length > 0),
    is_image: isImage,
    media_type: mediaType,
    raw_media_type: rawMediaType,
    media_urls: normalizedMediaUrls.length > 0 ? normalizedMediaUrls : null,
    bgm_url: musicPlayUrl || null,
    cover_url: cover || null,
    music: musicPlayUrl
      ? {
          title: String(musicSource?.title || source.music_title || ""),
          author: String(musicSource?.author || source.music_author || ""),
          play_url: musicPlayUrl,
          cover: String(musicSource?.cover || musicSource?.cover_thumb || ""),
          duration: Number(musicSource?.duration || source.music_duration || 0),
        }
      : null,
  };
}

export function normalizeVideos(videos: unknown): VideoInfo[] {
  if (!Array.isArray(videos)) return [];
  return videos.map(normalizeVideo).filter(Boolean) as VideoInfo[];
}

export function getErrorMessage(error: unknown, fallback: string): string {
  if (error instanceof Error && error.message) return error.message;
  if (typeof error === "string" && error.trim()) return error;
  return fallback;
}

// ── Tauri event listener ──

type TauriUnlisten = () => void;

export async function listenEvent<T>(event: string, handler: (payload: T) => void): Promise<TauriUnlisten> {
  const tauriListen = (window as Window & {
    __TAURI__?: { event?: { listen?: (event: string, cb: (ev: { payload: T }) => void) => Promise<TauriUnlisten> } };
  }).__TAURI__?.event?.listen;

  if (!tauriListen) {
    return () => {};
  }

  return tauriListen(event, (ev) => handler(ev.payload));
}

// ── Tauri invoke wrappers ──

export async function initClient(): Promise<{ success: boolean }> {
  return invoke("init_client");
}

export async function getAppVersion(): Promise<string> {
  return invoke("get_app_version");
}

export async function checkUpdate(): Promise<{ success: boolean; has_update: boolean; version?: string; current_version?: string; notes?: string; message?: string }> {
  return invoke("check_update");
}

export async function downloadUpdate(): Promise<{ success: boolean; message: string }> {
  return invoke("download_update");
}

export async function restartApp(): Promise<void> {
  return invoke("restart_app");
}

// Config

export async function getConfig(): Promise<AppConfig> {
  return invoke("get_config");
}

export async function saveConfig(config: Partial<AppConfig>): Promise<{ success: boolean; message: string }> {
  const current = await getConfig().catch(() => ({} as Partial<AppConfig>));
  const nextConfig: AppConfig = {
    download_path: config.download_path ?? config.download_dir ?? current.download_path ?? current.download_dir ?? "",
    filename_template: config.filename_template ?? current.filename_template ?? "{author}_{title}_{date}",
    max_concurrent: config.max_concurrent ?? current.max_concurrent ?? 3,
    download_quality: config.download_quality ?? current.download_quality ?? "auto",
    auto_create_folder: config.auto_create_folder ?? current.auto_create_folder ?? true,
    folder_name_template: config.folder_name_template ?? current.folder_name_template ?? "{author}",
    save_metadata: config.save_metadata ?? current.save_metadata ?? true,
    proxy: config.proxy ?? current.proxy ?? null,
    cookie: config.cookie ?? "",
    theme: config.theme ?? current.theme ?? "dark",
    language: config.language ?? current.language ?? "zh-CN",
  };
  return invoke("save_config", { config: nextConfig });
}

export async function selectDirectory(): Promise<string | null> {
  return invoke("select_directory");
}

// Search & User

export async function searchUser(keyword: string): Promise<SearchUserResponse> {
  const result = await invoke<SearchUserResponse>("search_user", { keyword });
  return {
    ...result,
    user: result.user ? normalizeUser(result.user) : undefined,
    users: Array.isArray(result.users) ? result.users.map(normalizeUser) : undefined,
  };
}

export async function getUserDetail(secUid: string, nickname?: string): Promise<UserDetailResponse> {
  const result = await invoke<UserDetailResponse>("get_user_detail", {
    secUid,
    sec_uid: secUid,
    nickname,
  });
  return {
    ...result,
    user: result.user ? normalizeUser(result.user) : undefined,
  };
}

export async function getUserVideos(secUid: string, count: number, cursor: number): Promise<UserVideosResponse> {
  const result = await invoke<UserVideosResponse & { videos?: unknown[] }>("get_user_videos", {
    secUid,
    sec_uid: secUid,
    count,
    cursor,
  });
  return {
    ...result,
    videos: normalizeVideos(result.videos),
  };
}

export async function getVideoDetail(awemeId: string): Promise<VideoDetailResponse> {
  const result = await invoke<VideoDetailResponse & { video?: unknown }>("get_video_detail", {
    awemeId,
    aweme_id: awemeId,
  });
  return {
    ...result,
    video: normalizeVideo(result.video) || undefined,
  };
}

// Links

export async function parseUrl(url: string): Promise<VideoInfo> {
  const result = await invoke<unknown>("parse_url", { url });
  return normalizeVideo(result) || (result as VideoInfo);
}

export async function parseLink(link: string): Promise<LinkParseResponse> {
  const result = await invoke<LinkParseResponse & { video?: unknown; videos?: unknown[]; user?: unknown }>("parse_link", { link });
  return {
    ...result,
    user: result.user ? normalizeUser(result.user) : undefined,
    video: normalizeVideo(result.video) || undefined,
    videos: normalizeVideos(result.videos),
  };
}

// Download

export async function downloadVideo(video: VideoInfo): Promise<ApiResponse & { task_id?: string }> {
  return invoke("download_video", { video });
}

export async function downloadUserVideos(
  secUid: string,
  nickname: string,
  awemeCount: number
): Promise<ApiResponse & { task_id?: string; total_videos?: number; nickname?: string }> {
  return invoke("download_user_videos", {
    secUid,
    sec_uid: secUid,
    nickname,
    awemeCount,
    aweme_count: awemeCount,
  });
}

export async function downloadLikedVideos(count: number): Promise<{ success: boolean; message: string }> {
  return invoke("download_liked_videos", { count });
}

export async function downloadLikedAuthors(count: number): Promise<{ success: boolean; message: string }> {
  return invoke("download_liked_authors", { count });
}

export async function addDownloadTask(video: VideoInfo, savePath?: string): Promise<string> {
  return invoke("add_download_task", { video, savePath, save_path: savePath });
}

export async function startDownload(taskId: string): Promise<void> {
  return invoke("start_download", { taskId, task_id: taskId });
}

export async function getDownloadTasks(): Promise<unknown[]> {
  const result = await invoke<{ success: boolean; tasks?: unknown[] }>("get_download_tasks");
  return result.tasks || [];
}

export async function cancelDownloadTask(taskId: string): Promise<ApiResponse> {
  return invoke("cancel_download_task", { taskId, task_id: taskId });
}

export async function removeDownloadTask(taskId: string): Promise<void> {
  return invoke("remove_download_task", { taskId, task_id: taskId });
}

export async function pauseDownload(taskId: string): Promise<ApiResponse> {
  return invoke("pause_download", { taskId, task_id: taskId });
}

export async function resumeDownload(taskId: string): Promise<ApiResponse> {
  return invoke("resume_download", { taskId, task_id: taskId });
}

// Feed & Likes

export async function getRecommended(cursor: number, count: number): Promise<RecommendedResponse> {
  const result = await invoke<RecommendedResponse & { videos?: unknown[] }>("get_recommended", { cursor, count });
  return {
    ...result,
    videos: normalizeVideos(result.videos),
  };
}

export async function getLikedVideos(
  count: number,
  secUid = "",
  cursor = 0
): Promise<LikedVideosResponse> {
  const result = await invoke<LikedVideosResponse & { data?: unknown[] }>("get_liked_videos", {
    count,
    secUid,
    sec_uid: secUid,
    cursor,
  });

  return {
    ...result,
    data: Array.isArray(result.data)
      ? result.data.map(normalizeLikedVideo).filter(Boolean) as VideoInfo[]
      : [],
  };
}

export async function getLikedAuthors(count: number): Promise<LikedAuthorsResponse> {
  const result = await invoke<LikedAuthorsResponse & { data?: unknown[] }>("get_liked_authors", { count });
  return {
    ...result,
    data: Array.isArray(result.data) ? result.data.map(normalizeUser) : [],
  };
}

export async function getComments(awemeId: string, count: number, cursor?: number): Promise<unknown> {
  return invoke("get_comments", { awemeId, count, cursor });
}

// Cookie

export async function verifyCookie(): Promise<CookieStatus> {
  return invoke("verify_cookie");
}

export async function cookieBrowserLogin(timeout?: number, browser?: string): Promise<{ success: boolean; message: string }> {
  return invoke("cookie_browser_login", { timeout, browser });
}

export async function cancelCookieBrowserLogin(): Promise<{ success: boolean; message: string }> {
  return invoke("cancel_cookie_browser_login");
}

export async function openVerifyBrowser(targetUrl?: string): Promise<{ success: boolean; message: string }> {
  return invoke("open_verify_browser", { targetUrl, target_url: targetUrl });
}

// History

function normalizeHistoryItem(value: unknown): HistoryItem | null {
  if (!value || typeof value !== "object") return null;
  const item = value as Record<string, unknown>;
  const awemeId = String(item.aweme_id || item.id || "").trim();
  const title = String(item.title || item.filename || item.desc || awemeId || "未命名作品").trim();
  const filePath = String(item.file_path || item.path || "").trim();
  const fileSize = Number(item.file_size ?? item.size ?? 0) || 0;
  const createTime = Number(item.create_time ?? item.timestamp ?? 0) || 0;
  const mediaType = String(item.media_type || item.file_type || "").trim();

  return {
    id: awemeId || filePath || title,
    aweme_id: awemeId,
    filename: title,
    title,
    path: filePath,
    file_path: filePath,
    author: String(item.author || "").trim(),
    author_id: String(item.author_id || "").trim(),
    desc: title,
    size: fileSize,
    file_size: fileSize,
    timestamp: createTime,
    create_time: createTime,
    file_type: mediaType,
    media_type: mediaType,
    cover: String(item.cover || "").trim(),
  };
}

export async function getHistory(): Promise<HistoryItem[]> {
  const result = await invoke<{ success: boolean; items?: unknown[] }>("get_history");
  return (result.items || []).map(normalizeHistoryItem).filter(Boolean) as HistoryItem[];
}

export async function listDownloadFiles(): Promise<HistoryItem[]> {
  const result = await invoke<{ success: boolean; items?: unknown[] }>("list_download_files");
  return (result.items || []).map(normalizeHistoryItem).filter(Boolean) as HistoryItem[];
}

export async function clearHistory(): Promise<void> {
  return invoke("clear_history");
}

export async function deleteHistory(id: string): Promise<void> {
  return invoke("delete_history", { awemeId: id, aweme_id: id });
}

export async function addHistory(entry: Omit<HistoryItem, "id">): Promise<void> {
  return invoke("add_history", { entry });
}

// File operations

export async function openFile(path: string): Promise<void> {
  return invoke("open_file", { path });
}

export async function openDownloadDirectory(): Promise<void> {
  return invoke("open_download_directory");
}

export async function openFileLocation(path: string): Promise<void> {
  return invoke("open_file_location", { path });
}

export async function deleteFile(path: string): Promise<void> {
  return invoke("delete_file", { path });
}
