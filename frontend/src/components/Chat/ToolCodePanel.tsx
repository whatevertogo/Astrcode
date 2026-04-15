import { memo, useRef } from 'react';

import { codeBlockContent, codeBlockHeader, codeBlockShell } from '../../lib/styles';
import { cn } from '../../lib/utils';
import { useNestedScrollContainment } from './useNestedScrollContainment';

interface ToolCodePanelProps {
  title: string;
  tone?: 'normal' | 'error';
  content: string;
  scrollMode?: 'self' | 'inherit';
}

function ToolCodePanel({
  title,
  tone = 'normal',
  content,
  scrollMode = 'self',
}: ToolCodePanelProps) {
  const contentRef = useRef<HTMLPreElement>(null);
  const inactiveRef = useRef<HTMLPreElement>(null);
  useNestedScrollContainment(scrollMode === 'self' ? contentRef : inactiveRef);

  return (
    <div className={cn(codeBlockShell, 'my-0')}>
      <div className={codeBlockHeader}>
        <span>{title}</span>
      </div>
      <pre
        ref={contentRef}
        className={cn(
          codeBlockContent,
          scrollMode === 'self'
            ? 'max-h-[420px] overflow-auto whitespace-pre-wrap overflow-wrap-anywhere'
            : 'overflow-visible whitespace-pre-wrap overflow-wrap-anywhere',
          tone === 'error' ? 'text-danger' : 'text-code-text'
        )}
      >
        {content}
      </pre>
    </div>
  );
}

export default memo(ToolCodePanel);
