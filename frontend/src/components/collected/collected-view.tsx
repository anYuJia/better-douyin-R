import { useEffect, useState, useCallback } from "react";
import { motion } from "framer-motion";
import {
  Download,
  Heart,
  Loader2,
  RefreshCw,
  Star,
  ChevronLeft,
  ChevronRight,
  ArrowLeft,
  ListVideo,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { useDownloads } from "@/hooks/use-downloads";
import { useHistory } from "@/hooks/use-history";
import {
  getCollectedVideos,
  getCollectedMixes,
  getMixVideos,
  mediaProxyUrl,
  type VideoInfo,
  type CollectedMixItem,
} from "@/lib/tauri";
import { cn, formatNumber } from "@/lib/utils";
import { VideoCard } from "@/components/search/video-card";
import { VideoDetailModal } from "@/components/modals/video-detail";
import { FullscreenPlayer } from "@/components/player/fullscreen-player";

type CollectedTab = "videos" | "mixes";
const PAGE_SIZE = 12;

export function CollectedView() {
  const [tab, setTab] = useState<CollectedTab>("videos");
  const { downloadVideo, downloadBatch } = useDownloads();
  const { loadHistory } = useHistory();

  return (
    <>
      <div>
        <div className="flex items-center justify-between mb-4 gap-3 flex-wrap">
          <div className="flex items-center gap-2">
            <Star className="w-4 h-4 text-accent" />
            <h3 className="text-[0.9rem] font-semibold text-text">收藏内容</h3>
          </div>

          <div className="flex items-center gap-2 rounded-[14px] bg-surface p-1 border border-border">
            {[
              { key: "videos" as const, label: "视频", icon: Heart },
              { key: "mixes" as const, label: "合集", icon: ListVideo },
            ].map(({ key, label, icon: Icon }) => (
              <button
                key={key}
                onClick={() => setTab(key)}
                className={cn(
                  "relative flex h-9 items-center gap-2 overflow-hidden rounded-[10px] px-3 text-[0.78rem] font-semibold cursor-pointer transition-[color,opacity]",
                  tab === key
                    ? "text-accent"
                    : "text-text-muted hover:text-text hover:bg-surface-raised"
                )}
              >
                {tab === key && (
                  <motion.div
                    layoutId="collected-tab-active"
                    className="absolute inset-0 rounded-[10px] bg-accent-soft shadow-[inset_0_0_0_1px_var(--color-accent-ring)]"
                    transition={{ type: "spring", duration: 0.28, bounce: 0 }}
                  />
                )}
                <Icon className="relative w-3.5 h-3.5" />
                <span className="relative">{label}</span>
              </button>
            ))}
          </div>
        </div>

        {tab === "videos" ? (
          <CollectedVideosPanel
            onDownload={(video) => {
              void downloadVideo(video);
              void loadHistory();
            }}
            onDownloadAll={(videos) => {
              void downloadBatch(videos);
              void loadHistory();
            }}
          />
        ) : (
          <CollectedMixesPanel
            onDownload={(video) => {
              void downloadVideo(video);
              void loadHistory();
            }}
            onDownloadAll={(videos) => {
              void downloadBatch(videos);
              void loadHistory();
            }}
          />
        )}
      </div>
    </>
  );
}

// ═══════════════════════════════════════════════
// Collected Videos Panel
// ═══════════════════════════════════════════════

function CollectedVideosPanel({
  onDownload,
  onDownloadAll,
}: {
  onDownload: (video: VideoInfo) => void;
  onDownloadAll: (videos: VideoInfo[]) => void;
}) {
  const [videos, setVideos] = useState<VideoInfo[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [cursor, setCursor] = useState(0);
  const [hasMore, setHasMore] = useState(false);
  const [detailVideo, setDetailVideo] = useState<VideoInfo | null>(null);
  const [playerIndex, setPlayerIndex] = useState<number | null>(null);

  const loadVideos = useCallback(async (reset = false) => {
    setLoading(true);
    setError(null);
    try {
      const nextCursor = reset ? 0 : cursor;
      const result = await getCollectedVideos(nextCursor, PAGE_SIZE);
      if (result.success) {
        const newVideos = result.data || result.videos || [];
        setVideos((prev) => (reset ? newVideos : [...prev, ...newVideos]));
        setHasMore(result.has_more || false);
        setCursor(result.next_cursor || result.cursor || 0);
      } else {
        setError(result.message || "获取收藏视频失败");
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : "获取收藏视频失败");
    } finally {
      setLoading(false);
    }
  }, [cursor]);

  useEffect(() => {
    void loadVideos(true);
  }, []);

  const openPlayer = (video: VideoInfo) => {
    const index = videos.findIndex((item) => item.aweme_id === video.aweme_id);
    setPlayerIndex(index >= 0 ? index : 0);
  };

  return (
    <div>
      <div className="flex items-center justify-end gap-2 mb-4">
        <Button variant="outline" size="sm" onClick={() => void loadVideos(true)} disabled={loading}>
          {loading && videos.length === 0 ? (
            <Loader2 className="w-3.5 h-3.5 animate-spin" />
          ) : (
            <RefreshCw className="w-3.5 h-3.5" />
          )}
          刷新
        </Button>
        {videos.length > 0 && (
          <Button variant="default" size="sm" onClick={() => onDownloadAll(videos)}>
            <Download className="w-3.5 h-3.5" />
            下载全部
          </Button>
        )}
      </div>

      {loading && videos.length === 0 ? (
        <LoadingGrid />
      ) : error && videos.length === 0 ? (
        <ErrorState message={error} />
      ) : videos.length === 0 ? (
        <EmptyState title="暂无收藏视频" description="需要登录抖音账号后才能读取收藏视频列表" />
      ) : (
        <>
          <motion.div
            className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4 gap-3"
            initial={false}
            animate="show"
            variants={{ hidden: {}, show: { transition: { staggerChildren: 0.04 } } }}
          >
            {videos.map((video, index) => (
              <VideoCard
                key={video.aweme_id}
                video={video}
                index={index}
                animate={false}
                onSelect={openPlayer}
                onDetail={setDetailVideo}
                onDownload={onDownload}
              />
            ))}
          </motion.div>
          {hasMore && (
            <div className="flex justify-center mt-4">
              <Button
                variant="outline"
                size="sm"
                onClick={() => void loadVideos(false)}
                disabled={loading}
              >
                {loading ? (
                  <Loader2 className="w-3.5 h-3.5 animate-spin mr-2" />
                ) : (
                  <Download className="w-3.5 h-3.5 mr-2" />
                )}
                加载更多
              </Button>
            </div>
          )}
        </>
      )}

      <FullscreenPlayer
        videos={videos}
        initialIndex={playerIndex ?? 0}
        open={playerIndex !== null}
        onClose={() => setPlayerIndex(null)}
        onDownload={onDownload}
        onShowDetail={(video) => {
          setPlayerIndex(null);
          setDetailVideo(video);
        }}
      />

      <VideoDetailModal
        video={detailVideo}
        open={Boolean(detailVideo)}
        onOpenChange={(open) => {
          if (!open) setDetailVideo(null);
        }}
        onDownload={onDownload}
      />
    </div>
  );
}

// ═══════════════════════════════════════════════
// Collected Mixes Panel (合集列表)
// ═══════════════════════════════════════════════

function CollectedMixesPanel({
  onDownload,
  onDownloadAll,
}: {
  onDownload: (video: VideoInfo) => void;
  onDownloadAll: (videos: VideoInfo[]) => void;
}) {
  const [mixes, setMixes] = useState<CollectedMixItem[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [cursor, setCursor] = useState(0);
  const [hasMore, setHasMore] = useState(false);
  // 当前展开的合集视频列表
  const [expandedMix, setExpandedMix] = useState<ExpandedMix | null>(null);

  const loadMixes = useCallback(async (reset = false) => {
    // 如果正在查看合集内部，不刷新列表
    if (expandedMix) return;
    setLoading(true);
    setError(null);
    try {
      const nextCursor = reset ? 0 : cursor;
      const result = await getCollectedMixes(nextCursor, PAGE_SIZE);
      if (result.success) {
        const newMixes = result.data || [];
        setMixes((prev) => (reset ? newMixes : [...prev, ...newMixes]));
        setHasMore(result.has_more || false);
        setCursor(result.next_cursor || result.cursor || 0);
      } else {
        setError(result.message || "获取合集失败");
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : "获取合集失败");
    } finally {
      setLoading(false);
    }
  }, [cursor, expandedMix]);

  useEffect(() => {
    void loadMixes(true);
  }, []);

  const handleExpandMix = useCallback(async (mix: CollectedMixItem) => {
    setExpandedMix({
      mix,
      videos: [],
      loading: true,
      cursor: 0,
      hasMore: true,
    });
    try {
      const result = await getMixVideos(mix.mix_id, 0, PAGE_SIZE);
      setExpandedMix((prev) => prev ? {
        ...prev,
        videos: result.data || [],
        loading: false,
        cursor: result.next_cursor || result.cursor || 0,
        hasMore: result.has_more || false,
      } : null);
    } catch {
      setExpandedMix((prev) => prev ? { ...prev, loading: false } : null);
    }
  }, []);

  const handleLoadMoreMixVideos = useCallback(async () => {
    if (!expandedMix || !expandedMix.hasMore || expandedMix.loading) return;
    setExpandedMix((prev) => prev ? { ...prev, loading: true } : null);
    try {
      const result = await getMixVideos(expandedMix.mix.mix_id, expandedMix.cursor, PAGE_SIZE);
      setExpandedMix((prev) => prev ? {
        ...prev,
        videos: [...prev.videos, ...(result.data || [])],
        loading: false,
        cursor: result.next_cursor || result.cursor || 0,
        hasMore: result.has_more || false,
      } : null);
    } catch {
      setExpandedMix((prev) => prev ? { ...prev, loading: false } : null);
    }
  }, [expandedMix]);

  const handleBackToMixList = useCallback(() => {
    setExpandedMix(null);
  }, []);

  // 合集内部视频列表
  if (expandedMix) {
    return (
      <ExpandedMixPanel
        expandedMix={expandedMix}
        onBack={handleBackToMixList}
        onLoadMore={handleLoadMoreMixVideos}
        onDownload={onDownload}
        onDownloadAll={onDownloadAll}
      />
    );
  }

  return (
    <div>
      <div className="flex items-center justify-end gap-2 mb-4">
        <Button variant="outline" size="sm" onClick={() => void loadMixes(true)} disabled={loading}>
          {loading && mixes.length === 0 ? (
            <Loader2 className="w-3.5 h-3.5 animate-spin" />
          ) : (
            <RefreshCw className="w-3.5 h-3.5" />
          )}
          刷新
        </Button>
      </div>

      {loading && mixes.length === 0 ? (
        <div className="grid grid-cols-[repeat(auto-fit,minmax(260px,1fr))] gap-3">
          {Array.from({ length: 6 }).map((_, index) => (
            <div key={index} className="rounded-[16px] border border-border bg-surface-solid/70 p-4 h-[200px]">
              <div className="h-full rounded-[12px] bg-white/[0.05] animate-pulse" />
            </div>
          ))}
        </div>
      ) : error && mixes.length === 0 ? (
        <ErrorState message={error} />
      ) : mixes.length === 0 ? (
        <EmptyState title="暂无收藏合集" description="需要登录抖音账号后才能读取收藏合集列表" />
      ) : (
        <>
          <div className="grid grid-cols-[repeat(auto-fit,minmax(260px,1fr))] gap-3">
            {mixes.map((mix) => (
              <MixCard key={mix.mix_id} mix={mix} onExpand={() => void handleExpandMix(mix)} />
            ))}
          </div>
          {hasMore && (
            <div className="flex justify-center mt-4">
              <Button
                variant="outline"
                size="sm"
                onClick={() => void loadMixes(false)}
                disabled={loading}
              >
                {loading ? (
                  <Loader2 className="w-3.5 h-3.5 animate-spin mr-2" />
                ) : (
                  <Download className="w-3.5 h-3.5 mr-2" />
                )}
                加载更多合集
              </Button>
            </div>
          )}
        </>
      )}
    </div>
  );
}

// ═══════════════════════════════════════════════
// Expanded Mix Panel (合集内视频列表，带分页)
// ═══════════════════════════════════════════════

interface ExpandedMix {
  mix: CollectedMixItem;
  videos: VideoInfo[];
  loading: boolean;
  cursor: number;
  hasMore: boolean;
}

function ExpandedMixPanel({
  expandedMix,
  onBack,
  onLoadMore,
  onDownload,
  onDownloadAll,
}: {
  expandedMix: ExpandedMix;
  onBack: () => void;
  onLoadMore: () => void;
  onDownload: (video: VideoInfo) => void;
  onDownloadAll: (videos: VideoInfo[]) => void;
}) {
  const { mix, videos, loading, hasMore } = expandedMix;
  const [detailVideo, setDetailVideo] = useState<VideoInfo | null>(null);
  const [playerIndex, setPlayerIndex] = useState<number | null>(null);

  const openPlayer = (video: VideoInfo) => {
    const index = videos.findIndex((item) => item.aweme_id === video.aweme_id);
    setPlayerIndex(index >= 0 ? index : 0);
  };

  return (
    <div>
      <div className="flex items-center justify-between mb-4 flex-wrap gap-2">
        <div className="flex items-center gap-2">
          <Button variant="ghost" size="sm" onClick={onBack}>
            <ArrowLeft className="w-3.5 h-3.5 mr-1" />
            返回
          </Button>
          <div className="flex items-center gap-2">
            <ListVideo className="w-4 h-4 text-accent" />
            <h4 className="text-[0.85rem] font-semibold text-text truncate max-w-[300px]">
              {mix.mix_name}
            </h4>
            <Badge variant="secondary">{videos.length} 个视频</Badge>
          </div>
        </div>
        {videos.length > 0 && (
          <Button variant="default" size="sm" onClick={() => onDownloadAll(videos)}>
            <Download className="w-3.5 h-3.5" />
            下载全部
          </Button>
        )}
      </div>

      {loading && videos.length === 0 ? (
        <LoadingGrid />
      ) : videos.length === 0 ? (
        <EmptyState title="合集内暂无视频" description="该合集可能没有视频或暂未加载成功" />
      ) : (
        <>
          <motion.div
            className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4 gap-3"
            initial={false}
            animate="show"
            variants={{ hidden: {}, show: { transition: { staggerChildren: 0.04 } } }}
          >
            {videos.map((video, index) => (
              <VideoCard
                key={video.aweme_id}
                video={video}
                index={index}
                animate={false}
                onSelect={openPlayer}
                onDetail={setDetailVideo}
                onDownload={onDownload}
              />
            ))}
          </motion.div>
          {hasMore && (
            <div className="flex justify-center mt-4">
              <Button
                variant="outline"
                size="sm"
                onClick={onLoadMore}
                disabled={loading}
              >
                {loading ? (
                  <Loader2 className="w-3.5 h-3.5 animate-spin mr-2" />
                ) : (
                  <Download className="w-3.5 h-3.5 mr-2" />
                )}
                加载更多视频
              </Button>
            </div>
          )}
        </>
      )}

      <FullscreenPlayer
        videos={videos}
        initialIndex={playerIndex ?? 0}
        open={playerIndex !== null}
        onClose={() => setPlayerIndex(null)}
        onDownload={onDownload}
        onShowDetail={(video) => {
          setPlayerIndex(null);
          setDetailVideo(video);
        }}
      />

      <VideoDetailModal
        video={detailVideo}
        open={Boolean(detailVideo)}
        onOpenChange={(open) => {
          if (!open) setDetailVideo(null);
        }}
        onDownload={onDownload}
      />
    </div>
  );
}

// ═══════════════════════════════════════════════
// Mix Card
// ═══════════════════════════════════════════════

function MixCard({ mix, onExpand }: { mix: CollectedMixItem; onExpand: () => void }) {
  const coverUrl = mediaProxyUrl(mix.cover_url || "", "image");
  const authorName = mix.author?.nickname || "未知作者";
  const episodeCount = mix.statis?.updated_to_episode || 0;
  const playCount = mix.statis?.play_vv || 0;

  return (
    <motion.div
      initial={false}
      animate={{ opacity: 1, y: 0 }}
      className="rounded-[18px] border border-border bg-surface-solid/80 overflow-hidden cursor-pointer hover:border-border-strong hover:bg-surface-raised transition-all"
      onClick={onExpand}
    >
      <div className="relative h-[140px] bg-surface">
        {coverUrl ? (
          <img
            src={coverUrl}
            alt={mix.mix_name}
            className="w-full h-full object-cover"
            loading="lazy"
          />
        ) : (
          <div className="w-full h-full flex items-center justify-center">
            <ListVideo className="w-10 h-10 text-text-muted" />
          </div>
        )}
        <div className="absolute inset-x-0 bottom-0 bg-gradient-to-t from-black/70 to-transparent px-3 pb-2 pt-8">
          <span className="text-[0.7rem] font-semibold text-white/90">
            {episodeCount > 0 ? `共 ${episodeCount} 集` : "合集"}
          </span>
        </div>
      </div>
      <div className="p-3">
        <div className="text-[0.82rem] font-semibold text-text truncate mb-1">
          {mix.mix_name}
        </div>
        {mix.desc && (
          <div className="text-[0.7rem] text-text-muted line-clamp-2 mb-2 min-h-[30px]">
            {mix.desc}
          </div>
        )}
        <div className="flex items-center justify-between text-[0.65rem] text-text-muted">
          <span>@{authorName}</span>
          {playCount > 0 && <span>{formatNumber(playCount)} 播放</span>}
        </div>
      </div>
    </motion.div>
  );
}

// ═══════════════════════════════════════════════
// Shared Components
// ═══════════════════════════════════════════════

function LoadingGrid() {
  return (
    <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4 gap-3">
      {Array.from({ length: 8 }).map((_, index) => (
        <div
          key={index}
          className="h-[380px] overflow-hidden rounded-[var(--radius-lg)] border border-border bg-surface-solid/70"
        >
          <div className="h-[260px] bg-white/[0.05] animate-pulse" />
          <div className="h-[120px] p-3">
            <div className="h-4 rounded bg-white/[0.05] animate-pulse mb-2" />
            <div className="h-3 w-1/2 rounded bg-white/[0.05] animate-pulse mb-3" />
            <div className="mt-auto grid grid-cols-3 gap-1.5">
              <div className="h-7 rounded bg-white/[0.05] animate-pulse" />
              <div className="h-7 rounded bg-white/[0.05] animate-pulse" />
              <div className="h-7 rounded bg-white/[0.05] animate-pulse" />
            </div>
          </div>
        </div>
      ))}
    </div>
  );
}

function EmptyState({ title, description }: { title: string; description: string }) {
  return (
    <motion.div
      initial={false}
      animate={{ opacity: 1, y: 0 }}
      className="rounded-[16px] border border-border bg-surface-solid/70 p-8 text-center"
    >
      <div className="w-14 h-14 rounded-[18px] bg-accent/10 border border-accent/15 flex items-center justify-center mx-auto mb-4">
        <Star className="w-6 h-6 text-accent" />
      </div>
      <p className="text-[0.88rem] text-text-secondary mb-1">{title}</p>
      <p className="text-[0.76rem] text-text-muted">{description}</p>
    </motion.div>
  );
}

function ErrorState({ message }: { message: string }) {
  return (
    <motion.div
      initial={false}
      animate={{ opacity: 1, y: 0 }}
      className="rounded-[16px] border border-danger/20 bg-danger-soft p-5 text-danger"
    >
      <div className="text-[0.88rem] font-semibold mb-1">读取失败</div>
      <div className="text-[0.78rem] text-text-secondary">{message}</div>
    </motion.div>
  );
}
