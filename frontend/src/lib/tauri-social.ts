// ═══════════════════════════════════════════════
// Friends / IM: online status, messages, chat state
// ═══════════════════════════════════════════════

import type {
  ApiResponse,
  FriendChatStateResponse,
  FriendMessageHistoryResponse,
  FriendOnlineStatusResponse,
  NoticesResponse,
  SendFriendMessageResponse,
  ShareFriendsResponse,
  VideoInfo,
} from "./contracts";
import { invokeLocal, shouldUseBrowserBridge, requestJson } from "./tauri-core";

export async function getFriendOnlineStatus(
  secUserIds: string[],
  convIds: string[] = []
): Promise<FriendOnlineStatusResponse> {
  if (shouldUseBrowserBridge()) {
    return requestJson("/api/get_friend_online_status", {
      method: "POST",
      body: JSON.stringify({
        sec_user_ids: secUserIds,
        secUserIds,
        conv_ids: convIds,
        convIds,
      }),
      suppressCookieInvalidEvent: true,
    });
  }
  return invokeLocal("get_friend_online_status", {
    secUserIds,
    sec_user_ids: secUserIds,
    convIds,
    conv_ids: convIds,
  });
}

export async function getShareFriends(count = 50): Promise<ShareFriendsResponse> {
  const safeCount = Math.max(1, Math.min(100, Math.floor(Number(count) || 50)));
  if (shouldUseBrowserBridge()) {
    return requestJson("/api/get_share_friends", {
      method: "POST",
      body: JSON.stringify({ count: safeCount }),
      suppressCookieInvalidEvent: true,
    });
  }
  return invokeLocal("get_share_friends", { count: safeCount });
}

export async function sendFriendMessage(payload: {
  toUserId: string | number;
  content: string;
}): Promise<SendFriendMessageResponse> {
  if (shouldUseBrowserBridge()) {
    return requestJson("/api/send_friend_message", {
      method: "POST",
      body: JSON.stringify({
        to_user_id: payload.toUserId,
        toUserId: payload.toUserId,
        uid: payload.toUserId,
        content: payload.content,
      }),
      suppressCookieInvalidEvent: true,
    });
  }
  return invokeLocal("send_friend_message", {
    to_user_id: payload.toUserId,
    toUserId: payload.toUserId,
    uid: payload.toUserId,
    content: payload.content,
  });
}

export async function sendFriendVideoShare(payload: {
  toUserId: string | number;
  video: VideoInfo;
}): Promise<SendFriendMessageResponse> {
  const body = {
    to_user_id: payload.toUserId,
    toUserId: payload.toUserId,
    uid: payload.toUserId,
    video: payload.video,
  };
  if (shouldUseBrowserBridge()) {
    return requestJson("/api/send_friend_video_share", {
      method: "POST",
      body: JSON.stringify(body),
      suppressCookieInvalidEvent: true,
    });
  }
  return invokeLocal("send_friend_video_share", body);
}

export async function sendFriendImageMessage(payload: {
  toUserId: string | number;
  imageDataUrl: string;
  width?: number;
  height?: number;
  fileName?: string;
  mimeType?: string;
}): Promise<SendFriendMessageResponse> {
  const body = {
    to_user_id: payload.toUserId,
    toUserId: payload.toUserId,
    uid: payload.toUserId,
    image_data_url: payload.imageDataUrl,
    imageDataUrl: payload.imageDataUrl,
    width: payload.width,
    height: payload.height,
    file_name: payload.fileName,
    fileName: payload.fileName,
    mime_type: payload.mimeType,
    mimeType: payload.mimeType,
  };
  if (shouldUseBrowserBridge()) {
    return requestJson("/api/send_friend_image_message", {
      method: "POST",
      body: JSON.stringify(body),
      suppressCookieInvalidEvent: true,
    });
  }
  return invokeLocal("send_friend_image_message", body);
}

export async function getFriendMessageHistory(payload: {
  cursor?: number;
  toUserId?: string;
  conversationId?: string;
  conversationShortId?: string | number;
  conversationType?: string | number;
} = {}): Promise<FriendMessageHistoryResponse> {
  const body = {
    cursor: payload.cursor || 0,
    to_user_id: payload.toUserId,
    toUserId: payload.toUserId,
    conversation_id: payload.conversationId,
    conversationId: payload.conversationId,
    conversation_short_id: payload.conversationShortId,
    conversationShortId: payload.conversationShortId,
    conversation_type: payload.conversationType,
    conversationType: payload.conversationType,
  };
  if (shouldUseBrowserBridge()) {
    return requestJson("/api/get_friend_message_history", {
      method: "POST",
      body: JSON.stringify(body),
      suppressCookieInvalidEvent: true,
    });
  }
  return invokeLocal("get_friend_message_history", {
    ...body,
  });
}

export async function getFriendChatState(currentSecUid?: string): Promise<FriendChatStateResponse> {
  if (shouldUseBrowserBridge()) {
    return requestJson<FriendChatStateResponse>("/api/friend_chat_state");
  }
  return invokeLocal("get_friend_chat_state", { currentSecUid, current_sec_uid: currentSecUid });
}

export async function saveFriendChatState(payload: {
  summaries?: Record<string, unknown>;
  unreadCounts?: Record<string, number>;
}, currentSecUid?: string): Promise<ApiResponse> {
  if (shouldUseBrowserBridge()) {
    return requestJson<ApiResponse>("/api/friend_chat_state", {
      method: "POST",
      body: JSON.stringify(payload),
    });
  }
  return invokeLocal("save_friend_chat_state", { payload, currentSecUid, current_sec_uid: currentSecUid });
}

export async function getNotices(payload: {
  count?: number;
  maxTime?: number;
  minTime?: number;
  noticeGroup?: number;
} = {}): Promise<NoticesResponse> {
  const body = {
    count: payload.count ?? 10,
    max_time: payload.maxTime ?? 0,
    maxTime: payload.maxTime ?? 0,
    min_time: payload.minTime ?? 0,
    minTime: payload.minTime ?? 0,
    notice_group: payload.noticeGroup ?? 960,
    noticeGroup: payload.noticeGroup ?? 960,
  };
  if (shouldUseBrowserBridge()) {
    return requestJson<NoticesResponse>("/api/get_notices", {
      method: "POST",
      body: JSON.stringify(body),
      suppressCookieInvalidEvent: true,
    });
  }
  return invokeLocal<NoticesResponse>("get_notices", body);
}
