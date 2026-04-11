import { memo } from 'react';
import type { UserMessage as UserMessageType } from '../../types';

interface UserMessageProps {
  message: UserMessageType;
}

function UserMessage({ message }: UserMessageProps) {
  return (
    <div className="max-sm:gap-3 flex justify-end gap-4 py-2 animate-message-enter motion-reduce:animate-none">
      <div className="hidden" aria-hidden="true">
        U
      </div>
      <div className="flex-[0_1_auto] max-w-[85%] min-w-0 pt-0.5">
        <div className="inline-block whitespace-pre-wrap overflow-wrap-anywhere rounded-[14px] bg-user-bubble px-[18px] py-3 text-base text-text-primary">
          {message.text}
        </div>
      </div>
    </div>
  );
}

export default memo(UserMessage);
