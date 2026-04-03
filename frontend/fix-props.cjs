const fs = require('fs');

// Fix InputBar.tsx
let inputBar = fs.readFileSync('src/components/Chat/InputBar.tsx', 'utf-8');
// Import ComposerOption and fix type
inputBar = inputBar.replace(
    "import type { Phase } from '../../types';",
    "import type { ComposerOption, Phase } from '../../types';"
);
inputBar = inputBar.replace(
    "listComposerOptions: any;",
    "listComposerOptions: (sessionId: string, query: string, signal?: AbortSignal) => Promise<ComposerOption[]>;"
);
fs.writeFileSync('src/components/Chat/InputBar.tsx', inputBar);

// Fix index.tsx (Chat)
let index = fs.readFileSync('src/components/Chat/index.tsx', 'utf-8');
// Fix type
index = index.replace(
    "listComposerOptions: any;",
    "listComposerOptions: (sessionId: string, query: string, signal?: AbortSignal) => Promise<ComposerOption[]>;"
);
// Add listComposerOptions to destructuring
index = index.replace(
    "onInterrupt,",
    "onInterrupt,\n  listComposerOptions,"
);
// Add to InputBar component
index = index.replace(
    "onInterrupt={onInterrupt}",
    "onInterrupt={onInterrupt}\n        listComposerOptions={listComposerOptions}"
);
// Remove styles import if any
index = index.replace(/import styles from '\.\/[a-zA-Z0-9_\.]+\.module\.css';\n?/g, "");
// Replace className={styles.chat} with className="chat"
index = index.replace(/className=\{styles\.([a-zA-Z0-9_]+)\}/g, 'className="$1"');

fs.writeFileSync('src/components/Chat/index.tsx', index);
