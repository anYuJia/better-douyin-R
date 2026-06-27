// ═══════════════════════════════════════════════
// Media URL helpers
// ═══════════════════════════════════════════════

import { isTauriRuntime } from "./tauri-core";

export function mediaProxyUrl(url: string | null | undefined, mediaType = "image", extraParams: Record<string, string | undefined> = {}): string {
  const trimmed = (url || "").trim();
  if (!trimmed) return "";
  if (trimmed.startsWith("data:") || trimmed.startsWith("blob:")) return trimmed;
  if (
    trimmed.startsWith("/") ||
    trimmed.includes("127.0.0.1:39143/api/media/proxy") ||
    trimmed.includes("127.0.0.1:39143/api/local-media")
  ) {
    return trimmed;
  }

  try {
    const parsed = new URL(trimmed);
    if (parsed.protocol !== "http:" && parsed.protocol !== "https:") return trimmed;
    const base = isTauriRuntime()
      ? "http://127.0.0.1:39143/api/media/proxy"
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
    ? "http://127.0.0.1:39143/api/local-media"
    : "/api/local-media";
  return `${base}?path=${encodeURIComponent(trimmed)}`;
}
