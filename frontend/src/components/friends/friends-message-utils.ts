import {
  LIKE_NOTICE_PATTERN,
  type LocalChatMessage,
  type SharedMessageCard,
  type JsonRecord,
  type ChatMessages,
} from "./friends-status-types";
import {
  isRecord,
  stringField,
  numberField,
  firstUrl,
} from "./friends-response-map";
import {
  normalizeMessageStatus,
  normalizeMessageDirection,
  inlineImageDataUrl,
  parseJsonContent,
  latestChatMessage,
  fallbackMessageText,
  messagePreviewText,
  normalizeLikeNoticeText,
} from "./friends-message-format";
import { isSameMessageDate } from "./friends-status-format";

export function normalizeStoredChatMessage(secUid: string, message: JsonRecord): LocalChatMessage {
  const item: LocalChatMessage = {
    id: stringField(message, ["id"]) || `${secUid}-${numberField(message, ["createdAt"])}-${Math.random()}`,
    text: stringField(message, ["text"]),
    rawContent: stringField(message, ["rawContent", "raw_content"]) || undefined,
    imagePreviewUrl: stringField(message, ["imagePreviewUrl"]).startsWith("blob:") ? undefined : stringField(message, ["imagePreviewUrl"]) || undefined,
    createdAt: numberField(message, ["createdAt"]),
    status: normalizeMessageStatus(stringField(message, ["status"])),
    direction: normalizeMessageDirection(stringField(message, ["direction"])),
    senderUid: stringField(message, ["senderUid", "sender_uid"]),
    error: stringField(message, ["error"]) || undefined,
  };
  if (isLocalUnsentImagePlaceholder(item)) {
    return {
      ...item,
      status: "error",
      error: item.error || "图片未发送：缺少抖音上传凭证",
    };
  }
  return item;
}

export function isLocalUnsentImagePlaceholder(message: LocalChatMessage) {
  if (message.direction !== "out" || message.status === "error") return false;
  if (message.imagePreviewUrl) return false;
  const parsed = parseJsonContent(message.rawContent || "");
  if (!parsed || Number(parsed.aweType || 0) !== 2702) return false;
  const inlinePic = stringField(parsed, ["inline_pic", "inlinePic"]);
  const hasInlineImage = Boolean(inlineImageDataUrl(inlinePic));
  const hasUploadedResource = Boolean(firstUrl(parsed.resource_url) || firstUrl(parsed.url));
  return !hasInlineImage && !hasUploadedResource;
}

export function sanitizePersistedChatMessage(message: LocalChatMessage, rawLimit = 30000): LocalChatMessage {
  return {
    id: message.id,
    text: message.text,
    rawContent: compactRawContent(message.rawContent, rawLimit),
    imagePreviewUrl: message.imagePreviewUrl?.startsWith("blob:") ? undefined : message.imagePreviewUrl,
    createdAt: message.createdAt,
    status: message.status === "pending" ? "error" : message.status,
    direction: message.direction,
    senderUid: message.senderUid,
    error: message.status === "pending" ? "发送未完成，请重试" : message.error ? message.error.slice(0, 300) : undefined,
  };
}

export function compactRawContent(rawContent: string | undefined, maxLength = 30000) {
  if (!rawContent) return undefined;
  if (rawContent.length <= maxLength) return rawContent;
  return undefined;
}

export function compactChatMessagesForStorage(
  messages: ChatMessages,
  perFriendLimit = 40,
  rawLimit = 30000,
): ChatMessages {
  const compacted: ChatMessages = {};
  for (const [secUid, items] of Object.entries(messages)) {
    const kept = [...items]
      .sort((a, b) => a.createdAt - b.createdAt)
      .slice(-perFriendLimit)
      .map((message) => sanitizePersistedChatMessage(message, rawLimit));
    if (kept.length > 0) compacted[secUid] = kept;
  }
  return compacted;
}

export function parseNestedJsonField(record: JsonRecord, keys: string[]): JsonRecord | null {
  const value = stringField(record, keys);
  if (!value) return null;
  const parsed = parseJsonContent(value);
  return parsed;
}

export function normalizeSharedItemId(value: string): string {
  if (!value) return "";
  const match = value.match(/\d+/);
  return match ? match[0] : value;
}

export function uniqueTextParts(parts: string[]) {
  const seen = new Set<string>();
  return parts
    .map((item) => item.trim())
    .filter((item) => {
      if (!item || seen.has(item)) return false;
      seen.add(item);
      return true;
    });
}

export function imDynamicText(value: unknown): string {
  if (typeof value === "string") return value;
  if (isRecord(value)) {
    const text = stringField(value, ["text"]);
    if (text) return text;
  }
  return "";
}

