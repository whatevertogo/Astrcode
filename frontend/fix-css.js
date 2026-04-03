
const fs = require('fs');

let css = fs.readFileSync('src/components/Chat/AssistantMessage.module.css', 'utf8');

css = css.replace(/\.inlineCode\s*\{[^}]+\}/, \.inlineCode {
  background: #f4f4f5;
  border: 1px solid #e5e7eb;
  border-radius: 6px;
  padding: 3px 6px;
  font-size: 0.875em;
  color: #1f2937;
  font-family: ui-monospace, SFMono-Regular, Consolas, 'Liberation Mono', Menlo, monospace;
  word-break: break-word;
}\);

css = css.replace(/\.content p \{ margin: 0 0 16px; \}/, '.content p { margin: 0 0 1.25em; }');
css = css.replace(/\.content ul, \.content ol \{ margin: 8px 0 16px; padding-left: 24px; \}/, '.content ul, .content ol { margin: 0.75em 0 1.25em; padding-left: 1.75em; }');
css = css.replace(/\.content li \{ margin-bottom: 6px; \}/, '.content li { margin-bottom: 0.5em; }');

// Update code block slightly to make it flatter
css = css.replace(/\.codeBlock\s*\{[^}]+\}/, \.codeBlock {
  background: #f9fafb;
  border: 1px solid #e5e7eb;
  border-radius: 8px;
  padding: 16px;
  overflow-x: auto;
  margin: 1.25em 0;
  font-size: 0.875em;
  line-height: 1.6;
  color: #1f2937;
  font-family: ui-monospace, SFMono-Regular, Consolas, 'Liberation Mono', Menlo, monospace;
}\);

fs.writeFileSync('src/components/Chat/AssistantMessage.module.css', css);
console.log('Fixed inline code and spacing');

