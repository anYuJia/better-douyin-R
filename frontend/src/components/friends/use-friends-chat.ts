import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useAppStore } from "@/stores/app-store";
import {
  getFriendChatState,
  getFriendMessageHistory,
  listenEvent,
  saveFriendChatState,
  sendFriendImageMessage,
  sendFriendMessage,
} from "@/lib/tauri";
import type { FriendMessageHistoryItem, UserInfo } from "@/lib/contracts";
import {
  COOKIE_REQUIRED_PATTERN,
  DEFAULT_REFRESH_INTERVAL_SECONDS,
  MAX_SEND_IMAGE_BYTES,
  STORAGE_KEY,
  type ChatDrafts,
  type ChatMessages,
  type ChatSummaries,
  type FriendStatusItem,
  type HistoryPageState,
  type ImConnectionStatus,
  type JsonRecord,
  type LocalChatMessage,
  type UnreadCounts,
} from "./friends-status-types";
import {
  fallbackMessageText,
  imageMessageRawContent,
  isRecord,
  latestChatMessage,
  messagePreviewText,
  normalizeMessageDirection,
  normalizeMessageStatus,
  numberField,
  persistChatDrafts,
  persistChatMessages,
  persistChatSummaries,
  persistUnreadCounts,
  readChatDrafts,
  readChatMessages,
  readChatSummaries,
  readUnreadCounts,
  readFileAsDataUrl,
  readImageSize,
  stringField,
} from "./friends-status-utils";
import { useFriendsChatPersistence } from "./use-friends-chat-persistence";
import { useFriendsMessageSender } from "./use-friends-message-sender";

