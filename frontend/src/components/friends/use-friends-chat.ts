import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  GLOBAL_FRIEND_CHAT_UPDATED_EVENT,
  UNKNOWN_FRIEND_KEY_PREFIX,
} from "@/hooks/use-global-friends-im";
import { saveFriendChatState } from "@/lib/tauri";
import { useAppStore } from "@/stores/app-store";
import {
  type ChatDrafts,
  type ChatMessages,
  type ChatSummaries,
  type FriendStatusItem,
  type UnreadCounts,
} from "./friends-status-types";
import {
  persistChatDrafts,
  persistChatMessages,
  persistChatSummaries,
  persistUnreadCounts,
  readChatDrafts,
  readChatMessages,
  readChatSummaries,
  readUnreadCounts,
} from "./friends-status-utils";
import { useFriendsChatPersistence } from "./use-friends-chat-persistence";
import { useFriendsMessageSender } from "./use-friends-message-sender";
import { useFriendsMessageHistory } from "./use-friends-message-history";
import { useFriendsImEvents } from "./use-friends-im-events";

function unreadTotal(counts: UnreadCounts) {
  return Object.values(counts).reduce((sum, value) => sum + Math.max(0, Number(value) || 0), 0);
}

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

  const selectedFriendIdRef = useRef(selectedFriendId);
  const currentSecUidRef = useRef(currentSecUid);

  const setUnreadTotal = useCallback((counts: UnreadCounts) => {
    setFriendUnreadCount(unreadTotal(counts));
  }, [setFriendUnreadCount]);

  const reloadPersistedChatState = useCallback(() => {
    const nextMessages = readChatMessages(currentSecUidRef.current);
    const nextUnread = readUnreadCounts(currentSecUidRef.current);
    const nextSummaries = readChatSummaries(currentSecUidRef.current);
    setChatMessages(nextMessages);
    setUnreadCounts(nextUnread);
    setChatSummaries(nextSummaries);
    setUnreadTotal(nextUnread);
  }, [setUnreadTotal]);

  const mergeUnknownFriendConversations = useCallback(() => {
    if (friends.length === 0) return false;
    const uidToSecUid = new Map(
      friends
        .filter((friend) => friend.uid && friend.secUid)
        .map((friend) => [friend.uid, friend.secUid] as const),
    );
    if (uidToSecUid.size === 0) return false;

    const nextMessages = readChatMessages(currentSecUidRef.current);
    const nextUnread = readUnreadCounts(currentSecUidRef.current);
    const nextSummaries = readChatSummaries(currentSecUidRef.current);
    let changed = false;

    for (const key of Object.keys(nextMessages)) {
      if (!key.startsWith(UNKNOWN_FRIEND_KEY_PREFIX)) continue;
      const uid = key.slice(UNKNOWN_FRIEND_KEY_PREFIX.length);
      const secUid = uidToSecUid.get(uid);
      if (!secUid || secUid === key) continue;

      const existing = nextMessages[secUid] || [];
      const merged = [...existing];
      for (const message of nextMessages[key] || []) {
        if (!merged.some((item) => item.id === message.id)) {
          merged.push(message);
        }
      }
      if (merged.length > 0) {
        nextMessages[secUid] = merged.sort((a, b) => a.createdAt - b.createdAt);
      }
      delete nextMessages[key];

      const isSelectedConversation = secUid === selectedFriendIdRef.current;
      const sourceUnread = Math.max(0, nextUnread[key] || 0);
      if (isSelectedConversation) {
        delete nextUnread[secUid];
      } else if (sourceUnread > 0) {
        nextUnread[secUid] = (nextUnread[secUid] || 0) + sourceUnread;
      }
      delete nextUnread[key];

      const sourceSummary = nextSummaries[key];
      if (sourceSummary) {
        const currentSummary = nextSummaries[secUid];
        nextSummaries[secUid] = {
          latestMessage: sourceSummary.latestMessage || currentSummary?.latestMessage,
          latestMessageAt: Math.max(sourceSummary.latestMessageAt || 0, currentSummary?.latestMessageAt || 0),
          unreadCount: isSelectedConversation
            ? 0
            : (currentSummary?.unreadCount || 0) + Math.max(0, sourceSummary.unreadCount || 0),
        };
      }
      delete nextSummaries[key];
      changed = true;
    }

    if (!changed) return false;
    persistChatMessages(nextMessages, currentSecUidRef.current);
    persistUnreadCounts(nextUnread, currentSecUidRef.current);
    persistChatSummaries(nextSummaries, currentSecUidRef.current);
    setChatMessages(nextMessages);
    setUnreadCounts(nextUnread);
    setChatSummaries(nextSummaries);
    setUnreadTotal(nextUnread);
    void saveFriendChatState({ summaries: nextSummaries, unreadCounts: nextUnread }, currentSecUidRef.current).catch(() => undefined);
    return true;
  }, [friends, setUnreadTotal]);

  useEffect(() => {
    currentSecUidRef.current = currentSecUid;
    if (currentSecUid) {
      setChatDrafts(readChatDrafts(currentSecUid));
      reloadPersistedChatState();
    }
  }, [currentSecUid, reloadPersistedChatState]);

  useEffect(() => {
    selectedFriendIdRef.current = selectedFriendId;
  }, [selectedFriendId]);

  useEffect(() => {
    const handler = () => {
      if (!mergeUnknownFriendConversations()) {
        reloadPersistedChatState();
      }
    };
    window.addEventListener(GLOBAL_FRIEND_CHAT_UPDATED_EVENT, handler);
    return () => window.removeEventListener(GLOBAL_FRIEND_CHAT_UPDATED_EVENT, handler);
  }, [mergeUnknownFriendConversations, reloadPersistedChatState]);

  useEffect(() => {
    void mergeUnknownFriendConversations();
  }, [mergeUnknownFriendConversations]);

  const selectedFriend = useMemo(
    () => friends.find((friend) => friend.secUid === selectedFriendId) || null,
    [friends, selectedFriendId],
  );

  const selectedMessages = selectedFriend ? chatMessages[selectedFriend.secUid] || [] : [];

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
  } = useFriendsMessageSender({
    currentSecUid,
    setChatMessages,
    updateDraft,
    setError,
  });

  const clearUnread = useCallback((secUid: string) => {
    const namespace = currentSecUidRef.current;
    const friend = friends.find((item) => item.secUid === secUid);
    const unknownKey = friend?.uid ? `${UNKNOWN_FRIEND_KEY_PREFIX}${friend.uid}` : "";
    const nextUnread = { ...readUnreadCounts(namespace) };
    delete nextUnread[secUid];
    if (unknownKey) delete nextUnread[unknownKey];

    const nextSummaries = { ...readChatSummaries(namespace) };
    const summary = nextSummaries[secUid];
    if (summary) {
      nextSummaries[secUid] = {
        ...summary,
        unreadCount: 0,
      };
    }
    if (unknownKey && nextSummaries[unknownKey]) {
      delete nextSummaries[unknownKey];
    }

    persistUnreadCounts(nextUnread, namespace);
    persistChatSummaries(nextSummaries, namespace);
    setUnreadCounts(nextUnread);
    setChatSummaries(nextSummaries);
    setUnreadTotal(nextUnread);
    void saveFriendChatState({ summaries: nextSummaries, unreadCounts: nextUnread }, namespace).catch(() => undefined);
  }, [friends, setUnreadTotal]);

  const selectFriend = useCallback((friend: FriendStatusItem) => {
    setSelectedFriendId(friend.secUid);
    clearUnread(friend.secUid);
  }, [clearUnread]);

  const {
    historyState,
    selectedHistory,
    loadHistoryMessages,
  } = useFriendsMessageHistory({
    friends,
    chatMessages,
    setChatMessages,
    currentSecUid,
    selectedFriend,
  });

  const { imStatus } = useFriendsImEvents();

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
