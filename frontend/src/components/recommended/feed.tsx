import { motion } from "framer-motion";
import { VideoCard } from "@/components/search/video-card";
import { Sparkles, RefreshCw, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import type { VideoItem } from "@/lib/tauri";

export function RecommendedFeed() {
  // Placeholder - will connect to Tauri
  const videos: VideoItem[] = [];

  return (
    <div>
      <div className="flex items-center justify-between mb-4">
        <div className="flex items-center gap-2">
          <Sparkles className="w-4 h-4 text-accent" />
          <h3 className="text-[0.9rem] font-semibold text-text">推荐视频</h3>
        </div>
        <div className="flex items-center gap-2">
          <Button variant="outline" size="sm">
            <RefreshCw className="w-3.5 h-3.5" />
            刷新
          </Button>
        </div>
      </div>

      {videos.length === 0 ? (
        <motion.div
          initial={{ opacity: 0, y: 12 }}
          animate={{ opacity: 1, y: 0 }}
          className="flex flex-col items-center justify-center py-20 text-center"
        >
          <div className="w-16 h-16 rounded-[20px] bg-accent/10 border border-accent/15 flex items-center justify-center mb-4">
            <Sparkles className="w-7 h-7 text-accent" />
          </div>
          <p className="text-[0.9rem] text-text-secondary mb-1">正在加载推荐内容...</p>
          <p className="text-[0.8rem] text-text-muted">需要配置 Cookie 后才能获取推荐视频</p>
        </motion.div>
      ) : (
        <motion.div
          className="grid grid-cols-[repeat(auto-fill,minmax(180px,1fr))] gap-3"
          initial="hidden"
          animate="show"
          variants={{ hidden: {}, show: { transition: { staggerChildren: 0.04 } } }}
        >
          {videos.map((video, i) => (
            <VideoCard key={video.aweme_id} video={video} index={i} />
          ))}
        </motion.div>
      )}
    </div>
  );
}
