import { useState, useRef, useEffect, useCallback } from "react";
import { motion, AnimatePresence } from "framer-motion";
import {
  X,
  Play,
  Pause,
  Volume2,
  VolumeX,
  Download,
  Maximize2,
  SkipForward,
  SkipBack,
  Music,
} from "lucide-react";
import { cn, formatDuration } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import type { VideoItem } from "@/lib/tauri";

interface VideoPlayerProps {
  video: VideoItem | null;
  videos?: VideoItem[];
  open: boolean;
  onClose: () => void;
  onDownload?: (video: VideoItem) => void;
}

export function VideoPlayer({ video, videos = [], open, onClose, onDownload }: VideoPlayerProps) {
  const videoRef = useRef<HTMLVideoElement>(null);
  const [playing, setPlaying] = useState(false);
  const [currentTime, setCurrentTime] = useState(0);
  const [duration, setDuration] = useState(0);
  const [volume, setVolume] = useState(100);
  const [muted, setMuted] = useState(false);
  const [playbackRate, setPlaybackRate] = useState(1);
  const [showControls, setShowControls] = useState(true);
  const [showRatePanel, setShowRatePanel] = useState(false);
  const [showMusicPanel, setShowMusicPanel] = useState(false);
  const [currentIndex, setCurrentIndex] = useState(0);
  const hideTimer = useRef<ReturnType<typeof setTimeout>>(undefined);

  const activeVideo = video || videos[currentIndex] || null;

  useEffect(() => {
    if (!open) {
      setPlaying(false);
      setCurrentTime(0);
    }
  }, [open]);

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

  const toggleMute = useCallback(() => {
    const v = videoRef.current;
    if (!v) return;
    v.muted = !v.muted;
    setMuted(!muted);
  }, [muted]);

  const handleVolumeChange = useCallback((val: number) => {
    const v = videoRef.current;
    if (!v) return;
    v.volume = val / 100;
    setVolume(val);
    if (val === 0) {
      v.muted = true;
      setMuted(true);
    } else if (muted) {
      v.muted = false;
      setMuted(false);
    }
  }, [muted]);

  const handleSeek = useCallback((e: React.MouseEvent<HTMLDivElement>) => {
    const v = videoRef.current;
    if (!v || !duration) return;
    const rect = e.currentTarget.getBoundingClientRect();
    const pct = (e.clientX - rect.left) / rect.width;
    v.currentTime = pct * duration;
  }, [duration]);

  const handleRateChange = useCallback((rate: number) => {
    const v = videoRef.current;
    if (!v) return;
    v.playbackRate = rate;
    setPlaybackRate(rate);
    setShowRatePanel(false);
  }, []);

  const handleNext = useCallback(() => {
    if (videos.length > 0 && currentIndex < videos.length - 1) {
      setCurrentIndex((i) => i + 1);
    }
  }, [videos.length, currentIndex]);

  const handlePrev = useCallback(() => {
    if (currentIndex > 0) {
      setCurrentIndex((i) => i - 1);
    }
  }, [currentIndex]);

  const handleMouseMove = useCallback(() => {
    setShowControls(true);
    clearTimeout(hideTimer.current);
    hideTimer.current = setTimeout(() => {
      if (playing) setShowControls(false);
    }, 3000);
  }, [playing]);

  const progressPct = duration > 0 ? (currentTime / duration) * 100 : 0;

  const rates = [0.5, 0.75, 1, 1.25, 1.5, 2];

  if (!open || !activeVideo) return null;

  return (
    <AnimatePresence>
      <motion.div
        initial={{ opacity: 0 }}
        animate={{ opacity: 1 }}
        exit={{ opacity: 0 }}
        className="fixed inset-0 z-[9999] flex flex-col bg-black"
        onMouseMove={handleMouseMove}
      >
        {/* Header */}
        <motion.div
          initial={{ opacity: 0, y: -20 }}
          animate={{ opacity: showControls ? 1 : 0, y: showControls ? 0 : -20 }}
          transition={{ duration: 0.2 }}
          className="absolute top-0 left-0 right-0 z-10 flex items-center justify-between p-4 bg-gradient-to-b from-black/70 to-transparent"
        >
          <div className="flex-1 text-white text-[0.85rem] font-medium truncate mr-4">
            {activeVideo.desc}
          </div>
          {videos.length > 0 && (
            <span className="text-white/50 text-[0.75rem] mr-4 shrink-0">
              {currentIndex + 1}/{videos.length}
            </span>
          )}
          <button
            onClick={onClose}
            className="w-8 h-8 rounded-full bg-white/10 hover:bg-white/20 flex items-center justify-center text-white cursor-pointer transition-all"
          >
            <X className="w-4 h-4" />
          </button>
        </motion.div>

        {/* Video */}
        <div
          className="flex-1 flex items-center justify-center overflow-hidden cursor-pointer"
          onClick={togglePlay}
        >
          <video
            ref={videoRef}
            src={activeVideo.video?.play_addr}
            className="max-w-full max-h-full object-contain"
            autoPlay
          />
        </div>

        {/* Bottom Controls */}
        <motion.div
          initial={{ opacity: 0, y: 20 }}
          animate={{ opacity: showControls ? 1 : 0, y: showControls ? 0 : 20 }}
          transition={{ duration: 0.2 }}
          className="absolute bottom-0 left-0 right-0 z-10 bg-gradient-to-t from-black/70 to-transparent pt-16 pb-5 px-4"
        >
          {/* Progress Bar */}
          <div
            className="relative w-full h-[3px] bg-white/20 rounded-full cursor-pointer mb-3 group hover:h-[5px] transition-all"
            onClick={handleSeek}
          >
            <div
              className="absolute left-0 top-0 h-full bg-white/60 rounded-full transition-[width] duration-100"
              style={{ width: `${progressPct}%` }}
            />
            <div
              className="absolute top-1/2 -translate-y-1/2 w-3 h-3 bg-white rounded-full opacity-0 group-hover:opacity-100 transition-opacity shadow-md"
              style={{ left: `calc(${progressPct}% - 6px)` }}
            />
          </div>

          {/* Control Row */}
          <div className="flex items-center justify-between">
            {/* Left: Time */}
            <span className="text-white/50 text-[0.7rem] font-mono shrink-0">
              {formatDuration(currentTime)} / {formatDuration(duration)}
            </span>

            {/* Center: Play Controls */}
            <div className="flex items-center gap-3">
              {videos.length > 1 && (
                <button onClick={handlePrev} className="text-white/70 hover:text-white cursor-pointer transition-colors">
                  <SkipBack className="w-5 h-5" />
                </button>
              )}
              <button
                onClick={togglePlay}
                className="w-12 h-12 rounded-full bg-white/15 border border-white/20 flex items-center justify-center text-white hover:bg-white/25 cursor-pointer transition-all"
              >
                {playing ? <Pause className="w-5 h-5" /> : <Play className="w-5 h-5 ml-0.5" />}
              </button>
              {videos.length > 1 && (
                <button onClick={handleNext} className="text-white/70 hover:text-white cursor-pointer transition-colors">
                  <SkipForward className="w-5 h-5" />
                </button>
              )}
            </div>

            {/* Right: Extra Controls */}
            <div className="flex items-center gap-2">
              {/* Volume */}
              <div className="relative group/vol">
                <button
                  onClick={toggleMute}
                  className="text-white/70 hover:text-white cursor-pointer transition-colors"
                >
                  {muted || volume === 0 ? <VolumeX className="w-4 h-4" /> : <Volume2 className="w-4 h-4" />}
                </button>
                <div className="absolute bottom-full left-1/2 -translate-x-1/2 mb-2 opacity-0 group-hover/vol:opacity-100 transition-opacity pointer-events-none group-hover/vol:pointer-events-auto">
                  <div className="bg-black/80 backdrop-blur-md rounded-[var(--radius-sm)] p-2 border border-white/10">
                    <input
                      type="range"
                      min={0}
                      max={100}
                      value={muted ? 0 : volume}
                      onChange={(e) => handleVolumeChange(Number(e.target.value))}
                      className="w-20 h-1 accent-accent"
                    />
                  </div>
                </div>
              </div>

              {/* Speed */}
              <div className="relative">
                <button
                  onClick={() => setShowRatePanel(!showRatePanel)}
                  className="text-white/70 hover:text-white text-[0.75rem] font-mono cursor-pointer transition-colors px-1"
                >
                  {playbackRate}x
                </button>
                <AnimatePresence>
                  {showRatePanel && (
                    <motion.div
                      initial={{ opacity: 0, y: 4 }}
                      animate={{ opacity: 1, y: 0 }}
                      exit={{ opacity: 0, y: 4 }}
                      className="absolute bottom-full right-0 mb-2 bg-black/80 backdrop-blur-md rounded-[var(--radius-sm)] p-1 border border-white/10 flex gap-0.5"
                    >
                      {rates.map((r) => (
                        <button
                          key={r}
                          onClick={() => handleRateChange(r)}
                          className={cn(
                            "px-2 py-1 rounded text-[0.72rem] font-mono cursor-pointer transition-all",
                            r === playbackRate
                              ? "bg-accent text-white"
                              : "text-white/60 hover:text-white hover:bg-white/10"
                          )}
                        >
                          {r}x
                        </button>
                      ))}
                    </motion.div>
                  )}
                </AnimatePresence>
              </div>

              {/* Download */}
              <button
                onClick={() => onDownload?.(activeVideo)}
                className="text-white/70 hover:text-white cursor-pointer transition-colors"
                title="下载"
              >
                <Download className="w-4 h-4" />
              </button>

              {/* Music */}
              {activeVideo.music?.title && (
                <div className="relative">
                  <button
                    onClick={() => setShowMusicPanel(!showMusicPanel)}
                    className="text-white/70 hover:text-white cursor-pointer transition-colors"
                  >
                    <Music className="w-4 h-4" />
                  </button>
                  <AnimatePresence>
                    {showMusicPanel && (
                      <motion.div
                        initial={{ opacity: 0, y: 4 }}
                        animate={{ opacity: 1, y: 0 }}
                        exit={{ opacity: 0, y: 4 }}
                        className="absolute bottom-full right-0 mb-2 w-56 bg-black/80 backdrop-blur-md rounded-[var(--radius-md)] p-3 border border-white/10"
                      >
                        <div className="text-[0.8rem] font-medium text-white mb-0.5">{activeVideo.music.title}</div>
                        {activeVideo.music.author && (
                          <div className="text-[0.72rem] text-white/50 mb-2">{activeVideo.music.author}</div>
                        )}
                        {activeVideo.music.play_url && (
                          <audio src={activeVideo.music.play_url} controls className="w-full h-7" />
                        )}
                      </motion.div>
                    )}
                  </AnimatePresence>
                </div>
              )}
            </div>
          </div>
        </motion.div>
      </motion.div>
    </AnimatePresence>
  );
}
