import { useState, useCallback } from "react";
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogFooter } from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Progress } from "@/components/ui/progress";
import { Badge } from "@/components/ui/badge";
import { ScrollArea } from "@/components/ui/scroll-area";
import { CloudDownload, RefreshCw, ArrowRight } from "lucide-react";

interface UpdateModalProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  currentVersion?: string;
  newVersion?: string;
  releaseNotes?: string;
}

export function UpdateModal({
  open,
  onOpenChange,
  currentVersion = "0.0.10",
  newVersion = "0.1.0",
  releaseNotes = "",
}: UpdateModalProps) {
  const [status, setStatus] = useState<"idle" | "downloading" | "ready">("idle");
  const [progress, setProgress] = useState(0);

  const handleDownload = useCallback(() => {
    setStatus("downloading");
    setProgress(0);

    // Simulate download progress
    // In real app: invoke("download_update")
    const interval = setInterval(() => {
      setProgress((prev) => {
        if (prev >= 100) {
          clearInterval(interval);
          setStatus("ready");
          return 100;
        }
        return prev + Math.random() * 15;
      });
    }, 300);
  }, []);

  const handleRestart = useCallback(() => {
    // invoke("restart_app")
    onOpenChange(false);
  }, [onOpenChange]);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-md rounded-[var(--radius-xl)]">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <CloudDownload className="w-5 h-5 text-accent" />
            发现新版本
          </DialogTitle>
        </DialogHeader>

        <div className="space-y-4">
          {/* Version info */}
          <div className="flex items-center gap-3">
            <div className="flex items-center gap-2">
              <span className="text-[0.8rem] text-text-muted">当前版本</span>
              <Badge variant="secondary">{currentVersion}</Badge>
            </div>
            <ArrowRight className="w-4 h-4 text-text-muted" />
            <div className="flex items-center gap-2">
              <span className="text-[0.8rem] text-text-muted">最新版本</span>
              <Badge variant="default">{newVersion}</Badge>
            </div>
          </div>

          {/* Release notes */}
          {releaseNotes && (
            <ScrollArea className="max-h-[200px] rounded-[var(--radius-sm)] border border-border p-3 bg-surface/50">
              <div className="text-[0.8rem] text-text-secondary whitespace-pre-wrap leading-relaxed">
                {releaseNotes}
              </div>
            </ScrollArea>
          )}

          {/* Progress */}
          {status === "downloading" && (
            <div className="space-y-2">
              <div className="flex items-center justify-between">
                <span className="text-[0.75rem] text-text-muted">下载进度</span>
                <span className="text-[0.75rem] text-text-secondary font-mono">
                  {Math.min(100, Math.round(progress))}%
                </span>
              </div>
              <Progress value={progress} />
            </div>
          )}

          {/* Ready message */}
          {status === "ready" && (
            <div className="flex items-center gap-2 p-3 rounded-[var(--radius-sm)] bg-success-soft border border-success/25 text-[0.8rem] text-success">
              <RefreshCw className="w-4 h-4" />
              下载完成，点击"重启应用"以完成更新
            </div>
          )}
        </div>

        <DialogFooter>
          {status === "idle" && (
            <>
              <Button variant="ghost" size="sm" onClick={() => onOpenChange(false)}>
                稍后再说
              </Button>
              <Button variant="default" size="sm" onClick={handleDownload}>
                <CloudDownload className="w-3.5 h-3.5" />
                立即更新
              </Button>
            </>
          )}
          {status === "downloading" && (
            <Button variant="ghost" size="sm" disabled>
              <RefreshCw className="w-3.5 h-3.5 animate-spin" />
              下载中...
            </Button>
          )}
          {status === "ready" && (
            <Button variant="default" size="sm" onClick={handleRestart}>
              <RefreshCw className="w-3.5 h-3.5" />
              重启应用
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
