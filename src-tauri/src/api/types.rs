//! API 类型定义

use serde::{Deserialize, Serialize};

/// 视频信息
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct VideoInfo {
    pub aweme_id: String,
    pub desc: String,
    pub create_time: i64,
    pub author: AuthorInfo,
    pub video: VideoData,
    pub statistics: Statistics,
    pub status: Status,
    /// 图片URL列表 (前端期望字段名为 images)
    #[serde(rename = "images")]
    pub image_urls: Option<Vec<String>>,
    pub is_image: bool,
    pub media_type: MediaType,
    pub has_live_photo: bool,
    pub is_liked: bool,
    pub is_collected: bool,
    /// 实况照片视频URL列表 (前端期望字段名为 live_photos)
    #[serde(rename = "live_photos")]
    pub live_photo_urls: Option<Vec<String>>,
    pub music: Option<MusicInfo>,
    pub raw_media_type: Option<i32>,
    pub text_extra: Option<Vec<TextExtra>>,
}

/// 作者信息
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct AuthorInfo {
    pub uid: String,
    pub sec_uid: String,
    pub nickname: String,
    pub avatar_thumb: String,
    pub avatar_medium: String,
    pub signature: String,
    pub follower_count: i64,
    pub following_count: i64,
    pub aweme_count: i64,
    pub favoriting_count: i64,
    pub is_follow: bool,
    pub follow_status: i32,
    pub verify_status: i32,
    pub unique_id: String,
}

/// 视频数据
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct VideoData {
    pub preview_addr: Option<String>, // 直接存储 URL 字符串
    pub play_addr: String,            // 直接存储 URL 字符串
    pub play_addr_candidates: Vec<String>,
    pub dash_addr: Option<String>,
    pub audio_addr: Option<String>,
    pub play_addr_h264: Option<String>,
    pub play_addr_lowbr: Option<String>,
    pub download_addr: Option<String>,
    pub cover: String,
    pub dynamic_cover: String,
    pub origin_cover: String,
    pub width: i32,
    pub height: i32,
    pub duration: i64,
    pub ratio: String,
    pub bit_rate: Option<Vec<BitRateInfo>>,
}

/// 视频比特率信息
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct BitRateInfo {
    pub gear_name: String,
    pub format: String,
    pub bit_rate: i64,
    pub quality_type: i32,
    pub is_h265: bool,
    pub data_size: i64,
    pub width: i32,
    pub height: i32,
    pub play_addr: Option<String>,
    pub play_addr_h264: Option<String>,
    pub play_addr_candidates: Vec<String>,
    pub play_addr_h264_candidates: Vec<String>,
}

/// 视频 URL
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct VideoUrl {
    pub url_list: Vec<String>,
    pub uri: String,
}

/// 统计数据
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Statistics {
    pub play_count: i64,
    pub digg_count: i64,
    pub comment_count: i64,
    pub share_count: i64,
    pub collect_count: i64,
    pub forward_count: i64,
}

/// 状态
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Status {
    pub is_delete: bool,
    pub private_status: i32,
    pub review_status: i32,
    pub with_goods: bool,
    pub is_prohibited: bool,
}

/// 音乐信息
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct MusicInfo {
    pub id: String,
    pub title: String,
    pub author: String,
    pub play_url: Option<String>, // 直接存储 URL 字符串
    pub cover_thumb: String,
    pub duration: i64,
}

/// 文本额外信息 (话题/提及)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct TextExtra {
    pub text: String,
    pub r#type: i32,
    pub hashtag_name: Option<String>,
    pub aweme_id: Option<String>,
    pub sec_uid: Option<String>,
    pub user_id: Option<String>,
}

/// 评论信息
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct CommentInfo {
    pub cid: String,
    pub text: String,
    pub create_time: i64,
    pub user: CommentUser,
    pub digg_count: i64,
    pub user_digged: i32,
    pub reply_comment_total: i64,
    pub sub_comments: Option<Vec<CommentInfo>>,
    pub status: i32,
    pub ip_label: String,
    pub sticker_url: String,
}

/// 评论用户
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct CommentUser {
    pub uid: String,
    pub nickname: String,
    pub avatar_thumb: String,
    pub sec_uid: String,
}

/// 用户信息
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct UserInfo {
    pub uid: String,
    pub nickname: String,
    pub avatar_thumb: String,
    pub avatar_medium: String,
    pub avatar_larger: String,
    pub signature: String,
    pub follower_count: i64,
    pub following_count: i64,
    pub total_favorited: i64,
    pub aweme_count: i64,
    pub favoriting_count: i64,
    pub is_follow: bool,
    pub follow_status: i32,
    pub sec_uid: String,
    pub unique_id: String,
    pub verify_status: i32,
}

/// 用户详情 (包含关注状态等)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct UserDetail {
    #[serde(flatten)]
    pub info: UserInfo,
    pub is_favorite: bool,
    pub follow_status: i32,
    pub story_count: i64,
    pub friend_status: i32,
}

/// Python 版本 `/api/get_liked_videos` 的媒体项结构
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct LikedVideoMediaUrl {
    pub r#type: String,
    pub url: String,
}

/// 下载媒体项
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct DownloadMediaItem {
    pub r#type: String,
    pub url: String,
    pub fallback_urls: Vec<String>,
}

/// Python 版本 `/api/get_liked_videos` 的作者结构
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct LikedVideoAuthor {
    pub nickname: String,
    pub sec_uid: String,
    pub avatar_thumb: String,
}

