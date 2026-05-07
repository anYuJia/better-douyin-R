import { useCallback } from "react";
import { useDownloadStore, useLogStore } from "@/stores/app-store";
import type { VideoItem } from "@/lib/tauri";

// ═══════════════════════════════════════════════
// Download Hook
// ═══════════════════════════════════════════════

export function useDownloads() {
  const updateTask = useDownloadStore((s) => s.updateTask);
  const removeTask = useDownloadStore((s) => s.removeTask);
  const clearCompleted = useDownloadStore((s) => s.clearCompleted);
  const addLog = useLogStore((s) => s.addLog);

  const downloadVideo = useCallback(
    async (video: VideoItem) => {
      const taskId = video.aweme_id;
      addLog(`开始下载: ${video.desc?.slice(0, 30) || video.aweme_id}`, "info");

      updateTask({
        id: taskId,
        filename: `${video.author_nickname}_${video.aweme_id}.mp4`,
        progress: 0,
        status: "downloading",
        startTime: Date.now(),
      });

      try {
        // In real app: await invoke("download_video", { awemeId: video.aweme_id, videoUrl: video.video_url })
        // Progress updates come via WebSocket
        addLog(`下载完成: ${video.aweme_id}`, "success");
      } catch (e) {
        const msg = e instanceof Error ? e.message : "下载失败";
        updateTask({ id: taskId, status: "error" });
        addLog(msg, "error");
      }
    },
    [updateTask, addLog]
  );

  const downloadBatch = useCallback(
    async (videos: VideoItem[]) => {
      addLog(`批量下载 ${videos.length} 个视频`, "info");

      for (const video of videos) {
        updateTask({
          id: video.aweme_id,
          filename: `${video.author_nickname}_${video.aweme_id}.mp4`,
          progress: 0,
          status: "pending",
        });
      }

      try {
        // const items = videos.map(v => ({ aweme_id: v.aweme_id, url: v.video_url }))
        // await invoke("download_batch", { items })
        addLog("批量下载已提交", "success");
      } catch (e) {
        const msg = e instanceof Error ? e.message : "批量下载失败";
        addLog(msg, "error");
      }
    },
    [updateTask, addLog]
  );

  const cancelDownload = useCallback(
    (taskId: string) => {
      removeTask(taskId);
      addLog(`取消下载: ${taskId}`, "warning");
    },
    [removeTask, addLog]
  );

  return { downloadVideo, downloadBatch, cancelDownload, removeTask, clearCompleted };
}
