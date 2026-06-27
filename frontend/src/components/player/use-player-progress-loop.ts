import { useCallback, useRef } from "react";
import { finiteMediaTime, readMediaDuration } from "./player-utils";

interface UsePlayerProgressLoopProps {
  videoRef: React.RefObject<HTMLVideoElement | null>;
  setCurrentTime: React.Dispatch<React.SetStateAction<number>>;
  setDuration: React.Dispatch<React.SetStateAction<number>>;
}

export function usePlayerProgressLoop({
  videoRef,
  setCurrentTime,
  setDuration,
}: UsePlayerProgressLoopProps) {
  const videoProgressRafRef = useRef<number | null>(null);
  const progressSampleRef = useRef(0);

  const stopVideoProgressLoop = useCallback(() => {
    if (videoProgressRafRef.current === null) return;
    window.cancelAnimationFrame(videoProgressRafRef.current);
    videoProgressRafRef.current = null;
  }, []);

  const syncVideoProgress = useCallback((node: HTMLVideoElement) => {
    const nextTime = finiteMediaTime(node.currentTime);
    const nextDuration = readMediaDuration(node);
    setCurrentTime((current) => {
      if (nextTime > 0 || current <= 0) return nextTime;
      if (node.readyState < HTMLMediaElement.HAVE_CURRENT_DATA || nextDuration <= 0) {
        return current;
      }
      return nextTime;
    });
    if (nextDuration > 0) {
      setDuration(nextDuration);
    }
  }, [setCurrentTime, setDuration]);

  const startVideoProgressLoop = useCallback(() => {
    if (videoProgressRafRef.current !== null) return;

    const tick = () => {
      const node = videoRef.current;
      if (!node) {
        videoProgressRafRef.current = null;
        return;
      }

      const now = performance.now();
      if (now - progressSampleRef.current >= 50 || node.paused || node.ended) {
        progressSampleRef.current = now;
        syncVideoProgress(node);
      }

      if (!node.paused && !node.ended) {
        videoProgressRafRef.current = window.requestAnimationFrame(tick);
      } else {
        videoProgressRafRef.current = null;
      }
    };

    videoProgressRafRef.current = window.requestAnimationFrame(tick);
  }, [syncVideoProgress, videoRef]);

  return {
    stopVideoProgressLoop,
    syncVideoProgress,
    startVideoProgressLoop,
    videoProgressRafRef,
    progressSampleRef,
  };
}
