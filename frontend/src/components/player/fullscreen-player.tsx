import { useState, useRef, useEffect, useCallback } from "react";
import { motion, AnimatePresence } from "framer-motion";
import {
  X,
  Heart,
  MessageCircle,
  Share2,
  Download,
  Music,
  ChevronUp,
  ChevronDown,
  Play,
  Pause,
} from "lucide-react";
import { cn, formatNumber, formatDuration } from "@/lib/utils";
import type { VideoItem } from "@/lib/tauri";

interface FullscreenPlayerProps {
  videos: VideoItem[];
  initialIndex?: number;
  open: boolean;
  onClose: () => void;
  onDownload?: (video: VideoItem) => void;
  onLoadMore?: () => void;
}

export function FullscreenPlayer({
  videos,
  initialIndex = 0,
  open,
  onClose,
  onDownload,
  onLoadMore,
}: FullscreenPlayerProps) {
  const [currentIndex, setCurrentIndex] = useState(initialIndex);
  const [playing, setPlaying] = useState(false);
  const [currentTime, setCurrentTime] = useState(0);
  const [duration, setDuration] = useState(0);
  const videoRef = useRef<HTMLVideoElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const touchStartY = useRef(0);
  const isWheelLocked = useRef(false);

  const video = videos[currentIndex];

  useEffect(() => {
    if (open) {
      setCurrentIndex(initialIndex);
      setPlaying(false);
    }
  }, [open, initialIndex]);

  useEffect(() => {
    const v = videoRef.current;
    if (!v) return;
    const onTime = () => setCurrentTime(v.currentTime);
    const onDuration = () => setDuration(v.duration);
    const onEnded = () => {
      setPlaying(false);
      handleNext();
    };
    v.addEventListener("timeupdate", onTime);
    v.addEventListener("loadedmetadata", onDuration);
    v.addEventListener("ended", onEnded);
    return () => {
      v.removeEventListener("timeupdate", onTime);
      v.removeEventListener("loadedmetadata", onDuration);
      v.removeEventListener("ended", onEnded);
    };
  });

  useEffect(() => {
    // Auto-play when video changes
    const v = videoRef.current;
    if (v && open) {
      v.currentTime = 0;
      v.play().then(() => setPlaying(true)).catch(() => setPlaying(false));
    }
  }, [currentIndex, open]);

  // Load more when near end
  useEffect(() => {
    if (currentIndex >= videos.length - 3 && onLoadMore) {
      onLoadMore();
    }
  }, [currentIndex, videos.length, onLoadMore]);

  const handleNext = useCallback(() => {
    if (currentIndex < videos.length - 1) {
      setCurrentIndex((i) => i + 1);
    }
  }, [currentIndex, videos.length]);

  const handlePrev = useCallback(() => {
    if (currentIndex > 0) {
      setCurrentIndex((i) => i - 1);
    }
  }, [currentIndex]);

  const togglePlay = useCallback(() => {
    const v = videoRef.current;
    if (!v) return;
    if (v.paused) {
      v.play();
      setPlaying(true);
    } else {
      v.pause();
      setPlaying(false);
    }
  }, []);

  // Keyboard navigation
  useEffect(() => {
    if (!open) return;
    const handleKey = (e: KeyboardEvent) => {
      switch (e.key) {
        case "ArrowUp":
        case "k":
          handlePrev();
          break;
        case "ArrowDown":
        case "j":
          handleNext();
          break;
        case " ":
          e.preventDefault();
          togglePlay();
          break;
        case "Escape":
          onClose();
          break;
      }
    };
    window.addEventListener("keydown", handleKey);
    return () => window.removeEventListener("keydown", handleKey);
  }, [open, handleNext, handlePrev, togglePlay, onClose]);

  // Wheel navigation
  const handleWheel = useCallback(
    (e: React.WheelEvent) => {
      if (isWheelLocked.current) return;
      isWheelLocked.current = true;
      setTimeout(() => { isWheelLocked.current = false; }, 500);

      if (e.deltaY > 30) handleNext();
      else if (e.deltaY < -30) handlePrev();
    },
    [handleNext, handlePrev]
  );

  // Touch navigation
  const handleTouchStart = (e: React.TouchEvent) => {
    touchStartY.current = e.touches[0].clientY;
  };

  const handleTouchEnd = (e: React.TouchEvent) => {
    const deltaY = touchStartY.current - e.changedTouches[0].clientY;
    if (deltaY > 80) handleNext();
    else if (deltaY < -80) handlePrev();
  };

  const progressPct = duration > 0 ? (currentTime / duration) * 100 : 0;

  if (!open || !video) return null;

  return (
    <AnimatePresence>
      <motion.div
        ref={containerRef}
        initial={{ opacity: 0 }}
        animate={{ opacity: 1 }}
        exit={{ opacity: 0 }}
        className="fixed inset-0 z-[9999] bg-black flex flex-col overflow-hidden"
        onWheel={handleWheel}
        onTouchStart={handleTouchStart}
        onTouchEnd={handleTouchEnd}
      >
        {/* Header */}
        <div className="absolute top-0 left-0 right-0 z-20 flex items-center justify-between p-4 bg-gradient-to-b from-black/60 to-transparent">
          <button
            onClick={onClose}
            className="w-10 h-10 rounded-full bg-white/10 hover:bg-white/20 flex items-center justify-center text-white cursor-pointer transition-all"
          >
            <X className="w-5 h-5" />
          </button>
          <span className="text-white text-[0.8rem] font-medium">
            {currentIndex + 1}/{videos.length}
          </span>
        </div>

        {/* Video */}
        <div className="flex-1 flex items-center justify-center relative" onClick={togglePlay}>
          <video
            ref={videoRef}
            src={video.video_url}
            className="max-w-full max-h-full object-contain"
          />
          {!playing && (
            <div className="absolute inset-0 flex items-center justify-center pointer-events-none">
              <div className="w-16 h-16 rounded-full bg-white/15 backdrop-blur-md flex items-center justify-center">
                <Play className="w-7 h-7 text-white fill-white ml-1" />
              </div>
            </div>
          )}
        </div>

        {/* Right Sidebar (TikTok-style) */}
        <div className="absolute right-4 bottom-32 z-20 flex flex-col items-center gap-5">
          <div className="w-12 h-12 rounded-full bg-accent overflow-hidden border-2 border-white shadow-md">
            {video.author_avatar && (
              <img src={video.author_avatar} alt="" className="w-full h-full object-cover" />
            )}
          </div>

          <SidebarButton
            icon={Heart}
            count={video.digg_count}
          />
          <SidebarButton
            icon={MessageCircle}
            count={video.comment_count}
          />
          <SidebarButton
            icon={Share2}
            count={video.share_count}
          />

          <button
            onClick={() => onDownload?.(video)}
            className="w-12 h-12 rounded-full bg-white/20 flex items-center justify-center text-white hover:bg-white/30 cursor-pointer transition-all"
          >
            <Download className="w-5 h-5" />
          </button>
        </div>

        {/* Bottom Info */}
        <div className="absolute left-4 right-20 bottom-20 z-20">
          <div className="text-white text-[15px] font-semibold mb-1.5 drop-shadow-md">
            @{video.author_nickname}
          </div>
          <div className="text-white/90 text-[13px] mb-2 line-clamp-2 drop-shadow-md">
            {video.desc}
          </div>
          {video.music_title && (
            <div className="flex items-center gap-2 text-white/70 text-[12px]">
              <Music className="w-3 h-3 animate-spin" style={{ animationDuration: "3s" }} />
              <span className="truncate">{video.music_title}</span>
            </div>
          )}
        </div>

        {/* Progress Bar */}
        <div className="absolute bottom-12 left-0 right-20 px-4 z-20">
          <div className="relative w-full h-[3px] bg-white/15 rounded-full cursor-pointer group hover:h-[5px] transition-all">
            <div
              className="absolute left-0 top-0 h-full bg-white/50 rounded-full transition-[width] duration-100"
              style={{ width: `${progressPct}%` }}
            />
            <div
              className="absolute top-1/2 -translate-y-1/2 w-3 h-3 bg-white rounded-full opacity-0 group-hover:opacity-100 transition-opacity shadow-md"
              style={{ left: `calc(${progressPct}% - 6px)` }}
            />
          </div>
          <div className="flex justify-between mt-1.5">
            <span className="text-white/40 text-[10px] font-mono">{formatDuration(currentTime)}</span>
            <span className="text-white/40 text-[10px] font-mono">{formatDuration(duration)}</span>
          </div>
        </div>

        {/* Nav Hint */}
        <div className="absolute bottom-3 left-1/2 -translate-x-1/2 z-20 flex flex-col items-center gap-0.5 text-white/40 text-[11px]">
          <ChevronUp className="w-4 h-4" />
          <span>上下滑动切换</span>
        </div>
      </motion.div>
    </AnimatePresence>
  );
}

function SidebarButton({ icon: Icon, count }: { icon: React.ElementType; count?: number }) {
  const [active, setActive] = useState(false);
  return (
    <div className="flex flex-col items-center gap-1">
      <button
        onClick={() => setActive(!active)}
        className="w-12 h-12 rounded-full flex items-center justify-center cursor-pointer transition-all"
      >
        <Icon
          className={cn(
            "w-7 h-7 drop-shadow-md transition-colors",
            active ? "text-accent fill-accent" : "text-white"
          )}
        />
      </button>
      {count !== undefined && (
        <span className="text-white/60 text-[11px] font-medium drop-shadow-md">
          {formatNumber(count)}
        </span>
      )}
    </div>
  );
}
