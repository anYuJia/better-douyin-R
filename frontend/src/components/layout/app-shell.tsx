import { useLayoutEffect, useRef } from "react";
import { useAppStore } from "@/stores/app-store";
import { Sidebar } from "./sidebar";
import { BottomBar } from "./bottom-bar";
import { CommandPopover } from "./command-popover";
import { Hero } from "@/components/home/hero";
import { SearchView } from "@/components/search/search-view";
import { VideoGrid } from "@/components/search/video-grid";
import { UserDetail } from "@/components/search/user-detail";
import { LinkView } from "@/components/link/link-view";
import { RecommendedFeed } from "@/components/recommended/feed";
import { DownloadsView } from "@/components/downloads/downloads-view";
import { SettingsView } from "@/components/settings/settings-view";
import { LikedView } from "@/components/liked/liked-view";
import { CollectedView } from "@/components/collected/collected-view";
import { FriendsStatusView } from "@/components/friends/friends-status-view";
import { AnimatePresence, motion } from "framer-motion";
import { easeConfig } from "@/lib/utils";

const TAURI_DRAG_HEIGHT = 36;

function isInteractiveElement(target: EventTarget | null) {
  if (!(target instanceof Element)) return false;
  return Boolean(target.closest("button, a, input, textarea, select, [role='button'], [data-no-window-drag]"));
}

async function startWindowDrag() {
  if (typeof window === "undefined" || !(window as any).__TAURI_INTERNALS__) return;

  try {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    await getCurrentWindow().startDragging();
  } catch {
    // Dragging is best-effort and only exists in the desktop shell.
  }
}

export function AppShell() {
  const currentView = useAppStore((s) => s.currentView);
  const commandOpen = useAppStore((s) => s.commandOpen);
  const scrollRef = useRef<HTMLDivElement>(null);

  useLayoutEffect(() => {
    scrollRef.current?.scrollTo({ top: 0, behavior: "auto" });
  }, [currentView]);

  const handleWindowDrag = (event: React.PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0 || event.clientY > TAURI_DRAG_HEIGHT || isInteractiveElement(event.target)) return;
    void startWindowDrag();
  };

  return (
    <div className="relative flex h-screen w-screen overflow-hidden" onPointerDownCapture={handleWindowDrag}>
      {/* Sidebar */}
      <Sidebar />

      {/* Main Content Area */}
      <main className="relative flex min-w-0 flex-1 flex-col">
        <div ref={scrollRef} className="flex-1 overflow-x-hidden overflow-y-auto">
          <AnimatePresence initial={false} mode="wait">
            {currentView !== "friends-status" ? renderView(currentView) : null}
          </AnimatePresence>
          <div className={currentView === "friends-status" ? "box-border h-full min-h-0 p-4" : "hidden"}>
            <FriendsStatusView />
          </div>
        </div>
        <BottomBar />
      </main>

      {/* Command Popover (Raycast-style) */}
      <AnimatePresence>
        {commandOpen && <CommandPopover />}
      </AnimatePresence>
    </div>
  );
}

function renderView(view: string) {
  const variants = {
    initial: { opacity: 0, y: 8, scale: 0.985 },
    animate: { opacity: 1, y: 0, scale: 1 },
    exit: { opacity: 0, y: -4, scale: 0.99 },
  };

  const transition = {
    duration: 0.22,
    ease: easeConfig,
    opacity: { duration: 0.15 },
  };

  switch (view) {
    case "home":
      return (
        <motion.div key="home" {...variants} transition={transition} className="h-full">
          <Hero />
        </motion.div>
      );
    case "search":
      return (
        <motion.div key="search" {...variants} transition={transition} className="p-6">
          <SearchView />
        </motion.div>
      );
    case "user":
      return (
        <motion.div key="user" {...variants} transition={transition} className="p-6">
          <UserDetail />
          <VideoGrid />
        </motion.div>
      );
    case "link":
      return (
        <motion.div key="link" {...variants} transition={transition} className="p-6">
          <LinkView />
        </motion.div>
      );
    case "recommended":
      return (
        <motion.div key="recommended" {...variants} transition={transition} className="p-6">
          <RecommendedFeed />
        </motion.div>
      );
    case "downloads":
      return (
        <motion.div key="downloads" {...variants} transition={transition} className="p-6">
          <DownloadsView />
        </motion.div>
      );
    case "liked":
      return (
        <motion.div key="liked" {...variants} transition={transition} className="p-6">
          <LikedView />
        </motion.div>
      );
    case "collected":
      return (
        <motion.div key="collected" {...variants} transition={transition} className="p-6">
          <CollectedView />
        </motion.div>
      );
    case "settings":
      return (
        <motion.div key="settings" {...variants} transition={transition}>
          <SettingsView />
        </motion.div>
      );
    default:
      return (
        <motion.div key="home" {...variants} transition={transition} className="h-full">
          <Hero />
        </motion.div>
      );
  }
}
