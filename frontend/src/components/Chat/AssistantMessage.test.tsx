import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';

import AssistantMessage from './AssistantMessage';

describe('AssistantMessage streaming markdown', () => {
  it('renders assistant markdown while streaming instead of falling back to raw fences', () => {
    const html = renderToStaticMarkup(
      <AssistantMessage
        message={{
          id: 'assistant-1',
          kind: 'assistant',
          text: '```ts\nconst answer = 42;\n```',
          reasoningText: '```rust\nfn main() {}\n```',
          streaming: true,
          timestamp: Date.now(),
        }}
      />
    );

    expect(html).not.toContain('```ts');
    expect(html).not.toContain('```rust');
    expect(html).toContain('const answer = 42;');
    expect(html).toContain('fn main() {}');
    expect(html).toContain('Thinking');
  });

  it('renders sub-run assistant content without thinking chrome or token footer', () => {
    const html = renderToStaticMarkup(
      <AssistantMessage
        presentation="subRun"
        message={{
          id: 'assistant-subrun-1',
          kind: 'assistant',
          text: '最终结论',
          reasoningText: '中间推理',
          streaming: false,
          timestamp: Date.now(),
        }}
      />
    );

    expect(html).toContain('中间推理');
    expect(html).toContain('最终结论');
    expect(html).not.toContain('Thinking');
    expect(html).not.toContain('tokens');
  });
});
