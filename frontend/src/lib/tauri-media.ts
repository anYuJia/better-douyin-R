// ═══════════════════════════════════════════════
// Media URL helpers
// ═══════════════════════════════════════════════

import { isTauriRuntime } from "./tauri-core";

const DEFAULT_MEDIA_PROXY_BASE_URL = "http://127.0.0.1:39143";
let mediaProxyBaseUrl = DEFAULT_MEDIA_PROXY_BASE_URL;

export function configureMediaProxyBaseUrl(baseUrl: string | null | undefined) {
  const normalized = String(baseUrl || "").trim().replace(/\/+$/, "");
  if (!normalized) return;

  try {
    const parsed = new URL(normalized);
    if (
      parsed.protocol === "http:" &&
      (parsed.hostname === "127.0.0.1" || parsed.hostname === "localhost")
    ) {
      mediaProxyBaseUrl = parsed.origin;
    }
  } catch {
    // Keep the preferred default when the desktop command returns an invalid URL.
  }
}

function isLocalMediaProxyUrl(url: string) {
  try {
    const parsed = new URL(url);
    return (
      (parsed.hostname === "127.0.0.1" || parsed.hostname === "localhost") &&
      (parsed.pathname === "/api/media/proxy" || parsed.pathname === "/api/local-media")
    );
  } catch {
    return false;
  }
}

export function mediaProxyUrl(url: string | null | undefined, mediaType = "image", extraParams: Record<string, string | undefined> = {}): string {
  const trimmed = (url || "").trim();
  if (!trimmed) return "";
  if (trimmed.startsWith("data:") || trimmed.startsWith("blob:")) return trimmed;
  if (trimmed.startsWith("/") || isLocalMediaProxyUrl(trimmed)) {
    return trimmed;
  }

  try {
    const parsed = new URL(trimmed);
    if (parsed.protocol !== "http:" && parsed.protocol !== "https:") return trimmed;
    const base = isTauriRuntime()
      ? `${mediaProxyBaseUrl}/api/media/proxy`
      : "/api/media/proxy";
    const extra = Object.entries(extraParams)
      .filter(([, value]) => value)
      .map(([key, value]) => `&${encodeURIComponent(key)}=${encodeURIComponent(value || "")}`)
      .join("");
    return `${base}?url=${encodeURIComponent(trimmed)}&media_type=${encodeURIComponent(mediaType)}${extra}`;
  } catch {
    return trimmed;
  }
}

export function localFileAssetUrl(path: string | null | undefined): string {
  const trimmed = (path || "").trim();
  if (!trimmed) return "";
  if (trimmed.startsWith("http://") || trimmed.startsWith("https://")) return trimmed;
  const base = isTauriRuntime()
    ? `${mediaProxyBaseUrl}/api/local-media`
    : "/api/local-media";
  return `${base}?path=${encodeURIComponent(trimmed)}`;
}
