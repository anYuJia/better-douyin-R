import { useState } from "react";
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogFooter } from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { ScrollArea } from "@/components/ui/scroll-area";
import {
  Download,
  Music,
  Heart,
  MessageCircle,
  Share2,
  Link2,
  Copy,
  Image as ImageIcon,
  PlayCircle,
  X,
} from "lucide-react";
import { cn, formatNumber, formatTime } from "@/lib/utils";
import type { VideoItem } from "@/lib/tauri";

interface VideoDetailModalProps {
  video: VideoItem | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onDownload?: (video: VideoItem) => void;
}

export function VideoDetailModal({ video, open, onOpenChange, onDownload }: VideoDetailModalProps) {
  const [activeTab, setActiveTab] = useState<"cover" | "video" | "images">("cover");
  const [currentImageIndex, setCurrentImageIndex] = useState(0);

  if (!video) return null;

  const isImageSet = video.aweme_type === 68 || (video.image_urls && video.image_urls.length > 0);
  const hasVideo = !!video.video_url;
  const images = video.image_urls || [];

  const stats = [
    { icon: Heart, label: "点赞", value: video.digg_count, color: "text-accent" },
    { icon: MessageCircle, label: "评论", value: video.comment_count, color: "text-cyan-400" },
    { icon: Share2, label: "分享", value: video.share_count, color: "text-green-400" },
  ];

  const mediaLinks = [
    video.video_url && { label: "视频", url: video.video_url, type: "video" },
    video.cover_url && { label: "封面", url: video.cover_url, type: "image" },
    video.music_url && { label: "音频", url: video.music_url, type: "audio" },
  ].filter(Boolean) as { label: string; url: string; type: string }[];

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-3xl p-0 overflow-hidden rounded-[var(--radius-xl)]">
        <div className="flex max-h-[80vh]">
          {/* Left: Media Preview */}
          <div className="w-1/2 border-r border-border flex flex-col items-center p-5 min-h-[300px] bg-surface/30">
            {/* Media Controls */}
            {(hasVideo || isImageSet) && (
              <div className="flex gap-1 mb-3">
                <Button
                  variant={activeTab === "cover" ? "default" : "ghost"}
                  size="sm"
                  onClick={() => setActiveTab("cover")}
                  className="h-7 text-[0.7rem] rounded-[6px]"
                >
                  <ImageIcon className="w-3 h-3" />
                  封面
                </Button>
                {hasVideo && (
                  <Button
                    variant={activeTab === "video" ? "default" : "ghost"}
                    size="sm"
                    onClick={() => setActiveTab("video")}
                    className="h-7 text-[0.7rem] rounded-[6px]"
                  >
                    <PlayCircle className="w-3 h-3" />
                    视频
                  </Button>
                )}
                {isImageSet && (
                  <Button
                    variant={activeTab === "images" ? "default" : "ghost"}
                    size="sm"
                    onClick={() => setActiveTab("images")}
                    className="h-7 text-[0.7rem] rounded-[6px]"
                  >
                    <ImageIcon className="w-3 h-3" />
                    图片 ({images.length})
                  </Button>
                )}
              </div>
            )}

            {/* Media Display */}
            <div className="flex-1 flex items-center justify-center w-full overflow-hidden rounded-[var(--radius-md)] bg-black/20">
              {activeTab === "cover" && (
                <img
                  src={video.cover_url}
                  alt={video.desc}
                  className="max-w-full max-h-[420px] object-contain rounded-[var(--radius-md)]"
                />
              )}
              {activeTab === "video" && hasVideo && (
                <video
                  src={video.video_url}
                  controls
                  className="max-w-full max-h-[420px] rounded-[var(--radius-md)]"
                />
              )}
              {activeTab === "images" && isImageSet && (
                <div className="relative w-full h-full flex items-center justify-center">
                  <img
                    src={images[currentImageIndex]}
                    alt={`图片 ${currentImageIndex + 1}`}
                    className="max-w-full max-h-[420px] object-contain rounded-[var(--radius-md)]"
                  />
                  {images.length > 1 && (
                    <div className="absolute bottom-3 left-1/2 -translate-x-1/2 flex gap-1">
                      {images.map((_, i) => (
                        <button
                          key={i}
                          onClick={() => setCurrentImageIndex(i)}
                          className={cn(
                            "w-2 h-2 rounded-full transition-all cursor-pointer",
                            i === currentImageIndex
                              ? "bg-accent w-4"
                              : "bg-white/40 hover:bg-white/60"
                          )}
                        />
                      ))}
                    </div>
                  )}
                </div>
              )}
            </div>
          </div>

          {/* Right: Info */}
          <div className="w-1/2 flex flex-col">
            <DialogHeader className="px-5 pt-4 pb-3 border-b border-border">
              <div className="flex items-center gap-2">
                <DialogTitle className="text-[0.95rem]">作品详情</DialogTitle>
                {isImageSet && <Badge variant="default" size="sm">图集</Badge>}
              </div>
            </DialogHeader>

            <ScrollArea className="flex-1">
              <div className="p-5 flex flex-col gap-4">
                {/* Author */}
                <div className="flex items-center gap-3 p-3 rounded-[var(--radius-md)] bg-surface border border-border">
                  <img
                    src={video.author_avatar}
                    alt={video.author_nickname}
                    className="w-10 h-10 rounded-full object-cover border-2 border-border-strong shrink-0"
                  />
                  <div className="min-w-0">
                    <div className="text-[0.85rem] font-semibold text-text truncate">
                      {video.author_nickname}
                    </div>
                    <div className="text-[0.72rem] text-text-muted">
                      {formatTime(video.create_time)}
                    </div>
                  </div>
                </div>

                {/* Description */}
                <p className="text-[0.85rem] text-text-secondary leading-relaxed break-words">
                  {video.desc}
                </p>

                {/* Stats */}
                <div className="grid grid-cols-3 gap-2">
                  {stats.map((stat) => (
                    <div
                      key={stat.label}
                      className="flex flex-col items-center gap-1 p-3 rounded-[var(--radius-md)] bg-surface border border-border hover:border-border-strong hover:-translate-y-0.5 transition-all"
                    >
                      <stat.icon className={cn("w-4 h-4", stat.color)} />
                      <span className="text-[1rem] font-bold text-text">
                        {formatNumber(stat.value)}
                      </span>
                      <span className="text-[0.65rem] text-text-muted uppercase tracking-wider">
                        {stat.label}
                      </span>
                    </div>
                  ))}
                </div>

                {/* Media Links */}
                {mediaLinks.length > 0 && (
                  <div>
                    <div className="flex items-center gap-1.5 text-[0.72rem] font-bold text-text-muted uppercase tracking-wider mb-2">
                      <Link2 className="w-3 h-3 text-accent" />
                      媒体链接
                    </div>
                    <div className="flex flex-col gap-1.5">
                      {mediaLinks.map((link) => (
                        <div
                          key={link.type}
                          className="flex items-center gap-2 px-3 py-2 rounded-[var(--radius-sm)] bg-surface border border-border hover:border-border-strong transition-all group"
                        >
                          <Badge variant="secondary" size="sm" className="shrink-0">
                            {link.label}
                          </Badge>
                          <span className="text-[0.72rem] text-text-secondary truncate flex-1">
                            {link.url}
                          </span>
                          <button
                            onClick={() => navigator.clipboard?.writeText(link.url)}
                            className="opacity-0 group-hover:opacity-100 transition-opacity cursor-pointer"
                          >
                            <Copy className="w-3 h-3 text-text-muted hover:text-accent" />
                          </button>
                        </div>
                      ))}
                    </div>
                  </div>
                )}

                {/* Music */}
                {video.music_title && (
                  <div>
                    <div className="flex items-center gap-1.5 text-[0.72rem] font-bold text-text-muted uppercase tracking-wider mb-2">
                      <Music className="w-3 h-3 text-accent" />
                      音频 / BGM
                    </div>
                    <div className="p-3 rounded-[var(--radius-md)] bg-surface border border-border">
                      <div className="text-[0.8rem] font-medium text-text">{video.music_title}</div>
                      {video.music_author && (
                        <div className="text-[0.72rem] text-text-muted">{video.music_author}</div>
                      )}
                      {video.music_url && (
                        <audio src={video.music_url} controls className="w-full h-8 mt-2" />
                      )}
                    </div>
                  </div>
                )}
              </div>
            </ScrollArea>

            <DialogFooter className="px-5 py-3 border-t border-border">
              <Button variant="ghost" size="sm" onClick={() => onOpenChange(false)}>
                <X className="w-3.5 h-3.5" />
                关闭
              </Button>
              <Button variant="default" size="sm" onClick={() => onDownload?.(video)}>
                <Download className="w-3.5 h-3.5" />
                下载作品
              </Button>
            </DialogFooter>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}
