import { useState } from "react";
import { useAppStore } from "@/stores/app-store";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { Select, SelectTrigger, SelectValue, SelectContent, SelectItem } from "@/components/ui/select";
import { ScrollArea } from "@/components/ui/scroll-area";
import { motion, AnimatePresence } from "framer-motion";
import {
  X,
  Palette,
  Key,
  FolderOpen,
  Zap,
  Gauge,
  Info,
  RefreshCw,
  Sun,
  Moon,
  Monitor,
} from "lucide-react";
import { cn } from "@/lib/utils";
import type { ThemeMode } from "@/types";

export function SettingsSheet() {
  const settingsOpen = useAppStore((s) => s.settingsOpen);
  const setSettingsOpen = useAppStore((s) => s.setSettingsOpen);
  const theme = useAppStore((s) => s.theme);
  const setTheme = useAppStore((s) => s.setTheme);

  return (
    <AnimatePresence>
      {settingsOpen && (
        <>
          {/* Overlay */}
          <motion.div
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={{ duration: 0.2 }}
            className="fixed inset-0 z-[1060] bg-black/50 backdrop-blur-sm"
            onClick={() => setSettingsOpen(false)}
          />

          {/* Drawer */}
          <motion.div
            initial={{ x: "100%" }}
            animate={{ x: 0 }}
            exit={{ x: "100%" }}
            transition={{ type: "spring", stiffness: 350, damping: 30 }}
            className="fixed top-0 right-0 bottom-0 w-[340px] max-w-[90vw] z-[1070] bg-surface-solid border-l border-border flex flex-col shadow-[-8px_0_40px_rgba(0,0,0,0.4)]"
          >
            {/* Header */}
            <div className="flex items-center justify-between px-5 py-4 border-b border-border">
              <h6 className="text-[0.95rem] font-semibold text-text">设置</h6>
              <button
                onClick={() => setSettingsOpen(false)}
                className="w-8 h-8 rounded-[var(--radius-sm)] flex items-center justify-center text-text-muted hover:text-text hover:bg-surface-raised transition-all cursor-pointer"
              >
                <X className="w-4 h-4" />
              </button>
            </div>

            {/* Body */}
            <ScrollArea className="flex-1">
              <div className="p-5 flex flex-col gap-5">
                {/* Theme */}
                <SettingGroup icon={Palette} label="外观主题">
                  <div className="flex gap-1 p-0.5 rounded-[var(--radius-sm)] bg-surface border border-border">
                    {([
                      { value: "light", icon: Sun, label: "亮色" },
                      { value: "dark", icon: Moon, label: "暗色" },
                      { value: "auto", icon: Monitor, label: "系统" },
                    ] as const).map(({ value, icon: Icon, label }) => (
                      <button
                        key={value}
                        onClick={() => setTheme(value as ThemeMode)}
                        className={cn(
                          "flex-1 flex items-center justify-center gap-1.5 h-8 rounded-[6px] text-[0.8rem] font-medium transition-all duration-[var(--duration-fast)] cursor-pointer",
                          theme === value
                            ? "bg-accent text-white shadow-sm"
                            : "text-text-muted hover:text-text-secondary hover:bg-surface-raised"
                        )}
                      >
                        <Icon className="w-3.5 h-3.5" />
                        {label}
                      </button>
                    ))}
                  </div>
                </SettingGroup>

                {/* Cookie */}
                <SettingGroup icon={Key} label="Cookie">
                  <Button variant="default" className="w-full h-9 rounded-[var(--radius-sm)]">
                    登录抖音账号
                  </Button>
                  <Textarea placeholder="粘贴抖音 Cookie" rows={4} className="mt-2" />
                </SettingGroup>

                {/* Download Dir */}
                <SettingGroup icon={FolderOpen} label="下载目录">
                  <div className="flex gap-2">
                    <Input placeholder="data/" className="flex-1 h-9" />
                    <Button variant="outline" size="sm" className="h-9 shrink-0">
                      <FolderOpen className="w-3.5 h-3.5" />
                      选择
                    </Button>
                  </div>
                  <p className="text-[0.75rem] text-text-muted mt-1">默认：data/</p>
                </SettingGroup>

                {/* Quality */}
                <SettingGroup icon={Gauge} label="下载质量">
                  <Select defaultValue="auto">
                    <SelectTrigger className="h-9">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="auto">自动</SelectItem>
                      <SelectItem value="highest">最高质量</SelectItem>
                      <SelectItem value="h264">兼容优先 (H.264)</SelectItem>
                      <SelectItem value="smallest">最小体积</SelectItem>
                    </SelectContent>
                  </Select>
                </SettingGroup>

                {/* Concurrency */}
                <SettingGroup icon={Zap} label="并发下载数">
                  <Select defaultValue="3">
                    <SelectTrigger className="h-9">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {[1, 2, 3, 4, 5, 6, 8, 10].map((n) => (
                        <SelectItem key={n} value={String(n)}>
                          {n} 个{n === 3 ? " (推荐)" : ""}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </SettingGroup>

                {/* Save Button */}
                <Button variant="default" className="w-full h-9 rounded-[var(--radius-sm)]">
                  保存设置
                </Button>

                <hr className="border-border" />

                {/* About */}
                <SettingGroup icon={Info} label="关于">
                  <div className="flex items-center justify-between mb-2">
                    <span className="text-[0.8rem] text-text-muted">版本</span>
                    <span className="text-[0.8rem] text-text font-mono">0.0.10</span>
                  </div>
                  <Button variant="outline" className="w-full h-9 rounded-[var(--radius-sm)]">
                    <RefreshCw className="w-3.5 h-3.5" />
                    检查更新
                  </Button>
                </SettingGroup>
              </div>
            </ScrollArea>
          </motion.div>
        </>
      )}
    </AnimatePresence>
  );
}

function SettingGroup({
  icon: Icon,
  label,
  children,
}: {
  icon: React.ElementType;
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div>
      <label className="flex items-center gap-2 text-[0.8rem] font-medium text-text-secondary mb-2">
        <Icon className="w-3.5 h-3.5" />
        {label}
      </label>
      {children}
    </div>
  );
}
