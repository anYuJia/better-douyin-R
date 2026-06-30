import { useCallback, useEffect, useRef, useState } from "react";
import { Bell, Loader2, RefreshCw, Heart, UserPlus, MessageSquare, AtSign } from "lucide-react";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { getNotices, mediaProxyUrl } from "@/lib/tauri";
import type { NoticeItem, NoticeUser } from "@/lib/contracts";
import { cn } from "@/lib/utils";
import { useAppStore } from "@/stores/app-store";

// 通知类型 → 图标/配色（实测 notice_list_v2 的 type 值）。
const TYPE_META: Record<number, { icon: React.ElementType; tone: string }> = {
  1: { icon: Heart, tone: "text-danger" },
  41: { icon: Heart, tone: "text-danger" },
  31: { icon: MessageSquare, tone: "text-success" },
  33: { icon: UserPlus, tone: "text-accent" },
  45: { icon: AtSign, tone: "text-warning" },
};

function typeMeta(type: number) {
  return TYPE_META[type] || { icon: Bell, tone: "text-text-muted" };
}

// create_time 是秒级时间戳，转成相对时间文案。
function formatNoticeTime(seconds: number): string {
  if (!seconds) return "";
  const ts = seconds * 1000;
  const now = Date.now();
  const diff = Math.max(0, now - ts);
  const minute = 60_000;
  const hour = 60 * minute;
  const day = 24 * hour;
  if (diff < minute) return "刚刚";
  if (diff < hour) return `${Math.floor(diff / minute)}分钟前`;
  if (diff < day) return `${Math.floor(diff / hour)}小时前`;
  if (diff < 2 * day) return "昨天";
  if (diff < 7 * day) return `${Math.floor(diff / day)}天前`;
  const date = new Date(ts);
  if (Number.isNaN(date.getTime())) return "";
  const monthDay = `${date.getMonth() + 1}月${date.getDate()}日`;
  if (date.getFullYear() === new Date().getFullYear()) return monthDay;
  return `${date.getFullYear()}年${monthDay}`;
}

function AvatarStack({ users }: { users: NoticeUser[] }) {
  const list = users.slice(0, 3);
  if (list.length === 0) {
    return (
      <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-full bg-surface-raised text-text-muted">
        <Bell className="h-4 w-4" />
      </div>
    );
  }
  return (
    <div className="relative flex h-10 w-10 shrink-0 items-center justify-center">
      {list.map((user, index) => {
        const offset = index * 8;
        const avatar = mediaProxyUrl(user.avatar, "image");
        return (
          <img
            key={user.uid || index}
            src={avatar}
            alt={user.nickname}
            loading="lazy"
            className="absolute h-9 w-9 rounded-full border-2 border-surface-solid object-cover"
            style={{ left: offset, zIndex: list.length - index }}
            onError={(event) => {
              const target = event.currentTarget;
              target.style.visibility = "hidden";
            }}
          />
        );
      })}
    </div>
  );
}

function NoticeCard({ notice }: { notice: NoticeItem }) {
  const meta = typeMeta(notice.type);
  const Icon = meta.icon;
  const cover = mediaProxyUrl(notice.aweme?.cover, "image");
  const extraCount = notice.users.length - 3;

  return (
    <div
      className={cn(
        "group flex items-center gap-3 rounded-[var(--radius-md)] border px-3 py-2.5 transition-colors",
        notice.has_read
          ? "border-border/60 bg-surface-solid/40"
          : "border-accent/25 bg-accent-soft/50"
      )}
    >
      <div className="relative">
        <AvatarStack users={notice.users} />
        <span
          className={cn(
            "absolute -bottom-0.5 -right-0.5 flex h-4 w-4 items-center justify-center rounded-full border border-surface-solid bg-surface-solid",
            meta.tone
          )}
        >
          <Icon className="h-2.5 w-2.5" />
        </span>
      </div>

      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-1.5">
          {notice.label_text && (
            <span className="shrink-0 rounded-full bg-surface-raised px-1.5 py-0.5 text-[0.6rem] font-semibold text-text-muted">
              {notice.label_text}
            </span>
          )}
          <span className={cn("text-[0.7rem] font-semibold", meta.tone)}>{notice.type_label}</span>
          {!notice.has_read && (
            <span className="h-1.5 w-1.5 shrink-0 rounded-full bg-accent" title="未读" />
          )}
          <span className="ml-auto shrink-0 text-[0.66rem] text-text-muted">
            {formatNoticeTime(notice.create_time)}
          </span>
        </div>
        <p className="mt-0.5 line-clamp-2 text-[0.78rem] leading-snug text-text">
          {notice.content}
        </p>
        {notice.comment_text && (
          <p className="mt-0.5 truncate rounded-[var(--radius-sm)] bg-surface-raised/60 px-1.5 py-0.5 text-[0.7rem] text-text-secondary">
            “{notice.comment_text}”
          </p>
        )}
        {notice.users.length > 1 && (
          <p className="mt-0.5 truncate text-[0.68rem] text-text-muted">
            {notice.users.slice(0, 3).map((u) => u.nickname).filter(Boolean).join("、")}
            {extraCount > 0 ? ` 等 ${notice.merge_count || notice.users.length} 人` : ""}
          </p>
        )}
      </div>

      {cover && (
        <div className="relative h-12 w-12 shrink-0 overflow-hidden rounded-[var(--radius-sm)] bg-surface-raised">
          <img
            src={cover}
            alt=""
            loading="lazy"
            className="h-full w-full object-cover"
            onError={(event) => {
              const target = event.currentTarget;
              target.style.display = "none";
            }}
          />
        </div>
      )}
    </div>
  );
}

