import { useAppStore } from "@/stores/app-store";
import { motion } from "framer-motion";
import {
  Search,
  Link2,
  Sparkles,
  FolderOpen,
  ArrowRight,
  Command,
} from "lucide-react";
import { cn } from "@/lib/utils";

interface Shortcut {
  icon: React.ElementType;
  label: string;
  desc: string;
  iconBg: string;
  iconColor: string;
  view?: string;
  command?: "search" | "link";
  kbd?: string;
}

const shortcuts: Shortcut[] = [
  {
    icon: Search,
    label: "搜索用户",
    desc: "通过用户名或抖音号查找创作者",
    iconBg: "bg-accent/12",
    iconColor: "text-accent",
    command: "search",
    kbd: "⌘K",
  },
  {
    icon: Link2,
    label: "粘贴链接",
    desc: "解析分享链接，一键下载视频",
    iconBg: "bg-info/12",
    iconColor: "text-info",
    command: "link",
    kbd: "⌘L",
  },
  {
    icon: Sparkles,
    label: "推荐视频",
    desc: "浏览抖音推荐流内容",
    iconBg: "bg-purple-500/12",
    iconColor: "text-purple-400",
    view: "recommended",
  },
  {
    icon: FolderOpen,
    label: "我的下载",
    desc: "管理已下载的视频和图片",
    iconBg: "bg-success/12",
    iconColor: "text-success",
    view: "downloads",
    kbd: "⌘4",
  },
];

const container = {
  hidden: {},
  show: { transition: { staggerChildren: 0.07, delayChildren: 0.05 } },
};

const item = {
  hidden: { opacity: 0, y: 8 },
  show: { opacity: 1, y: 0, transition: { type: "spring" as const, stiffness: 400, damping: 28 } },
};

export function Hero() {
  const setView = useAppStore((s) => s.setView);
  const setCommandOpen = useAppStore((s) => s.setCommandOpen);
  const setCommandMode = useAppStore((s) => s.setCommandMode);

  const handleShortcut = (s: Shortcut) => {
    if (s.command) {
      setCommandMode(s.command);
      setCommandOpen(true);
    } else if (s.view) {
      setView(s.view as "recommended" | "downloads");
    }
  };

  return (
    <div className="flex items-center justify-center h-full px-8">
      <motion.div
        className="w-full max-w-[520px] flex flex-col items-center"
        variants={container}
        initial="hidden"
        animate="show"
      >
        {/* Status pill */}
        <motion.div variants={item} className="mb-5">
          <span className="inline-flex items-center gap-2 px-3 h-6 rounded-full bg-surface border border-border text-[0.68rem] font-semibold text-text-muted">
            <span className="w-1.5 h-1.5 rounded-full bg-success" />
            Ready
          </span>
        </motion.div>

        {/* Title — small, editorial */}
        <motion.h1
          variants={item}
          className="text-[1.15rem] font-[650] tracking-[-0.01em] text-text text-center mb-1.5"
        >
          从左侧导航开始
        </motion.h1>

        {/* Subtitle */}
        <motion.p
          variants={item}
          className="text-[0.8rem] text-text-muted text-center mb-8"
        >
          搜索用户或粘贴链接，解析预览一站完成
        </motion.p>

        {/* Shortcut Grid — 2x2, prominent cards */}
        <motion.div
          variants={container}
          className="w-full grid grid-cols-2 gap-2.5"
        >
          {shortcuts.map((s) => (
            <motion.button
              key={s.label}
              variants={item}
              onClick={() => handleShortcut(s)}
              className={cn(
                "group relative flex flex-col gap-3 p-5 rounded-[var(--radius-xl)] text-left cursor-pointer",
                "border border-border bg-surface-solid/60",
                "hover:border-border-strong hover:bg-surface-raised hover:shadow-[0_12px_40px_rgba(0,0,0,0.15)]",
                "transition-all duration-[var(--duration-base)] ease-[var(--ease-spring)]"
              )}
              whileHover={{ y: -3 }}
              whileTap={{ scale: 0.97 }}
            >
              {/* Icon + kbd row */}
              <div className="flex items-center justify-between w-full">
                <div className={cn(
                  "w-11 h-11 rounded-[12px] flex items-center justify-center",
                  s.iconBg
                )}>
                  <s.icon className={cn("w-[22px] h-[22px]", s.iconColor)} />
                </div>
                {s.kbd && (
                  <kbd className="text-[0.58rem] font-mono px-1.5 py-0.5 rounded bg-surface border border-border text-text-muted">
                    {s.kbd}
                  </kbd>
                )}
              </div>

              {/* Text */}
              <div>
                <div className="text-[0.88rem] font-semibold text-text mb-0.5">{s.label}</div>
                <div className="text-[0.72rem] text-text-muted leading-snug">{s.desc}</div>
              </div>

              {/* Arrow on hover */}
              <ArrowRight className="absolute bottom-5 right-5 w-4 h-4 text-text-muted opacity-0 group-hover:opacity-100 transition-opacity" />
            </motion.button>
          ))}
        </motion.div>

        {/* Keyboard hint */}
        <motion.div
          variants={item}
          className="mt-8 flex items-center gap-1.5 text-text-muted"
        >
          <kbd className="text-[0.58rem] font-mono px-1.5 py-0.5 rounded bg-surface border border-border">
            <Command className="w-2.5 h-2.5 inline" />
          </kbd>
          <span className="text-[0.65rem]">+</span>
          <kbd className="text-[0.58rem] font-mono px-1.5 py-0.5 rounded bg-surface border border-border">K</kbd>
          <span className="text-[0.65rem] ml-1">快速打开命令面板</span>
        </motion.div>
      </motion.div>
    </div>
  );
}
