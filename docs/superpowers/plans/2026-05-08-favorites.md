# 收藏列表功能实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 为 Douyin Downloader 添加收藏列表功能，支持查看收藏视频列表、单个下载、批量下载全部收藏视频。

**架构:** 完全复用点赞列表的架构模式 — 后端新增 API client 方法和 Tauri 命令，前端新增对应的 adapter 映射、UI section、事件处理函数和缓存。收藏视频项复用 `LikedVideoItem` 类型，解析复用 `build_liked_video_item()`。

**Tech Stack:** Rust (Tauri 2 / Tokio / Reqwest), 原生 HTML/CSS/JS (Bootstrap 5), localStorage 缓存

---

## Task 1: 后端 — API Client 新增收藏列表方法

**Files:**
- Modify: `src-tauri/src/api/client.rs`
- Test: `src-tauri/src/api/client.rs` (unit test)

- [ ] **Step 1: 在 `request_liked_videos_response()` 之后新增 `request_collected_videos_response()`**

在 `client.rs` 第 920 行（`request_liked_videos_response` 的 closing `}`）之后插入：

```rust
    async fn request_collected_videos_response(
        &self,
        max_cursor: i64,
        count: u32,
    ) -> Result<serde_json::Value> {
        let mut params = HashMap::new();
        params.insert("max_cursor", max_cursor.to_string());
        params.insert("count", count.to_string());

        let mut headers = HashMap::new();
        headers.insert("Referer".to_string(), "https://www.douyin.com/".to_string());

        let response = self
            .request_raw_json_with_options(
                "https://www.douyin.com/aweme/v1/web/aweme/listcollection/",
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
```

- [ ] **Step 2: 在 `get_liked_videos_python_style()` 之后新增 `get_collected_videos_python_style()`**

在第 941 行（`get_liked_videos_python_style` 的 closing `}`）之后插入：

```rust
    pub async fn get_collected_videos_python_style(
        &self,
        max_cursor: i64,
        count: u32,
    ) -> Result<Vec<LikedVideoItem>> {
        let response = self
            .request_collected_videos_response(max_cursor, count)
            .await?;

        Ok(response["aweme_list"]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|post| self.build_liked_video_item(post))
                    .collect()
            })
            .unwrap_or_default())
    }
```

- [ ] **Step 3: 编译验证**

