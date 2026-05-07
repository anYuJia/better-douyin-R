import { useState, useRef, useEffect } from "react";
import { useAppStore } from "@/stores/app-store";
import { motion } from "framer-motion";
import { Search, Link2, CornerDownLeft, Clock, ArrowUpRight, X } from "lucide-react";
import { cn } from "@/lib/utils";

const recentSearches = ["张同学", "疯狂小杨哥", "李子柒"];

export function CommandPopover() {
  const setCommandOpen = useAppStore((s) => s.setCommandOpen);
  const commandMode = useAppStore((s) => s.commandMode);
  const setView = useAppStore((s) => s.setView);
  const setCommandMode = useAppStore((s) => s.setCommandMode);
  const [value, setValue] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    const timer = setTimeout(() => inputRef.current?.focus(), 50);
    const handleKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setCommandOpen(false);
    };
    window.addEventListener("keydown", handleKey);
    return () => {
      clearTimeout(timer);
      window.removeEventListener("keydown", handleKey);
    };
  }, [setCommandOpen]);

  const handleSubmit = () => {
    if (!value.trim()) return;
    setView("search");
    setCommandOpen(false);
  };

  const handleRecentClick = (text: string) => {
    setValue(text);
    inputRef.current?.focus();
  };

  const isSearch = commandMode === "search";
  const Icon = isSearch ? Search : Link2;

  return (
    <>
      {/* Backdrop */}
      <motion.div
        initial={{ opacity: 0 }}
        animate={{ opacity: 1 }}
        exit={{ opacity: 0 }}
        transition={{ duration: 0.12 }}
        className="fixed inset-0 z-[1080] bg-black/50"
        onClick={() => setCommandOpen(false)}
      />

      {/* Command Panel */}
      <motion.div
        initial={{ opacity: 0, y: -10, scale: 0.98 }}
        animate={{ opacity: 1, y: 0, scale: 1 }}
        exit={{ opacity: 0, y: -6, scale: 0.98 }}
        transition={{ type: "spring", stiffness: 500, damping: 32 }}
        className={cn(
          "fixed z-[1090] flex flex-col overflow-hidden",
          "left-[calc(var(--sidebar-width)+50%-200px)] top-[20vh]",
          "w-[400px] max-w-[calc(100vw-var(--sidebar-width)-48px)]",
          "rounded-[var(--radius-xl)]",
          "border border-border-strong",
          "bg-surface-solid",
          "shadow-[0_40px_100px_rgba(0,0,0,0.4),0_0_0_1px_rgba(255,255,255,0.03)]"
        )}
      >
        {/* Input */}
        <div className="flex items-center gap-3 px-4 h-[50px] border-b border-border">
          <Icon className="w-[17px] h-[17px] text-text-muted shrink-0" />
          <input
            ref={inputRef}
            value={value}
            onChange={(e) => setValue(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && handleSubmit()}
            placeholder={isSearch ? "搜索用户名或抖音号..." : "粘贴抖音分享链接..."}
            className="flex-1 bg-transparent text-[0.88rem] text-text placeholder:text-text-muted outline-none"
          />
          <button
            onClick={() => setCommandOpen(false)}
            className="w-6 h-6 rounded flex items-center justify-center text-text-muted hover:text-text cursor-pointer transition-colors"
          >
            <X className="w-3.5 h-3.5" />
          </button>
        </div>

        {/* Mode Tabs */}
        <div className="flex items-center gap-1 px-3 py-2 border-b border-border">
          {([
            { mode: "search" as const, icon: Search, label: "搜索用户" },
            { mode: "link" as const, icon: Link2, label: "解析链接" },
          ]).map(({ mode, icon: TabIcon, label }) => (
            <button
              key={mode}
              onClick={() => {
                setCommandMode(mode);
                setValue("");
                setTimeout(() => inputRef.current?.focus(), 30);
              }}
              className={cn(
                "flex items-center gap-1.5 px-2.5 py-1.5 rounded-[8px] text-[0.75rem] font-medium cursor-pointer transition-all duration-[var(--duration-fast)]",
                commandMode === mode
                  ? "bg-accent/10 text-accent"
                  : "text-text-muted hover:text-text-secondary hover:bg-surface-raised"
              )}
            >
              <TabIcon className="w-3 h-3" />
              {label}
            </button>
          ))}

          <div className="ml-auto flex items-center gap-2 text-text-muted">
            <span className="flex items-center gap-1">
              <kbd className="inline-flex items-center justify-center min-w-[18px] h-[18px] px-1 text-[0.58rem] font-mono rounded bg-surface-raised border border-border">
                ↵
              </kbd>
              <span className="text-[0.62rem]">执行</span>
            </span>
            <span className="flex items-center gap-1">
              <kbd className="inline-flex items-center justify-center min-w-[24px] h-[18px] px-1 text-[0.58rem] font-mono rounded bg-surface-raised border border-border">
                esc
              </kbd>
              <span className="text-[0.62rem]">关闭</span>
            </span>
          </div>
        </div>

        {/* Content Area */}
        <div className="max-h-[260px] overflow-y-auto">
          {/* Search mode: recent */}
          {isSearch && !value && (
            <div className="px-3 py-2.5">
              <div className="flex items-center gap-1.5 px-2 mb-2">
                <Clock className="w-3 h-3 text-text-muted" />
                <span className="text-[0.65rem] font-bold text-text-muted uppercase tracking-[0.06em]">
                  最近搜索
                </span>
              </div>
              <div className="flex flex-col gap-0.5">
                {recentSearches.map((item) => (
                  <button
                    key={item}
                    onClick={() => handleRecentClick(item)}
                    className="flex items-center gap-2.5 w-full px-2.5 py-2 rounded-[10px] text-left text-[0.82rem] text-text-secondary hover:text-text hover:bg-surface-raised cursor-pointer transition-all group"
                  >
                    <Search className="w-3.5 h-3.5 text-text-muted group-hover:text-accent transition-colors" />
                    <span className="flex-1">{item}</span>
                    <ArrowUpRight className="w-3 h-3 text-text-muted opacity-0 group-hover:opacity-60 transition-opacity" />
                  </button>
                ))}
              </div>
            </div>
          )}

          {/* Link mode: empty state */}
          {!isSearch && !value && (
            <div className="px-4 py-8 text-center">
              <div className="w-11 h-11 rounded-[12px] bg-surface border border-border flex items-center justify-center mx-auto mb-3">
                <Link2 className="w-5 h-5 text-text-muted" />
              </div>
              <p className="text-[0.82rem] font-medium text-text-secondary mb-1">粘贴抖音分享链接</p>
              <p className="text-[0.72rem] text-text-muted">支持分享链接、短链接和完整 URL</p>
            </div>
          )}

          {/* Input active: preview action */}
          {value && (
            <div className="px-3 py-2.5">
              <button
                onClick={handleSubmit}
                className="flex items-center gap-3 w-full px-3 py-3 rounded-[12px] text-left bg-accent/[0.06] hover:bg-accent/[0.1] border border-accent/10 cursor-pointer transition-all group"
              >
                <div className="w-9 h-9 rounded-[10px] bg-accent/12 flex items-center justify-center shrink-0">
                  <Icon className="w-[18px] h-[18px] text-accent" />
                </div>
                <div className="flex-1 min-w-0">
                  <div className="text-[0.82rem] font-medium text-text truncate">{value}</div>
                  <div className="text-[0.68rem] text-text-muted">{isSearch ? "搜索用户" : "解析链接"}</div>
                </div>
                <CornerDownLeft className="w-3.5 h-3.5 text-accent/50 shrink-0" />
              </button>
            </div>
          )}
        </div>
      </motion.div>
    </>
  );
}
