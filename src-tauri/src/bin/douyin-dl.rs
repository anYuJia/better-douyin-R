//! 抖音视频下载器 CLI
//!
//! Usage:
//!   douyin-dl parse <URL>          Parse a video URL and show metadata
//!   douyin-dl download <URL>       Download a video
//!   douyin-dl config show          Show current config
//!   douyin-dl config set <k> <v>   Update a config value
//!   douyin-dl search <keyword>     Search users
//!   douyin-dl user <sec_uid>       Get user profile and recent videos
//!   douyin-dl feed                 Get recommended feed

use anyhow::{anyhow, Result};
use app_lib::api::{DouyinClient, SearchUserResult};
use app_lib::config::AppConfig;
use app_lib::downloader::{video_quality_diagnostic, Downloader, DownloaderEvent};
use app_lib::media_utils;
use clap::{Parser, Subcommand};
use std::io::IsTerminal;
use std::path::PathBuf;
use tokio::sync::mpsc;

// ---------------------------------------------------------------------------
// CLI argument definitions
// ---------------------------------------------------------------------------

#[derive(Parser)]
#[command(
    name = "douyin-dl",
    version,
    about = "抖音视频下载器 - CLI",
    long_about = "下载抖音视频、解析链接信息、搜索用户的命令行工具。\n\n使用前请确保已通过 config set cookie 设置了有效的抖音 Cookie。"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// 解析抖音视频链接，展示视频元信息
    Parse {
        /// 抖音视频 URL、分享短链、或纯数字 aweme_id
        url: String,
        /// 输出格式：json（默认，适合管道）或 plain（人类可读）
        #[arg(short = 'f', long, default_value = "json")]
        format: String,
    },

    /// 下载单个视频
    Download {
        /// 抖音视频 URL、分享短链、或纯数字 aweme_id
        url: String,
        /// 覆盖下载目录（默认使用配置中的 download_path）
        #[arg(short = 'o', long)]
        output: Option<String>,
    },

    /// 诊断视频清晰度候选和最终选择
    Quality {
        /// 抖音视频 URL、分享短链、或纯数字 aweme_id
        url: String,
        /// 输出格式：json（默认）或 plain
        #[arg(short = 'f', long, default_value = "json")]
        format: String,
        /// 下载质量：auto / highest / h264 / smallest / 480p / 720p / 1080p / 2k / 1440p / 4k / 2160p
        #[arg(short = 'q', long)]
        quality: Option<String>,
    },

    /// 查看或修改配置
    #[command(subcommand)]
    Config(ConfigCommands),

    /// 搜索抖音用户
    Search {
        /// 搜索关键词（昵称、用户名等）
        keyword: String,
        /// 输出格式：json 或 plain
        #[arg(short = 'f', long, default_value = "json")]
        format: String,
    },

    /// 获取用户详情和近期视频
    User {
        /// 用户的 sec_uid（可从 parse 输出或用户主页链接获取）
        sec_uid: String,
        /// 要列出的近期视频数量
        #[arg(short = 'l', long, default_value_t = 20)]
        limit: u32,
        /// 输出格式：json 或 plain
        #[arg(short = 'f', long, default_value = "json")]
        format: String,
    },

    /// 获取推荐视频流（需要登录 Cookie）
    Feed {
        /// 视频数量
        #[arg(short = 'c', long, default_value_t = 10)]
        count: u32,
    },
}

