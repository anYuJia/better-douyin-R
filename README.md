<div align="center">

<img src="frontend/public/animated_icon.svg" width="120" height="120" alt="Douyin Downloader Logo">

# Douyin Downloader

跨平台桌面抖音下载工具，支持用户搜索、链接解析、批量下载、推荐流预览、本地播放和下载管理。

<p>
  <img src="https://img.shields.io/badge/Rust-1.77%2B-orange?style=flat-square&logo=rust" alt="Rust">
  <img src="https://img.shields.io/badge/Tauri-2-blue?style=flat-square&logo=tauri" alt="Tauri">
  <img src="https://img.shields.io/badge/Platform-macOS%20%7C%20Windows%20%7C%20Linux-555?style=flat-square" alt="Platform">
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-MIT-2ea44f?style=flat-square" alt="License"></a>
</p>

[下载安装](#下载安装) · [界面预览](#界面预览) · [首次使用](#首次使用) · [常见问题](#常见问题)

</div>

---

## 项目选择

当前有两个版本：

| 版本 | 适合人群 |
|:---|:---|
| **Rust / Tauri 版** | 推荐日常桌面使用，体积更小，启动和本地播放体验更好 |
| **Python 版** | 适合想直接改源码，或更熟悉 Python 生态的用户 |

Python 版见：[DY_video_downloader](https://github.com/anYuJia/DY_video_downloader)。

## 主要功能

- 搜索抖音用户，查看用户主页、作品、收藏、点赞等内容
- 粘贴分享链接解析单条作品，并支持直接下载
- 批量下载视频、图集和部分 Live Photo 内容
- 推荐视频流预览，支持沉浸式播放、滚轮切换和一键下载
- 本地播放器支持拖动进度、重试和更清晰的加载提示
- “我的下载”支持文件模式/作品模式、搜索、播放、定位和删除
- 自动识别已下载作品，避免重复下载
- Cookie 支持内置登录、浏览器读取和手动粘贴
- 数据、Cookie 和下载文件均保存在本机

## 界面预览

<p align="center">
  <a href="docs/home.jpg"><img src="docs/preview/home.jpg" width="100%" alt="首页"></a>
  <br>
  <strong>首页 / 主界面</strong>
</p>

<p align="center">
  <a href="docs/get_user.jpg"><img src="docs/preview/get_user.jpg" width="100%" alt="搜索用户"></a>
  <br>
  <strong>搜索用户</strong>
</p>

<p align="center">
  <a href="docs/user_detail.jpg"><img src="docs/preview/user_detail.jpg" width="100%" alt="用户主页"></a>
  <br>
  <strong>用户主页 / 批量下载</strong>
</p>

<p align="center">
  <a href="docs/recommend.jpg"><img src="docs/preview/recommend.jpg" width="100%" alt="推荐视频流"></a>
  <br>
  <strong>推荐视频流</strong>
</p>

<p align="center">
  <a href="docs/playvideo.jpg"><img src="docs/preview/playvideo.jpg" width="100%" alt="沉浸式播放器"></a>
  <br>
  <strong>沉浸式播放器</strong>
</p>

## 下载安装

从 [Releases](https://github.com/anYuJia/douyin-downloader-rust/releases/latest) 下载对应平台的安装包。

| 平台 | 推荐文件 | 说明 |
|:---|:---|:---|
| Windows | `*_x64-setup.exe` | 常规安装版，适合长期使用 |
| Windows | `*_x64_portable.exe` | 便携版，不需要安装 |
| macOS Apple Silicon | `*_aarch64.dmg` 或 `*_macos-arm64_portable.zip` | M1/M2/M3/M4 等芯片 |
| macOS Intel | `*_x64.dmg` 或 `*_macos-x64_portable.zip` | Intel 芯片 |
| Linux Debian/Ubuntu | `*_amd64.deb` | 适合 Debian、Ubuntu、Linux Mint 等 |
| Linux Fedora/openSUSE/RHEL | `*.x86_64.rpm` | 适合 RPM 系发行版 |
| Linux 通用 | `*_amd64.AppImage` | 免安装便携运行 |

`.sig`、`latest.json`、`windows.json`、`darwin.json`、`linux.json` 主要用于自动更新和签名校验，普通用户通常不需要手动下载。

macOS 首次运行如果提示“无法验证开发者”，可执行：

```bash
sudo xattr -rd com.apple.quarantine /Applications/Douyin\ Downloader.app
```

## 首次使用

1. 打开应用，在设置中配置 Cookie 和下载目录。
2. 通过内置登录、浏览器 Cookie 读取或手动粘贴完成登录态配置。
3. 使用“搜索用户”“解析链接”“推荐视频”“收藏视频”或“点赞列表”获取内容。
4. 选择单个作品下载，或进入用户主页、收藏、点赞列表进行批量下载。
5. 在底部任务面板查看进度，在“我的下载”中管理本地文件。

## Cookie、数据与隐私

- Cookie 只用于本机请求抖音相关接口，不会上传到本项目服务器
- 下载历史、应用配置和缓存数据保存在本机应用数据目录
- 下载文件默认保存在设置中配置的下载目录
- 推荐、收藏、点赞和部分批量能力依赖有效 Cookie
- 如果接口突然不可用，优先检查 Cookie 是否过期、账号是否需要重新验证、网络是否可访问抖音相关域名

## CLI 工具

项目也提供 `douyin-dl` 命令行工具，适合脚本调用或服务器环境。

常用命令：

```bash
douyin-dl parse "https://www.douyin.com/video/..."
douyin-dl download "https://v.douyin.com/..."
douyin-dl search "用户昵称"
douyin-dl config show
douyin-dl config set cookie "your-cookie"
```

如果只是普通桌面使用，可以忽略这一节。

## 从源码运行

需要 Rust、Node.js 和 Tauri 所需系统依赖。系统依赖可参考 [Tauri 官方文档](https://tauri.app/start/prerequisites/)。

```bash
git clone https://github.com/anYuJia/douyin-downloader-rust.git
cd douyin-downloader-rust/src-tauri
cargo tauri dev
```

构建发布版：

```bash
cargo tauri build
```

## 常见问题

### 为什么有些功能需要登录？

推荐视频、收藏视频、点赞列表和部分批量下载能力依赖有效 Cookie。未登录时，接口可能拒绝访问或返回不完整数据。

### 可以只下载单个视频吗？

可以。粘贴分享链接解析后即可下载单个作品。

### 下载文件保存到哪里？

下载目录可以在设置中修改。“我的下载”支持文件模式/作品模式、搜索、播放、定位文件夹和删除本地文件。

### 播放器加载失败怎么办？

先点击播放器中的“重试”。如果仍失败，通常是播放地址过期、Cookie 失效、平台拒绝或网络暂时不可用。可以刷新详情、重新登录，或稍后再试。

### 自动更新失败怎么办？

自动更新依赖 GitHub Release。若当前网络无法访问 GitHub，可手动打开 Releases 页面下载新版本安装包。

## 已知限制

- 对登录态和 Cookie 有依赖
- 抖音 Web 接口可能随平台策略变化而失效
- 某些平台首次运行需要额外系统权限或安全确认
- 当前主要面向本地桌面使用，不是云服务方案

## 反馈与贡献

- 发现问题：欢迎提交 [Issue](https://github.com/anYuJia/douyin-downloader-rust/issues)
- 想改进功能：欢迎发起 Pull Request
- 包管理器分发模板见 [packaging/package-managers](packaging/package-managers)

## 免责声明

本项目仅供个人学习、研究和内容备份使用。请遵守相关法律法规、平台规则和内容版权要求，不得用于商业采集或大规模爬取。因不当使用造成的后果由使用者自行承担。

## Star History

<p align="center">
  <a href="https://star-history.com/#anYuJia/douyin-downloader-rust&Date">
    <img src="https://api.star-history.com/svg?repos=anYuJia/douyin-downloader-rust&type=Date" width="100%" alt="douyin-downloader-rust Star History Chart">
  </a>
</p>

---

<p align="center">如果这个项目对你有帮助，欢迎 Star 支持。</p>
