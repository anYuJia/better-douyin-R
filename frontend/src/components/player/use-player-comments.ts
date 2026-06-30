import { useCallback, useEffect, useRef, useState, type UIEvent as ReactUIEvent, type MouseEvent as ReactMouseEvent, type PointerEvent as ReactPointerEvent } from "react";
import { getComments, getCommentReplies, publishComment, setCommentLiked, type CommentInfo, type VideoInfo } from "@/lib/tauri";
import type { CommentRepliesState, CommentReplyTarget, PlayerPanel } from "./player-types";

interface UsePlayerCommentsProps {
  open: boolean;
  openPanel: PlayerPanel | null;
  currentVideo: VideoInfo | null;
  showNavigationNotice: (message: string) => void;
  clearPanelCloseTimer: () => void;
  setOpenPanel: (panel: PlayerPanel | null) => void;
  openComments?: boolean;
  initialComment?: {
    cid: string;
    text: string;
    digg_count: number;
    create_time: number;
    user: { uid: string; nickname: string; sec_uid: string; avatar: string };
  } | null;
}

export function usePlayerComments({
  open,
  openPanel,
  currentVideo,
  showNavigationNotice,
  clearPanelCloseTimer,
  setOpenPanel,
  openComments = false,
  initialComment = null,
}: UsePlayerCommentsProps) {
  const [commentsOpen, setCommentsOpen] = useState(false);
  const [comments, setComments] = useState<CommentInfo[]>([]);
  const [commentsLoading, setCommentsLoading] = useState(false);
  const [commentsError, setCommentsError] = useState("");
  const [commentsCursor, setCommentsCursor] = useState(0);
  const [commentsHasMore, setCommentsHasMore] = useState(false);
  const [commentsTotal, setCommentsTotal] = useState(0);
  const [commentsLoadedAwemeId, setCommentsLoadedAwemeId] = useState("");
  const [commentReplies, setCommentReplies] = useState<CommentRepliesState>({});
  const [expandedCommentReplyIds, setExpandedCommentReplyIds] = useState<Set<string>>(() => new Set());
  const [commentDiggingIds, setCommentDiggingIds] = useState<Set<string>>(() => new Set());
  const [commentDraft, setCommentDraft] = useState("");
  const [commentSubmitting, setCommentSubmitting] = useState(false);
  const [commentReplyTarget, setCommentReplyTarget] = useState<CommentReplyTarget>(null);
  // 通知跳转定位：高亮目标 cid + 定位失败提示。
  const [highlightCid, setHighlightCid] = useState("");
  const [locatePrompt, setLocatePrompt] = useState<"" | "deleted" | "not_in_first_pages">("");
  const commentItemRefs = useRef<Map<string, HTMLDivElement>>(new Map());
  const replyItemRefs = useRef<Map<string, HTMLDivElement>>(new Map());
  const locateDoneRef = useRef(false);
  const locatePageRef = useRef(0);

  const commentsHoverCloseTimerRef = useRef<ReturnType<typeof window.setTimeout> | null>(null);
  const commentsPanelStickyRef = useRef(false);

  const clearCommentsHoverCloseTimer = useCallback(() => {
    if (commentsHoverCloseTimerRef.current) {
      window.clearTimeout(commentsHoverCloseTimerRef.current);
      commentsHoverCloseTimerRef.current = null;
    }
  }, []);

  useEffect(() => {
    return () => {
      if (commentsHoverCloseTimerRef.current) {
        window.clearTimeout(commentsHoverCloseTimerRef.current);
      }
    };
  }, []);

  // Reset comments state on video change
  useEffect(() => {
    setOpenPanel(null);
    setComments([]);
    setCommentsError("");
    setCommentsCursor(0);
    setCommentsHasMore(false);
    setCommentsTotal(0);
    setCommentsLoadedAwemeId("");
    setCommentReplies({});
    setExpandedCommentReplyIds(new Set());
  }, [currentVideo?.aweme_id, setOpenPanel]);

  // Handle comments-specific resets on player close
  useEffect(() => {
    if (!open) {
      clearCommentsHoverCloseTimer();
      commentsPanelStickyRef.current = false;
      setCommentsOpen(false);
    }
  }, [open, clearCommentsHoverCloseTimer]);

  // Close comments panel if another tool panel gets opened
  useEffect(() => {
    if (openPanel) {
      clearCommentsHoverCloseTimer();
      commentsPanelStickyRef.current = false;
      setCommentsOpen(false);
    }
  }, [openPanel, clearCommentsHoverCloseTimer]);

  const loadComments = useCallback(async (mode: "initial" | "more" = "initial") => {
    if (!currentVideo?.aweme_id || commentsLoading) return;
    const isMore = mode === "more";
    setCommentsLoading(true);
    setCommentsError("");
    try {
      const result = await getComments(currentVideo.aweme_id, 20, isMore ? commentsCursor : 0);
      if (!result.success) {
        throw new Error(result.message || "获取评论失败");
      }
      const nextComments = Array.isArray(result.comments) ? result.comments : [];
      setComments((prev) => (isMore ? [...prev, ...nextComments] : nextComments));
      setCommentsCursor(Number(result.cursor || 0));
      setCommentsHasMore(Boolean(result.has_more));
      setCommentsTotal(Number(result.total || 0));
      setCommentsLoadedAwemeId(currentVideo.aweme_id);
    } catch (error) {
      setCommentsError(error instanceof Error ? error.message : "获取评论失败");
      if (!isMore) {
        setComments([]);
        setCommentsHasMore(false);
      }
    } finally {
      setCommentsLoading(false);
    }
  }, [commentsCursor, commentsLoading, currentVideo?.aweme_id]);

  const loadCommentReplies = useCallback(async (comment: CommentInfo, mode: "initial" | "more" = "initial") => {
    if (!currentVideo?.aweme_id || !comment.cid) return;
    const currentState = commentReplies[comment.cid];
    if (currentState?.loading) return;
    const isMore = mode === "more";
    const cursor = isMore ? currentState?.cursor || 0 : 0;

    setCommentReplies((prev) => ({
      ...prev,
      [comment.cid]: {
        items: isMore ? prev[comment.cid]?.items || [] : prev[comment.cid]?.items || [],
        cursor,
        hasMore: isMore ? prev[comment.cid]?.hasMore ?? false : prev[comment.cid]?.hasMore ?? false,
        loading: true,
        error: "",
        total: prev[comment.cid]?.total || comment.reply_comment_total || 0,
        loaded: prev[comment.cid]?.loaded || false,
      },
    }));

    try {
      const result = await getCommentReplies(currentVideo.aweme_id, comment.cid, 6, cursor);
      if (!result.success) {
        throw new Error(result.message || "获取回复失败");
      }
      const nextReplies = Array.isArray(result.comments) ? result.comments : [];
      setCommentReplies((prev) => {
        const previous = prev[comment.cid];
        return {
          ...prev,
          [comment.cid]: {
            items: isMore ? [...(previous?.items || []), ...nextReplies] : nextReplies,
            cursor: Number(result.cursor || 0),
            hasMore: Boolean(result.has_more),
            loading: false,
            error: "",
            total: Number(result.total || comment.reply_comment_total || nextReplies.length || 0),
            loaded: true,
          },
        };
      });
    } catch (error) {
      setCommentReplies((prev) => ({
        ...prev,
        [comment.cid]: {
          items: prev[comment.cid]?.items || [],
          cursor: prev[comment.cid]?.cursor || 0,
          hasMore: prev[comment.cid]?.hasMore || false,
          loading: false,
          error: error instanceof Error ? error.message : "获取回复失败",
          total: prev[comment.cid]?.total || comment.reply_comment_total || 0,
          loaded: true,
        },
      }));
    }
  }, [commentReplies, currentVideo?.aweme_id]);

  const toggleCommentReplies = useCallback((comment: CommentInfo) => {
    const willExpand = !expandedCommentReplyIds.has(comment.cid);
    setExpandedCommentReplyIds((prev) => {
      const next = new Set(prev);
      if (willExpand) {
        next.add(comment.cid);
      } else {
        next.delete(comment.cid);
      }
      return next;
    });
    const replyState = commentReplies[comment.cid];
    if (willExpand && !replyState?.loaded && !replyState?.loading) {
      void loadCommentReplies(comment, "initial");
    }
  }, [commentReplies, expandedCommentReplyIds, loadCommentReplies]);

  const updateCommentById = useCallback((commentId: string, updater: (comment: CommentInfo) => CommentInfo) => {
    setComments((prev) => prev.map((comment) => (comment.cid === commentId ? updater(comment) : comment)));
    setCommentReplies((prev) => {
      const next: CommentRepliesState = {};
      let anyChanged = false;
      for (const [cid, state] of Object.entries(prev)) {
        let itemsChanged = false;
        const nextItems = state.items.map((reply) => {
          if (reply.cid !== commentId) return reply;
          itemsChanged = true;
          return updater(reply);
        });
        if (itemsChanged) {
          anyChanged = true;
          next[cid] = { ...state, items: nextItems };
        } else {
          next[cid] = state;
        }
      }
      return anyChanged ? next : prev;
    });
  }, []);

  const toggleCommentLike = useCallback(async (comment: CommentInfo, level: number) => {
    if (!currentVideo?.aweme_id || !comment.cid || commentDiggingIds.has(comment.cid)) return;
    const wasLiked = Number(comment.user_digged || 0) > 0;
    const nextLiked = !wasLiked;
    const delta = nextLiked ? 1 : -1;

    setCommentDiggingIds((prev) => new Set(prev).add(comment.cid));
    updateCommentById(comment.cid, (item) => ({
      ...item,
      user_digged: nextLiked ? 1 : 0,
      digg_count: Math.max(0, Number(item.digg_count || 0) + delta),
    }));

    try {
      const result = await setCommentLiked(currentVideo.aweme_id, comment.cid, nextLiked, level);
      if (!result.success) {
        throw new Error(result.message || "评论点赞失败");
      }
    } catch (error) {
      updateCommentById(comment.cid, (item) => ({
        ...item,
        user_digged: wasLiked ? 1 : 0,
        digg_count: Math.max(0, Number(item.digg_count || 0) - delta),
      }));
      showNavigationNotice(error instanceof Error ? error.message : "评论点赞失败");
    } finally {
      setCommentDiggingIds((prev) => {
        const next = new Set(prev);
        next.delete(comment.cid);
        return next;
      });
    }
  }, [commentDiggingIds, currentVideo?.aweme_id, showNavigationNotice, updateCommentById]);

  const submitComment = useCallback(async () => {
    const text = commentDraft.trim();
    if (!currentVideo?.aweme_id || !text || commentSubmitting) return;
    const target = commentReplyTarget;
    setCommentSubmitting(true);
    try {
      const result = await publishComment(
        currentVideo.aweme_id,
        text,
        target?.replyId || "",
        target?.replyToReplyId || ""
      );
      if (!result.success) {
        throw new Error(result.message || "发表评论失败");
      }
      const created = result.comment;
      if (created?.cid) {
        if (target?.replyId) {
          setExpandedCommentReplyIds((prev) => new Set(prev).add(target.replyId));
          setCommentReplies((prev) => {
            const current = prev[target.replyId] || {
              items: [],
              cursor: 0,
              hasMore: false,
              loading: false,
              error: "",
              total: 0,
              loaded: true,
            };
            return {
              ...prev,
              [target.replyId]: {
                ...current,
                items: [created, ...current.items],
                total: current.total + 1,
                loaded: true,
              },
            };
          });
          updateCommentById(target.replyId, (item) => ({
            ...item,
            reply_comment_total: Number(item.reply_comment_total || 0) + 1,
          }));
        } else {
          setComments((prev) => [created, ...prev]);
          setCommentsTotal((prev) => prev + 1);
        }
      } else if (target?.replyId) {
        void loadCommentReplies({ cid: target.replyId } as CommentInfo, "initial");
      } else {
        void loadComments("initial");
      }
      setCommentDraft("");
      setCommentReplyTarget(null);
      showNavigationNotice("评论已发布");
    } catch (error) {
      showNavigationNotice(error instanceof Error ? error.message : "发表评论失败");
    } finally {
      setCommentSubmitting(false);
    }
  }, [
    commentDraft,
    commentReplyTarget,
    commentSubmitting,
    currentVideo?.aweme_id,
    loadCommentReplies,
    loadComments,
    showNavigationNotice,
    updateCommentById,
  ]);

  const handleCommentsScroll = useCallback((event: ReactUIEvent<HTMLDivElement>) => {
    const target = event.currentTarget;
    const distanceToBottom = target.scrollHeight - target.scrollTop - target.clientHeight;
    if (distanceToBottom > 96 || commentsLoading || !commentsHasMore) return;
    void loadComments("more");
  }, [commentsHasMore, commentsLoading, loadComments]);

  const openCommentsPanel = useCallback((event?: ReactMouseEvent | ReactPointerEvent<HTMLElement>, options?: { sticky?: boolean }) => {
    event?.stopPropagation();
    clearPanelCloseTimer();
    clearCommentsHoverCloseTimer();
    if (options?.sticky) {
      commentsPanelStickyRef.current = true;
    } else if (!commentsOpen) {
      commentsPanelStickyRef.current = false;
    }
    setOpenPanel(null);
    setCommentsOpen(true);
  }, [clearCommentsHoverCloseTimer, clearPanelCloseTimer, commentsOpen, setOpenPanel]);

  const markCommentsPanelSticky = useCallback((event?: ReactMouseEvent | ReactPointerEvent<HTMLElement>) => {
    event?.stopPropagation();
    clearCommentsHoverCloseTimer();
    commentsPanelStickyRef.current = true;
  }, [clearCommentsHoverCloseTimer]);

  const scheduleTransientCommentsClose = useCallback((event?: ReactMouseEvent | ReactPointerEvent<HTMLElement>) => {
    event?.stopPropagation();
    clearCommentsHoverCloseTimer();
    commentsHoverCloseTimerRef.current = window.setTimeout(() => {
      if (!commentsPanelStickyRef.current) {
        setCommentsOpen(false);
      }
      commentsHoverCloseTimerRef.current = null;
    }, 180);
  }, [clearCommentsHoverCloseTimer]);

  const closeCommentsPanel = useCallback((event?: ReactMouseEvent) => {
    event?.stopPropagation();
    clearCommentsHoverCloseTimer();
    commentsPanelStickyRef.current = false;
    setCommentsOpen(false);
  }, [clearCommentsHoverCloseTimer]);

  useEffect(() => {
    if (!commentsOpen || !currentVideo?.aweme_id || commentsLoadedAwemeId === currentVideo.aweme_id) return;
    void loadComments("initial");
  }, [commentsLoadedAwemeId, commentsOpen, currentVideo?.aweme_id, loadComments]);

  // 通知跳转：打开时强制展开评论区（降级类型 41/45 也展开）。
  useEffect(() => {
    if (!open) return;
    if (openComments || initialComment) {
      setCommentsOpen(true);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  // 通知跳转：用 insert_ids=cid 拉评论列表，目标评论会被插入返回。
  // cid 在根列表 → 置顶高光根评论；cid 在某根评论的 sub_comments → 展开该根评论高光子评论。
  // 不等 commentsLoadedAwemeId——抢先调 insert_ids 一步到位，省掉自动拉取的普通请求，
  // 避免普通+insert_ids 两次串行请求导致进入慢。
  useEffect(() => {
    if (!initialComment || locateDoneRef.current) return;
    if (locatePageRef.current > 0) return; // 仅拉一次
    locatePageRef.current = 1;
    const { cid, text, digg_count, create_time, user } = initialComment;
    const awemeId = currentVideo?.aweme_id;
    if (!awemeId) return;
    // 立即占位标记已加载，阻止自动拉取 effect 并发跑普通请求。
    setCommentsLoadedAwemeId(awemeId);
    setCommentsLoading(true);

    const highlight = (targetCid: string) => {
      setHighlightCid(targetCid);
      window.setTimeout(() => setHighlightCid((cur) => (cur === targetCid ? "" : cur)), 2400);
      window.requestAnimationFrame(() => {
        const node = commentItemRefs.current.get(targetCid) || replyItemRefs.current.get(targetCid);
        if (node) node.scrollIntoView({ block: "center" });
      });
    };

    void (async () => {
      try {
        const result = await getComments(awemeId, 20, 0, cid);
        if (!result.success || !result.comments) {
          locateDoneRef.current = true;
          setLocatePrompt("deleted");
          return;
        }
        // cid 在根列表 → 置顶高光。
        const asRoot = result.comments.find((c) => c.cid === cid);
        if (asRoot) {
          const ordered = [asRoot, ...result.comments.filter((c) => c.cid !== cid)];
          setComments(ordered);
          setCommentsLoadedAwemeId(awemeId);
          locateDoneRef.current = true;
          highlight(cid);
          return;
        }
        // cid 在某根评论的 sub_comments → 展开该根评论高光子评论。
        const host = result.comments.find((c) => (c.sub_comments || []).some((s) => s.cid === cid));
        if (host) {
          const subReplies = (host.sub_comments || []) as CommentInfo[];
          const ordered = [
            ...subReplies.filter((r) => r.cid === cid),
            ...subReplies.filter((r) => r.cid !== cid),
          ];
          const hostOrdered = [host, ...result.comments.filter((c) => c.cid !== host.cid)];
          setComments(hostOrdered);
          setCommentsLoadedAwemeId(awemeId);
          setCommentReplies((prev) => ({
            ...prev,
            [host.cid]: {
              items: ordered,
              cursor: 0,
              hasMore: false,
              loading: false,
              error: "",
              total: subReplies.length,
              loaded: true,
            },
          }));
          setExpandedCommentReplyIds((prev) => new Set(prev).add(host.cid));
          locateDoneRef.current = true;
          highlight(cid);
          return;
        }
        // 兜底：评论已删或接口未返回，用通知数据构造置顶。
        const pinned: CommentInfo = {
          cid,
          text,
          create_time,
          digg_count,
          user_digged: 0,
          reply_comment_total: 0,
          sub_comments: null,
          user: { uid: user.uid, nickname: user.nickname, sec_uid: user.sec_uid, avatar_thumb: user.avatar },
        };
        setComments((prev) => [pinned, ...prev]);
        locateDoneRef.current = true;
        setLocatePrompt("deleted");
        highlight(cid);
      } catch {
        locateDoneRef.current = true;
        setLocatePrompt("deleted");
      } finally {
        setCommentsLoading(false);
      }
    })();
  }, [initialComment, currentVideo?.aweme_id]);

  // 评论项 ref 回调工厂：el 为 null 时清理 stale 条目。
  const registerCommentRef = useCallback((cid: string) => (el: HTMLDivElement | null) => {
    if (el) commentItemRefs.current.set(cid, el);
    else commentItemRefs.current.delete(cid);
  }, []);
  const registerReplyRef = useCallback((cid: string) => (el: HTMLDivElement | null) => {
    if (el) replyItemRefs.current.set(cid, el);
    else replyItemRefs.current.delete(cid);
  }, []);

  // 播放器关闭时重置定位状态（覆盖同实例复用情况）。
  useEffect(() => {
    if (open) return;
    locateDoneRef.current = false;
    locatePageRef.current = 0;
    setLocatePrompt("");
    setHighlightCid("");
    commentItemRefs.current.clear();
    replyItemRefs.current.clear();
  }, [open]);

  return {
    commentsOpen,
    comments,
    commentsLoading,
    commentsError,
    commentsHasMore,
    commentsTotal,
    commentReplies,
    expandedCommentReplyIds,
    commentDiggingIds,
    commentDraft,
    commentSubmitting,
    commentReplyTarget,
    highlightCid,
    locatePrompt,
    registerCommentRef,
    registerReplyRef,
    setLocatePrompt,
    setCommentDraft,
    setCommentReplyTarget,
    loadCommentReplies,
    toggleCommentReplies,
    toggleCommentLike,
    submitComment,
    handleCommentsScroll,
    openCommentsPanel,
    markCommentsPanelSticky,
    scheduleTransientCommentsClose,
    closeCommentsPanel,
    loadComments,
  };
}
