import { motion } from "framer-motion";
import { VideoCard } from "./video-card";
import type { VideoItem } from "@/lib/tauri";
import { Grid3x3, Download } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";

// Placeholder data for demo
const mockVideos: VideoItem[] = [];

export function VideoGrid() {
  if (mockVideos.length === 0) return null;

  return (
    <div>
      <div className="flex items-center justify-between mb-4">
        <div className="flex items-center gap-2">
          <Grid3x3 className="w-4 h-4 text-success" />
          <h3 className="text-[0.9rem] font-semibold text-text">作品列表</h3>
          <Badge variant="secondary">{mockVideos.length} 个作品</Badge>
        </div>
        <div className="flex items-center gap-2">
          <Button variant="outline" size="sm">
            <Download className="w-3.5 h-3.5" />
            下载全部
          </Button>
        </div>
      </div>

      <motion.div
        className="grid grid-cols-[repeat(auto-fill,minmax(180px,1fr))] gap-3"
        initial="hidden"
        animate="show"
        variants={{ hidden: {}, show: { transition: { staggerChildren: 0.04 } } }}
      >
        {mockVideos.map((video, i) => (
          <VideoCard key={video.aweme_id} video={video} index={i} />
        ))}
      </motion.div>
    </div>
  );
}
