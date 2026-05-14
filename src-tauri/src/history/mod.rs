//! 下载历史管理

use crate::api::DownloadHistory;
use anyhow::Result;
use serde_json;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

const MAX_HISTORY_ITEMS: usize = 1000;

/// 历史管理器
pub struct HistoryManager {
    history: Vec<DownloadHistory>,
    file_path: PathBuf,
    /// HashSet 索引加速 is_downloaded 查询
    id_index: HashSet<String>,
}

impl HistoryManager {
    /// 加载历史记录
    pub fn load() -> Self {
        let file_path = Self::get_history_path();

        let history: Vec<DownloadHistory> = if file_path.exists() {
            fs::read_to_string(&file_path)
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default()
        } else {
            vec![]
        };

        let id_index: HashSet<String> = history.iter().map(|h| h.aweme_id.clone()).collect();

        Self {
            history,
            file_path,
            id_index,
        }
    }

    /// 获取历史文件路径
    fn get_history_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("douyin-downloader")
            .join("history.json")
    }

    /// 保存到文件
    fn save(&self) -> Result<()> {
        if let Some(parent) = self.file_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut json = serde_json::to_string_pretty(&self.history)?;
        json.push('\n');
        write_file_atomically(&self.file_path, json.as_bytes())?;

        Ok(())
    }

    /// 获取所有历史
    pub fn get_all(&self) -> Vec<DownloadHistory> {
        self.history.clone()
    }

    /// 添加历史记录
    pub fn add(&mut self, record: DownloadHistory) -> Result<()> {
        // 如果已存在，先删除旧记录并更新索引
        if self.id_index.contains(&record.aweme_id) {
            if let Some(pos) = self
                .history
                .iter()
                .position(|h| h.aweme_id == record.aweme_id)
            {
                self.history.remove(pos);
            }
        }

        // 添加到开头并更新索引
        self.id_index.insert(record.aweme_id.clone());
        self.history.insert(0, record);

        // 限制数量，同时维护索引一致性
        if self.history.len() > MAX_HISTORY_ITEMS {
            self.history.truncate(MAX_HISTORY_ITEMS);
            // 重新构建索引以确保一致性
            self.rebuild_index();
        }

        self.save()
    }

    /// 删除历史记录
    pub fn delete(&mut self, aweme_id: &str) -> Result<()> {
        self.history.retain(|h| h.aweme_id != aweme_id);
        self.id_index.remove(aweme_id);
        self.save()
    }

    /// 清空历史
    pub fn clear(&mut self) -> Result<()> {
        self.history.clear();
        self.id_index.clear();
        self.save()
    }

    /// 检查是否已下载 (O(1))
    pub fn is_downloaded(&self, aweme_id: &str) -> bool {
        self.id_index.contains(aweme_id)
    }

    /// 获取记录
    pub fn get(&self, aweme_id: &str) -> Option<&DownloadHistory> {
        self.history.iter().find(|h| h.aweme_id == aweme_id)
    }

    /// 重建索引
    fn rebuild_index(&mut self) {
        self.id_index = self.history.iter().map(|h| h.aweme_id.clone()).collect();
    }
}

fn write_file_atomically(path: &Path, content: &[u8]) -> Result<()> {
    let temp_path = path.with_extension("tmp");
    fs::write(&temp_path, content)?;
    if let Err(error) = fs::rename(&temp_path, path) {
        let _ = fs::remove_file(&temp_path);
        return Err(error.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::DownloadHistory;

    fn sample_history(aweme_id: &str) -> DownloadHistory {
        DownloadHistory {
            aweme_id: aweme_id.to_string(),
            title: format!("test {}", aweme_id),
            author: "author".to_string(),
            author_id: "123".to_string(),
            cover: "".to_string(),
            file_path: "/tmp/test".to_string(),
            media_type: "video".to_string(),
            file_size: 1024,
            create_time: 0,
        }
    }

    fn test_history_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "douyin-downloader-{}-{}.json",
            name,
            uuid::Uuid::new_v4()
        ))
    }

    #[test]
    fn adds_and_checks_downloaded_status() {
        let mut mgr = HistoryManager {
            history: vec![],
            file_path: test_history_path("adds"),
            id_index: HashSet::new(),
        };

        assert!(!mgr.is_downloaded("123"));

        mgr.add(sample_history("123")).unwrap();
        assert!(mgr.is_downloaded("123"));
        assert!(!mgr.is_downloaded("456"));

        // 添加重复项会替换旧项
        mgr.add(sample_history("123")).unwrap();
        assert_eq!(mgr.history.len(), 1);
    }

    #[test]
    fn deletes_record_and_updates_index() {
        let mut mgr = HistoryManager {
            history: vec![],
            file_path: test_history_path("deletes"),
            id_index: HashSet::new(),
        };

        mgr.add(sample_history("123")).unwrap();
        mgr.add(sample_history("456")).unwrap();
        assert!(mgr.is_downloaded("456"));

        mgr.delete("456").unwrap();
        assert!(!mgr.is_downloaded("456"));
        assert!(mgr.is_downloaded("123"));
    }

    #[test]
    fn truncates_history_and_rebuilds_index() {
        let mut mgr = HistoryManager {
            history: vec![],
            file_path: test_history_path("truncates"),
            id_index: HashSet::new(),
        };

        for i in 0..(MAX_HISTORY_ITEMS + 5) {
            mgr.add(sample_history(&format!("{}", i))).unwrap();
        }

        assert_eq!(mgr.history.len(), MAX_HISTORY_ITEMS);
        // 索引应与历史保持一致
        assert_eq!(mgr.id_index.len(), MAX_HISTORY_ITEMS);
    }
}
