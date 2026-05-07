import { useState, useCallback } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { useDownloadStore } from "@/stores/app-store";
import { TaskCard } from "./task-card";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Badge } from "@/components/ui/badge";
import { ScrollArea } from "@/components/ui/scroll-area";
import {
  Select,
  SelectTrigger,
  SelectValue,
  SelectContent,
  SelectItem,
} from "@/components/ui/select";
import {
  FolderOpen,
  Search,
  RefreshCw,
  Trash2,
  CheckSquare,
  Square,
  Play,
  Folder,
} from "lucide-react";
import { cn, formatBytes } from "@/lib/utils";

interface DownloadFile {
  id: string;
  filename: string;
  path: string;
  author: string;
  size: number;
  timestamp: number;
  fileType: string;
}

export function DownloadsView() {
  const tasks = useDownloadStore((s) => s.tasks);
  const removeTask = useDownloadStore((s) => s.removeTask);
  const clearCompleted = useDownloadStore((s) => s.clearCompleted);

  const [searchQuery, setSearchQuery] = useState("");
  const [typeFilter, setTypeFilter] = useState("all");
  const [sortBy, setSortBy] = useState("date_desc");
  const [selectedFiles, setSelectedFiles] = useState<Set<string>>(new Set());
  const [selectAll, setSelectAll] = useState(false);

  const tasksList = Object.values(tasks);

  const handleRefresh = useCallback(() => {
    // invoke("list_download_files", { dir: config.download_dir })
  }, []);

  const handleOpenDir = useCallback(() => {
    // invoke("open_path", { path: config.download_dir })
  }, []);

  const toggleSelectAll = useCallback(() => {
    if (selectAll) {
      setSelectedFiles(new Set());
      setSelectAll(false);
    } else {
      setSelectedFiles(new Set(tasksList.map((t) => t.id)));
      setSelectAll(true);
    }
  }, [selectAll, tasksList]);

  const activeTasks = tasksList.filter(
    (t) => t.status === "downloading" || t.status === "pending"
  );
  const completedTasks = tasksList.filter((t) => t.status === "completed");

  return (
    <div>
      {/* Header */}
      <div className="flex items-center justify-between mb-4">
        <div className="flex items-center gap-2">
          <FolderOpen className="w-4 h-4 text-accent" />
          <h3 className="text-[0.9rem] font-semibold text-text">我的下载</h3>
          <Badge variant="secondary">{tasksList.length} 个任务</Badge>
        </div>
        <div className="flex items-center gap-2">
          <Button variant="outline" size="sm" onClick={handleOpenDir}>
            <Folder className="w-3.5 h-3.5" />
            打开目录
          </Button>
          <Button variant="outline" size="sm" onClick={handleRefresh}>
            <RefreshCw className="w-3.5 h-3.5" />
            刷新
          </Button>
        </div>
      </div>

      {/* Filter Bar */}
      <div className="flex items-center gap-2 mb-4 flex-wrap">
        <div className="relative flex-1 min-w-[200px]">
          <Search className="absolute left-3 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-text-muted" />
          <Input
            placeholder="搜索文件名、作者..."
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            className="pl-8 h-8 text-[0.8rem]"
          />
        </div>
        <Select value={typeFilter} onValueChange={setTypeFilter}>
          <SelectTrigger className="w-[120px] h-8 text-[0.8rem]">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="all">全部类型</SelectItem>
            <SelectItem value="video">视频</SelectItem>
            <SelectItem value="image">图片</SelectItem>
            <SelectItem value="audio">音频</SelectItem>
          </SelectContent>
        </Select>
        <Select value={sortBy} onValueChange={setSortBy}>
          <SelectTrigger className="w-[120px] h-8 text-[0.8rem]">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="date_desc">最新优先</SelectItem>
            <SelectItem value="date_asc">最早优先</SelectItem>
            <SelectItem value="size_desc">最大优先</SelectItem>
            <SelectItem value="size_asc">最小优先</SelectItem>
          </SelectContent>
        </Select>
      </div>

      {/* Batch Actions */}
      {tasksList.length > 0 && (
        <div className="flex items-center gap-2 mb-3">
          <button
            onClick={toggleSelectAll}
            className="flex items-center gap-1.5 px-2.5 h-8 rounded-[var(--radius-sm)] bg-surface border border-border text-[0.78rem] text-text-secondary hover:text-text cursor-pointer transition-all"
          >
            {selectAll ? (
              <CheckSquare className="w-3.5 h-3.5 text-accent" />
            ) : (
              <Square className="w-3.5 h-3.5" />
            )}
            全选
          </button>
          {selectedFiles.size > 0 && (
            <>
              <Badge variant="default">{selectedFiles.size} 已选</Badge>
              <Button variant="outline" size="sm">
                <Play className="w-3 h-3" />
                打开
              </Button>
              <Button variant="danger-outline" size="sm">
                <Trash2 className="w-3 h-3" />
                删除
              </Button>
            </>
          )}
          {completedTasks.length > 0 && (
            <Button variant="ghost" size="sm" onClick={clearCompleted} className="ml-auto">
              清除已完成
            </Button>
          )}
        </div>
      )}

      {/* Active Tasks */}
      {activeTasks.length > 0 && (
        <div className="mb-4">
          <div className="text-[0.7rem] font-bold text-text-muted uppercase tracking-wider mb-2">
            进行中 ({activeTasks.length})
          </div>
          <div className="flex flex-col gap-1.5">
            <AnimatePresence mode="popLayout">
              {activeTasks.map((task) => (
                <TaskCard
                  key={task.id}
                  task={task}
                  onCancel={removeTask}
                  onRemove={removeTask}
                />
              ))}
            </AnimatePresence>
          </div>
        </div>
      )}

      {/* Completed Tasks */}
      {completedTasks.length > 0 && (
        <div>
          <div className="text-[0.7rem] font-bold text-text-muted uppercase tracking-wider mb-2">
            已完成 ({completedTasks.length})
          </div>
          <div className="flex flex-col gap-1.5">
            <AnimatePresence mode="popLayout">
              {completedTasks.map((task) => (
                <TaskCard
                  key={task.id}
                  task={task}
                  onRemove={removeTask}
                />
              ))}
            </AnimatePresence>
          </div>
        </div>
      )}

      {/* Empty State */}
      {tasksList.length === 0 && (
        <motion.div
          initial={{ opacity: 0, y: 12 }}
          animate={{ opacity: 1, y: 0 }}
          className="flex flex-col items-center justify-center py-16 text-center"
        >
          <div className="w-14 h-14 rounded-[18px] bg-surface border border-border flex items-center justify-center mb-4">
            <FolderOpen className="w-6 h-6 text-text-muted" />
          </div>
          <p className="text-[0.85rem] text-text-secondary mb-1">暂无下载任务</p>
          <p className="text-[0.8rem] text-text-muted">搜索用户或粘贴链接开始下载</p>
        </motion.div>
      )}
    </div>
  );
}
