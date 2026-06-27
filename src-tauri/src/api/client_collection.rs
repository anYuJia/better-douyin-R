//! 收藏合集客户端逻辑 - 喜欢/收藏/合集视频

use anyhow::{anyhow, Result};
use std::collections::HashMap;

use super::client::DouyinClient;
use super::client_content::{clean_video_media_url, is_dash_video_only_url};
use super::types::*;
use crate::config::get_user_agent;
use crate::sign;

impl DouyinClient {
    pub(super) fn extract_liked_media_info(
        &self,
        post: &serde_json::Value,
    ) -> (String, Vec<LikedVideoMediaUrl>) {
        let mut urls = Vec::new();
        let mut media_type = "unknown".to_string();

        if let Some(images) = post.get("images").and_then(|value| value.as_array()) {
            let mut has_live = false;
            let mut has_image = false;

            for image in images {
                if let Some(video_urls) = image
                    .get("video")
                    .and_then(|value| value.get("play_addr"))
                    .and_then(|value| value.get("url_list"))
                    .and_then(|value| value.as_array())
                {
                    has_live = true;
                    if let Some(url) = video_urls.first().and_then(|value| value.as_str()) {
                        urls.push(LikedVideoMediaUrl {
                            r#type: "live_photo".to_string(),
                            url: url.to_string(),
                        });
                    }
                } else if let Some(image_urls) =
                    image.get("url_list").and_then(|value| value.as_array())
                {
                    if let Some(url) = image_urls.last().and_then(|value| value.as_str()) {
                        has_image = true;
                        urls.push(LikedVideoMediaUrl {
                            r#type: "image".to_string(),
                            url: url.to_string(),
                        });
                    }
                }
            }

            media_type = if has_live && has_image {
                "mixed".to_string()
            } else if has_live {
                "live_photo".to_string()
            } else if has_image {
                "image".to_string()
            } else {
                "unknown".to_string()
            };
        } else if let Some(video_urls) = post
            .get("video")
            .and_then(|value| value.get("play_addr"))
            .and_then(|value| value.get("url_list"))
            .and_then(|value| value.as_array())
        {
            if let Some(url) = video_urls.first().and_then(|value| value.as_str()) {
                let clean_url = clean_video_media_url(url);
                if !clean_url.is_empty() && !is_dash_video_only_url(&clean_url) {
                    media_type = "video".to_string();
                    urls.push(LikedVideoMediaUrl {
                        r#type: "video".to_string(),
                        url: clean_url,
                    });
                }
            }
        }

        (media_type, urls)
    }

    fn extract_liked_bgm_url(&self, post: &serde_json::Value) -> Option<String> {
        let music = post.get("music")?;
        let mut bgm_url = self.extract_music_play_url_value(music);

        if bgm_url
            .as_ref()
            .map(|value| value.is_empty())
            .unwrap_or(true)
        {
            let h5_url = music
                .get("h5_url")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            let web_url = music
                .get("web_url")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            bgm_url = Some(if !h5_url.is_empty() {
                h5_url.to_string()
            } else {
                web_url.to_string()
            });
        }

        if bgm_url
            .as_ref()
            .map(|value| value.is_empty())
            .unwrap_or(true)
        {
            if let Some(music_file) = music.get("music_file") {
                if music_file.is_object() {
                    bgm_url = self.get_first_url_opt(&music_file["url_list"]);
                } else if let Some(url) = music_file.as_str() {
                    bgm_url = Some(url.to_string());
                }
            }
        }

        bgm_url
    }