#[derive(Subcommand)]
enum ConfigCommands {
    /// 显示当前配置（Cookie 值隐藏，仅显示是否设置）
    Show,
    /// 设置配置项
    ///
    /// 支持的 key:
    ///   cookie           - 抖音 Cookie 字符串（从浏览器复制）
    ///   download_path    - 下载目录（绝对路径）
    ///   max_concurrent   - 最大并发下载数（1-20）
    ///   download_quality - 下载质量：auto / highest / h264 / smallest / 480p / 720p / 1080p / 2k / 1440p / 4k / 2160p
    ///   filename_template    - 文件名模板，支持 {title} {aweme_id} {author} {date} {time}
    ///   folder_name_template - 文件夹模板
    ///   auto_create_folder   - 是否自动创建子目录：true / false
    ///   save_metadata        - 是否保存元数据：true / false
    ///   proxy            - HTTP 代理地址
    ///   theme            - 主题：dark / light
    ///   language         - 语言：zh-CN / en
    Set {
        /// 配置项的 key
        key: String,
        /// 配置项的新值
        value: String,
    },
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Parse { url, format } => cmd_parse(&url, &format).await,
        Commands::Download { url, output } => cmd_download(&url, output.as_deref()).await,
        Commands::Quality {
            url,
            format,
            quality,
        } => cmd_quality(&url, &format, quality.as_deref()).await,
        Commands::Config(cmd) => cmd_config(cmd),
        Commands::Search { keyword, format } => cmd_search(&keyword, &format).await,
        Commands::User {
            sec_uid,
            limit,
            format,
        } => cmd_user(&sec_uid, limit, &format).await,
        Commands::Feed { count } => cmd_feed(count).await,
    }
}

