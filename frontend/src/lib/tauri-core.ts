// ═══════════════════════════════════════════════
// Tauri runtime detection & invoke infrastructure
// ═══════════════════════════════════════════════

import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen as tauriListen } from "@tauri-apps/api/event";

type TauriInvoke = <T>(command: string, args?: Record<string, unknown>) => Promise<T>;
type BrowserSocketListener = (payload: unknown) => void;
type BrowserSocket = {
  on: (event: string, listener: BrowserSocketListener) => void;
  off: (event: string, listener: BrowserSocketListener) => void;
  connected?: boolean;
};

declare global {
  interface Window {
    __TAURI__?: {
      core?: {
        invoke?: TauriInvoke;
      };
      event?: {
        listen?: <T>(event: string, cb: (ev: { payload: T }) => void) => Promise<() => void>;
      };
    };
    __TAURI_INTERNALS__?: unknown;
    io?: (options?: { transports?: string[] }) => BrowserSocket;
    SOCKET_TRANSPORTS?: string[];
  }
}

export function isTauriRuntime() {
  return Boolean(window.__TAURI__?.core?.invoke || window.__TAURI_INTERNALS__);
}

export function invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  return invokeWithCookieInvalidEvent(command, args, true);
}

export function invokeLocal<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  return invokeWithCookieInvalidEvent(command, args, false);
}

function invokeWithCookieInvalidEvent<T>(
  command: string,
  args: Record<string, unknown> | undefined,
  emitCookieInvalidEvent: boolean
): Promise<T> {
  const invokeFn = window.__TAURI__?.core?.invoke || tauriInvoke;

  if (!isTauriRuntime()) {
    return Promise.reject(new Error("Tauri API unavailable"));
  }

  return invokeFn<T>(command, args)
    .then((result) => {
      if (emitCookieInvalidEvent) {
        emitCookieInvalidIfNeeded(result);
      }
      return result;
    })
    .catch((error) => {
      if (emitCookieInvalidEvent) {
        emitCookieInvalidFromError(error);
      }
      throw error;
    });
}

export function emitCookieInvalidIfNeeded(payload: unknown) {
  if (!payload || typeof payload !== "object") return;
  const data = payload as Record<string, unknown>;
  if (data.security_blocked) return;
  const message = String(data.message || "Cookie 已失效，请重新登录").trim();
  if (isLocalLoginPromptMessage(message)) return;
  if (data.need_login !== true) return;

  window.dispatchEvent(new CustomEvent("dy-cookie-invalid", { detail: { message } }));
}

export function emitCookieInvalidFromError(error: unknown) {
  if (!error || typeof error !== "object") return;
  emitCookieInvalidIfNeeded(error);
}

export function isLocalLoginPromptMessage(message: string) {
  return /请先设置\s*Cookie|未配置\s*Cookie|请登录后获取(?:点赞视频|收藏视频|收藏合集)/i.test(message);
}

export function shouldUseBrowserBridge() {
  return !isTauriRuntime();
}

let browserSocket: BrowserSocket | null = null;

export function getBrowserSocket() {
  if (isTauriRuntime()) return null;
  if (browserSocket) return browserSocket;
  if (typeof window.io !== "function") return null;

  browserSocket = window.io({
    transports:
      Array.isArray(window.SOCKET_TRANSPORTS) && window.SOCKET_TRANSPORTS.length > 0
        ? window.SOCKET_TRANSPORTS
        : ["websocket", "polling"],
  });

  return browserSocket;
}

type RequestJsonOptions = RequestInit & {
  suppressCookieInvalidEvent?: boolean;
};

export async function requestJson<T>(path: string, init: RequestJsonOptions = {}): Promise<T> {
  const { suppressCookieInvalidEvent, ...fetchInit } = init;
  const headers = new Headers(fetchInit.headers || {});
  if (!headers.has("Content-Type") && fetchInit.body) {
    headers.set("Content-Type", "application/json");
  }

  const response = await fetch(path, {
    credentials: "same-origin",
    ...fetchInit,
    headers,
  });

  const contentType = response.headers.get("content-type") || "";
  const data = contentType.includes("application/json")
    ? await response.json().catch(() => ({}))
    : {};

  if (!suppressCookieInvalidEvent) {
    emitCookieInvalidIfNeeded(data);
  }

  if (!response.ok) {
    const message =
      data && typeof data === "object" && "message" in data
        ? String((data as Record<string, unknown>).message || "").trim()
        : "";
    throw new Error(message || `${response.status} ${response.statusText}`.trim());
  }

  return data as T;
}

export async function writeTextWithBrowserClipboard(text: string): Promise<boolean> {
  if (window.navigator?.clipboard?.writeText) {
    try {
      await window.navigator.clipboard.writeText(text);
      return true;
    } catch {
      // Embedded WebViews can reject clipboard writes even after a click.
    }
  }

  const textarea = document.createElement("textarea");
  textarea.value = text;
  textarea.setAttribute("readonly", "");
  textarea.style.position = "fixed";
  textarea.style.left = "-9999px";
  textarea.style.top = "0";
  document.body.appendChild(textarea);
  textarea.select();
  textarea.setSelectionRange(0, textarea.value.length);

  try {
    return document.execCommand("copy");
  } catch {
    return false;
  } finally {
    document.body.removeChild(textarea);
  }
}

export function toFiniteNumber(value: unknown) {
  const n = Number(value);
  return Number.isFinite(n) ? n : undefined;
}
