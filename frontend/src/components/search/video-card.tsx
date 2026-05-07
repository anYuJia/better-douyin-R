import { motion } from "framer-motion";
import { Heart, MessageCircle, Share2, Download, Play } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { cn, formatNumber, formatTime } from "@/lib/utils";
import type { VideoItem } from "@/lib/tauri";

interface VideoCardProps {
  video: VideoItem;
  index?: number;
  onSelect?: (video: VideoItem) => void;
  onDownload?: (video: VideoItem) => void;
  selected?: boolean;
}

export function VideoCard({ video, index = 0, onSelect, onDownload, selected }: VideoCardProps) {
  const isImageSet = video.aweme_type === 68 || (video.image_urls && video.image_urls.length > 0);

  return (
    <motion.div
      initial={{ opacity: 0, y: 12 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ delay: index * 0.05, type: "spring", stiffness: 350, damping: 28 }}
      className={cn(
        "group relative rounded-[var(--radius-md)] border border-border bg-surface-solid/80 overflow-hidden transition-all duration-[var(--duration-base)] ease-[var(--ease-spring)]",
        "hover:border-border-strong hover:-translate-y-1 hover:shadow-md",
        selected && "border-accent shadow-[var(--shadow-glow)]"
      )}
      style={{ breakInside: "avoid" }}
    >
      {/* Cover */}
      <div
        className="relative aspect-[9/16] overflow-hidden cursor-pointer bg-surface"
        onClick={() => onSelect?.(video)}
      >
        <img
          src={video.cover_url}
          alt={video.desc}
          className="w-full h-full object-cover transition-transform duration-[var(--duration-slow)] group-hover:scale-[1.04]"
          loading="lazy"
        />

        {/* Play icon on hover */}
        <div className="absolute inset-0 flex items-center justify-center opacity-0 group-hover:opacity-100 transition-opacity duration-[var(--duration-fast)]">
          <div className="w-12 h-12 rounded-full bg-black/40 backdrop-blur-md flex items-center justify-center">
            <Play className="w-5 h-5 text-white fill-white ml-0.5" />
          </div>
        </div>

        {/* Type badge */}
        {isImageSet && (
          <Badge variant="default" size="sm" className="absolute top-2 left-2 bg-black/40 backdrop-blur-sm text-white border-white/20">
            图集
          </Badge>
        )}

        {/* Stats overlay */}
        <div className="absolute bottom-0 left-0 right-0 p-3 bg-gradient-to-t from-black/80 to-transparent opacity-0 group-hover:opacity-100 transition-opacity duration-[var(--duration-fast)]">
          <div className="flex items-center justify-around text-white text-[0.7rem] font-medium">
            <span className="flex items-center gap-1">
              <Heart className="w-3.5 h-3.5 text-accent" />
              {formatNumber(video.digg_count)}
            </span>
            <span className="flex items-center gap-1">
              <MessageCircle className="w-3.5 h-3.5 text-cyan-400" />
              {formatNumber(video.comment_count)}
            </span>
            <span className="flex items-center gap-1">
              <Share2 className="w-3.5 h-3.5 text-green-400" />
              {formatNumber(video.share_count)}
            </span>
          </div>
        </div>
      </div>

      {/* Body */}
      <div className="p-3 flex flex-col h-[100px]">
        <p className="text-[0.8rem] text-text leading-snug truncate mb-1">
          {video.desc}
        </p>
        <span className="text-[0.7rem] text-text-muted mb-auto">
          {formatTime(video.create_time)}
        </span>
        <div className="flex gap-1.5 mt-2">
          <Button
            variant="outline"
            size="sm"
            className="flex-1 h-7 text-[0.7rem] rounded-[6px]"
            onClick={() => onDownload?.(video)}
          >
            <Download className="w-3 h-3" />
            下载
          </Button>
          <Button
            variant="ghost"
            size="icon-sm"
            onClick={() => onSelect?.(video)}
          >
            <Play className="w-3.5 h-3.5" />
          </Button>
        </div>
      </div>
    </motion.div>
  );
}
