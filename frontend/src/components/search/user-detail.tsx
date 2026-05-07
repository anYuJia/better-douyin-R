import { motion } from "framer-motion";
import { Download, Grid3x3 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { formatNumber } from "@/lib/utils";

// Placeholder - will connect to Tauri store
export function UserDetail() {
  return null;
}

interface UserDetailCardProps {
  user: {
    nickname: string;
    unique_id: string;
    signature: string;
    avatar_url: string;
    aweme_count: number;
    follower_count: number;
    following_count: number;
    total_favorited: number;
  };
  onDownloadAll?: () => void;
  onViewVideos?: () => void;
}

export function UserDetailCard({ user, onDownloadAll, onViewVideos }: UserDetailCardProps) {
  const stats = [
    { label: "作品", value: user.aweme_count },
    { label: "粉丝", value: user.follower_count },
    { label: "关注", value: user.following_count },
    { label: "获赞", value: user.total_favorited },
  ];

  return (
    <motion.div
      initial={{ opacity: 0, y: 16 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ type: "spring", stiffness: 350, damping: 28 }}
      className="relative rounded-[18px] border border-info/15 p-5 overflow-hidden"
      style={{
        background: `
          radial-gradient(circle at top right, oklch(58% 0.18 266 / 0.1), transparent 32%),
          linear-gradient(180deg, oklch(25% 0.014 282 / 0.04), oklch(22% 0.014 282 / 0.015)),
          oklch(18% 0.012 282 / 0.7)
        `,
      }}
    >
      <div className="flex items-center gap-4 flex-wrap">
        <img
          src={user.avatar_url}
          alt={user.nickname}
          className="w-[76px] h-[76px] rounded-full object-cover border-[3px] border-white/10 shadow-[0_8px_24px_rgba(0,0,0,0.3)] shrink-0"
        />

        <div className="flex-1 min-w-[200px]">
          <h3 className="text-[1.2rem] font-[780] tracking-tight text-text mb-1.5">
            {user.nickname}
          </h3>
          <span className="inline-flex items-center px-2.5 py-1 rounded-full bg-surface border border-border text-[0.72rem] font-mono text-text-secondary">
            {user.unique_id}
          </span>
          {user.signature && (
            <p className="text-[0.8rem] text-text-secondary mt-2 line-clamp-2 leading-relaxed">
              {user.signature}
            </p>
          )}
        </div>

        {/* Stats */}
        <div className="flex items-center gap-0 shrink-0">
          {stats.map((stat, i) => (
            <div key={stat.label} className="flex items-baseline gap-1.5 px-4 relative">
              <span className="text-[1.15rem] font-[780] tracking-tight text-text">
                {formatNumber(stat.value)}
              </span>
              <span className="text-[0.75rem] font-medium text-text-secondary">
                {stat.label}
              </span>
              {i < stats.length - 1 && (
                <div className="absolute right-0 top-[20%] bottom-[20%] w-px bg-gradient-to-b from-transparent via-border-strong to-transparent" />
              )}
            </div>
          ))}
        </div>
      </div>

      {/* Actions */}
      <div className="flex gap-2.5 mt-4 flex-wrap">
        <Button variant="default" size="sm" onClick={onDownloadAll}>
          <Download className="w-3.5 h-3.5" />
          下载所有作品
        </Button>
        <Button variant="info-outline" size="sm" onClick={onViewVideos}>
          <Grid3x3 className="w-3.5 h-3.5" />
          查看作品列表
        </Button>
      </div>
    </motion.div>
  );
}
