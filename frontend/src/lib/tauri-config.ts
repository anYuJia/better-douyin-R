// ═══════════════════════════════════════════════
// Config, accounts, auth, update, verify
// ═══════════════════════════════════════════════

import type {
  AppConfig,
  ApiResponse,
  CookieStatus,
} from "./contracts";
import { getErrorMessage } from "./normalizers";
import { invoke, invokeLocal, shouldUseBrowserBridge, requestJson } from "./tauri-core";

export async function initClient(): Promise<{ success: boolean }> {
  if (shouldUseBrowserBridge()) return { success: true };
  return invoke("init_client");
}

export async function getAppVersion(): Promise<string> {
  if (shouldUseBrowserBridge()) {
    const result = await requestJson<string | { version?: string }>("/api/get_app_version");
    return typeof result === "string" ? result : String(result?.version || "");
  }
  return invoke("get_app_version");
}

export async function checkUpdate(): Promise<{
  success: boolean;
  has_update: boolean;
  version?: string;
  current_version?: string;
  notes?: string;
  message?: string;
  html_url?: string;
  download_url?: string;
  asset_name?: string;
  asset_size?: number;
  portable?: boolean;
  install_mode?: string;
}> {
  if (shouldUseBrowserBridge()) {
    return requestJson("/api/check_update");
  }
  return invoke("check_update");
}

export async function downloadUpdate(): Promise<{
  success: boolean;
  message: string;
  mode?: string;
  portable?: boolean;
  install_mode?: string;
  restart_required?: boolean;
  download_url?: string;
  file_path?: string;
}> {
  if (shouldUseBrowserBridge()) {
    return requestJson("/api/download_update");
  }
  return invoke("download_update");
}

export async function restartApp(): Promise<void> {
  if (shouldUseBrowserBridge()) {
    const result = await requestJson<{ success?: boolean; message?: string }>("/api/restart_app");
    if (result && result.success === false) {
      throw new Error(result.message || "重启失败");
    }
    return;
  }
  return invoke("restart_app");
}

export async function getConfig(): Promise<AppConfig> {
  if (shouldUseBrowserBridge()) {
    const result = await requestJson<Record<string, unknown>>("/api/config");
    return {
      download_path: String(result.download_path || result.download_dir || ""),
      download_dir: String(result.download_dir || result.download_path || ""),
      filename_template: String(result.filename_template || "{title}"),
      max_concurrent: Number(result.max_concurrent || 3) || 3,
      download_quality: String(result.download_quality || "auto"),
      download_live_photo_video: Boolean(result.download_live_photo_video ?? true),
      download_live_photo_image: Boolean(result.download_live_photo_image ?? true),
      auto_create_folder: Boolean(result.auto_create_folder ?? true),
      folder_name_template: String(result.folder_name_template || "{author}"),
      save_metadata: Boolean(result.save_metadata ?? true),
      proxy: (result.proxy as string | null) ?? null,
      cookie: "",
      im_friend_sec_user_ids: Array.isArray(result.im_friend_sec_user_ids)
        ? result.im_friend_sec_user_ids.filter((item): item is string => typeof item === "string")
        : [],
      im_friend_include_all_users: Boolean(result.im_friend_include_all_users ?? false),
      im_friend_refresh_interval_seconds: Number(result.im_friend_refresh_interval_seconds || 30) || 30,
      theme: String(result.theme || "dark"),
      language: String(result.language || "zh-CN"),
      cookie_set: Boolean(result.cookie_set ?? false),
    };
  }
  return invoke("get_config");
}

