import { memo, useRef } from 'react';

import { codeBlockContent, codeBlockHeader, codeBlockShell } from '../../lib/styles';
import { cn } from '../../lib/utils';
import { useNestedScrollContainment } from './useNestedScrollContainment';

interface ToolCodePanelProps {
  title: string;
  tone?: 'normal' | 'error';
  content: string;
}

function ToolCodePanel({ title, tone = 'normal', content }: ToolCodePanelProps) {
  const contentRef = useRef<HTMLPreElement>(null);
  useNestedScrollContainment(contentRef);

  return (
    <div className={cn(codeBlockShell, 'my-0')}>
      <div className={codeBlockHeader}>
        <span>{title}</span>
      </div>
      <pre
        ref={contentRef}
        className={cn(
          codeBlockContent,
          'max-h-[420px] overflow-auto whitespace-pre-wrap overflow-wrap-anywhere',
          tone === 'error' ? 'text-danger' : 'text-code-text'
        )}
      >
        {content}
      </pre>
    </div>
  );
}

export default memo(ToolCodePanel);
