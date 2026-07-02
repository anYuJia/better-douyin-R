import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Activity, Loader2, RefreshCw } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { FullscreenPlayer } from "@/components/player/lazy-fullscreen-player";
import { useDownloads } from "@/hooks/use-downloads";
import {
  getAccounts,
  getConfig,
  getFriendOnlineStatus,
  getUserDetail,
  getVideoDetail,
  listenEvent,
  saveConfig,
  verifyCookie,
} from "@/lib/tauri";
import { cn } from "@/lib/utils";
import { useAppStore } from "@/stores/app-store";
import { useSearchStore } from "@/stores/search-store";
import type { UserInfo, VideoInfo } from "@/lib/contracts";
import {
  COOKIE_REQUIRED_PATTERN,
  CURRENT_USER_AVATAR_KEY,
  DEFAULT_REFRESH_INTERVAL_SECONDS,
  MIN_BACKGROUND_REFRESH_INTERVAL_MS,
  STORAGE_KEY,
  type FriendListItem,
  type FriendStatusItem,
  type SharedMessageCard,
} from "./friends-status-types";
import {
  extractIds,
  formatUpdateTime,
  latestChatMessage,
  mapResponse,
  messagePreviewText,
  stringField,
} from "./friends-status-utils";
import { ChatWorkspace } from "./friends-status-components";
import { FriendListPanel } from "./friends-list-panel";
import { useFriendsChat } from "./use-friends-chat";

const CONTACT_PAGE_SIZE = 20;

function isPlainRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value && typeof value === "object" && !Array.isArray(value));
}

function dataArray(value: unknown) {
  return isPlainRecord(value) && Array.isArray(value.data) ? value.data : [];
}

function mergeFriendStatusResponse(current: any, result: any, append: boolean) {
  if (!append || !current) return result;
  const mergeSection = (left: unknown, right: unknown) => ({
    ...(isPlainRecord(left) ? left : {}),
    ...(isPlainRecord(right) ? right : {}),
    data: [...dataArray(left), ...dataArray(right)],
  });
  return {
    ...result,
    sec_user_ids: Array.from(new Set([...(current.sec_user_ids || []), ...(result.sec_user_ids || [])])),
    all_sec_user_ids: result.all_sec_user_ids || current.all_sec_user_ids,
    user_info: mergeSection(current.user_info, result.user_info),
    active_status: mergeSection(current.active_status, result.active_status),
  };
}