async fn cmd_quality(url: &str, format: &str, quality: Option<&str>) -> Result<()> {
    let url = url.trim();
    if url.is_empty() {
        return Err(anyhow!("URL 不能为空"));
    }

    let config = AppConfig::load();
    let client = make_client(&config)?;
    let video = resolve_video(&client, url).await?;
    let quality = quality.unwrap_or(&config.download_quality);
    let diagnostic = video_quality_diagnostic(&video, quality);

    if format == "plain" {
        println!("aweme_id: {}", video.aweme_id);
        println!("requested_quality: {}", quality);
        println!(
            "supported_heights: {}",
            diagnostic["supported_heights"]
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .map(|item| item.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default()
        );
        println!(
            "selected_url: {}",
            diagnostic["selected_url"].as_str().unwrap_or("")
        );
        println!(
            "candidate_count: {}",
            diagnostic["candidates"].as_array().map(|items| items.len()).unwrap_or(0)
        );
    } else {
        println!("{}", serde_json::to_string_pretty(&diagnostic)?);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Helper: create client or return error
// ---------------------------------------------------------------------------

fn make_client(config: &AppConfig) -> Result<DouyinClient> {
    DouyinClient::new(config.clone()).map_err(|e| anyhow!("创建客户端失败：{}", e))
}

// ---------------------------------------------------------------------------
// Helper: resolve video from URL, share link, or raw aweme_id
// ---------------------------------------------------------------------------

async fn resolve_video(client: &DouyinClient, input: &str) -> Result<app_lib::api::VideoInfo> {
    // Try extracting aweme_id directly first
    if let Some(aweme_id) = DouyinClient::extract_aweme_id(input) {
        return client
            .get_video_detail(&aweme_id)
            .await
            .map_err(|e| anyhow!("获取视频详情失败（aweme_id={}）：{}", aweme_id, e));
    }

    // Try as share link (follows redirects)
    client
        .parse_share_link(input)
        .await
        .map_err(|e| anyhow!("解析分享链接失败：{}", e))
}

// ---------------------------------------------------------------------------
// parse command
// ---------------------------------------------------------------------------

async fn cmd_parse(url: &str, format: &str) -> Result<()> {
    let url = url.trim();
    if url.is_empty() {
        return Err(anyhow!("URL 不能为空"));
    }

    let config = AppConfig::load();
    let client = make_client(&config)?;
    let video = resolve_video(&client, url).await?;

    match format {
        "plain" => print_video_plain(&video),
        _ => {
            let json = media_utils::python_video_detail_value(&video);
            println!("{}", serde_json::to_string_pretty(&json)?);
        }
    }
    Ok(())
}

fn print_video_plain(video: &app_lib::api::VideoInfo) {
    let media_type = media_utils::python_media_type(video);
    println!("aweme_id:    {}", video.aweme_id);
    println!("标题:        {}", video.desc);
    println!(
        "作者:        {} (UID: {})",
        video.author.nickname, video.author.uid
    );
    println!("作者 sec_uid: {}", video.author.sec_uid);
    println!("发布时间:    {}", video.create_time);
    println!("媒体类型:    {}", media_type);
    println!("点赞:        {}", video.statistics.digg_count);
    println!("评论:        {}", video.statistics.comment_count);
    println!("分享:        {}", video.statistics.share_count);
    println!("播放:        {}", video.statistics.play_count);
    println!("封面:        {}", media_utils::python_cover_url(video));
    if !video.video.play_addr.is_empty() {
        println!("播放地址:    {}", video.video.play_addr);
    }
    if let Some(ref download_addr) = video.video.download_addr {
        if !download_addr.is_empty() {
            println!("下载地址:    {}", download_addr);
        }
    }
}

// ---------------------------------------------------------------------------
// download command
// ---------------------------------------------------------------------------

async fn cmd_download(url: &str, output: Option<&str>) -> Result<()> {
    let url = url.trim();
    if url.is_empty() {
        return Err(anyhow!("URL 不能为空"));
    }

    let mut config = AppConfig::load();

    // Override download path if specified
    if let Some(dir) = output {
        config.download_path = dir.to_string();
    }

    // Create client and resolve video
    let client = make_client(&config)?;
    let video = resolve_video(&client, url).await?;

    eprintln!("解析成功: {} — {}", video.desc, video.author.nickname);

    // Set up progress channel
    let (progress_tx, mut progress_rx) = mpsc::channel::<DownloaderEvent>(100);

    let downloader =
        Downloader::new(config, Some(progress_tx)).map_err(|e| anyhow!("创建下载器失败：{}", e))?;

    // Spawn progress listener
    let progress_handle = tokio::spawn(async move {
        let mut last_progress = -1f64;
        while let Some(event) = progress_rx.recv().await {
            match event.name {
                "download-started" => {
                    let display_name = event.payload["display_name"].as_str().unwrap_or("");
                    let media_type = event.payload["media_type"].as_str().unwrap_or("");
                    eprintln!(
                        "[开始] {} ({}) → {}",
                        display_name,
                        media_type,
                        event.payload["save_path"].as_str().unwrap_or("")
                    );
                }
                "download-progress" => {
                    let progress = event.payload["progress"].as_f64().unwrap_or(0.0);
                    // Only print at significant milestones to avoid flooding
                    let milestone = (progress / 10.0).floor();
                    let last_milestone = (last_progress / 10.0).floor();
                    if milestone > last_milestone || (progress - last_progress).abs() >= 10.0 {
                        let desc = event.payload["desc"].as_str().unwrap_or("");
                        let completed = event.payload["completed"].as_u64().unwrap_or(0);
                        let total = event.payload["total"].as_u64().unwrap_or(0);
                        eprintln!(
                            "[进度] {:3.0}% ({}/{}) {}",
                            progress, completed, total, desc
                        );
                        last_progress = progress;
                    }
                }
                "download-log" => {
                    let msg = event.payload["message"].as_str().unwrap_or("");
                    eprintln!("[日志] {}", msg);
                }
                "download-completed" => {
                    let file_path = event.payload["file_path"]
                        .as_str()
                        .or_else(|| event.payload["save_path"].as_str())
                        .unwrap_or("");
                    eprintln!("[完成] 下载成功 → {}", file_path);
                    // Print file path to stdout for piping
                    if !file_path.is_empty() {
                        println!("{}", file_path);
                    }
                }
                "download-failed" | "download-error" => {
                    let msg = event.payload["error"]
                        .as_str()
                        .or_else(|| event.payload["message"].as_str())
                        .unwrap_or("未知错误");
                    eprintln!("[错误] {}", msg);
                }
                "download-cancelled" => {
                    eprintln!("[取消] 下载已取消");
                }
                _ => {}
            }
        }
    });

    // Add and start download
    let task_id = downloader
        .add_task(&video, None)
        .await
        .map_err(|e| anyhow!("创建下载任务失败：{}", e))?;

    if let Err(e) = downloader.start_download(&task_id).await {
        return Err(anyhow!("启动下载失败：{}", e));
    }

    // Wait for download to complete
    progress_handle.await.ok();

    Ok(())
}

// ---------------------------------------------------------------------------
// config command
// ---------------------------------------------------------------------------

fn cmd_config(cmd: ConfigCommands) -> Result<()> {
    match cmd {
        ConfigCommands::Show => {
            let config = AppConfig::load();
            let has_cookie = !config.cookie.is_empty();
            let json = serde_json::json!({
                "download_path": config.download_path,
                "cookie_set": has_cookie,
                "cookie": if has_cookie { "***" } else { "" },
                "proxy": config.proxy,
                "max_concurrent": config.max_concurrent,
                "download_quality": config.download_quality,
                "filename_template": config.filename_template,
                "folder_name_template": config.folder_name_template,
                "auto_create_folder": config.auto_create_folder,
                "save_metadata": config.save_metadata,
                "theme": config.theme,
                "language": config.language,
                "config_file": dirs::config_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join("better-douyin-R")
                    .join("config.json")
                    .display()
                    .to_string(),
            });
            println!("{}", serde_json::to_string_pretty(&json)?);
        }
        ConfigCommands::Set { key, value } => {
            let mut config = AppConfig::load();
            let config_file = dirs::config_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("better-douyin-R")
                .join("config.json");

            match key.as_str() {
                "download_path" => config.download_path = value,
                "cookie" => {
                    if value.trim().is_empty() {
                        return Err(anyhow!("Cookie 不能为空"));
                    }
                    config.cookie = value;
                }
                "proxy" => config.proxy = Some(value),
                "max_concurrent" => {
                    let n: usize = value
                        .parse()
                        .map_err(|_| anyhow!("max_concurrent 必须是整数（1-20）"))?;
                    config.max_concurrent = n;
                }
                "download_quality" => {
                    let Some(v) = AppConfig::canonical_download_quality(&value) else {
                        return Err(anyhow!(
                            "download_quality 必须是 auto / highest / h264 / smallest / 480p / 720p / 1080p / 2k / 1440p / 4k / 2160p"
                        ));
                    };
                    config.download_quality = v.to_string();
                }
                "filename_template" => config.filename_template = value,
                "folder_name_template" => config.folder_name_template = value,
                "auto_create_folder" => {
                    let b: bool = value
                        .parse()
                        .map_err(|_| anyhow!("auto_create_folder 必须是 true 或 false"))?;
                    config.auto_create_folder = b;
                }
                "save_metadata" => {
                    let b: bool = value
                        .parse()
                        .map_err(|_| anyhow!("save_metadata 必须是 true 或 false"))?;
                    config.save_metadata = b;
                }
                "theme" => config.theme = value,
                "language" => config.language = value,
                _ => {
                    return Err(anyhow!(
                        "未知的配置项: {}\n支持: download_path, cookie, proxy, max_concurrent, \
                         download_quality, filename_template, folder_name_template, \
                         auto_create_folder, save_metadata, theme, language",
                        key
                    ));
                }
            }

            config.save().map_err(|e| anyhow!("保存配置失败：{}", e))?;
            eprintln!("[配置] {} 已更新 → {}", key, config_file.display());
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// search command
// ---------------------------------------------------------------------------

async fn cmd_search(keyword: &str, format: &str) -> Result<()> {
    let keyword = keyword.trim();
    if keyword.is_empty() {
        return Err(anyhow!("搜索关键词不能为空"));
    }

    let config = AppConfig::load();
    let client = make_client(&config)?;

    match client.search_user(keyword).await? {
        SearchUserResult::NeedVerify { verify_url } => {
            if format == "plain" {
                println!("需要滑块验证");
                println!("验证链接: {}", verify_url);
            } else {
                println!(
                    "{}",
                    serde_json::json!({
                        "success": false,
                        "need_verify": true,
                        "verify_url": verify_url,
                        "message": "需要滑块验证"
                    })
                );
            }
        }
        SearchUserResult::NotFound => {
            if format == "plain" {
                println!("未找到用户: {}", keyword);
            } else {
                println!(
                    "{}",
                    serde_json::json!({"success": false, "message": "未找到用户"})
                );
            }
        }
        SearchUserResult::Single(user) => {
            let v = media_utils::python_user_value(&user);
            if format == "plain" {
                print_user_plain(&serde_json::json!({
                    "users": [v],
                    "count": 1
                }));
            } else {
                println!("{}", serde_json::to_string_pretty(&v)?);
            }
        }
        SearchUserResult::Multiple(users) => {
            if format == "plain" {
                let items: Vec<serde_json::Value> =
                    users.iter().map(media_utils::python_user_value).collect();
                print_user_plain(&serde_json::json!({
                    "users": items,
                    "count": users.len()
                }));
            } else {
                let items: Vec<serde_json::Value> =
                    users.iter().map(media_utils::python_user_value).collect();
                println!("{}", serde_json::to_string_pretty(&items)?);
            }
        }
    }
    Ok(())
}

fn print_user_plain(payload: &serde_json::Value) {
    if let Some(users) = payload["users"].as_array() {
        let count = payload["count"].as_u64().unwrap_or(users.len() as u64);
        println!("找到 {} 个用户:", count);
        for (i, user) in users.iter().enumerate() {
            println!(
                "  [{i}] {} (@{}) - 粉丝:{} 作品:{}",
                user["nickname"].as_str().unwrap_or(""),
                user["unique_id"].as_str().unwrap_or(""),
                user["follower_count"].as_i64().unwrap_or(0),
                user["aweme_count"].as_i64().unwrap_or(0),
            );
            println!("      sec_uid: {}", user["sec_uid"].as_str().unwrap_or(""));
            if let Some(sig) = user["signature"].as_str() {
                if !sig.is_empty() {
                    println!("      签名: {}", sig);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// user command
// ---------------------------------------------------------------------------

async fn cmd_user(sec_uid: &str, limit: u32, format: &str) -> Result<()> {
    let sec_uid = sec_uid.trim();
    if sec_uid.is_empty() {
        return Err(anyhow!("sec_uid 不能为空"));
    }

    let config = AppConfig::load();
    let client = make_client(&config)?;

    let user_detail = client
        .get_user_detail(sec_uid)
        .await
        .map_err(|e| anyhow!("获取用户详情失败：{}", e))?;

    let (videos, _cursor, _has_more) = client
        .get_user_videos(sec_uid, 0, limit.min(50))
        .await
        .map_err(|e| anyhow!("获取用户视频失败：{}", e))?;

    if format == "plain" {
        println!(
            "用户: {} (@{})",
            user_detail.info.nickname, user_detail.info.unique_id
        );
        println!("UID: {}", user_detail.info.uid);
        println!("sec_uid: {}", user_detail.info.sec_uid);
        println!("粉丝: {}", user_detail.info.follower_count);
        println!("关注: {}", user_detail.info.following_count);
        println!("作品数: {}", user_detail.info.aweme_count);
        println!("获赞: {}", user_detail.info.total_favorited);
        if !user_detail.info.signature.is_empty() {
            println!("签名: {}", user_detail.info.signature);
        }
        println!();
        println!("近期视频 ({}):", videos.len());
        for (i, video) in videos.iter().enumerate() {
            let mt = media_utils::python_media_type(video);
            println!(
                "  [{i}] {} | {} | ❤️{} | {}",
                video.aweme_id,
                truncate_str(&video.desc, 40),
                video.statistics.digg_count,
                mt,
            );
        }
    } else {
        let video_summaries: Vec<serde_json::Value> = videos
            .iter()
            .map(|v| media_utils::python_video_summary(v, true, true))
            .collect();
        let output = serde_json::json!({
            "user": media_utils::python_user_value(&user_detail.info),
            "videos": video_summaries,
            "video_count": videos.len(),
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    }
    Ok(())
}

fn truncate_str(s: &str, max_chars: usize) -> String {
    let s = s.trim();
    if s.chars().count() > max_chars {
        format!("{}...", s.chars().take(max_chars).collect::<String>())
    } else {
        s.to_string()
    }
}

// ---------------------------------------------------------------------------
// feed command
// ---------------------------------------------------------------------------

async fn cmd_feed(count: u32) -> Result<()> {
    let config = AppConfig::load();
    let client = make_client(&config)?;

    let (videos, _cursor, _has_more) = client
        .get_recommended_feed(0, count.min(50), "featured")
        .await
        .map_err(|e| anyhow!("获取推荐视频失败：{}", e))?;

    let items: Vec<serde_json::Value> = videos
        .iter()
        .map(|v| media_utils::python_video_summary(v, true, true))
        .collect();

    // Always output JSON for feed (pipe-friendly)
    let output = serde_json::json!({
        "videos": items,
        "count": items.len(),
    });

    if std::io::stdout().is_terminal() {
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("{}", serde_json::to_string(&output)?);
    }
    Ok(())
}
