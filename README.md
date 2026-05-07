<div align="center">

<img src="src-tauri/icons/icon.png" width="128" height="128" alt="Douyin Downloader Logo">

# Douyin Downloader

**抖音视频下载器 · Rust / Tauri 桌面重构版**

[![Rust](https://img.shields.io/badge/Rust-1.77.2+-orange.svg?style=flat-square&logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![Tauri](https://img.shields.io/badge/Tauri-2.0-blue.svg?style=flat-square&logo=tauri&logoColor=white)](https://tauri.app/)
[![CI](https://img.shields.io/github/actions/workflow/status/anYuJia/douyin-downloader-rust/ci.yml?branch=main&style=flat-square&label=CI)](https://github.com/anYuJia/douyin-downloader-rust/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/anYuJia/douyin-downloader-rust?style=flat-square)](https://github.com/anYuJia/douyin-downloader-rust/releases)
[![License](https://img.shields.io/badge/License-MIT-green.svg?style=flat-square)](LICENSE)
[![Platform](https://img.shields.io/badge/Platform-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey.svg?style=flat-square)](https://github.com/anYuJia/douyin-downloader-rust/releases/latest)

基于 Rust + Tauri 2.0 的跨平台桌面版抖音下载工具，支持用户检索、批量下载、推荐视频浏览、点赞列表获取、多媒体作品下载与实时下载进度。

<p>
  <a href="https://github.com/anYuJia/douyin-downloader-rust/releases/latest"><strong>下载最新版</strong></a>
  ·
  <a href="#界面预览"><strong>界面预览</strong></a>
  ·
  <a href="#快速开始"><strong>快速开始</strong></a>
  ·
  <a href="#从源码构建"><strong>源码构建</strong></a>
  ·
  <a href="#常见问题"><strong>常见问题</strong></a>
</p>

</div>

---

## 项目简介

Douyin Downloader 是 [DY_video_downloader](https://github.com/anYuJia/DY_video_downloader) 的 Rust / Tauri 重构版本，目标是把原先偏脚本化的下载流程整理成更轻量、稳定、易分发的桌面应用。

相比 Python 打包版，当前版本更适合作为长期维护的桌面工具：

- **更轻量**：原生桌面应用，无需 Python 运行时
- **更易分发**：支持 Windows / macOS / Linux 安装包与便携包
- **更集中**：搜索、推荐、点赞、批量下载、历史管理集中在一个界面
- **更清晰**：登录态、下载任务、文件位置和错误状态都有可见反馈

---

## 功能亮点

| 能力 | 说明 |
|:---|:---|
| 用户检索 | 支持昵称、抖音号、分享链接搜索用户 |
| 批量下载 | 支持下载用户作品、点赞作品、作者列表作品 |
| 推荐视频 | 支持推荐 feed 浏览与沉浸式播放器预览 |
| 点赞列表 | 支持获取并浏览自己的点赞视频与点赞作者 |
| 多媒体作品 | 支持视频、图集、Live Photo、混合媒体 |
| 下载质量 | 支持最高质量、兼容优先、最小体积等策略 |
| 实时进度 | 下载任务状态、当前进度、日志实时更新 |
| 浏览器登录 | 内置登录窗口，用于获取可用 Cookie |
| 本地管理 | 支持下载历史、文件搜索、批量打开和定位 |
| 自动更新 | 基于 GitHub Release updater metadata 检查新版本 |

---

## 界面预览

### 首页

搜索用户、粘贴链接、进入推荐视频和我的下载入口。

<p align="center">
  <a href="docs/home.png">
    <img src="docs/home.png" width="100%" alt="Douyin Downloader 首页">
  </a>
</p>

### 用户详情

查看用户资料、作品列表，并执行批量下载或单个作品下载。

<p align="center">
  <a href="docs/user_detail.png">
    <img src="docs/user_detail.png" width="100%" alt="Douyin Downloader 用户详情">
  </a>
</p>

### 播放器

推荐视频和作品详情使用沉浸式预览，弱网场景会显示更明确的加载、重试和错误提示。

<p align="center">
  <a href="docs/playvideo.png">
    <img src="docs/playvideo.png" width="100%" alt="Douyin Downloader 播放器">
  </a>
</p>

---

## 快速开始

### 下载安装

从 [Releases](https://github.com/anYuJia/douyin-downloader-rust/releases/latest) 下载对应平台的安装包：

| 平台 | 推荐文件 | 说明 |
|:---|:---|:---|
| Windows | `Douyin.Downloader_*_x64-setup.exe` | 常规安装版，适合长期使用 |
| Windows | `Douyin-Downloader_*_x64_portable.exe` | 便携版，不需要安装 |
| macOS Apple Silicon | `Douyin.Downloader_*_aarch64.dmg` / `*_macos-arm64_portable.zip` | M1/M2/M3/M4 等芯片 |
| macOS Intel | `Douyin.Downloader_*_x64.dmg` / `*_macos-x64_portable.zip` | Intel 芯片 |
| Linux Debian/Ubuntu | `Douyin.Downloader_*_amd64.deb` | 适合 Debian、Ubuntu、Linux Mint 等 |
| Linux Fedora/openSUSE/RHEL | `Douyin.Downloader-*-1.x86_64.rpm` | 适合 RPM 系发行版 |
| Linux 通用 | `Douyin.Downloader_*_amd64.AppImage` | 免安装便携运行 |

`.sig`、`latest.json`、`windows.json`、`darwin.json`、`linux.json` 主要用于自动更新和签名校验，普通安装通常不需要手动下载。

### 首次使用

1. 打开应用后，先在设置中完成 Cookie / 登录配置
2. 使用搜索、推荐视频、点赞列表或粘贴链接解析内容
3. 选择单个作品下载，或进入用户/点赞列表执行批量下载
4. 在底部下载面板查看实时进度，在“我的下载”中管理本地文件

> **macOS 用户**
>
> 首次运行若提示“无法验证开发者”，可执行：
>
> ```bash
> sudo xattr -rd com.apple.quarantine /Applications/Douyin\ Downloader.app
> ```

---

## Cookie、数据与隐私

- Cookie 仅用于本地请求抖音相关接口，不会上传到本项目的服务器
- 下载历史、应用配置和缓存数据保存在本机应用数据目录
- 下载文件默认保存在设置中配置的下载目录
- 推荐视频、点赞列表和部分批量下载能力依赖有效登录态
- 如果接口突然失效，优先检查 Cookie 是否过期、账号是否需要重新验证、网络是否可访问相关域名

---

## 包管理器分发

项目已准备 Homebrew Cask、Scoop 和 winget 的分发模板，见 [packaging/package-managers](packaging/package-managers)。

当前这些模板用于维护和提交包管理器清单。正式进入对应包管理器仓库后，将可以通过命令行安装和更新。

---

## 从源码构建

### 环境要求

- Rust 1.77.2+
- Node.js 18+（可选，用于前端静态检查和分发脚本）
- 系统依赖见 [Tauri 官方文档](https://tauri.app/start/prerequisites/)

### 开发模式运行

```bash
git clone https://github.com/anYuJia/douyin-downloader-rust.git
cd douyin-downloader-rust

cd src-tauri
cargo tauri dev
```

### 构建发布版

```bash
cd src-tauri
cargo tauri build
```

### 本地检查

```bash
cd src-tauri
cargo fmt --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

前端构建检查：

```bash
cd frontend
npm run build
```

---

## 技术栈

- **桌面框架**：Tauri 2
- **后端**：Rust、Tokio、Reqwest、Axum
- **前端**：React 19、Vite、TypeScript、Tailwind CSS
- **更新机制**：Tauri updater + GitHub Release metadata
- **分发产物**：Windows NSIS / portable exe、macOS dmg / app zip、Linux deb / rpm / AppImage

---

## 常见问题

### 为什么有些功能需要登录？

推荐视频、点赞列表、部分批量下载能力依赖有效 Cookie / 登录态。未登录时，接口可能拒绝访问或返回不完整数据。

### 可以只下载单个视频吗？

可以。除了批量下载，也支持通过粘贴链接解析后进行单个下载。

### 下载文件保存到哪里？

下载目录可以在设置中修改。历史记录和“我的下载”页面也支持直接打开文件或定位到文件夹。

### 推荐视频接口为什么有时不稳定？

推荐流、详情、点赞等接口都可能受到平台风控、Cookie 状态和网络环境影响。这类现象属于预期范围。

### 播放器提示加载失败怎么办？

先确认网络连接是否稳定，再点击播放器中的“重试”。如果仍失败，通常是播放地址过期、Cookie 失效、平台拒绝或本地媒体代理暂时无法取得资源。可以刷新详情、重新登录，或稍后再试。

### 自动更新失败怎么办？

自动更新依赖 GitHub Release。若当前网络无法访问 GitHub，可手动打开 Releases 页面下载对应平台的新版本安装包。

### 为什么头像或封面偶尔显示默认图？

头像、封面由平台接口返回。接口未返回、图片过期或网络异常时，应用会显示默认占位图，不影响下载功能。

---

## 已知限制

- 对登录态和 Cookie 有依赖
- 接口可能随抖音策略变化而失效或返回结构变化
- 某些平台首次运行需要额外系统权限或安全确认
- 当前仍以桌面端本地使用为主，不是云服务方案
- 移动端暂未产品化；如果后续尝试，会优先验证 Android WebView 登录和 Cookie 获取方案

---

## 贡献与反馈

- 发现问题：欢迎提交 [Issue](https://github.com/anYuJia/douyin-downloader-rust/issues)
- 想改进功能：欢迎发起 Pull Request
- 发布与包管理器分发相关脚本见 [scripts](scripts) 和 [packaging/package-managers](packaging/package-managers)

---

## 相关项目

- [DY_video_downloader](https://github.com/anYuJia/DY_video_downloader) - Python 原版

---

## License

本项目基于 [MIT License](LICENSE) 开源。

---

## 免责声明

本工具仅供个人学习研究使用，请勿用于商业用途或大规模爬取。因滥用导致的后果，项目贡献者不承担责任。

---

## Star History

<a href="https://star-history.com/#anYuJia/douyin-downloader-rust&Date">
  <img src="https://api.star-history.com/svg?repos=anYuJia/douyin-downloader-rust&type=Date" width="100%" alt="douyin-downloader-rust Star History Chart">
</a>

<p align="center">
  <a href="https://star-history.com/#anYuJia/douyin-downloader-rust&Date">https://star-history.com/#anYuJia/douyin-downloader-rust&Date</a>
</p>

---

<p align="center">觉得有用？给个 Star 支持一下</p>
