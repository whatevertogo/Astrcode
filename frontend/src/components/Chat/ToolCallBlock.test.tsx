import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';

import ToolCallBlock from './ToolCallBlock';

describe('ToolCallBlock', () => {
  it('renders real tool args in the collapsed summary and grouped result output in the body', () => {
    const html = renderToStaticMarkup(
      <ToolCallBlock
        message={{
          id: 'tool-call-1',
          kind: 'toolCall',
          toolCallId: 'call-1',
          toolName: 'readFile',
          status: 'ok',
          args: {
            path: 'Cargo.toml',
            limit: 220,
          },
          output: '[workspace]',
          timestamp: Date.now(),
        }}
        streams={[
          {
            id: 'tool-stream-1',
            kind: 'toolStream',
            toolCallId: 'call-1',
            stream: 'stdout',
            status: 'ok',
            content: '[workspace]\nmembers = [\n  "crates/core"\n]\n',
            timestamp: Date.now(),
          },
        ]}
      />
    );

    expect(html).toContain('已运行 readFile');
    expect(html).toContain('path=&quot;Cargo.toml&quot;');
    expect(html).toContain('limit=220');
    expect(html).toContain('[workspace]');
    expect(html).toContain('调用参数');
  });

  it('renders fallback result surface when no streamed output exists', () => {
    const html = renderToStaticMarkup(
      <ToolCallBlock
        message={{
          id: 'tool-call-2',
          kind: 'toolCall',
          toolCallId: 'call-2',
          toolName: 'findFiles',
          status: 'ok',
          args: {
            pattern: '*.rs',
          },
          output: '找到 12 个文件',
          timestamp: Date.now(),
        }}
      />
    );

    expect(html).toContain('找到 12 个文件');
    expect(html).toContain('结果');
  });
});