```bash
cd src-tauri && cargo check
```
Expected: 编译通过，无 warning

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/api/client.rs
git commit -m "feat(api): 添加收藏列表 API client 方法"
```

---

## Task 2: 后端 — Tauri 命令新增收藏列表命令

**Files:**
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: 在 `get_liked_videos()` 命令之后新增 `get_collected_videos` 命令**

在 `lib.rs` 第 803 行（`get_liked_videos` 的 closing `}`）之后插入：

```rust
/// 获取收藏视频列表
#[tauri::command]
async fn get_collected_videos(
    state: State<'_, AppState>,
    cursor: i64,
    count: u32,
) -> Result<serde_json::Value, String> {
    let client = match get_client(&state).await {
        Ok(client) => client,
        Err(_) => {
            return Ok(serde_json::json!({
                "success": false,
                "message": "请先设置Cookie"
            }));
        }
    };

    match client
        .get_collected_videos_python_style(cursor, count)
        .await
    {
        Ok(videos) if !videos.is_empty() => {
            let count = videos.len();
            Ok(serde_json::json!({
                "success": true,
                "data": videos,
                "count": count
            }))
        }
        Ok(_) => Ok(serde_json::json!({
            "success": false,
            "message": "获取收藏视频失败。该接口需要登录态，请确认Cookie有效且包含完整的登录信息。如果Cookie已过期请重新获取。"
        })),
        Err(e) => Ok(serde_json::json!({
            "success": false,
            "message": format!("获取收藏视频失败: {}", e)
        })),
    }
}
```

- [ ] **Step 2: 在 `download_liked_videos()` 命令之后新增 `download_collected_videos` 命令**

找到 `download_liked_videos` 命令（约第 1166-1232 行），在其 closing `}` 之后插入：

```rust
/// 下载收藏视频
#[tauri::command]
async fn download_collected_videos(
    state: State<'_, AppState>,
    count: u32,
) -> Result<serde_json::Value, String> {
    let client = match get_client(&state).await {
        Ok(client) => client,
        Err(_) => {
            return Ok(serde_json::json!({
                "success": false,
                "message": "请先设置Cookie"
            }));
        }
    };

    let (videos, _, _) = match client.get_collected_videos_python_style(0, count).await {
        Ok(result) => result,
        Err(e) => {
            return Ok(serde_json::json!({
                "success": false,
                "message": format!("获取收藏列表失败: {}", e)
            }));
        }
    };

    if videos.is_empty() {
        return Ok(serde_json::json!({
            "success": false,
            "message": "没有找到收藏视频"
        }));
    }

    let batch_task_id = uuid::Uuid::new_v4().to_string();
    let total_videos = videos.len();
    let batch_task_id_clone = batch_task_id.clone();

    {
        let downloader_guard = state.downloader.lock().await;
        let downloader = match downloader_guard.as_ref() {
            Some(d) => d,
            None => {
                return Ok(serde_json::json!({
                    "success": false,
                    "message": "服务未完全初始化"
                }));
            }
        };

        let downloader_clone = downloader.clone();
        let videos_clone = videos.clone();

        tokio::spawn(async move {
            if let Err(e) = downloader_clone
                .start_batch_download(videos_clone, batch_task_id_clone, "收藏视频".to_string())
            {
                log::error!("Batch download error: {}", e);
            }
        });
    }

    Ok(serde_json::json!({
        "success": true,
        "task_id": batch_task_id,
        "message": format!("开始下载 {} 个收藏视频", total_videos),
        "total_videos": total_videos
    }))
}
```

注意：由于 `get_collected_videos_python_style` 返回 `Vec<LikedVideoItem>` 而非 `(Vec<VideoInfo>, i64, bool)`，批量下载需要先将 `LikedVideoItem` 转换为 `VideoInfo` 或直接使用 `download_video` 逐个下载。参照 `downloadAllLikedVideos` 的前端逐个下载模式，此处 `download_collected_videos` 命令仅返回视频列表，批量下载由前端逐个调用 `download_video` 完成。

因此，简化 `download_collected_videos` 为直接返回视频列表，不启动 batch download：

```rust
/// 下载收藏视频（批量下载入口）
#[tauri::command]
async fn download_collected_videos(
    state: State<'_, AppState>,
    count: u32,
) -> Result<serde_json::json!(), String> {
    let client = match get_client(&state).await {
        Ok(client) => client,
        Err(_) => {
            return Ok(serde_json::json!({
                "success": false,
                "message": "请先设置Cookie"
            }));
        }
    };

    match client.get_collected_videos_python_style(0, count).await {
        Ok(videos) if !videos.is_empty() => {
            let total = videos.len();
            Ok(serde_json::json!({
                "success": true,
                "data": videos,
                "total_videos": total,
                "message": format!("获取到 {} 个收藏视频，开始逐个下载", total)
            }))
        }
        Ok(_) => Ok(serde_json::json!({
            "success": false,
            "message": "没有找到收藏视频"
        })),
        Err(e) => Ok(serde_json::json!({
            "success": false,
            "message": format!("获取收藏列表失败: {}", e)
        })),
    }
}
```

- [ ] **Step 3: 在 `invoke_handler![]` 中注册新命令**

在 `lib.rs` 的 `invoke_handler![]` 宏中（约第 2015-2058 行），在 `get_liked_videos` 之后添加 `get_collected_videos`，在 `download_liked_videos` 之后添加 `download_collected_videos`：

```rust
            get_liked_videos,
            get_collected_videos,
            ...
            download_liked_videos,
            download_collected_videos,
```

- [ ] **Step 4: 编译验证**

```bash
cd src-tauri && cargo check
```
Expected: 编译通过

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/lib.rs
git commit -m "feat(commands): 添加 get_collected_videos / download_collected_videos Tauri 命令"
```

---

## Task 3: 前端 — index.html 添加收藏列表 UI

**Files:**
- Modify: `dist/index.html`

- [ ] **Step 1: 在"更多"下拉菜单中添加收藏获取入口**

在 `liked-authors-count` input 之后（约第 84 行），`</div>` (dropdown-section closing) 之前插入：

```html
                            <div class="dropdown-item-compact">
                                <span class="dropdown-item-label"><i class="bi bi-star-fill"></i> 收藏获取</span>
                                <input type="number" class="form-control form-control-sm count-input" id="collected-videos-count" value="20" min="1" max="100">
                                <button class="btn btn-sm btn-warning" id="download-collected-btn">获取</button>
                            </div>
```

- [ ] **Step 2: 添加收藏视频列表 Section**