    fn build_liked_video_item(
        &self,
        post: &serde_json::Value,
        default_liked: bool,
        default_collected: bool,
    ) -> Option<LikedVideoItem> {
        let aweme_id = post.get("aweme_id")?.as_str()?.to_string();
        let (media_type, media_urls) = self.extract_liked_media_info(post);
        let video_data = &post["video"];
        let dash_addr = Self::select_dash_video_url(video_data);
        let audio_addr = Self::select_dash_audio_url(video_data);
        let raw_play_addr = self.get_first_url(&video_data["play_addr"]);
        let selected_play_addr = clean_video_media_url(&raw_play_addr);
        let selected_play_addr = if is_dash_video_only_url(&selected_play_addr) {
            String::new()
        } else {
            selected_play_addr
        };

        let cover_url = post
            .get("video")
            .and_then(|value| value.get("cover"))
            .and_then(|value| value.get("url_list"))
            .and_then(|value| self.get_first_url_opt(value))
            .or_else(|| {
                post.get("images")
                    .and_then(|value| value.as_array())
                    .and_then(|images| images.first())
                    .and_then(|image| image.get("url_list"))
                    .and_then(|value| self.get_last_url_opt(value))
            })
            .unwrap_or_default();
        let fallback_media_url = media_urls
            .first()
            .map(|media| media.url.clone())
            .unwrap_or_default();
        let preview_addr = if selected_play_addr.is_empty() {
            fallback_media_url.clone()
        } else {
            selected_play_addr.clone()
        };
        let duration = video_data["duration"].as_i64().unwrap_or(0);
        let bit_rate = video_data["bit_rate"].as_array().and_then(|arr| {
            let items = arr
                .iter()
                .filter_map(|b| {
                    let play_addr = self.get_first_url_opt(&b["play_addr"]);
                    let play_addr_h264 = self.get_first_url_opt(&b["play_addr_h264"]);
                    if play_addr.is_none() && play_addr_h264.is_none() {
                        return None;
                    }
                    Some(BitRateInfo {
                        gear_name: b["gear_name"].as_str().unwrap_or_default().to_string(),
                        format: b["format"].as_str().unwrap_or_default().to_string(),
                        bit_rate: b["bit_rate"].as_i64().unwrap_or(0),
                        quality_type: b["quality_type"].as_i64().unwrap_or(0) as i32,
                        is_h265: b["is_h265"].as_bool().unwrap_or(false),
                        data_size: b["data_size"].as_i64().unwrap_or(0),
                        width: b["width"].as_i64().unwrap_or(0) as i32,
                        height: b["height"].as_i64().unwrap_or(0) as i32,
                        play_addr,
                        play_addr_h264,
                    })
                })
                .collect::<Vec<_>>();
            if items.is_empty() {
                None
            } else {
                Some(items)
            }
        });

        Some(LikedVideoItem {
            aweme_id,
            desc: post["desc"].as_str().unwrap_or_default().to_string(),
            create_time: post["create_time"].as_i64().unwrap_or(0),
            digg_count: post["statistics"]["digg_count"].as_i64().unwrap_or(0),
            comment_count: post["statistics"]["comment_count"].as_i64().unwrap_or(0),
            share_count: post["statistics"]["share_count"].as_i64().unwrap_or(0),
            cover_url: cover_url.clone(),
            duration,
            media_type: media_type.clone(),
            raw_media_type: media_type,
            media_urls,
            bgm_url: self.extract_liked_bgm_url(post),
            is_liked: Self::json_boolish_any(post, &["user_digged", "is_liked", "digg_status"])
                || default_liked,
            is_collected: Self::json_boolish_any(
                post,
                &[
                    "is_collected",
                    "is_collect",
                    "collect_status",
                    "collect_stat",
                ],
            ) || default_collected,
            statistics: Statistics {
                digg_count: post["statistics"]["digg_count"].as_i64().unwrap_or(0),
                comment_count: post["statistics"]["comment_count"].as_i64().unwrap_or(0),
                share_count: post["statistics"]["share_count"].as_i64().unwrap_or(0),
                play_count: post["statistics"]["play_count"].as_i64().unwrap_or(0),
                collect_count: post["statistics"]["collect_count"].as_i64().unwrap_or(0),
                ..Default::default()
            },
            video: VideoData {
                preview_addr: if preview_addr.is_empty() {
                    None
                } else {
                    Some(preview_addr.clone())
                },
                play_addr: if selected_play_addr.is_empty() {
                    fallback_media_url.clone()
                } else {
                    selected_play_addr
                },
                dash_addr,
                audio_addr,
                play_addr_h264: self.get_first_url_opt(&video_data["play_addr_h264"]),
                play_addr_lowbr: self.get_first_url_opt(&video_data["play_addr_lowbr"]),
                download_addr: self.get_first_url_opt(&video_data["download_addr"]),
                cover: cover_url.clone(),
                dynamic_cover: self
                    .get_first_url_opt(&video_data["dynamic_cover"]["url_list"])
                    .unwrap_or_else(|| cover_url.clone()),
                origin_cover: self
                    .get_first_url_opt(&video_data["origin_cover"]["url_list"])
                    .unwrap_or_else(|| cover_url.clone()),
                width: video_data["width"].as_i64().unwrap_or(0) as i32,
                height: video_data["height"].as_i64().unwrap_or(0) as i32,
                duration,
                ratio: video_data["ratio"].as_str().unwrap_or_default().to_string(),
                bit_rate,
            },
            author: LikedVideoAuthor {
                nickname: post["author"]["nickname"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                sec_uid: post["author"]["sec_uid"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                avatar_thumb: post
                    .get("author")
                    .and_then(|value| value.get("avatar_thumb"))
                    .and_then(|value| value.get("url_list"))
                    .and_then(|value| self.get_first_url_opt(value))
                    .unwrap_or_default(),
            },
        })
    }

    pub(super) async fn request_liked_videos_response(
        &self,
        sec_uid: &str,
        max_cursor: i64,
        count: u32,
    ) -> Result<serde_json::Value> {
        let mut params = HashMap::new();
        params.insert("max_cursor", max_cursor.to_string());
        params.insert("count", count.to_string());
        if !sec_uid.is_empty() {
            params.insert("sec_user_id", sec_uid.to_string());
        }

        let mut headers = HashMap::new();
        headers.insert("Referer".to_string(), "https://www.douyin.com/".to_string());

        let response = self
            .request_raw_json_with_options(
                "https://www.douyin.com/aweme/v1/web/aweme/favorite/",
                Some(params),
                "GET",
                Some(headers),
                true,
            )
            .await?;

        let status_code = response["status_code"].as_i64().unwrap_or(0);
        if status_code != 0 {
            let status_msg = response["status_msg"].as_str().unwrap_or("unknown error");
            return Err(anyhow!("API error: {}", status_msg));
        }

        Ok(response)
    }

    pub async fn get_liked_videos_python_style(
        &self,
        sec_uid: &str,
        max_cursor: i64,
        count: u32,
    ) -> Result<(Vec<LikedVideoItem>, i64, bool)> {
        let response = self
            .request_liked_videos_response(sec_uid, max_cursor, count)
            .await?;

        let cursor = response["max_cursor"]
            .as_i64()
            .or_else(|| response["cursor"].as_i64())
            .or_else(|| response["min_cursor"].as_i64())
            .unwrap_or(0);
        let has_more = response["has_more"].as_i64().unwrap_or(0) == 1
            || response["has_more"].as_bool().unwrap_or(false);
        let videos = response["aweme_list"]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|post| self.build_liked_video_item(post, true, false))
                    .collect()
            })
            .unwrap_or_default();