export function useFriendsChat(
  friends: FriendStatusItem[],
  currentSecUid: string,
  setError: (msg: string) => void
) {
  const setFriendUnreadCount = useAppStore((state) => state.setFriendUnreadCount);

  const [chatDrafts, setChatDrafts] = useState<ChatDrafts>(() => readChatDrafts(currentSecUid));
  const [chatMessages, setChatMessages] = useState<ChatMessages>(() => readChatMessages(currentSecUid));
  const [unreadCounts, setUnreadCounts] = useState<UnreadCounts>(() => readUnreadCounts(currentSecUid));
  const [chatSummaries, setChatSummaries] = useState<ChatSummaries>(() => readChatSummaries(currentSecUid));
  const [selectedFriendId, setSelectedFriendId] = useState("");
  const [historyState, setHistoryState] = useState<HistoryPageState>({});

  const [imStatus, setImStatus] = useState<ImConnectionStatus>({
    connected: false,
    message: "接收通道未连接",
    updatedAt: 0,
  });

  const chatStateLoadedRef = useRef(false);
  const selectedFriendIdRef = useRef(selectedFriendId);
  const currentSecUidRef = useRef(currentSecUid);

  useEffect(() => {
    currentSecUidRef.current = currentSecUid;
    if (currentSecUid) {
      setChatDrafts(readChatDrafts(currentSecUid));
      setChatMessages(readChatMessages(currentSecUid));
      setUnreadCounts(readUnreadCounts(currentSecUid));
      setChatSummaries(readChatSummaries(currentSecUid));
    }
  }, [currentSecUid]);

  useEffect(() => {
    selectedFriendIdRef.current = selectedFriendId;
  }, [selectedFriendId]);

  const selectedFriend = useMemo(
    () => friends.find((friend) => friend.secUid === selectedFriendId) || null,
    [friends, selectedFriendId],
  );

  const selectedMessages = selectedFriend ? chatMessages[selectedFriend.secUid] || [] : [];
  const selectedHistory = selectedFriend ? historyState[selectedFriend.secUid] : undefined;

  const updateDraft = useCallback((secUid: string, value: string) => {
    setChatDrafts((current) => {
      const next = { ...current };
      if (value) {
        next[secUid] = value;
      } else {
        delete next[secUid];
      }
      persistChatDrafts(next, currentSecUidRef.current);
      return next;
    });
  }, []);
  const {
    sendLocalMessage,
    sendLocalImageMessage,
    patchMessage,
  } = useFriendsMessageSender({
    currentSecUid,
    setChatMessages,
    updateDraft,
    setError,
  });

  const clearUnread = useCallback((secUid: string) => {
    setUnreadCounts((current) => {
      const next = { ...current };
      delete next[secUid];
      persistUnreadCounts(next, currentSecUidRef.current);
      return next;
    });
    setChatSummaries((current) => {
      const summary = current[secUid];
      if (!summary || summary.unreadCount === 0) return current;
      const next = {
        ...current,
        [secUid]: {
          ...summary,
          unreadCount: 0,
        },
      };
      persistChatSummaries(next, currentSecUidRef.current);
      return next;
    });
  }, []);


  const selectFriend = useCallback((friend: FriendStatusItem) => {
    setSelectedFriendId(friend.secUid);
    clearUnread(friend.secUid);
  }, [clearUnread]);

  const mergeHistoryMessages = useCallback((items: FriendMessageHistoryItem[], fallbackFriend?: FriendStatusItem | null) => {
    if (!items.length) return 0;
    let mergedCount = 0;
    setChatMessages((current) => {
      const next: ChatMessages = { ...current };
      for (const item of items) {
        const conversationId = stringField(item as JsonRecord, ["conversation_id", "conversationId"]);
        const senderUid = stringField(item as JsonRecord, ["sender_uid", "senderUid"]);
        const rawContent = stringField(item as JsonRecord, ["raw_content", "rawContent"]) || undefined;
        const text = stringField(item as JsonRecord, ["content", "text"]) || fallbackMessageText(rawContent);
        const messageId = stringField(item as JsonRecord, ["server_message_id", "message_id", "id"]);
        if (!text) continue;
        if (text.trim().startsWith('{') && text.includes("command_type")) {
          continue;
        }
        const friend = fallbackFriend || friends.find((candidate) =>
          (senderUid && candidate.uid === senderUid) ||
          (candidate.uid && conversationId.includes(candidate.uid))
        );
        if (!friend) continue;
        const rawCreatedAt = numberField(item as JsonRecord, ["created_at", "createdAt", "create_time", "createTime"]);
        const createdAt = rawCreatedAt > 0 && rawCreatedAt < 10_000_000_000
          ? rawCreatedAt * 1000
          : rawCreatedAt || Date.now();
        const message: LocalChatMessage = {
          id: messageId || `${friend.secUid}-${createdAt}`,
          text,
          rawContent,
          createdAt,
          status: "sent",
          direction: senderUid && senderUid === friend.uid ? "in" : "out",
          senderUid,
        };
        const currentMessages = next[friend.secUid] || [];
        if (currentMessages.some((existing) => existing.id === message.id)) continue;
        next[friend.secUid] = [...currentMessages, message].sort((a, b) => a.createdAt - b.createdAt);
        mergedCount += 1;
      }
      if (mergedCount > 0) {
        persistChatMessages(next, currentSecUidRef.current);
        return next;
      }
      return current;
    });
    return mergedCount;
  }, [friends]);

  const loadHistoryMessages = useCallback(async (friend: FriendStatusItem, cursor = 0) => {
    const current = historyState[friend.secUid];
    if (current?.loading) return;
    if (cursor > 0 && current?.hasMore === false) return;
    const currentMessages = chatMessages[friend.secUid] || [];
    setHistoryState((state) => ({
      ...state,
      [friend.secUid]: {
        loaded: Boolean(state[friend.secUid]?.loaded),
        loading: true,
        nextCursor: state[friend.secUid]?.nextCursor || 0,
        hasMore: state[friend.secUid]?.hasMore ?? true,
        error: "",
      },
    }));
    try {
      const result = await getFriendMessageHistory({ cursor, toUserId: friend.uid });
      if (!result.success) {
        throw new Error(result.message || "获取历史消息失败");
      }
      const messages = Array.isArray(result.messages) ? result.messages : [];
      mergeHistoryMessages(messages, friend);
      const nextCursor = Number(result.next_cursor || 0) || 0;
      setHistoryState((state) => ({
        ...state,
        [friend.secUid]: {
          loaded: true,
          loading: false,
          nextCursor,
          hasMore: Boolean(nextCursor && messages.length > 0),
          error: "",
        },
      }));
    } catch (caught) {
      setHistoryState((state) => ({
        ...state,
        [friend.secUid]: {
          loaded: cursor === 0 ? true : Boolean(state[friend.secUid]?.loaded),
          loading: false,
          nextCursor: state[friend.secUid]?.nextCursor || 0,
          hasMore: false,
          error: cursor === 0 && currentMessages.length === 0
            ? caught instanceof Error ? caught.message : "获取历史消息失败"
            : "",
        },
      }));
    }
  }, [chatMessages, historyState, mergeHistoryMessages]);

  useEffect(() => {
    if (!selectedFriend || !selectedFriend.uid) return;
    const current = historyState[selectedFriend.secUid];
    if (current?.loaded || current?.loading) return;
    void loadHistoryMessages(selectedFriend, 0);
  }, [historyState, loadHistoryMessages, selectedFriend]);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listenEvent<Record<string, unknown>>("im-status", (payload) => {
      if (disposed || !payload || typeof payload !== "object") return;
      setImStatus({
        connected: Boolean(payload.connected),
        message: stringField(payload, ["message"]) || (payload.connected ? "私信接收已连接" : "私信接收未连接"),
        updatedAt: numberField(payload, ["updated_at", "updatedAt"]) || Date.now(),
      });
    }).then((cleanup) => {
      unlisten = cleanup;
    });
    return () => {
      disposed = true;
      if (unlisten) unlisten();
    };
  }, []);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listenEvent<Record<string, unknown>>("im-message", (payload) => {
      if (disposed || !payload || typeof payload !== "object") return;
      const senderUid = stringField(payload, ["sender_uid", "senderUid"]);
      const rawContent = stringField(payload, ["raw_content", "rawContent"]) || undefined;
      const text = stringField(payload, ["content", "text"]) || fallbackMessageText(rawContent);
      const serverMessageId = stringField(payload, ["server_message_id", "message_id", "id"]);
      if (!senderUid || !text) return;
      const friend = friends.find((item) => item.uid === senderUid);
      if (!friend) return;
      const message: LocalChatMessage = {
        id: serverMessageId || `${friend.secUid}-${Date.now()}`,
        text,
        rawContent,
        createdAt: numberField(payload, ["created_at", "createdAt"]) || Date.now(),
        status: "sent",
        direction: "in",
        senderUid,
      };
      setChatMessages((current) => {
        const currentMessages = current[friend.secUid] || [];
        if (currentMessages.some((item) => item.id === message.id)) return current;
        const next = {
          ...current,
          [friend.secUid]: [...currentMessages, message],
        };
        persistChatMessages(next, currentSecUidRef.current);
        return next;
      });
      if (friend.secUid !== selectedFriendIdRef.current) {
        setUnreadCounts((current) => {
          const next = {
            ...current,
            [friend.secUid]: (current[friend.secUid] || 0) + 1,
          };
          persistUnreadCounts(next, currentSecUidRef.current);
          return next;
        });
      }
    }).then((cleanup) => {
      unlisten = cleanup;
    });
    return () => {
      disposed = true;
      if (unlisten) unlisten();
    };
  }, [friends]);

  useEffect(() => {
    if (friends.length === 0) {
      setSelectedFriendId("");
      return;
    }
    if (selectedFriendId && !friends.some((friend) => friend.secUid === selectedFriendId)) {
      setSelectedFriendId("");
    }
  }, [friends, selectedFriendId]);

  useEffect(() => {
    if (selectedFriend) {
      clearUnread(selectedFriend.secUid);
    }
  }, [clearUnread, selectedFriend]);

  // Call the persistence hook to manage syncing summaries and unread counts
  useFriendsChatPersistence({
    currentSecUid,
    selectedFriendIdRef,
    chatMessages,
    unreadCounts,
    chatSummaries,
    setChatMessages,
    setUnreadCounts,
    setChatSummaries,
  });

  return {
    chatDrafts,
    chatMessages,
    unreadCounts,
    chatSummaries,
    historyState,
    selectedFriendId,
    setSelectedFriendId,
    selectedFriend,
    selectedMessages,
    selectedHistory,
    imStatus,
    setChatDrafts,
    setChatMessages,
    setUnreadCounts,
    setChatSummaries,
    updateDraft,
    sendLocalMessage,
    sendLocalImageMessage,
    loadHistoryMessages,
    clearUnread,
    selectFriend,
  };
}
