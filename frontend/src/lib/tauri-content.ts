// ═══════════════════════════════════════════════
// User search, video detail, feeds, collections, comments
// ═══════════════════════════════════════════════

import type {
  ApiResponse,
  CollectedMixItem,
  CollectedMixesResponse,
  CollectedVideosResponse,
  CommentDiggResponse,
  CommentsResponse,
  FollowResponse,
  LikedAuthorsResponse,
  LikedVideosResponse,
  LinkParseResponse,
  MixVideosResponse,
  PublishCommentResponse,
  RecommendedFeedType,
  RecommendedResponse,
  SearchUserResponse,
  UserDetailResponse,
  UserInfo,
  UserVideosResponse,
  VideoDetailResponse,
  VideoInfo,
  VideoRelationResponse,
} from "./contracts";
import {
  normalizeLikedVideo,
  normalizeUser,
  normalizeVideo,
  normalizeVideos,
} from "./normalizers";
import { invoke, invokeLocal, shouldUseBrowserBridge, requestJson } from "./tauri-core";

export async function searchUser(keyword: string): Promise<SearchUserResponse> {
  if (shouldUseBrowserBridge()) {
    const result = await requestJson<SearchUserResponse>("/api/search_user", {
      method: "POST",
      suppressCookieInvalidEvent: true,
      body: JSON.stringify({ keyword }),
    });
    return {
      ...result,
      user: result.user ? normalizeUser(result.user) : undefined,
      users: Array.isArray(result.users) ? result.users.map(normalizeUser) : undefined,
    };
  }
  const result = await invokeLocal<SearchUserResponse>("search_user", { keyword });
  return {
    ...result,
    user: result.user ? normalizeUser(result.user) : undefined,
    users: Array.isArray(result.users) ? result.users.map(normalizeUser) : undefined,
  };
}

export async function getUserDetail(secUid: string, nickname?: string): Promise<UserDetailResponse> {
  if (shouldUseBrowserBridge()) {
    const result = await requestJson<UserDetailResponse>("/api/user_detail", {
      method: "POST",
      suppressCookieInvalidEvent: true,
      body: JSON.stringify({ sec_uid: secUid, nickname }),
    });
    return { ...result, user: result.user ? normalizeUser(result.user) : undefined };
  }
  const result = await invokeLocal<UserDetailResponse>("get_user_detail", {
    secUid,
    sec_uid: secUid,
    nickname,
  });
  return {
    ...result,
    user: result.user ? normalizeUser(result.user) : undefined,
  };
}

export async function getUserVideos(secUid: string, count: number, cursor: number): Promise<UserVideosResponse> {
  if (shouldUseBrowserBridge()) {
    const result = await requestJson<UserVideosResponse & { videos?: unknown[] }>("/api/user_videos", {
      method: "POST",
      suppressCookieInvalidEvent: true,
      body: JSON.stringify({ sec_uid: secUid, count, cursor }),
    });
    return {
      ...result,
      videos: normalizeVideos(result.videos),
    };
  }
  const result = await invokeLocal<UserVideosResponse & { videos?: unknown[] }>("get_user_videos", {
    secUid,
    sec_uid: secUid,
    count,
    cursor,
  });
  return {
    ...result,
    videos: normalizeVideos(result.videos),
  };
}

export async function getVideoDetail(awemeId: string): Promise<VideoDetailResponse> {
  if (shouldUseBrowserBridge()) {
    const result = await requestJson<VideoDetailResponse & { video?: unknown }>("/api/video_detail", {
      method: "POST",
      body: JSON.stringify({ aweme_id: awemeId }),
    });
    return {
      ...result,
      video: normalizeVideo(result.video) || undefined,
    };
  }
  const result = await invoke<VideoDetailResponse & { video?: unknown }>("get_video_detail", {
    awemeId,
    aweme_id: awemeId,
  });
  return {
    ...result,
    video: normalizeVideo(result.video) || undefined,
  };
}

export async function parseUrl(url: string): Promise<VideoInfo> {
  const result = await parseLink(url);
  return result.video || (normalizeVideo(result as unknown) as VideoInfo) || (result as unknown as VideoInfo);
}