        Ok((videos, cursor, has_more))
    }

    async fn request_collected_videos_response(
        &self,
        cursor: i64,
        count: u32,
    ) -> Result<serde_json::Value> {
        let url = "https://www.douyin.com/aweme/v1/web/aweme/listcollection/";
        let mut query_params = crate::config::get_common_params();
        query_params.insert("count".to_string(), count.to_string());
        query_params.insert("cursor".to_string(), cursor.to_string());

        let mut headers = crate::config::get_common_headers(&self.config.cookie);
        headers.insert(
            "Referer".to_string(),
            "https://www.douyin.com/user/self?from_tab_name=main&showTab=favorite_collection"
                .to_string(),
        );
        headers.insert("Origin".to_string(), "https://www.douyin.com".to_string());
        headers.insert(
            "Content-Type".to_string(),
            "application/x-www-form-urlencoded; charset=UTF-8".to_string(),
        );

        self.enrich_request(&mut query_params, &mut headers).await;

        let params_str = serde_urlencoded::to_string(&query_params)?;
        let user_agent = headers
            .get("User-Agent")
            .map(String::as_str)
            .unwrap_or_else(|| get_user_agent());
        query_params.insert(
            "a_bogus".to_string(),
            sign::sign_detail(&params_str, user_agent),
        );

        let mut body_params = HashMap::new();
        body_params.insert("count", count.to_string());
        body_params.insert("cursor", cursor.to_string());

        let mut req = self
            .client
            .post(url)
            .query(&query_params)
            .form(&body_params);
        for (key, value) in &headers {
            req = req.header(key, value);
        }

        let response = req
            .send()
            .await
            .map_err(|error| anyhow!("HTTP request failed: {}", error))?;
        if !response.status().is_success() {
            return Err(anyhow!("HTTP error: {}", response.status()));
        }

        let json = response.json::<serde_json::Value>().await?;
        Self::ensure_status_ok(&json)?;
        Ok(json)
    }

    pub async fn get_collected_videos_python_style(
        &self,
        cursor: i64,
        count: u32,
    ) -> Result<(Vec<LikedVideoItem>, i64, bool)> {
        let response = self
            .request_collected_videos_response(cursor, count)
            .await?;

        let videos = response["aweme_list"]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|post| self.build_liked_video_item(post, false, true))
                    .collect()
            })
            .unwrap_or_default();

        Ok((
            videos,
            Self::json_cursor(&response),
            Self::json_has_more(&response),
        ))
    }

    /// 获取收藏视频列表（返回 VideoInfo，用于批量下载）
    pub async fn get_collected_videos(
        &self,
        cursor: i64,
        count: u32,
    ) -> Result<(Vec<VideoInfo>, i64, bool)> {
        let response = self
            .request_collected_videos_response(cursor, count)
            .await?;

        let videos = response["aweme_list"]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|post| self.parse_video_info(post).ok())
                    .collect()
            })
            .unwrap_or_default();

        Ok((
            videos,
            Self::json_cursor(&response),
            Self::json_has_more(&response),
        ))
    }

    /// 获取收藏合集列表
    pub async fn get_collected_mixes(
        &self,
        cursor: i64,
        count: u32,
    ) -> Result<(Vec<CollectionMixItem>, i64, bool)> {
        let mut params = HashMap::new();
        params.insert("count", count.to_string());
        params.insert("cursor", cursor.to_string());

        let mut headers = HashMap::new();
        headers.insert(
            "Referer".to_string(),
            "https://www.douyin.com/user/self?from_tab_name=main&showTab=favorite_collection"
                .to_string(),
        );

        let response = self
            .request_raw_json_with_options(
                "https://www.douyin.com/aweme/v1/web/mix/listcollection/",
                Some(params),
                "GET",
                Some(headers),
                false,
            )
            .await?;
        Self::ensure_status_ok(&response)?;

        let mixes = response["mix_infos"]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| self.build_collection_mix_item(item))
                    .collect()
            })
            .unwrap_or_default();

        Ok((
            mixes,
            Self::json_cursor(&response),
            Self::json_has_more(&response),
        ))
    }

    fn build_collection_mix_item(&self, item: &serde_json::Value) -> Option<CollectionMixItem> {
        let mix_id = item["mix_id"].as_str().unwrap_or_default().to_string();
        if mix_id.is_empty() {
            return None;
        }

        let author = item.get("author");
        let statis = item.get("statis");

        Some(CollectionMixItem {
            mix_id,
            mix_name: item["mix_name"].as_str().unwrap_or_default().to_string(),
            desc: item["desc"].as_str().unwrap_or_default().to_string(),
            cover_url: item
                .get("cover_url")
                .and_then(|value| value.get("url_list"))
                .and_then(|value| self.get_first_url_opt(value))
                .unwrap_or_default(),
            author: CollectionMixAuthor {
                nickname: author
                    .and_then(|value| value["nickname"].as_str())
                    .unwrap_or_default()
                    .to_string(),
                sec_uid: author
                    .and_then(|value| value["sec_uid"].as_str())
                    .unwrap_or_default()
                    .to_string(),
                avatar_thumb: author
                    .and_then(|value| value.get("avatar_thumb"))
                    .and_then(|value| value.get("url_list"))
                    .and_then(|value| self.get_first_url_opt(value))
                    .unwrap_or_default(),
            },
            statis: CollectionMixStats {
                collect_vv: statis
                    .and_then(|value| value["collect_vv"].as_i64())
                    .unwrap_or(0),
                play_vv: statis
                    .and_then(|value| value["play_vv"].as_i64())
                    .unwrap_or(0),
                updated_to_episode: statis
                    .and_then(|value| value["updated_to_episode"].as_i64())
                    .unwrap_or(0),
            },
            create_time: item["create_time"].as_i64().unwrap_or(0),
            update_time: item["update_time"].as_i64().unwrap_or(0),
            mix_type: item["mix_type"].as_i64().unwrap_or(0) as i32,
        })
    }

    /// 获取合集内的视频列表
    pub async fn get_mix_videos(
        &self,
        series_id: &str,
        cursor: i64,
        count: u32,
    ) -> Result<(Vec<VideoInfo>, i64, bool)> {
        let mut params = HashMap::new();
        params.insert("series_id", series_id.to_string());
        params.insert("pull_type", "2".to_string());
        params.insert("cursor", cursor.to_string());
        params.insert("count", count.to_string());

        let mut headers = HashMap::new();
        headers.insert(
            "Referer".to_string(),
            "https://www.douyin.com/user/self?from_tab_name=main&showTab=favorite_collection"
                .to_string(),
        );

        let response = self
            .request_raw_json_with_options(
                "https://www.douyin.com/aweme/v1/web/series/aweme/",
                Some(params),
                "GET",
                Some(headers),
                false,
            )
            .await?;
        Self::ensure_status_ok(&response)?;

        let videos = response["aweme_list"]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|post| self.parse_video_info(post).ok())
                    .collect()
            })
            .unwrap_or_default();

        Ok((
            videos,
            Self::json_cursor(&response),
            Self::json_has_more(&response),
        ))
    }
}