在 `likedAuthorsSection` closing `</div>` 之后（约第 315 行），`<!-- My Downloads Section -->` 之前插入：

```html
            <!-- Collected videos (hidden by default) -->
            <div id="collectedVideosSection" class="mb-3" style="display: none;">
                <div class="section-panel">
                    <div class="section-panel-header section-panel-header--warning">
                        <div class="section-panel-title">
                            <i class="bi bi-star-fill"></i> 收藏视频列表
                        </div>
                        <div class="section-panel-actions">
                            <span id="collectedVideoCount" class="badge section-count-badge">0 个视频</span>
                            <button onclick="downloadAllCollectedVideos()" class="btn btn-outline-light btn-sm">
                                <i class="bi bi-download"></i> 下载全部
                            </button>
                            <button onclick="goBackToHome()" class="btn btn-outline-light btn-sm">
                                <i class="bi bi-x-lg"></i>
                            </button>
                        </div>
                    </div>
                    <div class="section-panel-body">
                        <div id="collectedVideosList" class="row"></div>
                    </div>
                </div>
            </div>
```

- [ ] **Step 3: 在空状态快捷入口中添加收藏卡片**

在"点赞视频"shortcut-card 之后（约第 144 行）插入：

```html
                    <div class="shortcut-card" onclick="document.getElementById('download-collected-btn').click()">
                        <i class="bi bi-star"></i>
                        <span>收藏视频</span>
                    </div>
```

- [ ] **Step 4: 更新 Cookie 引导文案**

将 `cookieOnboardingCopy` 段落中的"点赞列表"改为"点赞列表、收藏列表"：

```html
                        配置后可稳定使用搜索、推荐、点赞列表、收藏列表和批量下载；完成后再搜索用户或粘贴链接。
```

- [ ] **Step 5: Commit**

```bash
git add dist/index.html
git commit -m "feat(ui): 添加收藏列表 HTML 入口和 Section"
```

---

## Task 4: 前端 — tauri-adapter.js 添加收藏 API 映射

**Files:**
- Modify: `dist/js/tauri-adapter.js`

- [ ] **Step 1: 在 `/api/download_liked` 映射之后添加 `/api/get_collected_videos` 和 `/api/download_collected` 映射**

在 adapter 中找到 `/api/download_liked` 的映射块（约第 712-716 行），在其后插入：

```javascript
        if (path === '/api/get_collected_videos') {
            const result = await invoke('get_collected_videos', {
                cursor: 0,
                count: Number(body.count || params.count || 20)
            });
            const rawVideos = result && (result.data || result.videos);
            const videos = normalizeVideos(rawVideos);

            if (!Array.isArray(videos) || videos.length === 0) {
                return {
                    success: false,
                    message: (result && result.message) || '获取收藏视频失败。该接口需要登录态，请确认Cookie有效且包含完整的登录信息。如果Cookie已过期请重新获取。'
                };
            }

            return {
                success: true,
                data: videos,
                count: Number(result && result.count) || videos.length,
                cursor: Number(result && result.cursor) || 0,
                has_more: Boolean(result && result.has_more)
            };
        }

        if (path === '/api/download_collected') {
            return invoke('download_collected_videos', {
                count: Number(body.count || params.count || 20)
            });
        }
```

- [ ] **Step 2: Commit**

```bash
git add dist/js/tauri-adapter.js
git commit -m "feat(adapter): 添加收藏列表 API 映射"
```

---

## Task 5: 前端 — app.js 添加收藏列表事件处理

**Files:**
- Modify: `dist/js/app.js`

- [ ] **Step 1: 新增 `downloadCollectedVideos()` 函数**

在 `downloadLikedAuthors()` 函数之后（约第 1681 行），`displayLikedVideos()` 之前插入：

```javascript
async function downloadCollectedVideos() {
    try {
        setButtonLoading('download-collected-btn', true, '获取中');
        var count = document.getElementById('collected-videos-count').value || 20;
        var response = await fetch('/api/get_collected_videos', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ count: parseInt(count) }) });
        var result = await response.json();
        if (result.success) {
            isHomeView = false; hideAllSections();
            displayCollectedVideos(result.data);
            CollectedDataCache.saveCollectedVideos(result.data, result.data.length);
            CollectedDataCache.currentDisplayType = 'collected';
            showToast('获取到 ' + result.data.length + ' 个收藏视频', 'success');
        } else {
            showToast('获取收藏视频失败，请先登录抖音账号', 'error');
            addLog('收藏视频需要登录态，请点击设置 → 登录抖音账号', 'warning');
        }
    } catch (error) { showToast('获取收藏视频失败，请先登录抖音账号', 'error'); }
    finally { setButtonLoading('download-collected-btn', false); }
}
```

