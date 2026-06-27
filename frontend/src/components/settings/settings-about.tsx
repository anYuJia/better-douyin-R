import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { openExternalUrl } from "@/lib/tauri";
import {
  Info,
  RefreshCw,
  Users,
} from "lucide-react";
import type { UpdateInfo, UpdateStatus } from "@/stores/app-store";
import { SettingGroup } from "./settings-components";

function formatBytes(bytes?: number) {
  if (!bytes || bytes <= 0) return "";
  const units = ["B", "KB", "MB", "GB"];
  let value = bytes;
  let unitIndex = 0;
  while (value >= 1024 && unitIndex < units.length - 1) {
    value /= 1024;
    unitIndex++;
  }
  return `${value.toFixed(unitIndex === 0 ? 0 : 1)} ${units[unitIndex]}`;
}

interface SettingsAboutTabProps {
  appVersion: string;
  updateStatus: UpdateStatus;
  updateMessage: string;
  updateInfo: UpdateInfo | null;
  updateProgress: number;
  updateCanRestart: boolean;
  handleCheckUpdate: () => void;
  handleDownloadUpdate: () => void;
  handleRestart: () => void;
}

export function SettingsAboutTab({
  appVersion,
  updateStatus,
  updateMessage,
  updateInfo,
  updateProgress,
  updateCanRestart,
  handleCheckUpdate,
  handleDownloadUpdate,
  handleRestart,
}: SettingsAboutTabProps) {
  return (
    <div className="space-y-4">
      <SettingGroup icon={Info} label="关于">
        <div className="space-y-3">
          <div className="flex items-center justify-between py-2 px-3 rounded-[8px] bg-white/[0.02] border border-white/[0.04]">
            <span className="text-[0.78rem] text-text-muted">当前版本</span>
            <span className="text-[0.78rem] text-text font-mono font-semibold">
              {appVersion ? `v${appVersion}` : "读取中"}
            </span>
          </div>

          {updateMessage && (
            <div
              className={cn(
                "rounded-[8px] border px-3 py-1.5 text-[0.72rem]",
                updateStatus === "error"
                  ? "border-white/[0.06] bg-danger-soft text-danger"
                  : updateStatus === "available"
                    ? "border-info/20 bg-info/10 text-info"
                    : updateStatus === "ready"
                      ? "border-success/20 bg-success-soft text-success"
                      : "border-border bg-white/[0.02] text-text-muted"
              )}
            >
              {updateMessage}
            </div>
          )}

          {updateInfo?.notes && (
            <div className="max-h-[140px] overflow-y-auto rounded-[8px] border border-border bg-white/[0.01] p-2.5 text-[0.7rem] leading-relaxed text-text-secondary whitespace-pre-wrap font-mono">
              {updateInfo.notes}
            </div>
          )}

          {updateInfo?.asset_name && updateStatus === "available" && (
            <div className="flex items-center justify-between gap-3 rounded-[8px] border border-border bg-white/[0.02] px-3 py-1.5 text-[0.7rem] text-text-muted">
              <span className="min-w-0 truncate">{updateInfo.asset_name}</span>
              {formatBytes(updateInfo.asset_size) && (
                <span className="shrink-0 font-mono">{formatBytes(updateInfo.asset_size)}</span>
              )}
            </div>
          )}

          {updateStatus === "downloading" && (
            <div className="rounded-[8px] bg-white/[0.02] border border-white/[0.04] p-3">
              <div className="mb-1 flex items-center justify-between text-[0.68rem] text-text-muted">
                <span>正在下载更新文件</span>
                <span className="font-mono">{Math.round(updateProgress)}%</span>
              </div>
              <div className="h-1.5 overflow-hidden rounded-full bg-white/[0.08]">
                <div className="h-full rounded-full bg-accent transition-[width]" style={{ width: `${updateProgress}%` }} />
              </div>
            </div>
          )}

          <div className="flex gap-2">
            <Button
              variant="outline"
              onClick={handleCheckUpdate}
              disabled={updateStatus === "checking" || updateStatus === "downloading"}
              className="flex-1 h-9 rounded-[8px] text-[0.76rem] gap-1 cursor-pointer"
            >
              <RefreshCw className={cn("w-3.5 h-3.5", updateStatus === "checking" && "animate-spin")} />
              {updateStatus === "checking" ? "检查中" : "检查新版本"}
            </Button>

            {updateStatus === "available" && (
              <Button
                variant="default"
                onClick={handleDownloadUpdate}
                className="flex-1 h-9 rounded-[8px] text-[0.76rem] gap-1 cursor-pointer"
              >
                <RefreshCw className="w-3.5 h-3.5" />
                下载安装更新
              </Button>
            )}

            {updateStatus === "ready" && updateCanRestart && (
              <Button
                variant="default"
                onClick={handleRestart}
                className="flex-1 h-9 rounded-[8px] text-[0.76rem] gap-1 cursor-pointer"
              >
                <RefreshCw className="w-3.5 h-3.5" />
                立即重启升级
              </Button>
            )}
          </div>
        </div>
      </SettingGroup>

      <SettingGroup icon={Users} label="交流与支持">
        <div className="space-y-4">
          {/* GitHub Star Card */}
          <div className="flex flex-col sm:flex-row items-start sm:items-center justify-between gap-4 p-4 rounded-[12px] bg-white/[0.02] border border-white/[0.04] backdrop-blur-xl">
            <div className="flex items-start gap-3">
              <div className="w-10 h-10 rounded-[12px] bg-white/[0.04] flex items-center justify-center shrink-0 border border-white/[0.05]">
                <svg className="w-5.5 h-5.5 text-text" viewBox="0 0 24 24" fill="currentColor">
                  <path fillRule="evenodd" clipRule="evenodd" d="M12 2C6.477 2 2 6.484 2 12.017c0 4.425 2.865 8.18 6.839 9.504.5.092.682-.217.682-.483 0-.237-.008-.868-.013-1.703-2.782.605-3.369-1.343-3.369-1.343-.454-1.158-1.11-1.466-1.11-1.466-.908-.62.069-.608.069-.608 1.003.07 1.531 1.032 1.531 1.032.892 1.53 2.341 1.088 2.91.832.092-.647.35-1.088.636-1.338-2.22-.253-4.555-1.113-4.555-4.951 0-1.093.39-1.988 1.029-2.688-.103-.253-.446-1.272.098-2.65 0 0 .84-.27 2.75 1.026A9.564 9.564 0 0112 6.844c.85.004 1.705.115 2.504.337 1.909-1.296 2.747-1.027 2.747-1.027.546 1.379.202 2.398.1 2.651.64.7 1.028 1.595 1.028 2.688 0 3.848-2.339 4.695-4.566 4.943.359.309.678.92.678 1.855 0 1.338-.012 2.419-.012 2.747 0 .268.18.58.688.482C19.138 20.193 22 16.44 22 12.017 22 6.484 17.522 2 12 2z" />
                </svg>
              </div>
              <div>
                <div className="text-[0.82rem] font-bold text-text">喜欢这个项目吗？</div>
                <div className="text-[0.72rem] text-text-secondary mt-0.5 leading-relaxed">
                  这个项目花费了作者大量的时间和精力。如果它帮到了你，欢迎去 GitHub 点个 Star 支持作者继续走下去！
                </div>
              </div>
            </div>
            <button
              type="button"
              onClick={() => void openExternalUrl("https://github.com/anYuJia/better-douyin-R")}
              className="w-full sm:w-auto shrink-0 group relative flex h-8.5 items-center justify-center rounded-[8px] bg-accent px-4 text-[0.74rem] font-black text-white shadow-lg shadow-accent/20 active:scale-[0.96] transition-[background-color,color,box-shadow,transform,opacity]"
            >
              <span className="relative z-10">去 GitHub 点 Star</span>
              <div className="absolute inset-0 rounded-[8px] bg-white opacity-0 group-hover:opacity-10 transition-opacity" />
            </button>
          </div>

          {/* QQ Group Card */}
          <div className="flex flex-col md:flex-row items-center gap-5 p-4 rounded-[12px] bg-white/[0.02] border border-white/[0.04] backdrop-blur-xl">
            <div className="flex-1 min-w-0">
              <div className="text-[0.82rem] font-bold text-text">官方交流群</div>
              <div className="text-[0.72rem] text-text-secondary mt-1.5 leading-relaxed">
                欢迎加入官方交流群，与其他用户以及作者一起交流、反馈建议或分享使用心得。
              </div>
              <div className="mt-3.5 flex flex-col gap-2">
                <div className="flex items-center justify-between py-2 px-3 rounded-[8px] bg-white/[0.01] border border-white/[0.03]">
                  <span className="text-[0.74rem] text-text-muted">QQ 群号</span>
                  <span className="text-[0.78rem] text-text font-mono font-bold select-all">438407379</span>
                </div>
              </div>
            </div>
            <div className="w-[180px] rounded-[10px] bg-white p-1 flex items-center justify-center shrink-0 shadow-lg border border-white/10">
              <img
                 src="/qq-group.jpg"
                 alt="QQ群二维码"
                 className="w-full h-auto object-contain rounded-[6px]"
              />
            </div>
          </div>
        </div>
      </SettingGroup>
    </div>
  );
}
