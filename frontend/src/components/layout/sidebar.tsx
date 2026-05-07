import { useAppStore, useDownloadStore } from "@/stores/app-store";
import type { ViewType } from "@/types";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import {
  Home,
  Search,
  Link2,
  Sparkles,
  FolderOpen,
  Heart,
  Settings,
  Circle,
  Play,
} from "lucide-react";
import { motion } from "framer-motion";
import { cn } from "@/lib/utils";

interface NavItem {
  id: ViewType;
  label: string;
  icon: React.ElementType;
  command?: "search" | "link";
}

const navItems: NavItem[] = [
  { id: "home", label: "首页", icon: Home },
  { id: "search", label: "搜索用户", icon: Search, command: "search" },
  { id: "home", label: "粘贴链接", icon: Link2, command: "link" },
  { id: "recommended", label: "推荐视频", icon: Sparkles },
  { id: "downloads", label: "我的下载", icon: FolderOpen },
  { id: "liked", label: "点赞视频", icon: Heart },
];

const containerVariants = {
  hidden: {},
  show: {
    transition: { staggerChildren: 0.04, delayChildren: 0.1 },
  },
};

const itemVariants = {
  hidden: { opacity: 0, x: -8 },
  show: { opacity: 1, x: 0, transition: { type: "spring" as const, stiffness: 400, damping: 28 } },
};

export function Sidebar() {
  const currentView = useAppStore((s) => s.currentView);
  const setView = useAppStore((s) => s.setView);
  const setCommandOpen = useAppStore((s) => s.setCommandOpen);
  const setCommandMode = useAppStore((s) => s.setCommandMode);
  const setSettingsOpen = useAppStore((s) => s.setSettingsOpen);
  const activeCount = useDownloadStore((s) => s.activeCount);

  const handleNavClick = (item: NavItem) => {
    if (item.command) {
      setCommandMode(item.command);
      setCommandOpen(true);
    } else {
      setView(item.id);
    }
  };

  return (
    <aside className="flex flex-col w-[var(--sidebar-width)] h-full border-r border-border bg-gradient-to-b from-surface-solid/94 to-background-soft/86 shrink-0">
      {/* Brand */}
      <motion.div
        className="flex items-center gap-3 px-5 py-5"
        initial={{ opacity: 0 }}
        animate={{ opacity: 1 }}
        transition={{ duration: 0.3 }}
      >
        <div className="w-10 h-10 rounded-[14px] bg-gradient-to-br from-accent to-accent-hover flex items-center justify-center shadow-[0_8px_24px_oklch(62%_0.22_11/0.3)]">
          <Play className="w-4 h-4 text-white fill-white translate-x-[1px]" />
        </div>
        <div className="flex flex-col min-w-0">
          <span className="text-[0.9rem] font-[780] tracking-tight text-text truncate">
            Douyin Downloader
          </span>
          <span className="text-[0.7rem] font-semibold text-text-muted tracking-wide">
            本地媒体工作台
          </span>
        </div>
      </motion.div>

      {/* Navigation */}
      <motion.nav
        className="flex-1 flex flex-col gap-1 px-3 overflow-y-auto"
        variants={containerVariants}
        initial="hidden"
        animate="show"
      >
        <div className="text-[0.68rem] font-bold text-text-muted uppercase tracking-[0.08em] px-2 mb-2">
          导航
        </div>

        {navItems.map((item) => {
          const isActive = !item.command && currentView === item.id;
          const Icon = item.icon;

          return (
            <motion.button
              key={item.label}
              variants={itemVariants}
              onClick={() => handleNavClick(item)}
              className={cn(
                "group relative flex items-center gap-3 h-[42px] px-3 rounded-[14px] text-left transition-all duration-[var(--duration-fast)] ease-[var(--ease-spring)] cursor-pointer",
                isActive
                  ? "bg-accent-soft text-accent shadow-[inset_0_0_0_1px_var(--color-accent-ring)]"
                  : "text-text-muted hover:text-text hover:bg-surface-raised"
              )}
              whileHover={{ scale: 1.01 }}
              whileTap={{ scale: 0.98 }}
            >
              {isActive && (
                <motion.div
                  layoutId="sidebar-active"
                  className="absolute left-0 top-1/2 -translate-y-1/2 w-[3px] h-5 rounded-r-full bg-accent"
                  transition={{ type: "spring", stiffness: 400, damping: 30 }}
                />
              )}

              <Icon className="w-[18px] h-[18px] shrink-0" />
              <span className="text-[0.8125rem] font-semibold truncate">{item.label}</span>

              {item.id === "downloads" && activeCount > 0 && (
                <Badge variant="default" size="sm" className="ml-auto">
                  {activeCount}
                </Badge>
              )}
            </motion.button>
          );
        })}
      </motion.nav>

      {/* Status — outside nav so it's always visible */}
      <div className="px-3 py-2">
        <div className="flex items-center gap-2 px-3 h-9 rounded-[14px] bg-surface/50 text-text-muted">
          <Circle className="w-2 h-2 fill-success text-success" />
          <span className="text-[0.72rem] font-medium">就绪</span>
        </div>
      </div>

      {/* Footer */}
      <div className="px-3 py-3 border-t border-border">
        <Button
          variant="ghost"
          size="sm"
          className="w-full justify-start gap-2 h-9 rounded-[14px] text-text-muted hover:text-text"
          onClick={() => setSettingsOpen(true)}
        >
          <Settings className="w-[16px] h-[16px]" />
          <span className="text-[0.8125rem] font-semibold">设置</span>
        </Button>
      </div>
    </aside>
  );
}
