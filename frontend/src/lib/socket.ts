import { useEffect } from "react";
import { useDownloadStore, useLogStore } from "@/stores/app-store";

// ═══════════════════════════════════════════════
// WebSocket Connection Manager
// ═══════════════════════════════════════════════

interface SocketMessage {
  event: string;
  data: unknown;
}

let socketInstance: WebSocket | null = null;
let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
const listeners = new Set<(msg: SocketMessage) => void>();

export function useSocket() {
  const updateTask = useDownloadStore((s) => s.updateTask);
  const addLog = useLogStore((s) => s.addLog);

  useEffect(() => {
    const handleMessage = (msg: SocketMessage) => {
      switch (msg.event) {
        case "download_progress": {
          const d = msg.data as {
            task_id: string;
            filename: string;
            progress: number;
            speed: number;
            status: string;
            total_bytes?: number;
            downloaded_bytes?: number;
          };
          updateTask({
            id: d.task_id,
            filename: d.filename,
            progress: d.progress,
            speed: d.speed,
            status: d.status as "downloading" | "completed" | "error",
            totalBytes: d.total_bytes,
            downloadedBytes: d.downloaded_bytes,
          });
          break;
        }
        case "download_complete": {
          const d = msg.data as { task_id: string; filename: string; path: string };
          updateTask({ id: d.task_id, status: "completed" });
          addLog(`下载完成: ${d.filename}`, "success");
          break;
        }
        case "download_error": {
          const d = msg.data as { task_id: string; error: string };
          updateTask({ id: d.task_id, status: "error" });
          addLog(`下载失败: ${d.error}`, "error");
          break;
        }
        case "log": {
          const d = msg.data as { message: string; level: string };
          addLog(d.message, d.level as "info" | "success" | "error" | "warning");
          break;
        }
      }
    };

    listeners.add(handleMessage);

    if (!socketInstance || socketInstance.readyState === WebSocket.CLOSED) {
      connectSocket();
    }

    return () => {
      listeners.delete(handleMessage);
    };
  }, [updateTask, addLog]);
}

function connectSocket() {
  // Only connect when running inside Tauri
  if (typeof window === "undefined" || !("__TAURI__" in window)) return;

  try {
    socketInstance = new WebSocket("ws://127.0.0.1:39143/ws");

    socketInstance.onopen = () => {
      // Connected successfully
    };

    socketInstance.onmessage = (event) => {
      try {
        const msg = JSON.parse(event.data) as SocketMessage;
        listeners.forEach((fn) => fn(msg));
      } catch {
        // Ignore malformed messages
      }
    };

    socketInstance.onclose = () => {
      scheduleReconnect();
    };

    socketInstance.onerror = () => {
      socketInstance?.close();
    };
  } catch {
    // WebSocket not available (e.g., in browser preview)
  }
}

function scheduleReconnect() {
  if (reconnectTimer) return;
  reconnectTimer = setTimeout(() => {
    reconnectTimer = null;
    connectSocket();
  }, 3000);
}

export function disconnectSocket() {
  if (reconnectTimer) {
    clearTimeout(reconnectTimer);
    reconnectTimer = null;
  }
  socketInstance?.close();
  socketInstance = null;
}
