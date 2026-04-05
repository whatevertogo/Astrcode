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
});
