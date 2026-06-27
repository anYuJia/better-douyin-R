import { Loader2, Users, Wifi, WifiOff } from "lucide-react";
import type { FriendListItem, FriendStatusItem } from "./friends-status-types";
import { FriendRow, Metric } from "./friends-status-components";

interface FriendListPanelProps {
  friends: FriendStatusItem[];
  friendItems: FriendListItem[];
  selectedFriendId: string;
  onlineCount: number;
  offlineCount: number;
  isInitialLoading: boolean;
  idsLength: number;
  selectFriend: (friend: FriendStatusItem) => void;
  openFriendProfile: (friend: FriendStatusItem) => Promise<void>;
}

export function FriendListPanel({
  friends,
  friendItems,
  selectedFriendId,
  onlineCount,
  offlineCount,
  isInitialLoading,
  idsLength,
  selectFriend,
  openFriendProfile,
}: FriendListPanelProps) {
  return (
    <section className="flex min-h-0 flex-col rounded-[var(--radius-lg)] border border-border bg-surface-solid/70 p-3 shadow-[var(--shadow-sm)]">
      <div className="mb-3 grid shrink-0 grid-cols-3 gap-1.5">
        <Metric label="总数" value={friends.length || idsLength} icon={Users} />
        <Metric label="在线" value={onlineCount} icon={Wifi} tone="success" />
        <Metric label="未在线" value={offlineCount} icon={WifiOff} tone="muted" />
      </div>

      {isInitialLoading ? (
        <div className="flex min-h-[280px] items-center justify-center text-[0.82rem] text-text-muted">
          <Loader2 className="mr-2 h-4 w-4 animate-spin" />
          正在查询
        </div>
      ) : friendItems.length === 0 ? (
        <div className="flex min-h-[280px] flex-col items-center justify-center text-center">
          <div className="mb-3 flex h-12 w-12 items-center justify-center rounded-[16px] border border-border bg-surface">
            <Users className="h-5 w-5 text-text-muted" />
          </div>
          <p className="text-[0.86rem] text-text-secondary">等待查询</p>
          <p className="mt-1 text-[0.75rem] text-text-muted">点刷新自动获取；若没有返回列表，可展开备用输入缓存一次</p>
        </div>
      ) : (
        <div className="grid min-h-0 flex-1 content-start gap-1.5 overflow-y-auto pr-1">
          {friendItems.map((friend) => (
            <FriendRow
              key={friend.secUid}
              friend={friend}
              selected={friend.secUid === selectedFriendId}
              onSelect={selectFriend}
              onOpenProfile={openFriendProfile}
            />
          ))}
        </div>
      )}
    </section>
  );
}