export function NoticesView() {
  const setNoticeUnreadCount = useAppStore((state) => state.setNoticeUnreadCount);
  const cookieLoggedIn = useAppStore((state) => state.cookieLoggedIn);

  const [notices, setNotices] = useState<NoticeItem[]>([]);
  const [loading, setLoading] = useState(false);
  const [loadingMore, setLoadingMore] = useState(false);
  const [error, setError] = useState("");
  const [hasMore, setHasMore] = useState(false);
  const [cursor, setCursor] = useState(0);
  const [lastUpdatedAt, setLastUpdatedAt] = useState(0);
  const requestedRef = useRef(false);
  const viewportRef = useRef<HTMLDivElement>(null);
  const sentinelRef = useRef<HTMLDivElement>(null);

  const applyResponse = useCallback(
    (resp: Awaited<ReturnType<typeof getNotices>>, append: boolean) => {
      if (!resp.success) {
        setError(resp.message || "获取通知失败");
        return;
      }
      const next = resp.notices || [];
      setNotices((prev) => (append ? [...prev, ...next] : next));
      setHasMore(Boolean(resp.has_more));
      setCursor(Number(resp.cursor || 0));
      if (!append) {
        setNoticeUnreadCount(Number(resp.unread_count || 0));
        setLastUpdatedAt(Date.now());
      }
      setError("");
    },
    [setNoticeUnreadCount]
  );

  const query = useCallback(async () => {
    setLoading(true);
    setError("");
    try {
      const resp = await getNotices({ count: 20 });
      applyResponse(resp, false);
    } catch (e) {
      setError(e instanceof Error ? e.message : "获取通知失败");
    } finally {
      setLoading(false);
    }
  }, [applyResponse]);

  const loadOlder = useCallback(async () => {
    if (loadingMore || !hasMore || !cursor) return;
    setLoadingMore(true);
    try {
      const resp = await getNotices({ count: 20, maxTime: cursor });
      applyResponse(resp, true);
    } catch (e) {
      setError(e instanceof Error ? e.message : "加载更多失败");
    } finally {
      setLoadingMore(false);
    }
  }, [applyResponse, hasMore, loadingMore, cursor]);

  // 预翻页：sentinel 进入视口前 240px 即触发加载，让用户感觉不到"到底"。
  const loadOlderRef = useRef(loadOlder);
  loadOlderRef.current = loadOlder;

  useEffect(() => {
    const viewport = viewportRef.current;
    const sentinel = sentinelRef.current;
    if (!viewport || !sentinel) return;
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting)) {
          loadOlderRef.current();
        }
      },
      { root: viewport, rootMargin: "0px 0px 240px 0px" },
    );
    observer.observe(sentinel);
    return () => observer.disconnect();
  }, [hasMore]);

  useEffect(() => {
    if (requestedRef.current) return;
    requestedRef.current = true;
    if (cookieLoggedIn) {
      void query();
    }
  }, [cookieLoggedIn, query]);

  const unread = notices.filter((n) => !n.has_read).length;

  return (
    <div className="mx-auto flex h-full min-h-0 w-full max-w-[860px] flex-col gap-3 overflow-hidden">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="flex min-w-0 items-center gap-2">
          <Bell className="h-4 w-4 text-accent" />
          <h3 className="text-[0.95rem] font-semibold text-text">通知</h3>
          <span className="truncate text-[0.72rem] text-text-muted">
            {notices.length > 0
              ? unread > 0
                ? `${unread} 条未读 · 共 ${notices.length} 条`
                : `共 ${notices.length} 条`
              : "暂无通知"}
            {lastUpdatedAt ? ` · ${new Date(lastUpdatedAt).toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit" })}` : ""}
          </span>
        </div>
        <div className="flex items-center gap-2">
          <Button size="sm" onClick={() => void query()} disabled={loading} className="h-9">
            {loading ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <RefreshCw className="h-3.5 w-3.5" />}
            刷新
          </Button>
        </div>
      </div>

      {error && (
        <div className="rounded-[var(--radius-sm)] border border-white/[0.06] bg-danger-soft px-3 py-2 text-[0.78rem] text-danger">
          {error}
        </div>
      )}

      {!cookieLoggedIn && (
        <div className="rounded-[var(--radius-sm)] border border-border bg-surface-solid px-3 py-2 text-[0.78rem] text-text-muted">
          需要先登录 Cookie 才能获取通知。
        </div>
      )}

      <div className="min-h-0 flex-1">
        <ScrollArea className="h-full" viewportRef={viewportRef}>
          {loading && notices.length === 0 ? (
            <div className="flex h-full items-center justify-center text-[0.8125rem] text-text-muted">
              <Loader2 className="mr-2 h-4 w-4 animate-spin" />
              加载中…
            </div>
          ) : notices.length === 0 && !error ? (
            <div className="flex h-full flex-col items-center justify-center gap-2 text-text-muted">
              <Bell className="h-8 w-8 opacity-40" />
              <span className="text-[0.8125rem]">暂无通知</span>
            </div>
          ) : (
            <div className="flex flex-col gap-1.5 py-0.5">
              {notices.map((notice) => (
                <NoticeCard key={notice.id || `${notice.type}-${notice.create_time}`} notice={notice} />
              ))}
              {hasMore && (
                <div
                  ref={sentinelRef}
                  className="flex min-h-[28px] items-center justify-center gap-1.5 py-2 text-[0.72rem] text-text-muted"
                >
                  {loadingMore && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
                  {loadingMore ? "正在加载…" : "滚动加载更多"}
                </div>
              )}
              {!hasMore && notices.length > 0 && (
                <div className="py-2 text-center text-[0.7rem] text-text-muted">没有更早的通知了</div>
              )}
            </div>
          )}
        </ScrollArea>
      </div>
    </div>
  );
}
