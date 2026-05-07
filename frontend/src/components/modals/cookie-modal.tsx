import { useState, useCallback } from "react";
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription } from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { Select, SelectTrigger, SelectValue, SelectContent, SelectItem } from "@/components/ui/select";
import { Badge } from "@/components/ui/badge";
import { Tabs, TabsList, TabsTrigger, TabsContent } from "@/components/ui/tabs";
import { motion, AnimatePresence } from "framer-motion";
import {
  ShieldCheck,
  Globe,
  ClipboardPaste,
  CheckCircle2,
  XCircle,
  Loader2,
  ExternalLink,
  Info,
  KeyRound,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { useLogStore } from "@/stores/app-store";

interface CookieModalProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onCookieSaved?: (cookie: string) => void;
}

type LoginStatus = "idle" | "waiting" | "success" | "error";

export function CookieModal({ open, onOpenChange, onCookieSaved }: CookieModalProps) {
  const [tab, setTab] = useState("browser");
  const [browserType, setBrowserType] = useState("chrome");
  const [loginStatus, setLoginStatus] = useState<LoginStatus>("idle");
  const [loginMessage, setLoginMessage] = useState("");
  const [manualCookie, setManualCookie] = useState("");
  const [validationResult, setValidationResult] = useState<{ valid: boolean; missing: string[] } | null>(null);
  const addLog = useLogStore((s) => s.addLog);

  const handleBrowserLogin = useCallback(async () => {
    setLoginStatus("waiting");
    setLoginMessage("正在打开浏览器...");
    addLog(`启动 ${browserType} 浏览器登录`, "info");

    try {
      // In real app: invoke("start_browser_login", { browserType })
      await new Promise((resolve) => setTimeout(resolve, 2000));
      setLoginStatus("success");
      setLoginMessage("Cookie 已成功获取并保存");
      addLog("浏览器登录成功", "success");
      onCookieSaved?.("");
    } catch {
      setLoginStatus("error");
      setLoginMessage("登录失败，请重试");
      addLog("浏览器登录失败", "error");
    }
  }, [browserType, addLog, onCookieSaved]);

  const handleCancelLogin = useCallback(() => {
    setLoginStatus("idle");
    setLoginMessage("");
    // invoke("cancel_browser_login")
  }, []);

  const handleManualSave = useCallback(async () => {
    if (!manualCookie.trim()) return;
    addLog("手动保存 Cookie", "info");
    // invoke("save_cookie", { cookie: manualCookie })
    onCookieSaved?.(manualCookie);
    onOpenChange(false);
  }, [manualCookie, addLog, onCookieSaved, onOpenChange]);

  const handleTestCookie = useCallback(async () => {
    if (!manualCookie.trim()) return;
    addLog("测试 Cookie 有效性...", "info");
    // const result = await invoke("validate_cookie", { cookie: manualCookie })
    // Mock result:
    const result = { valid: manualCookie.length > 50, missing: manualCookie.length > 50 ? [] : ["sessionid", "ttwid"] };
    setValidationResult(result);
    addLog(result.valid ? "Cookie 有效" : "Cookie 无效", result.valid ? "success" : "error");
  }, [manualCookie, addLog]);

  const steps = [
    { num: 1, text: `点击下方按钮，系统将打开 ${browserType === "chrome" ? "Chrome" : browserType === "edge" ? "Edge" : "Chromium"} 窗口` },
    { num: 2, text: "在弹出的浏览器中登录你的抖音账号" },
    { num: 3, text: "登录成功后，Cookie 将被自动提取并保存" },
  ];

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-md rounded-[var(--radius-xl)]">
        <DialogHeader>
          <div className="flex items-center gap-3 mb-1">
            <div className="w-10 h-10 rounded-full bg-accent/15 flex items-center justify-center">
              <KeyRound className="w-5 h-5 text-accent" />
            </div>
            <div>
              <DialogTitle>配置 Cookie</DialogTitle>
              <DialogDescription>需要配置抖音 Cookie 才能使用全部功能</DialogDescription>
            </div>
          </div>
        </DialogHeader>

        <Tabs value={tab} onValueChange={setTab}>
          <TabsList className="w-full mb-4">
            <TabsTrigger value="browser" className="flex-1 gap-1.5">
              <Globe className="w-3.5 h-3.5" />
              登录账号
            </TabsTrigger>
            <TabsTrigger value="manual" className="flex-1 gap-1.5">
              <ClipboardPaste className="w-3.5 h-3.5" />
              手动填入
            </TabsTrigger>
          </TabsList>

          {/* Browser Login Tab */}
          <TabsContent value="browser">
            <div className="space-y-4">
              <div>
                <label className="text-[0.8rem] font-medium text-text-secondary mb-1.5 block">
                  选择浏览器
                </label>
                <Select value={browserType} onValueChange={setBrowserType}>
                  <SelectTrigger className="h-9">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="chrome">Google Chrome</SelectItem>
                    <SelectItem value="edge">Microsoft Edge</SelectItem>
                    <SelectItem value="chromium">Chromium（内置）</SelectItem>
                  </SelectContent>
                </Select>
              </div>

              {/* Steps */}
              <div className="flex flex-col gap-3">
                {steps.map((step) => (
                  <div key={step.num} className="flex items-center gap-3 text-[0.82rem] text-text-secondary">
                    <span className="w-7 h-7 shrink-0 rounded-full bg-gradient-to-br from-accent to-accent-hover text-white text-[0.75rem] font-semibold flex items-center justify-center shadow-sm">
                      {step.num}
                    </span>
                    {step.text}
                  </div>
                ))}
              </div>

              {/* Status */}
              <AnimatePresence>
                {loginStatus !== "idle" && (
                  <motion.div
                    initial={{ opacity: 0, height: 0 }}
                    animate={{ opacity: 1, height: "auto" }}
                    exit={{ opacity: 0, height: 0 }}
                    className={cn(
                      "flex items-center gap-2.5 px-4 py-3 rounded-[var(--radius-sm)] border text-[0.8rem]",
                      loginStatus === "waiting" && "border-border bg-surface text-text-secondary",
                      loginStatus === "success" && "border-success/40 bg-success-soft text-success",
                      loginStatus === "error" && "border-danger/40 bg-danger-soft text-danger"
                    )}
                  >
                    {loginStatus === "waiting" && <Loader2 className="w-4 h-4 animate-spin" />}
                    {loginStatus === "success" && <CheckCircle2 className="w-4 h-4" />}
                    {loginStatus === "error" && <XCircle className="w-4 h-4" />}
                    {loginMessage}
                  </motion.div>
                )}
              </AnimatePresence>

              {/* Actions */}
              {loginStatus !== "waiting" ? (
                <Button variant="default" className="w-full h-10" onClick={handleBrowserLogin}>
                  <ExternalLink className="w-4 h-4" />
                  打开浏览器登录
                </Button>
              ) : (
                <Button variant="danger-outline" className="w-full h-10" onClick={handleCancelLogin}>
                  取消
                </Button>
              )}
            </div>
          </TabsContent>

          {/* Manual Tab */}
          <TabsContent value="manual">
            <div className="space-y-3">
              <div className="flex items-start gap-2 p-3 rounded-[var(--radius-sm)] bg-info-soft border border-info/20 text-[0.78rem] text-text-secondary">
                <Info className="w-4 h-4 text-info shrink-0 mt-0.5" />
                从浏览器开发者工具复制抖音的 Cookie 并粘贴到下方
              </div>

              <Textarea
                placeholder="粘贴抖音 Cookie..."
                rows={5}
                value={manualCookie}
                onChange={(e) => {
                  setManualCookie(e.target.value);
                  setValidationResult(null);
                }}
              />

              {/* Validation */}
              <AnimatePresence>
                {validationResult && (
                  <motion.div
                    initial={{ opacity: 0, y: -4 }}
                    animate={{ opacity: 1, y: 0 }}
                    exit={{ opacity: 0 }}
                    className={cn(
                      "flex items-center gap-2 text-[0.78rem] px-3 py-2 rounded-[var(--radius-sm)]",
                      validationResult.valid
                        ? "text-success bg-success-soft"
                        : "text-danger bg-danger-soft"
                    )}
                  >
                    {validationResult.valid ? (
                      <CheckCircle2 className="w-3.5 h-3.5" />
                    ) : (
                      <XCircle className="w-3.5 h-3.5" />
                    )}
                    {validationResult.valid
                      ? "Cookie 格式有效"
                      : `缺少关键参数: ${validationResult.missing.join(", ")}`}
                  </motion.div>
                )}
              </AnimatePresence>

              <Button variant="default" className="w-full h-10" onClick={handleManualSave}>
                保存 Cookie
              </Button>
              <Button variant="outline" className="w-full h-9" onClick={handleTestCookie}>
                测试有效性
              </Button>
            </div>
          </TabsContent>
        </Tabs>
      </DialogContent>
    </Dialog>
  );
}