export async function saveConfig(config: Partial<AppConfig>): Promise<{ success: boolean; message: string }> {
  const hasProxyPatch = Object.prototype.hasOwnProperty.call(config, "proxy");
  if (shouldUseBrowserBridge()) {
    const current = await getConfig().catch(() => ({} as Partial<AppConfig>));
    const payload: Record<string, unknown> = {
      download_dir: config.download_path ?? config.download_dir ?? current.download_path ?? current.download_dir ?? "",
      download_quality: config.download_quality ?? current.download_quality ?? "auto",
      download_live_photo_video: config.download_live_photo_video ?? current.download_live_photo_video ?? true,
      download_live_photo_image: config.download_live_photo_image ?? current.download_live_photo_image ?? true,
      max_concurrent: config.max_concurrent ?? current.max_concurrent ?? 3,
      filename_template: config.filename_template ?? current.filename_template ?? "{title}",
      folder_name_template: config.folder_name_template ?? current.folder_name_template ?? "{author}",
      auto_create_folder: config.auto_create_folder ?? current.auto_create_folder ?? true,
      im_friend_sec_user_ids: config.im_friend_sec_user_ids ?? current.im_friend_sec_user_ids ?? [],
      im_friend_include_all_users:
        config.im_friend_include_all_users ?? current.im_friend_include_all_users ?? false,
      im_friend_refresh_interval_seconds:
        config.im_friend_refresh_interval_seconds ?? current.im_friend_refresh_interval_seconds ?? 30,
      proxy: hasProxyPatch ? (config.proxy ?? null) : (current.proxy ?? null),
    };
    if (typeof config.cookie === "string") {
      payload.cookie = config.cookie;
    }
    return requestJson("/api/config", {
      method: "POST",
      body: JSON.stringify(payload),
    });
  }

  const current = await getConfig().catch(() => ({} as Partial<AppConfig>));
  const nextConfig: AppConfig = {
    download_path: config.download_path ?? config.download_dir ?? current.download_path ?? current.download_dir ?? "",
    filename_template: config.filename_template ?? current.filename_template ?? "{title}",
    max_concurrent: config.max_concurrent ?? current.max_concurrent ?? 3,
    download_quality: config.download_quality ?? current.download_quality ?? "auto",
    download_live_photo_video: config.download_live_photo_video ?? current.download_live_photo_video ?? true,
    download_live_photo_image: config.download_live_photo_image ?? current.download_live_photo_image ?? true,
    auto_create_folder: config.auto_create_folder ?? current.auto_create_folder ?? true,
    folder_name_template: config.folder_name_template ?? current.folder_name_template ?? "{author}",
    save_metadata: config.save_metadata ?? current.save_metadata ?? true,
    proxy: hasProxyPatch ? (config.proxy ?? null) : (current.proxy ?? null),
    cookie: config.cookie ?? "",
    im_friend_sec_user_ids: config.im_friend_sec_user_ids ?? current.im_friend_sec_user_ids ?? [],
    accounts: config.accounts ?? current.accounts ?? [],
    current_sec_uid: config.current_sec_uid ?? current.current_sec_uid ?? "",
    im_friend_include_all_users:
      config.im_friend_include_all_users ?? current.im_friend_include_all_users ?? false,
    im_friend_refresh_interval_seconds:
      config.im_friend_refresh_interval_seconds ?? current.im_friend_refresh_interval_seconds ?? 30,
    theme: config.theme ?? current.theme ?? "dark",
    language: config.language ?? current.language ?? "zh-CN",
  };
  return invoke("save_config", { config: nextConfig });
}

export async function logoutCookie(): Promise<{ success: boolean; message: string }> {
  if (shouldUseBrowserBridge()) {
    return saveConfig({ cookie: "" });
  }
  return invoke("logout_cookie");
}

export type AccountInfo = {
  sec_uid: string;
  nickname: string;
  avatar_thumb?: string;
  cookie?: string;
  is_valid?: boolean;
};

export type AccountsResponse = {
  success: boolean;
  accounts: AccountInfo[];
  current_sec_uid: string;
  message?: string;
};

const NON_AVATAR_URL_MARKERS = [
  "emblem",
  "logo",
  "badge",
  "icon",
  "sprite",
  "placeholder",
  "default-avatar",
  "default_avatar",
];
const AVATAR_URL_MARKERS = [
  "avatar",
  "aweme-avatar",
  "user-avatar",
  "avatar_",
  "avatar-",
  "300x300",
  "168x168",
  "100x100",
];

function sanitizeAvatarUrl(value: string | null | undefined): string {
  const url = String(value || "").trim();
  if (!url) return "";
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return "";
  }
  if (parsed.protocol !== "http:" && parsed.protocol !== "https:") return "";

  const lowered = url.toLowerCase();
  if (NON_AVATAR_URL_MARKERS.some((marker) => lowered.includes(marker))) return "";
  if (AVATAR_URL_MARKERS.some((marker) => lowered.includes(marker))) return url;

  const host = parsed.hostname.toLowerCase();
  const path = parsed.pathname.toLowerCase();
  if (
    ["douyinpic.com", "byteimg.com", "bytedance.com"].some((token) => host.includes(token)) &&
    [".jpg", ".jpeg", ".png", ".webp"].some((ext) => path.endsWith(ext))
  ) {
    return url;
  }
  return "";
}

export async function getAccounts(): Promise<AccountsResponse> {
  if (shouldUseBrowserBridge()) {
    return requestJson("/api/accounts");
  }
  return invoke("get_accounts");
}

