import { useCallback, useEffect, useMemo, useRef, useState } from "react";
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

  // Im status is managed by the useFriendsImEvents hook

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

  const { imStatus } = useFriendsImEvents({
    friends,
    currentSecUid,
    selectedFriendIdRef,
    setChatMessages,
    setUnreadCounts,
  });

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