- [ ] **Step 2: 新增 `displayCollectedVideos()` 函数**

在 `displayLikedAuthors()` 函数之后（约第 1728 行）插入：

```javascript
function displayCollectedVideos(videos) {
    var section = document.getElementById('collectedVideosSection');
    var videosList = document.getElementById('collectedVideosList');
    videosList.innerHTML = '';
    document.getElementById('collectedVideoCount').textContent = videos.length + ' 个视频';
    window.currentCollectedVideos = videos;
    videos.forEach(function(v) { VideoStorage.saveVideo(v); });
    videos.forEach(function(video) {
        var vc = createVideoCardElement(video, {
            showAuthorButton: true
        });
        videosList.appendChild(vc);
    });
    revealSectionById('collectedVideosSection');
    _hideEmptyState();
}
```

- [ ] **Step 3: 新增 `handleCollectedVideosClick()` 和 `downloadAllCollectedVideos()` 函数**

在 `handleLikedAuthorsClick()` 之后（约第 1833 行）插入：

```javascript
async function handleCollectedVideosClick() {
    var s = document.getElementById('collectedVideosSection');
    if ((s && s.style.display === 'block') || CollectedDataCache.currentDisplayType === 'collected') { await downloadCollectedVideos(); return; }
    var cached = CollectedDataCache.getCollectedVideos();
    if (cached && cached.data && cached.data.length > 0) { hideAllSections(true); displayCollectedVideos(cached.data); CollectedDataCache.currentDisplayType = 'collected'; showToast('显示缓存的 ' + cached.data.length + ' 个收藏视频', 'info'); }
    else await downloadCollectedVideos();
}

async function downloadAllCollectedVideos() {
    if (!window.currentCollectedVideos || window.currentCollectedVideos.length === 0) { showToast('没有可下载的收藏视频', 'warning'); return; }
    var videos = window.currentCollectedVideos, total = videos.length;
    var batchId = 'batch_collected_videos_' + Date.now();
    createDownloadProgressElement(batchId, '批量下载收藏视频 (' + total + '个)');
    var ok = 0, fail = 0, done = 0;
    for (var i = 0; i < videos.length; i++) {
        var v = videos[i];
        var vd = { aweme_id: v.aweme_id, desc: v.desc || '收藏视频_' + (i + 1), media_urls: v.media_urls || [], raw_media_type: v.raw_media_type || v.media_type || 'video', author_name: v.author ? v.author.nickname : '未知作者' };
        vd.media_urls = normalizeMediaUrlsForDownload(vd.media_urls, vd.raw_media_type);
        if (!vd.media_urls.length) { fail++; done++; continue; }
        try {
            var r = await fetch('/api/download_single_video', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(vd) });
            var res = await r.json();
            res.success ? ok++ : fail++;
        } catch (e) { fail++; }
        done++;
        updateDownloadProgress(Math.round(done / total * 100), done, total, batchId);
        if (i < videos.length - 1) await new Promise(function(res) { setTimeout(res, 500); });
    }
    updateDownloadProgress(100, total, total, batchId);
    showToast('完成！成功: ' + ok + ', 失败: ' + fail, ok > 0 ? 'success' : 'warning');
    setTimeout(function() { removeProgressElement(batchId); }, 3000);
}
```

- [ ] **Step 4: 添加事件监听绑定**

在 `likedAuthorsBtn` 事件监听之后（约第 102 行）插入：

```javascript
    var collectedBtn = document.getElementById('download-collected-btn');
    if (collectedBtn) collectedBtn.addEventListener('click', function(e) {
        if (!checkLoginRequired(collectedBtn)) {
            e.preventDefault();
            return;
        }
        handleCollectedVideosClick();
    });
```

- [ ] **Step 5: 在 `hideAllSections()` 中确保收藏 section 也被隐藏**

检查 `dist/js/ui.js` 中的 `hideAllSections()` 函数，确保 `collectedVideosSection` 被包含在隐藏列表中。如果该函数使用通配符或列举方式，确认新 ID 被加入。

在 `hideAllSections()` 中加入：
```javascript
hideSectionById('collectedVideosSection');
```

- [ ] **Step 6: Commit**

