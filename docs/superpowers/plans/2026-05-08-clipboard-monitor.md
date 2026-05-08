# 剪切板监听功能实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 为 Douyin Downloader 添加剪切板监听功能，自动检测抖音分享链接并弹出确认框，用户确认后自动填充链接并开始解析下载。

**架构:** 前端 `setInterval` 每 1.5 秒轮询 → 调用后端 `read_clipboard` 命令读取系统剪切板 → 正则匹配抖音链接 → 去重后弹出 `confirm()` 确认框 → 确认后填充输入框并触发 `downloadFromLink()`。

**Tech Stack:** tauri-plugin-clipboard-manager, tauri-plugin-dialog (已有), 原生 JS confirm()

---

## Task 1: 后端 — 添加剪切板插件依赖和命令

**Files:**
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/tauri.conf.json`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: 添加 clipboard-manager 依赖**

在 `Cargo.toml` 中 `tauri-plugin-updater` 之后添加：
```toml
tauri-plugin-clipboard-manager = "2"
```

- [ ] **Step 2: 在 tauri.conf.json 中注册插件**

在 `plugins` 中添加：
```json
"clipboard-manager": {},
```

- [ ] **Step 3: 在 lib.rs 中新增 `read_clipboard` 命令**

在 `get_current_user` 之后、下载 API 之前插入：
```rust
#[tauri::command]
async fn read_clipboard(
    state: State<'_, AppState>,
) -> Result<String, String> {
    let app_handle = state.app_handle.lock().await.clone()
        .ok_or_else(|| "AppHandle 未初始化".to_string())?;
    let clipboard = app_handle.clipboard();
    match clipboard.read_text() {
        Ok(Some(text)) => Ok(text),
        Ok(None) => Ok(String::new()),
        Err(e) => {
            log::warn!("读取剪切板失败: {}", e);
            Ok(String::new())
        }
    }
}
```

- [ ] **Step 4: 在 `invoke_handler![]` 中注册**

在 `get_current_user` 之后添加 `read_clipboard`。

- [ ] **Step 5: 在 Tauri builder 中初始化插件**

在 `.plugin(tauri_plugin_fs::init())` 之后添加：
```rust
.plugin(tauri_plugin_clipboard_manager::init())
```

- [ ] **Step 6: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/tauri.conf.json src-tauri/src/lib.rs
git commit -m "feat(clipboard): 添加剪切板插件和 read_clipboard 命令"
```

---

## Task 2: 前端 — 添加剪切板监听逻辑

**Files:**
- Modify: `dist/js/tauri-adapter.js`
- Modify: `dist/js/app.js`

- [ ] **Step 1: 在 tauri-adapter.js 中添加 `/api/read_clipboard` 映射**

在 `console.warn('[Tauri Adapter] Unmatched API')` 之前添加：
```javascript
if (path === '/api/read_clipboard') {
    return invoke('read_clipboard');
}
```

- [ ] **Step 2: 在 app.js 中添加剪切板监听函数**

在 paste handler 之后、keyboard shortcuts 之前插入：
```javascript
var lastClipboardUrl = null;
var douyinUrlPattern = /https?:\/\/(?:www\.)?(?:v\.douyin\.com\/[a-zA-Z0-9]+\/?|douyin\.com\/(?:video|note)\/[0-9]+)/i;

function extractDouyinUrl(text) {
    if (!text) return null;
    var match = text.match(douyinUrlPattern);
    return match ? match[0] : null;
}

function isDouyinUrl(text) {
    return douyinUrlPattern.test(text || '');
}

async function checkDouyinClipboard() {
    try {
        var text = await fetch('/api/read_clipboard').then(function (r) { return r.text(); });
        if (!text || !isDouyinUrl(text)) return;
        var url = extractDouyinUrl(text);
        if (!url || url === lastClipboardUrl) return;
        lastClipboardUrl = url;
        var confirmed = confirm('检测到抖音链接，是否下载？\n\n' + url);
        if (confirmed) {
            document.getElementById('link-input').value = url;
            downloadFromLink();
        }
    } catch (e) {}
}
```

- [ ] **Step 3: 在 DOMContentLoaded 中启动轮询**

在 `setTimeout` (推荐预加载) 之后添加：
```javascript
setInterval(checkDouyinClipboard, 1500);
```

- [ ] **Step 4: JS 语法检查**

```bash
for file in dist/js/*.js; do node --check "$file"; done
```
Expected: 无输出

- [ ] **Step 5: Commit**

```bash
git add dist/js/tauri-adapter.js dist/js/app.js
git commit -m "feat(clipboard): 前端剪切板轮询检测和自动下载"
```

---

## Task 3: 验证

- [ ] **Step 1: JS 语法检查**

```bash
for file in dist/js/*.js; do node --check "$file" && echo "✓ $file" || echo "✗ $file"; done
```
Expected: 全部通过

- [ ] **Step 2: 检查 Cargo.toml 格式**

手动确认 `tauri-plugin-clipboard-manager = "2"` 已正确添加

---

## 文件变更汇总

| 文件 | 变更类型 | 说明 |
|------|---------|------|
| `src-tauri/Cargo.toml` | 修改 | 添加 clipboard-manager 依赖 |
| `src-tauri/tauri.conf.json` | 修改 | 注册 clipboard-manager 插件 |
| `src-tauri/src/lib.rs` | 修改 | 新增 `read_clipboard` 命令 + 注册 + 初始化插件 |
| `dist/js/tauri-adapter.js` | 修改 | `/api/read_clipboard` 映射 |
| `dist/js/app.js` | 修改 | 剪切板监听函数 + 轮询启动 |
