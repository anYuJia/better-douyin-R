import { motion } from "framer-motion";
import { Progress } from "@/components/ui/progress";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { cn, formatBytes, formatDuration } from "@/lib/utils";
import { Play, FolderOpen, X, RotateCw, CheckCircle2, AlertCircle, Loader2 } from "lucide-react";
import type { DownloadTask, DownloadStatus } from "@/types";

interface TaskCardProps {
  task: DownloadTask;
  onCancel?: (id: string) => void;
  onRetry?: (id: string) => void;
  onOpen?: (id: string) => void;
  onRemove?: (id: string) => void;
}

const statusConfig: Record<DownloadStatus, { icon: React.ElementType; label: string; color: string }> = {
  pending: { icon: Loader2, label: "等待中", color: "text-text-muted" },
  downloading: { icon: Loader2, label: "下载中", color: "text-accent" },
  completed: { icon: CheckCircle2, label: "已完成", color: "text-success" },
  error: { icon: AlertCircle, label: "失败", color: "text-danger" },
  paused: { icon: Play, label: "已暂停", color: "text-warning" },
};

export function TaskCard({ task, onCancel, onRetry, onOpen, onRemove }: TaskCardProps) {
  const cfg = statusConfig[task.status];
  const Icon = cfg.icon;

  return (
    <motion.div
      layout
      initial={{ opacity: 0, y: 8 }}
      animate={{ opacity: 1, y: 0 }}
      exit={{ opacity: 0, x: -20 }}
      transition={{ type: "spring", stiffness: 400, damping: 28 }}
      className={cn(
        "flex items-center gap-3 px-3 py-2.5 rounded-[var(--radius-md)] border transition-all",
        task.status === "completed"
          ? "border-success/20 bg-success-soft/30"
          : task.status === "error"
            ? "border-danger/20 bg-danger-soft/30"
            : "border-border bg-surface/50"
      )}
    >
      {/* Icon */}
      <div className={cn("shrink-0", cfg.color)}>
        <Icon
          className={cn(
            "w-4 h-4",
            task.status === "downloading" && "animate-spin",
            task.status === "pending" && "animate-pulse"
          )}
        />
      </div>

      {/* Info */}
      <div className="flex-1 min-w-0">
        <div className="flex items-center gap-2 mb-1">
          <span className="text-[0.78rem] font-medium text-text truncate">
            {task.filename || task.id}
          </span>
          <Badge
            variant={
              task.status === "completed"
                ? "success"
                : task.status === "error"
                  ? "danger"
                  : task.status === "downloading"
                    ? "default"
                    : "secondary"
            }
            size="sm"
          >
            {cfg.label}
          </Badge>
        </div>

        {(task.status === "downloading" || task.status === "pending") && (
          <Progress value={task.progress} className="h-1.5 mb-1" />
        )}

        <div className="flex items-center gap-3 text-[0.68rem] text-text-muted">
          {task.status === "downloading" && (
            <>
              <span>{task.progress.toFixed(1)}%</span>
              {task.speed > 0 && <span>{formatBytes(task.speed)}/s</span>}
              {task.downloadedBytes !== undefined && (
                <span>{formatBytes(task.downloadedBytes)}{task.totalBytes ? ` / ${formatBytes(task.totalBytes)}` : ""}</span>
              )}
            </>
          )}
          {task.status === "completed" && task.totalBytes && (
            <span>{formatBytes(task.totalBytes)}</span>
          )}
        </div>
      </div>

      {/* Actions */}
      <div className="flex items-center gap-1 shrink-0">
        {task.status === "completed" && (
          <Button variant="ghost" size="icon-sm" onClick={() => onOpen?.(task.id)} title="打开">
            <FolderOpen className="w-3.5 h-3.5" />
          </Button>
        )}
        {task.status === "error" && (
          <Button variant="ghost" size="icon-sm" onClick={() => onRetry?.(task.id)} title="重试">
            <RotateCw className="w-3.5 h-3.5" />
          </Button>
        )}
        {(task.status === "downloading" || task.status === "pending") && (
          <Button variant="ghost" size="icon-sm" onClick={() => onCancel?.(task.id)} title="取消">
            <X className="w-3.5 h-3.5" />
          </Button>
        )}
        {(task.status === "completed" || task.status === "error") && (
          <Button variant="ghost" size="icon-sm" onClick={() => onRemove?.(task.id)} title="移除">
            <X className="w-3.5 h-3.5" />
          </Button>
        )}
      </div>
    </motion.div>
  );
}