/// Python 版本 `/api/get_liked_videos` 的单项视频结构
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct LikedVideoItem {
    pub aweme_id: String,
    pub desc: String,
    pub create_time: i64,
    pub digg_count: i64,
    pub comment_count: i64,
    pub share_count: i64,
    pub cover_url: String,
    pub duration: i64,
    pub media_type: String,
    pub raw_media_type: String,
    pub media_urls: Vec<LikedVideoMediaUrl>,
    pub bgm_url: Option<String>,
    pub is_liked: bool,
    pub is_collected: bool,
    pub statistics: Statistics,
    pub video: VideoData,
    pub author: LikedVideoAuthor,
}

/// 收藏合集作者信息
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct CollectionMixAuthor {
    pub nickname: String,
    pub sec_uid: String,
    pub avatar_thumb: String,
}

/// 收藏合集统计信息
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct CollectionMixStats {
    pub collect_vv: i64,
    pub play_vv: i64,
    pub updated_to_episode: i64,
}

/// 收藏合集单项结构
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct CollectionMixItem {
    pub mix_id: String,
    pub mix_name: String,
    pub desc: String,
    pub cover_url: String,
    pub author: CollectionMixAuthor,
    pub statis: CollectionMixStats,
    pub create_time: i64,
    pub update_time: i64,
    pub mix_type: i32,
}

/// 下载任务
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadTask {
    pub id: String,
    pub aweme_id: String,
    pub url: String,
    pub media_urls: Vec<DownloadMediaItem>,
    pub title: String,
    pub author: String,
    pub author_id: String,
    pub cover: String,
    pub save_path: String,
    pub filename: String,
    pub media_type: MediaType,
    pub total_files: u32,
    pub completed_files: u32,
    pub status: DownloadStatus,
    pub progress: f32,
    pub total_size: u64,
    pub downloaded_size: u64,
    pub error_msg: Option<String>,
    pub create_time: i64,
    pub complete_time: Option<i64>,
    pub image_urls: Option<Vec<String>>,
}

/// 媒体类型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "snake_case")]
pub enum MediaType {
    #[default]
    Video,
    Image,
    LivePhoto,
    Mixed,
    Audio,
}

/// 下载状态
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub enum DownloadStatus {
    #[default]
    Pending,
    Downloading,
    Completed,
    Failed,
    Cancelled,
    Paused,
}

/// API 响应包装
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ApiResponse<T> {
    pub status_code: i64,
    pub status_msg: Option<String>,
    pub data: Option<T>,
    pub extra: Option<serde_json::Value>,
}

impl<T> Default for ApiResponse<T> {
    fn default() -> Self {
        Self {
            status_code: 0,
            status_msg: None,
            data: None,
            extra: None,
        }
    }
}

/// 搜索结果
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SearchResult<T> {
    pub items: Vec<T>,
    pub has_more: bool,
    pub cursor: i64,
    pub total: i64,
}

#[derive(Debug, Clone)]
pub enum SearchUserResult {
    NeedVerify { verify_url: String },
    NotFound,
    Single(Box<UserInfo>),
    Multiple(Vec<UserInfo>),
}

/// 推荐视频响应
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct RecommendResponse {
    pub aweme_list: Vec<serde_json::Value>,
    pub has_more: bool,
    pub cursor: i64,
}

/// 下载历史记录
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct DownloadHistory {
    pub aweme_id: String,
    pub title: String,
    pub author: String,
    pub author_id: String,
    pub cover: String,
    pub file_path: String,
    pub media_type: String,
    pub file_size: u64,
    pub create_time: i64,
}

/// Cookie 状态
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CookieStatus {
    pub valid: bool,
    pub user_name: Option<String>,
    pub user_id: Option<String>,
    pub sec_uid: Option<String>,
    pub avatar_thumb: Option<String>,
    pub avatar_medium: Option<String>,
    pub avatar_larger: Option<String>,
    pub expires_at: Option<i64>,
    pub message: String,
}

/// 批量下载进度
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchDownloadProgress {
    pub task_id: String,
    pub current: u32,
    pub total: u32,
    pub current_video: Option<VideoInfo>,
    pub status: String,
    pub error: Option<String>,
}

/// 应用配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub download_path: String,
    pub filename_template: String,
    pub max_concurrent: u32,
    pub auto_create_folder: bool,
    pub folder_name_template: String,
    pub save_metadata: bool,
    pub proxy: Option<String>,
    pub cookie: String,
    pub theme: String,
    pub language: String,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            download_path: dirs::download_dir()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|| ".".to_string()),
            filename_template: "{title}".to_string(),
            max_concurrent: 3,
            auto_create_folder: true,
            folder_name_template: "{author}".to_string(),
            save_metadata: true,
            proxy: None,
            cookie: String::new(),
            theme: "dark".to_string(),
            language: "zh-CN".to_string(),
        }
    }
}

/// 目录选择结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectoryResult {
    pub path: Option<String>,
}

/// 通用 API 响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenericResponse {
    pub success: bool,
    pub message: String,
    pub data: Option<serde_json::Value>,
}

impl GenericResponse {
    pub fn ok(msg: &str) -> Self {
        Self {
            success: true,
            message: msg.to_string(),
            data: None,
        }
    }

    pub fn error(msg: &str) -> Self {
        Self {
            success: false,
            message: msg.to_string(),
            data: None,
        }
    }

    pub fn with_data(msg: &str, data: serde_json::Value) -> Self {
        Self {
            success: true,
            message: msg.to_string(),
            data: Some(data),
        }
    }
}
