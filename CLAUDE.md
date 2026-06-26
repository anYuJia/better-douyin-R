# CLAUDE.md

本文件用于约束 AI（Claude Code 等）在本仓库的工作行为。

## Git 提交规范

### 提交信息格式

- 使用 conventional commits 前缀（`fix:`、`feat:`、`chore:`、`refactor:`、`docs:`、`style:`、`test:`、`perf:`），**前缀后面的描述用中文**。
- 示例：
  - `fix(player): 修复评论回复更新时标志泄漏导致的不必要重渲染`
  - `feat(ui): 新增下载进度全局同步`
  - `chore: 升级依赖版本`

### 提交信息正文

- 正文可选，使用中文，解释「为什么」而非「做了什么」。
- 不要添加 `Co-Authored-By: Claude` 或类似的 AI 署名行。
- 不要添加 `🤖 Generated with Claude Code` 之类的标记。

### 示例

```
fix(player): 修复评论回复更新时标志泄漏

updateCommentById 复用单个 changed 标志遍历所有回复容器，导致任一
回复匹配后，后续所有容器都被浅拷贝，即使内容未变也会触发不必要的
重渲染。改为按 cid 独立判断，仅浅拷贝实际变更的容器。
```

## 构建产物

- `dist/` 是 `npm run build` 生成的产物（gitignored），**不要提交到 git**。
- 修改前端后如需运行 release 构建，本地执行 `cd frontend && npm run build` 重新生成即可。

## Tauri 开发

- 启动开发模式：`./frontend/node_modules/.bin/tauri dev`（在仓库根目录执行）
- 发布构建：`./frontend/node_modules/.bin/tauri build`
- Tauri CLI 已作为 `@tauri-apps/cli` 安装在 `frontend/node_modules`，无需全局安装 `cargo-tauri`。

## 分支与推送

- 默认在 `main` 分支工作，提交后直接 `git push`。
- 不要使用 `--no-verify` 跳过 hook，不要 `--force` 推送到 main。