export async function parseLink(link: string): Promise<LinkParseResponse> {
  if (shouldUseBrowserBridge()) {
    const result = await requestJson<LinkParseResponse & { video?: unknown; videos?: unknown[]; user?: unknown }>("/api/parse_link", {
      method: "POST",
      body: JSON.stringify({ link }),
    });
    return {
      ...result,
      user: result.user ? normalizeUser(result.user) : undefined,
      video: normalizeVideo(result.video) || undefined,
      videos: normalizeVideos(result.videos),
    };
  }
  const result = await invoke<LinkParseResponse & { video?: unknown; videos?: unknown[]; user?: unknown }>("parse_link", { link });
  return {
    ...result,
    user: result.user ? normalizeUser(result.user) : undefined,
    video: normalizeVideo(result.video) || undefined,
    videos: normalizeVideos(result.videos),
  };
}

export async function setVideoLiked(awemeId: string, liked: boolean): Promise<VideoRelationResponse> {
  if (shouldUseBrowserBridge()) {
    return requestJson("/api/video_like", {
      method: "POST",
      body: JSON.stringify({ aweme_id: awemeId, liked }),
    });
  }
  return invoke("set_video_liked", { awemeId, aweme_id: awemeId, liked });
}

export async function setVideoCollected(awemeId: string, collected: boolean): Promise<VideoRelationResponse> {
  if (shouldUseBrowserBridge()) {
    return requestJson("/api/video_collect", {
      method: "POST",
      body: JSON.stringify({ aweme_id: awemeId, collected }),
    });
  }
  return invoke("set_video_collected", { awemeId, aweme_id: awemeId, collected });
}

export async function setUserFollowed(userId: string, follow: boolean): Promise<FollowResponse> {
  if (shouldUseBrowserBridge()) {
    return requestJson("/api/user_follow", {
      method: "POST",
      body: JSON.stringify({ user_id: userId, follow }),
    });
  }
  return invoke("set_user_followed", { userId, user_id: userId, follow });
}

export async function getRecommended(
  cursor: number,
  count: number,
  feedType: RecommendedFeedType = "featured"
): Promise<RecommendedResponse> {
  if (shouldUseBrowserBridge()) {
    const result = await requestJson<RecommendedResponse & { videos?: unknown[] }>("/api/recommended_feed", {
      method: "POST",
      body: JSON.stringify({ cursor, count, feed_type: feedType, feedType }),
    });
    return {
      ...result,
      videos: normalizeVideos(result.videos),
    };
  }
  const result = await invoke<RecommendedResponse & { videos?: unknown[] }>("get_recommended", {
    cursor,
    count,
    feedType,
    feed_type: feedType,
  });
  return {
    ...result,
    videos: normalizeVideos(result.videos),
  };
}

export async function getLikedVideos(
  count: number,
  secUid = "",
  cursor = 0
): Promise<LikedVideosResponse> {
  if (shouldUseBrowserBridge()) {
    const result = await requestJson<LikedVideosResponse & { data?: unknown[] }>("/api/get_liked_videos", {
      method: "POST",
      body: JSON.stringify({ count, sec_uid: secUid, cursor }),
      suppressCookieInvalidEvent: true,
    });
    return {
      ...result,
      data: Array.isArray(result.data)
        ? (result.data.map(normalizeLikedVideo).filter(Boolean) as VideoInfo[])
        : [],
    };
  }
  const result = await invokeLocal<LikedVideosResponse & { data?: unknown[] }>("get_liked_videos", {
    count,
    secUid,
    sec_uid: secUid,
    cursor,
  });

  return {
    ...result,
    data: Array.isArray(result.data)
      ? (result.data.map(normalizeLikedVideo).filter(Boolean) as VideoInfo[])
      : [],
  };
}

export async function getLikedAuthors(count: number): Promise<LikedAuthorsResponse> {
  if (shouldUseBrowserBridge()) {
    const result = await requestJson<LikedAuthorsResponse & { data?: unknown[] }>("/api/get_liked_authors", {
      method: "POST",
      body: JSON.stringify({ count }),
    });
    return {
      ...result,
      data: Array.isArray(result.data) ? result.data.map(normalizeUser) : [],
    };
  }
  const result = await invoke<LikedAuthorsResponse & { data?: unknown[] }>("get_liked_authors", { count });
  return {
    ...result,
    data: Array.isArray(result.data) ? result.data.map(normalizeUser) : [],
  };
}

