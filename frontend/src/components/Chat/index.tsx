import type { SubRunViewData, ThreadItem } from '../../types';
import { subRunNotice } from '../../lib/styles';
import InputBar from './InputBar';
import MessageList from './MessageList';
import { ChatScreenProvider, type ChatScreenContextValue } from './ChatScreenContext';
import TopBar from './TopBar';

interface ChatProps {
  threadItems: ThreadItem[];
  childSubRuns: SubRunViewData[];
  subRunViews: Map<string, SubRunViewData>;
  contentFingerprint: string;
  contextValue: ChatScreenContextValue;
}

export default function Chat({
  threadItems,
  childSubRuns,
  subRunViews,
  contentFingerprint,
  contextValue,
}: ChatProps) {
  return (
    <ChatScreenProvider value={contextValue}>
      <div className="flex h-full min-h-0 min-w-0 flex-col overflow-hidden bg-panel-bg">
        <TopBar />
        <MessageList
          threadItems={threadItems}
          childSubRuns={childSubRuns}
          subRunViews={subRunViews}
          contentFingerprint={contentFingerprint}
        />
        {contextValue.activeSubRunPath.length > 0 ? (
          <div className={subRunNotice}>
            当前正在查看子执行
            {contextValue.activeSubRunTitle ? `「${contextValue.activeSubRunTitle}」` : ''}{' '}
            的内容视图。下方会单独列出下一层子执行；返回主会话后可继续输入。
          </div>
        ) : (
          <InputBar />
        )}
      </div>
    </ChatScreenProvider>
  );
}
