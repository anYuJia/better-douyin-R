import { useCallback, useRef, useEffect } from "react";
import { sendFriendImageMessage, sendFriendMessage } from "@/lib/tauri";
import {
  MAX_SEND_IMAGE_BYTES,
  type ChatMessages,
  type FriendStatusItem,
  type LocalChatMessage,
} from "./friends-status-types";
import {
  imageMessageRawContent,
  persistChatMessages,
  readFileAsDataUrl,
  readImageSize,
} from "./friends-status-utils";

interface SenderProps {
  currentSecUid: string;
  setChatMessages: React.Dispatch<React.SetStateAction<ChatMessages>>;
  updateDraft: (secUid: string, value: string) => void;
  setError: (msg: string) => void;
}

export function useFriendsMessageSender({
  currentSecUid,
  setChatMessages,
  updateDraft,
  setError,
}: SenderProps) {
  const currentSecUidRef = useRef(currentSecUid);

  useEffect(() => {
    currentSecUidRef.current = currentSecUid;
  }, [currentSecUid]);

  const patchMessage = useCallback((secUid: string, messageId: string, patch: Partial<LocalChatMessage>) => {
    setChatMessages((current) => {
      const next = {
        ...current,
        [secUid]: (current[secUid] || []).map((message) =>
          message.id === messageId ? { ...message, ...patch } : message,
        ),
      };
      persistChatMessages(next, currentSecUidRef.current);
      return next;
    });
  }, [setChatMessages]);

  const sendLocalMessage = useCallback(async (friend: FriendStatusItem, value: string) => {
    const text = value.trim();
    if (!text) return;
    const message: LocalChatMessage = {
      id: `${friend.secUid}-${Date.now()}`,
      text,
      rawContent: undefined,
      createdAt: Date.now(),
      status: "pending",
      direction: "out",
    };
    setChatMessages((current) => {
      const next = {
        ...current,
        [friend.secUid]: [...(current[friend.secUid] || []), message],
      };
      persistChatMessages(next, currentSecUidRef.current);
      return next;
    });
    updateDraft(friend.secUid, "");

    if (!friend.uid) {
      patchMessage(friend.secUid, message.id, {
        status: "error",
        error: "缺少好友数字 uid，无法发送",
      });
      return;
    }

    try {
      const result = await sendFriendMessage({ toUserId: friend.uid, content: text });
      if (!result.success) {
        throw new Error(result.message || "发送失败");
      }
      patchMessage(friend.secUid, message.id, { status: "sent", error: "" });
    } catch (caught) {
      patchMessage(friend.secUid, message.id, {
        status: "error",
        error: caught instanceof Error ? caught.message : "发送失败",
      });
    }
  }, [patchMessage, updateDraft, setChatMessages]);

  const sendLocalImageMessage = useCallback(async (friend: FriendStatusItem, file: File) => {
    if (!file.type.startsWith("image/")) {
      setError("请选择图片文件");
      return;
    }
    if (file.size > MAX_SEND_IMAGE_BYTES) {
      setError("图片不能超过 8MB");
      return;
    }
    if (!friend.uid) {
      setError("缺少好友数字 uid，无法发送图片");
      return;
    }
    setError("");
    const imageDataUrl = await readFileAsDataUrl(file);
    if (!imageDataUrl) {
      setError("读取图片失败");
      return;
    }
    const size = await readImageSize(imageDataUrl);
    const message: LocalChatMessage = {
      id: `${friend.secUid}-${Date.now()}`,
      text: "[图片]",
      rawContent: imageMessageRawContent(imageDataUrl, size.width, size.height, file.name),
      imagePreviewUrl: URL.createObjectURL(file),
      createdAt: Date.now(),
      status: "pending",
      direction: "out",
    };
    setChatMessages((current) => {
      const next = {
        ...current,
        [friend.secUid]: [...(current[friend.secUid] || []), message],
      };
      persistChatMessages(next, currentSecUidRef.current);
      return next;
    });

    try {
      const result = await sendFriendImageMessage({
        toUserId: friend.uid,
        imageDataUrl,
        width: size.width,
        height: size.height,
        fileName: file.name,
        mimeType: file.type,
      });
      if (!result.success) {
        throw new Error(result.message || "发送图片失败");
      }
      patchMessage(friend.secUid, message.id, { status: "sent", error: "" });
    } catch (caught) {
      patchMessage(friend.secUid, message.id, {
        status: "error",
        error: caught instanceof Error ? caught.message : "发送图片失败",
      });
    }
  }, [patchMessage, setError, setChatMessages]);

  return {
    sendLocalMessage,
    sendLocalImageMessage,
    patchMessage,
  };
}