export async function getCollectedVideos(cursor: number, count: number): Promise<CollectedVideosResponse> {
  if (shouldUseBrowserBridge()) {
    const result = await requestJson<CollectedVideosResponse & { data?: unknown[] }>("/api/get_collected_videos", {
      method: "POST",
      body: JSON.stringify({ cursor, count }),
      suppressCookieInvalidEvent: true,
    });
    return {
      ...result,
      data: Array.isArray(result.data)
        ? (result.data.map(normalizeLikedVideo).filter(Boolean) as VideoInfo[])
        : [],
    };
  }
  const result = await invokeLocal<CollectedVideosResponse & { data?: unknown[] }>("get_collected_videos", {
    cursor,
    count,
  });
  return {
    ...result,
    data: Array.isArray(result.data)
      ? (result.data.map(normalizeLikedVideo).filter(Boolean) as VideoInfo[])
      : [],
  };
}

export async function getCollectedMixes(cursor: number, count: number): Promise<CollectedMixesResponse> {
  if (shouldUseBrowserBridge()) {
    const result = await requestJson<CollectedMixesResponse & { data?: CollectedMixItem[] }>("/api/get_collected_mixes", {
      method: "POST",
      body: JSON.stringify({ cursor, count }),
      suppressCookieInvalidEvent: true,
    });
    return {
      ...result,
      data: Array.isArray(result.data) ? result.data : [],
    };
  }
  const result = await invokeLocal<CollectedMixesResponse & { data?: CollectedMixItem[] }>("get_collected_mixes", {
    cursor,
    count,
  });
  return {
    ...result,
    data: Array.isArray(result.data) ? result.data : [],
  };
}

export async function getMixVideos(seriesId: string, cursor: number, count: number): Promise<MixVideosResponse> {
  if (shouldUseBrowserBridge()) {
    const result = await requestJson<MixVideosResponse & { data?: unknown[] }>("/api/get_mix_videos", {
      method: "POST",
      body: JSON.stringify({ series_id: seriesId, cursor, count }),
      suppressCookieInvalidEvent: true,
    });
    return {
      ...result,
      data: Array.isArray(result.data)
        ? (result.data.map(normalizeLikedVideo).filter(Boolean) as VideoInfo[])
        : [],
    };
  }
  const result = await invokeLocal<MixVideosResponse & { data?: unknown[] }>("get_mix_videos", {
    seriesId,
    series_id: seriesId,
    cursor,
    count,
  });
  return {
    ...result,
    data: Array.isArray(result.data) ? normalizeVideos(result.data) : [],
  };
}

export async function getComments(awemeId: string, count: number, cursor = 0, insertIds = ""): Promise<CommentsResponse> {
  if (shouldUseBrowserBridge()) {
    return requestJson("/api/get_comments", {
      method: "POST",
      body: JSON.stringify({ aweme_id: awemeId, count, cursor, insert_ids: insertIds, insertIds }),
    });
  }
  return invoke("get_comments", { awemeId, count, cursor, insertIds, insert_ids: insertIds });
}

export async function getCommentReplies(
  awemeId: string,
  commentId: string,
  count: number,
  cursor = 0
): Promise<CommentsResponse> {
  if (shouldUseBrowserBridge()) {
    return requestJson("/api/get_comment_replies", {
      method: "POST",
      body: JSON.stringify({ aweme_id: awemeId, comment_id: commentId, count, cursor }),
    });
  }
  return invoke("get_comment_replies", { awemeId, commentId, count, cursor });
}

export async function setCommentLiked(
  awemeId: string,
  commentId: string,
  liked: boolean,
  level = 1
): Promise<CommentDiggResponse> {
  if (shouldUseBrowserBridge()) {
    return requestJson("/api/comment_digg", {
      method: "POST",
      body: JSON.stringify({ aweme_id: awemeId, comment_id: commentId, liked, level }),
    });
  }
  return invoke("set_comment_liked", { awemeId, commentId, liked, level });
}

export async function publishComment(
  awemeId: string,
  text: string,
  replyId = "",
  replyToReplyId = ""
): Promise<PublishCommentResponse> {
  if (shouldUseBrowserBridge()) {
    return requestJson("/api/comment_publish", {
      method: "POST",
      body: JSON.stringify({ aweme_id: awemeId, text, reply_id: replyId, reply_to_reply_id: replyToReplyId }),
    });
  }
  return invoke("publish_comment", { awemeId, text, replyId, replyToReplyId });
}
