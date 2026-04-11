import { memo } from 'react';
import type { UserMessage as UserMessageType } from '../../types';

interface UserMessageProps {
  message: UserMessageType;
}

function UserMessage({ message }: UserMessageProps) {
  return (
    <div className="flex justify-end gap-4 animate-message-enter max-sm:gap-3 motion-reduce:animate-none py-2">
      <div className="hidden" aria-hidden="true">
        U
      </div>
      <div className="flex-[0_1_auto] max-w-[85%] min-w-0 pt-0.5">
        <div className="inline-block px-[18px] py-3 rounded-[14px] bg-[#f3f2ee] text-[#141413] text-base whitespace-pre-wrap overflow-wrap-anywhere">
          {message.text}
        </div>
      </div>
    </div>
  );
}

export default memo(UserMessage);