```bash
git add dist/js/app.js dist/js/ui.js
git commit -m "feat(app): 添加收藏列表事件处理和批量下载逻辑"
```

---

## Task 6: 前端 — storage.js 添加收藏缓存

**Files:**
- Modify: `dist/js/storage.js`

- [ ] **Step 1: 在 `LikedDataCache` 之后新增 `CollectedDataCache`**

在 `storage.js` 第 412 行（`LikedDataCache` 的 closing `}`）之后插入：

```javascript
// ═══════════════════════════════════════════════
// COLLECTED DATA CACHE
// ═══════════════════════════════════════════════
const CollectedDataCache = {
    COLLECTED_VIDEOS_KEY: 'collected_videos_cache',
    CACHE_VERSION: 1,
    currentDisplayType: null,

    saveCollectedVideos: function(videos, count) {
        const timestamp = Date.now();
        const normalizedVideos = Array.isArray(videos)
            ? videos.map(video => Object.assign({}, video, { media_fetched_at: video.media_fetched_at || timestamp }))
            : [];
        const cacheData = { version: this.CACHE_VERSION, data: normalizedVideos, count: count, timestamp: timestamp };
        localStorage.setItem(this.COLLECTED_VIDEOS_KEY, JSON.stringify(cacheData));
        _log(`已缓存 ${normalizedVideos.length} 个收藏视频`);
    },

    getCollectedVideos: function() {
        try {
            const cached = localStorage.getItem(this.COLLECTED_VIDEOS_KEY);
            if (cached) {
                const cacheData = JSON.parse(cached);
                if (cacheData.version !== this.CACHE_VERSION) {
                    localStorage.removeItem(this.COLLECTED_VIDEOS_KEY);
                    _log('收藏视频缓存版本已过期，已自动清理');
                    return null;
                }
                if (VideoStorage.isMediaExpired(cacheData.timestamp)) {
                    localStorage.removeItem(this.COLLECTED_VIDEOS_KEY);
                    _log('收藏视频缓存中的媒体地址已过期，已自动清理');
                    return null;
                }
                _log(`从缓存获取到 ${cacheData.data.length} 个收藏视频`);
                return cacheData;
            }
        } catch (error) {
            console.error('获取收藏视频缓存失败:', error);
        }
        return null;
    },

    clearAll: function() {
        localStorage.removeItem(this.COLLECTED_VIDEOS_KEY);
        _log('已清除所有收藏数据缓存');
    },

    isCacheExpired: function(timestamp, maxAge) {
        maxAge = maxAge || (24 * 60 * 60 * 1000);
        return Date.now() - timestamp > maxAge;
    }
};
```

- [ ] **Step 2: Commit**

```bash
git add dist/js/storage.js
git commit -m "feat(storage): 添加 CollectedDataCache 收藏数据缓存"
```

---

## Task 7: 端到端验证

- [ ] **Step 1: 前端 JS 语法检查**

```bash
for file in dist/js/*.js; do node --check "$file"; done
```
Expected: 无输出（全部通过）

- [ ] **Step 2: Rust 编译 + clippy**

```bash
cd src-tauri && cargo clippy --all-targets --all-features -- -D warnings
```
Expected: 无 warning

- [ ] **Step 3: 运行测试**

```bash
cd src-tauri && cargo test
```
Expected: 全部通过

- [ ] **Step 4: 最终 commit（如果有修复）**

```bash
git add -A
git commit -m "fix: 修复收藏列表功能 review 发现的问题"
```

---

## 文件变更汇总

| 文件 | 变更类型 | 说明 |
|------|---------|------|
| `src-tauri/src/api/client.rs` | 修改 | 新增 `request_collected_videos_response()` + `get_collected_videos_python_style()` |
| `src-tauri/src/lib.rs` | 修改 | 新增 `get_collected_videos` + `download_collected_videos` 命令，注册到 `invoke_handler![]` |
| `dist/index.html` | 修改 | 下拉菜单入口 + section + 空状态快捷入口 + 文案 |
| `dist/js/tauri-adapter.js` | 修改 | `/api/get_collected_videos` + `/api/download_collected` 映射 |
| `dist/js/app.js` | 修改 | `downloadCollectedVideos()` + `displayCollectedVideos()` + `handleCollectedVideosClick()` + `downloadAllCollectedVideos()` + 事件监听 |
| `dist/js/ui.js` | 修改 | `hideAllSections()` 加入 `collectedVideosSection` |
| `dist/js/storage.js` | 修改 | 新增 `CollectedDataCache` |