export function FriendsStatusView() {
  const setView = useAppStore((state) => state.setView);
  const openUser = useSearchStore((state) => state.openUser);
  const { downloadVideo } = useDownloads();
  const [input, setInput] = useState(() => localStorage.getItem(STORAGE_KEY) || "");
  const [currentSecUid, setCurrentSecUid] = useState<string>("");

  const [savedIds, setSavedIds] = useState<string[]>([]);
  const [savedCount, setSavedCount] = useState(0);
  const [includeAllUsers, setIncludeAllUsers] = useState(false);
  const [refreshIntervalSeconds, setRefreshIntervalSeconds] = useState(DEFAULT_REFRESH_INTERVAL_SECONDS);
  const [currentUserAvatar, setCurrentUserAvatar] = useState(() => localStorage.getItem(CURRENT_USER_AVATAR_KEY) || "");

  const [showManualInput, setShowManualInput] = useState(false);
  const [loading, setLoading] = useState(false);
  const [backgroundRefreshing, setBackgroundRefreshing] = useState(false);
  const [sharedPlayerVideos, setSharedPlayerVideos] = useState<VideoInfo[]>([]);
  const [sharedPlayerOpen, setSharedPlayerOpen] = useState(false);
  const [sharedPlayerLoadingId, setSharedPlayerLoadingId] = useState("");
  const [error, setError] = useState("");
  const [response, setResponse] = useState<any>(null);
  const [contactHasMore, setContactHasMore] = useState(false);
  const [contactNextOffset, setContactNextOffset] = useState(0);
  const [contactPageLoading, setContactPageLoading] = useState(false);
  const [lastUpdatedAt, setLastUpdatedAt] = useState(0);

  const idsRef = useRef<string[]>([]);
  const savedIdsRef = useRef<string[]>([]);
  const lastQueryIdsRef = useRef<string[]>([]);
  const lastQueryStartedAtRef = useRef(0);
  const queryInFlightRef = useRef(false);
  const contactPageLoadingRef = useRef(false);
  const contactHasMoreRef = useRef(false);
  const contactNextOffsetRef = useRef(0);
  const pendingBackgroundQueryRef = useRef(false);
  const pendingBackgroundTimerRef = useRef<number | null>(null);
  const cookieRetryTimerRef = useRef<number | null>(null);
  const avatarRetryTimerRef = useRef<number | null>(null);
  const initialInputRef = useRef(input);

  useEffect(() => {
    let active = true;
    getAccounts().then((res) => {
      if (active && res.success && res.current_sec_uid) {
        const uid = res.current_sec_uid;
        setCurrentSecUid(uid);
      }
    }).catch(() => {});
    return () => { active = false; };
  }, []);

  const ids = useMemo(() => extractIds(input), [input]);
  const friends = useMemo(() => {
    if (!response?.success) return [];
    const rawFriends = mapResponse(response);
    if (!currentSecUid) return rawFriends;
    return rawFriends.filter((f) => f.secUid !== currentSecUid);
  }, [response, currentSecUid]);

  const {
    chatDrafts,
    chatMessages,
    unreadCounts,
    chatSummaries,
    historyState,
    selectedFriendId,
    selectedFriend,
    selectedMessages,
    selectedHistory,
    imStatus,
    updateDraft,
    sendLocalMessage,
    sendLocalImageMessage,
    loadHistoryMessages,
    selectFriend,
  } = useFriendsChat(friends, currentSecUid, setError);

  const friendItems = useMemo<FriendListItem[]>(() => friends
    .map((friend) => {
      const latestMessage = latestChatMessage(chatMessages[friend.secUid]);
      const persistedSummary = chatSummaries[friend.secUid];
      const displayMessage = latestMessage && latestMessage.createdAt >= (persistedSummary?.latestMessageAt || 0)
        ? latestMessage
        : persistedSummary?.latestMessage;
      const displayText = messagePreviewText(displayMessage || latestMessage);
      const previewText = latestMessage
        ? `${displayMessage?.direction === "out" ? "我：" : ""}${displayText || latestMessage.text}`
        : displayMessage
          ? `${displayMessage.direction === "out" ? "我：" : ""}${displayText}`
          : friend.signature || friend.secUid;
      return {
        ...friend,
        latestMessage: displayMessage,
        latestMessageAt: Math.max(
          latestMessage?.createdAt || 0,
          persistedSummary?.latestMessageAt || 0,
          friend.serverLatestMessageAt || 0,
        ),
        previewText,
        unreadCount: Math.max(unreadCounts[friend.secUid] || 0, persistedSummary?.unreadCount || 0),
      };
    })
    .sort((a, b) => {
      if (a.latestMessageAt || b.latestMessageAt) {
        return b.latestMessageAt - a.latestMessageAt;
      }
      return 0;
    }), [chatMessages, chatSummaries, friends, unreadCounts]);

  const onlineCount = friends.filter((friend) => friend.online).length;
  const offlineCount = friends.filter((friend) => !friend.online).length;
  const isInitialLoading = loading && friends.length === 0;

  const openFriendProfile = useCallback(
    async (friend: FriendStatusItem) => {
      const user: UserInfo = {
        uid: friend.uid,
        nickname: friend.remarkName || friend.nickname || "未知用户",
        avatar_thumb: friend.avatar,
        avatar_medium: friend.avatar,
        avatar_larger: friend.avatar,
        signature: friend.signature,
        follower_count: 0,
        following_count: 0,
        total_favorited: 0,
        aweme_count: 0,
        favoriting_count: 0,
        is_follow: false,
        follow_status: 0,
        sec_uid: friend.secUid,
        unique_id: "",
        verify_status: 0,
      };
      setView("user");
      await openUser(user, { loadVideos: true });
    },
    [openUser, setView],
  );

  const openSharedVideo = useCallback(async (card: SharedMessageCard) => {
    if (!card.itemId || sharedPlayerLoadingId) return;
    setSharedPlayerLoadingId(card.itemId);
    setError("");
    try {
      const result = await getVideoDetail(card.itemId);
      if (!result.success || !result.video) {
        throw new Error(result.message || "无法加载分享视频");
      }
      setSharedPlayerVideos([result.video]);
      setSharedPlayerOpen(true);
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : "无法加载分享视频");
    } finally {
      setSharedPlayerLoadingId("");
    }
  }, [sharedPlayerLoadingId]);

  const query = useCallback(async (overrideIds?: string[], options?: { background?: boolean; retryCookie?: boolean; append?: boolean; offset?: number }) => {
    const background = Boolean(options?.background);
    const append = Boolean(options?.append);
    if (background && Date.now() - lastQueryStartedAtRef.current < MIN_BACKGROUND_REFRESH_INTERVAL_MS) {
      return;
    }
    if (queryInFlightRef.current) {
      if (background) pendingBackgroundQueryRef.current = true;
      return;
    }
    const retryCookie = options?.retryCookie !== false;
    const baseIds = overrideIds ?? savedIdsRef.current;
    const queryIds = Array.from(new Set([...baseIds, ...idsRef.current]));
    const offset = append ? Math.max(0, Number(options?.offset ?? contactNextOffsetRef.current) || 0) : 0;
    queryInFlightRef.current = true;
    lastQueryStartedAtRef.current = Date.now();
    if (append) {
      setContactPageLoading(true);
    } else if (!background) {
      setError("");
      setLoading(true);
    } else {
      setBackgroundRefreshing(true);
    }
    try {
      localStorage.setItem(STORAGE_KEY, queryIds.join("\n"));
      const result = await getFriendOnlineStatus(queryIds, [], { offset, limit: CONTACT_PAGE_SIZE });
      const pageFriends = mapResponse(result);
      const hasUsableData = Boolean(
        result.success ||
        (Array.isArray(result.sec_user_ids) && result.sec_user_ids.length > 0) ||
        pageFriends.length > 0,
      );
      if (hasUsableData) {
        setResponse((current: any) => mergeFriendStatusResponse(current, result, append));
        setContactHasMore(Boolean(result.has_more));
        setContactNextOffset(Number(result.next_offset || 0) || 0);
        lastQueryIdsRef.current = queryIds;
        setLastUpdatedAt(Date.now());
        setError("");
      }
      const allSecUserIds = Array.isArray(result.all_sec_user_ids)
        ? result.all_sec_user_ids
        : !append && Array.isArray(result.sec_user_ids)
          ? result.sec_user_ids
          : [];
      if (hasUsableData && allSecUserIds.length > 0) {
        setSavedIds(allSecUserIds);
        setSavedCount(Number(result.total_count || allSecUserIds.length) || allSecUserIds.length);
        setInput(allSecUserIds.join("\n"));
        localStorage.setItem(STORAGE_KEY, allSecUserIds.join("\n"));
      }
      if (!hasUsableData && !background) {
        const message = result.message || "获取好友在线状态失败";
        if (retryCookie && COOKIE_REQUIRED_PATTERN.test(message)) {
          const config = await getConfig().catch(() => null);
          if (config?.cookie_set) {
            setError("");
            if (cookieRetryTimerRef.current !== null) {
              window.clearTimeout(cookieRetryTimerRef.current);
            }
            cookieRetryTimerRef.current = window.setTimeout(() => {
              cookieRetryTimerRef.current = null;
              void query(queryIds, { background, retryCookie: false });
            }, 700);
          } else {
            setError(message);
          }
        } else {
          setError(message);
        }
      }
    } catch (caught) {
      if (!background) {
        const message = caught instanceof Error ? caught.message : "获取好友在线状态失败";
        if (retryCookie && COOKIE_REQUIRED_PATTERN.test(message)) {
          const config = await getConfig().catch(() => null);
          if (config?.cookie_set) {
            setError("");
            if (cookieRetryTimerRef.current !== null) {
              window.clearTimeout(cookieRetryTimerRef.current);
            }
            cookieRetryTimerRef.current = window.setTimeout(() => {
              cookieRetryTimerRef.current = null;
              void query(queryIds, { background, retryCookie: false });
            }, 700);
          } else {
            setError(message);
          }
        } else {
          setError(message);
        }
      }
    } finally {
      queryInFlightRef.current = false;
      if (append) {
        setContactPageLoading(false);
      } else if (background) {
        setBackgroundRefreshing(false);
      } else {
        setLoading(false);
      }
      if (pendingBackgroundQueryRef.current) {
        pendingBackgroundQueryRef.current = false;
        if (pendingBackgroundTimerRef.current !== null) {
          window.clearTimeout(pendingBackgroundTimerRef.current);
        }
        pendingBackgroundTimerRef.current = window.setTimeout(() => {
          pendingBackgroundTimerRef.current = null;
          void query(undefined, { background: true });
        }, MIN_BACKGROUND_REFRESH_INTERVAL_MS);
      }
    }
  }, []);

  const loadMoreContacts = useCallback(() => {
    if (queryInFlightRef.current || contactPageLoadingRef.current || !contactHasMoreRef.current) return;
    void query(lastQueryIdsRef.current, { append: true, offset: contactNextOffsetRef.current });
  }, [query]);

  useEffect(() => {
    idsRef.current = ids;
  }, [ids]);

  useEffect(() => {
    savedIdsRef.current = savedIds;
  }, [savedIds]);

  useEffect(() => {
    contactPageLoadingRef.current = contactPageLoading;
  }, [contactPageLoading]);

  useEffect(() => {
    contactHasMoreRef.current = contactHasMore;
  }, [contactHasMore]);

  useEffect(() => {
    contactNextOffsetRef.current = contactNextOffset;
  }, [contactNextOffset]);

  useEffect(() => () => {
    if (cookieRetryTimerRef.current !== null) {
      window.clearTimeout(cookieRetryTimerRef.current);
    }
    if (avatarRetryTimerRef.current !== null) {
      window.clearTimeout(avatarRetryTimerRef.current);
    }
    if (pendingBackgroundTimerRef.current !== null) {
      window.clearTimeout(pendingBackgroundTimerRef.current);
    }
  }, []);

  useEffect(() => {
    let disposed = false;
    void getConfig()
      .then((config) => {
        if (disposed) return;
        const savedIds = Array.isArray(config.im_friend_sec_user_ids)
          ? config.im_friend_sec_user_ids.filter(Boolean)
          : [];
        setSavedIds(savedIds);
        setSavedCount(savedIds.length);
        const nextInterval = Number(config.im_friend_refresh_interval_seconds) || DEFAULT_REFRESH_INTERVAL_SECONDS;
        setIncludeAllUsers(Boolean(config.im_friend_include_all_users));
        setRefreshIntervalSeconds(Math.max(0, nextInterval));
        if (!initialInputRef.current.trim() && savedIds.length > 0) {
          setInput(savedIds.join("\n"));
        }
        void query(savedIds);
      })
      .catch(() => {
        if (!disposed) void query([]);
      });
    return () => {
      disposed = true;
    };
  }, [query]);

  useEffect(() => {
    let disposed = false;
    let unlistenCookieLogin: (() => void) | null = null;
    const saveAvatar = (avatar: string) => {
      if (!avatar) return;
      setCurrentUserAvatar(avatar);
      localStorage.setItem(CURRENT_USER_AVATAR_KEY, avatar);
    };
    const retry = (attempt: number) => {
      if (disposed || attempt >= 8) return;
      if (avatarRetryTimerRef.current !== null) {
        window.clearTimeout(avatarRetryTimerRef.current);
      }
      avatarRetryTimerRef.current = window.setTimeout(() => {
        avatarRetryTimerRef.current = null;
        void loadAvatar(attempt + 1);
      }, 700 + attempt * 700);
    };
    const loadAvatar = async (attempt = 0) => {
      try {
        const config = await getConfig().catch(() => null);
        if (disposed || !config?.cookie_set) return;
        const status = await verifyCookie();
        if (disposed) return;
        if (!status.valid) {
          retry(attempt);
          return;
        }
        const directAvatar = status.avatar_thumb || status.avatar_medium || status.avatar_larger || "";
        if (directAvatar) {
          saveAvatar(directAvatar);
          return;
        }
        const secUid = status.sec_uid || (status.user_id?.startsWith("MS4") ? status.user_id : "");
        if (!secUid) {
          retry(attempt);
          return;
        }
        const detail = await getUserDetail(secUid).catch(() => null);
        if (disposed || !detail?.success || !detail.user) return;
        const detailAvatar = detail.user.avatar_thumb || detail.user.avatar_medium || detail.user.avatar_larger || "";
        if (detailAvatar) {
          saveAvatar(detailAvatar);
        } else {
          retry(attempt);
        }
      } catch (caught) {
        const message = caught instanceof Error ? caught.message : "";
        if (COOKIE_REQUIRED_PATTERN.test(message) || attempt < 8) retry(attempt);
      }
    };
    void loadAvatar();
    void listenEvent<{ event?: string; cookie_set?: boolean }>("cookie-login-status", (payload) => {
      if (payload?.cookie_set || payload?.event === "success") {
        void loadAvatar();
      }
    }).then((unlisten) => {
      if (disposed) {
        unlisten();
        return;
      }
      unlistenCookieLogin = unlisten;
    });
    return () => {
      disposed = true;
      unlistenCookieLogin?.();
      if (avatarRetryTimerRef.current !== null) {
        window.clearTimeout(avatarRetryTimerRef.current);
      }
    };
  }, []);

  const toggleIncludeAllUsers = async () => {
    const nextValue = !includeAllUsers;
    const previousValue = includeAllUsers;
    setIncludeAllUsers(nextValue);
    setError("");
    try {
      const result = await saveConfig({ im_friend_include_all_users: nextValue });
      if (!result.success) {
        throw new Error(result.message || "保存好友范围设置失败");
      }
      void query([], { background: friends.length > 0 });
    } catch (caught) {
      setIncludeAllUsers(previousValue);
      setError(caught instanceof Error ? caught.message : "保存好友范围设置失败");
    }
  };

  useEffect(() => {
    if (refreshIntervalSeconds <= 0) return;
    const timer = window.setInterval(() => {
      void query(undefined, { background: true });
    }, Math.max(1, refreshIntervalSeconds) * 1000);
    return () => window.clearInterval(timer);
  }, [query, refreshIntervalSeconds]);

  return (
    <div className="mx-auto flex h-full min-h-0 w-full max-w-[1320px] flex-col gap-3 overflow-hidden">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="flex min-w-0 items-center gap-2">
          <Activity className="h-4 w-4 text-accent" />
          <h3 className="text-[0.95rem] font-semibold text-text">好友</h3>
          <span className="truncate text-[0.72rem] text-text-muted">
            {savedCount > 0 ? `已保存 ${savedCount}` : `${friends.length || ids.length} 个好友`}
            {backgroundRefreshing
              ? " · 正在更新"
              : lastUpdatedAt
                ? ` · 上次更新于 ${formatUpdateTime(lastUpdatedAt)}`
                : ""}
          </span>
          <span
            className={cn(
              "flex h-6 shrink-0 items-center gap-1.5 rounded-full border px-2 text-[0.68rem]",
              imStatus.connected
                ? "border-success/25 bg-success-soft text-success"
                : "border-border bg-surface-solid text-text-muted",
            )}
            title={imStatus.updatedAt ? `${imStatus.message} · ${formatUpdateTime(imStatus.updatedAt)}` : imStatus.message}
          >
            <span
              className={cn(
                "h-1.5 w-1.5 rounded-full",
                imStatus.connected ? "bg-success" : "bg-text-muted",
              )}
            />
            {imStatus.connected ? "接收已连接" : "接收未连接"}
          </span>
        </div>
        <div className="relative flex flex-wrap items-center gap-2">
          <Button
            variant="outline"
            size="sm"
            onClick={() => setShowManualInput((value) => !value)}
            className="h-9"
          >
            备用 ID
            <Badge variant="secondary" size="sm">{ids.length}</Badge>
          </Button>
          {showManualInput && (
            <div className="absolute right-0 top-11 z-30 w-[min(420px,calc(100vw-2rem))] rounded-[var(--radius-lg)] border border-border bg-background p-3 shadow-[0_18px_42px_rgba(15,23,42,0.16)]">
              <div className="mb-2 flex items-center justify-between gap-2">
                <span className="text-[0.76rem] font-semibold text-text-secondary">备用 ID 输入</span>
                <button
                  type="button"
                  onClick={() => setShowManualInput(false)}
                  className="text-[0.72rem] text-text-muted hover:text-text"
                >
                  收起
                </button>
              </div>
              <Textarea
                value={input}
                onChange={(event) => setInput(event.target.value)}
                placeholder="MS4w... 每行一个，或粘贴 curl 参数"
                className="min-h-[136px] resize-none bg-surface-solid"
                spellCheck={false}
              />
            </div>
          )}
          <button
            type="button"
            role="switch"
            aria-checked={includeAllUsers}
            onClick={() => void toggleIncludeAllUsers()}
            disabled={loading}
            className={cn(
              "flex h-9 items-center gap-2 rounded-[var(--radius-sm)] border px-3 text-[0.76rem] transition",
              includeAllUsers
                ? "border-accent/35 bg-accent-soft text-accent"
                : "border-border bg-surface-solid text-text-muted hover:text-text",
              loading && "cursor-not-allowed opacity-60",
            )}
          >
            <span
              className={cn(
                "relative h-4 w-7 rounded-full transition",
                includeAllUsers ? "bg-accent" : "bg-border-strong",
              )}
            >
              <span
                className={cn(
                  "absolute top-0.5 h-3 w-3 rounded-full bg-white shadow-sm transition",
                  includeAllUsers ? "left-3.5" : "left-0.5",
                )}
              />
            </span>
            {includeAllUsers ? "全部用户" : "仅互关"}
          </button>
          <Button size="sm" onClick={() => void query()} disabled={loading} className="h-9">
            {loading ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <RefreshCw className="h-3.5 w-3.5" />}
            刷新状态
          </Button>
        </div>
      </div>

      {error && (
        <div className="rounded-[var(--radius-sm)] border border-white/[0.06] bg-danger-soft px-3 py-2 text-[0.78rem] text-danger">
          {error}
        </div>
      )}

      <div className="grid min-h-0 flex-1 gap-3 overflow-hidden lg:grid-cols-[380px_minmax(0,1fr)] xl:grid-cols-[420px_minmax(0,1fr)]">
        <FriendListPanel
          friends={friends}
          friendItems={friendItems}
          selectedFriendId={selectedFriendId}
          onlineCount={onlineCount}
          offlineCount={offlineCount}
          isInitialLoading={isInitialLoading}
          isLoadingMore={contactPageLoading}
          hasMore={contactHasMore}
          idsLength={ids.length}
          selectFriend={selectFriend}
          openFriendProfile={openFriendProfile}
          onLoadMore={loadMoreContacts}
        />

        <ChatWorkspace
          friend={selectedFriend}
          draft={selectedFriend ? chatDrafts[selectedFriend.secUid] || "" : ""}
          messages={selectedMessages}
          historyError={selectedHistory?.error || ""}
          historyLoading={Boolean(selectedHistory?.loading)}
          canLoadOlder={Boolean(selectedFriend && selectedHistory?.nextCursor && selectedHistory.hasMore !== false)}
          currentUserAvatar={currentUserAvatar}
          onDraftChange={updateDraft}
          onSendMessage={sendLocalMessage}
          onSendImage={sendLocalImageMessage}
          onLoadOlder={() => selectedFriend && selectedHistory?.nextCursor ? loadHistoryMessages(selectedFriend, selectedHistory.nextCursor) : Promise.resolve()}
          onOpenProfile={openFriendProfile}
          onOpenSharedVideo={openSharedVideo}
          sharedPlayerLoadingId={sharedPlayerLoadingId}
        />
      </div>
      <FullscreenPlayer
        videos={sharedPlayerVideos}
        initialIndex={0}
        open={sharedPlayerOpen}
        onClose={() => setSharedPlayerOpen(false)}
        onDownload={(video) => downloadVideo(video)}
      />
    </div>
  );
}