export async function switchAccount(secUid: string): Promise<{ success: boolean; message: string; nickname?: string }> {
  clearVerifyCookieCache();
  if (shouldUseBrowserBridge()) {
    return requestJson("/api/accounts/switch", {
      method: "POST",
      body: JSON.stringify({ sec_uid: secUid }),
    });
  }
  return invoke("switch_account", { secUid, sec_uid: secUid });
}

export async function deleteAccount(secUid: string): Promise<{ success: boolean; message: string }> {
  clearVerifyCookieCache();
  if (shouldUseBrowserBridge()) {
    return requestJson("/api/accounts", {
      method: "DELETE",
      body: JSON.stringify({ sec_uid: secUid }),
    });
  }
  return invoke("delete_account", { secUid, sec_uid: secUid });
}

export async function addAccount(
  cookie: string
): Promise<{ success: boolean; message: string; nickname?: string; sec_uid?: string; avatar_thumb?: string }> {
  clearVerifyCookieCache();
  if (shouldUseBrowserBridge()) {
    return requestJson("/api/accounts/add", {
      method: "POST",
      body: JSON.stringify({ cookie }),
    });
  }
  return invoke("add_account", { cookie });
}

export async function selectDirectory(): Promise<string | null> {
  if (shouldUseBrowserBridge()) {
    const result = await requestJson<{ success: boolean; path?: string; message?: string }>("/api/select_directory", {
      method: "POST",
    });
    if (result.success) {
      return result.path || null;
    }
    const message = result.message || "选择目录失败";
    if (/取消/.test(message)) {
      return null;
    }
    throw new Error(message);
  }
  return invoke("select_directory");
}

let verifyCookieInFlight: Promise<CookieStatus | null> | null = null;
let lastVerifyCookieResult: CookieStatus | null = null;
let lastVerifyCookieTime = 0;

export async function verifyCookie(): Promise<CookieStatus> {
  const now = Date.now();
  if (lastVerifyCookieResult && (now - lastVerifyCookieTime < 300_000)) {
    return lastVerifyCookieResult;
  }
  if (verifyCookieInFlight) {
    const result = await verifyCookieInFlight;
    if (!result) throw new Error("Cookie 校验失败");
    return result;
  }
  verifyCookieInFlight = (async () => {
    let result: CookieStatus;
    if (shouldUseBrowserBridge()) {
      result = await requestJson<CookieStatus>("/api/verify_cookie", {
        suppressCookieInvalidEvent: true,
      });
    } else {
      result = await invokeLocal<CookieStatus>("verify_cookie");
    }
    if (result && result.valid) {
      lastVerifyCookieResult = result;
      lastVerifyCookieTime = Date.now();
    }
    return result;
  })();
  try {
    const result = await verifyCookieInFlight;
    if (!result) throw new Error("Cookie 校验失败");
    return result;
  } finally {
    verifyCookieInFlight = null;
  }
}

export function clearVerifyCookieCache() {
  lastVerifyCookieResult = null;
  lastVerifyCookieTime = 0;
}

export async function cookieBrowserLogin(timeout?: number, browser?: string, cookie?: string): Promise<{ success: boolean; message: string }> {
  clearVerifyCookieCache();
  if (shouldUseBrowserBridge()) {
    return requestJson("/api/cookie/browser_login", {
      method: "POST",
      body: JSON.stringify({ timeout, browser, cookie }),
    });
  }
  return invoke("cookie_browser_login", { timeout, browser, cookie });
}

export async function cancelCookieBrowserLogin(): Promise<{ success: boolean; message: string }> {
  if (shouldUseBrowserBridge()) {
    return requestJson("/api/cookie/browser_login/cancel", { method: "POST" });
  }
  return invoke("cancel_cookie_browser_login");
}

type VerifyBrowserResponse = {
  success: boolean;
  message: string;
  open_url?: string;
};

export async function openVerifyBrowser(targetUrl?: string): Promise<VerifyBrowserResponse> {
  if (shouldUseBrowserBridge()) {
    try {
      return await requestJson<VerifyBrowserResponse>("/api/open_verify_browser", {
        method: "POST",
        body: JSON.stringify({ target_url: targetUrl }),
      });
    } catch (error) {
      return {
        success: false,
        message: getErrorMessage(error, "无法打开应用内验证窗口，请通过桌面版启动后重试"),
        open_url: targetUrl,
      };
    }
  }
  return invoke<VerifyBrowserResponse>("open_verify_browser", { targetUrl, target_url: targetUrl });
}