export function parseDynamicPatchCard(root: JsonRecord): SharedMessageCard | null {
  const patch = parseNestedJsonField(root, ["dynamic_patch", "dynamicPatch"]);
  if (!patch) return null;
  const schema = stringField(patch, ["schema"]);
  const isVideo = schema.includes("aweme/detail") || schema.includes("note/detail");
  const query = schema.split("?")[1] || "";
  const params = new URLSearchParams(query);
  const itemId = normalizeSharedItemId(
    params.get("id") || params.get("aweme_id") || params.get("group_id") || "",
  );
  const rawList = patch.raw_list || patch.rawList;
  const records: JsonRecord[] = [];
  if (Array.isArray(rawList)) {
    rawList.forEach((item) => {
      if (isRecord(item)) records.push(item);
    });
  }
  let title = "";
  let subtitle = "";
  let coverUrl = "";
  let authorName = "";
  let avatarUrl = "";
  records.forEach((record) => {
    const text = imDynamicText(record.text);
    if (!text) return;
    const isHeaderLabel = record.color && stringField(record, ["color"]) === "GG";
    if (isHeaderLabel) {
      subtitle = text;
      return;
    }
    const isTitleLabel = record.color && stringField(record, ["color"]) === "E1";
    if (isTitleLabel) {
      title = text;
      return;
    }
    if (!title && text.length > 4 && !text.includes(":") && !text.includes("：")) {
      title = text;
    }
  });
  const attachment = parseNestedJsonField(root, ["attachment"]);
  if (attachment) {
    const attachments = attachment.attachments;
    if (Array.isArray(attachments) && attachments.length > 0) {
      const first = attachments[0];
      if (isRecord(first)) {
        const image = first.image || first.video;
        if (isRecord(image)) {
          const urls = image.url_list || image.urlList;
          if (Array.isArray(urls) && urls.length > 0) {
            coverUrl = String(urls[0] || "");
          }
        }
      }
    }
  }
  if (!title) {
    title = stringField(root, ["tips"]) || stringField(patch, ["tips"]) || "动态分享";
  }
  return {
    kind: isVideo ? "video" : "share",
    title,
    subtitle: subtitle || (isVideo ? "视频分享" : "动态分享"),
    coverUrl,
    avatarUrl,
    authorName,
    itemId,
  };
}

export function parseSharedMessage(message: LocalChatMessage): SharedMessageCard | null {
  const content = message.rawContent || message.text;
  if (!content) return null;
  const root = parseJsonContent(content);
  if (!root) return null;
  const aweType = numberField(root, ["aweType", "awe_type", "type"]);
  const isVideo = aweType === 800 || aweType === 2701 || aweType === 5 || aweType === 8;
  const isComment =
    aweType === 10500 ||
    aweType === 2702 ||
    aweType === 6 ||
    Boolean(stringField(root, ["comment_id", "commentId", "comment", "comment_content", "commentContent"]));
  const isImage =
    aweType === 501 ||
    aweType === 2704 ||
    aweType === 7 ||
    Boolean(stringField(root, ["emoji_type", "emojiType", "image_type", "imageType"])) ||
    numberField(root, ["sticker_type", "stickerType"]) > 0;
  const isShare = aweType === 2705 || aweType === 9;
  const isLocation = aweType === 2706 || aweType === 10;
  const isProduct = aweType === 2707 || aweType === 11;
  const isDynamicPatch = aweType === 2708 || aweType === 12 || root.dynamic_patch || root.dynamicPatch;
  if (isDynamicPatch) {
    return parseDynamicPatchCard(root);
  }
  if (!isVideo && !isComment && !isImage && !isShare && !isLocation && !isProduct) {
    return null;
  }
  const commentText = stringField(root, ["comment", "comment_content", "commentContent", "comment_text", "commentText"]);
  const title = commentText || stringField(root, ["title", "content_title", "contentTitle", "aweme_title", "awemeTitle", "desc", "text", "name"]) || "";
  const commentUserName = stringField(root, ["comment_user_name", "commentUserName"]);
  const awemeTitle = stringField(root, ["aweme_title", "awemeTitle"]);
  const subtitle =
    stringField(root, ["sub_title", "subtitle", "hint", "anchor_name"]) ||
    [
      isComment ? "分享评论" : "",
      commentUserName ? `评论者：${commentUserName}` : "",
      awemeTitle ? `作品：${awemeTitle}` : "",
    ].filter(Boolean).join(" · ");
  const coverUrl =
    firstUrl(root.cover_url) ||
    firstUrl(root.coverUrl) ||
    firstUrl(root.url) ||
    firstUrl(root.resource_url) ||
    firstUrl(root.resourceUrl) ||
    stringField(root, ["cover_url", "coverUrl", "image_url", "imageUrl"]) ||
    "";
  const skey = stringField(root, ["skey"]) || undefined;
  const avatarUrl =
    firstUrl(root.content_thumb) ||
    firstUrl(root.contentThumb) ||
    stringField(root, ["avatar_url", "avatarUrl", "author_avatar", "authorAvatar"]) ||
    "";
  const authorName = stringField(root, ["author_name", "authorName", "nickname"]) || "";
  const itemId = normalizeSharedItemId(
    stringField(root, ["item_id", "itemId", "id", "gid", "group_id", "aweme_id", "awemeId"]) || "",
  );
  let kind: SharedMessageCard["kind"] = "share";
  if (isVideo) kind = "video";
  else if (isComment) kind = "comment";
  else if (isImage) kind = "image";
  else if (isLocation) kind = "location";
  else if (isProduct) kind = "product";
  return {
    kind,
    title,
    subtitle: subtitle || (kind === "video" ? "视频分享" : kind === "image" ? "图片分享" : "分享"),
    coverUrl,
    skey,
    avatarUrl,
    authorName,
    itemId,
  };
}

export function centerNoticeText(message: LocalChatMessage) {
  if (message.direction === "in" && LIKE_NOTICE_PATTERN.test(message.text)) {
    return "对方点赞了你的作品";
  }
  if (message.text.includes("已成为好友") || message.text.includes("开始聊天吧")) {
    return message.text;
  }
  return null;
}

export function hasFramedMessageBody(message: LocalChatMessage) {
  if (message.imagePreviewUrl) return true;
  const shared = parseSharedMessage(message);
  return Boolean(shared);
}
