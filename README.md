<div align="center">

<img src="frontend/public/animated_icon.svg" width="120" height="120" alt="better-douyin-R Logo">

# better-douyin-R

更轻、更快的 Rust / Tauri 版抖音桌面工具。面向日常使用，覆盖内容搜索、链接解析、批量下载、沉浸式播放、推荐流预览和本地下载管理。

<p>
  <img src="https://img.shields.io/badge/Rust-1.80%2B-orange?style=flat-square&logo=rust" alt="Rust">
  <img src="https://img.shields.io/badge/Tauri-2-blue?style=flat-square&logo=tauri" alt="Tauri">
  <a href="https://github.com/anYuJia/better-douyin-R/releases/latest"><img src="https://img.shields.io/github/v/release/anYuJia/better-douyin-R?style=flat-square" alt="Release"></a>
  <a href="https://github.com/anYuJia/better-douyin-R/releases"><img src="https://img.shields.io/github/downloads/anYuJia/better-douyin-R/total?style=flat-square" alt="Downloads"></a>
  <img src="https://img.shields.io/badge/Platform-macOS%20%7C%20Windows%20%7C%20Linux-555?style=flat-square" alt="Platform">
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-MIT-2ea44f?style=flat-square" alt="License"></a>
</p>

[下载安装](#下载安装) · [功能能力](#功能能力) · [界面预览](#界面预览) · [首次使用](#首次使用) · [加入交流群](#加入交流群)

</div>

---

## 项目定位

better-douyin-R 是 better-douyin 的 Rust / Tauri 桌面版本，重点放在更轻的运行时、更稳定的本地播放、更顺滑的桌面体验和更可靠的跨平台分发。

如果你主要是日常使用，推荐使用本项目。如果你更关心 Python 源码、脚本改造或服务端集成，可以查看 [better-douyin](https://github.com/anYuJia/better-douyin)。

## 功能能力

- 搜索抖音用户，查看主页作品、收藏、点赞等内容
- 粘贴分享链接解析单条作品，支持视频、图集和部分 Live Photo 内容
- 批量下载用户作品、搜索结果、推荐流、收藏列表和点赞列表
- 沉浸式播放器支持多媒体左右切换、进度控制、音量控制、倍速、清晰度切换和失败重试
- 播放器支持自动播放下一条作品，可在播放界面快速开关
- 支持下载视频原声 / BGM，并写入下载任务和下载记录
- 推荐流预览支持滚轮切换、快速播放和一键下载
- 侧边栏支持展开 / 收缩，窄栏模式只保留图标，适合小屏和长期停靠
- “我的下载”支持任务进度、文件视图、作品视图、搜索、播放、定位和删除
- Cookie、配置、下载历史和本地文件均保存在本机

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

从 [Releases](https://github.com/anYuJia/better-douyin-R/releases/latest) 下载对应平台文件。

| 平台 | 推荐下载 |
|:---|:---|
| Windows 安装版 | `better-douyin-R-v*-windows-x64-installer.exe` |
| Windows 便携版 | `better-douyin-R-v*-windows-x64-portable.zip` |
| macOS Apple Silicon / M 系列 | `better-douyin-R-v*-macos-arm64.dmg` |
| macOS Intel | `better-douyin-R-v*-macos-x64.dmg` |
| Linux Debian / Ubuntu | `better-douyin-R-v*-linux-x64.deb` |
| Linux Fedora / openSUSE / RHEL | `better-douyin-R-v*-linux-x64.rpm` |
| Linux 通用便携 | `better-douyin-R-v*-linux-x64.AppImage` |

`*-updater.*`、`.sig`、`latest.json`、`windows.json`、`darwin.json`、`linux.json` 主要用于自动更新和签名校验，普通用户通常不需要手动下载。

macOS 首次运行如提示无法验证开发者：

```bash
sudo xattr -rd com.apple.quarantine /Applications/better-douyin-R.app
```

## 首次使用

1. 打开设置，配置 Cookie 和下载目录。
2. 通过内置登录、浏览器读取或手动粘贴完成登录态配置。
3. 使用搜索用户、解析链接、推荐视频、收藏视频或点赞列表获取内容。
4. 下载单个作品，或在列表中执行批量下载。
5. 在底部任务面板查看进度，在“我的下载”管理本地文件。

## CLI 工具

`douyin-dl` 适合脚本调用或服务器环境：

```bash
douyin-dl parse "https://www.douyin.com/video/..."
douyin-dl download "https://v.douyin.com/..."
douyin-dl search "用户昵称"
douyin-dl config show
douyin-dl config set cookie "your-cookie"
```

## 源码运行

需要 Rust、Node.js 和 Tauri 系统依赖。依赖安装参考 [Tauri 官方文档](https://tauri.app/start/prerequisites/)。

```bash
git clone https://github.com/anYuJia/better-douyin-R.git
cd better-douyin-R
npm --prefix frontend install
npm --prefix frontend run tauri -- dev
```

构建发布版：

```bash
npm --prefix frontend run tauri -- build
```

常用检查：

```bash
npm run build
cd src-tauri && cargo check
```

## Cookie 与隐私

- Cookie 仅用于本机请求抖音相关接口，不会上传到本项目服务器
- 下载历史、应用配置、账号信息和缓存数据保存在本机应用数据目录
- 推荐、收藏、点赞、评论和部分批量能力依赖有效 Cookie
- 如果接口异常，优先检查 Cookie、账号验证状态、网络环境和目标内容权限

## 常见问题

### 为什么有些功能需要登录？

推荐、收藏、点赞、评论和部分批量下载能力依赖有效 Cookie。未登录时接口可能拒绝访问或返回不完整数据。

### 下载慢或失败怎么办？

下载速度受网络、资源可用性、平台响应和 Cookie 状态影响。可以刷新 Cookie、降低并发、切换网络，或稍后重试。

### 播放器加载失败怎么办？

先点击重试。若仍失败，通常是播放地址过期、Cookie 失效、平台拒绝访问或网络暂时不可用。

### 自动更新失败怎么办？

自动更新依赖 GitHub Release。若网络无法访问 GitHub，可手动打开 Releases 页面下载对应安装包覆盖安装。

## 已知限制

- 对登录态和 Cookie 有依赖
- 抖音 Web 接口可能随平台策略变化而失效
- 某些平台首次运行需要额外系统权限或安全确认
- 当前主要面向本地桌面使用，不是云服务方案

## 加入交流群

欢迎加入 QQ 群交流使用体验、问题反馈与功能建议。

<p align="center">
  <img src="docs/community/qq-group.jpg" width="220" alt="QQ 群二维码">
  <br>
  <strong>QQ群：438407379</strong>
</p>

## 反馈与贡献

- 发现问题：欢迎提交 [Issue](https://github.com/anYuJia/better-douyin-R/issues)
- 想改进功能：欢迎发起 Pull Request
- 包管理器分发模板见 [packaging/package-managers](packaging/package-managers)

## 免责声明

本项目仅供个人学习、研究和内容备份使用。请遵守相关法律法规、平台规则和内容版权要求，不得用于商业采集或大规模爬取。因不当使用造成的后果由使用者自行承担。

## Star History

<p align="center">
  <a href="https://star-history.com/#anYuJia/better-douyin-R&Date">
    <img src="https://api.star-history.com/svg?repos=anYuJia/better-douyin-R&type=Date" width="100%" alt="better-douyin-R Star History Chart">
  </a>
</p>

---

<p align="center">如果这个项目对你有帮助，欢迎 Star 支持。</p>
