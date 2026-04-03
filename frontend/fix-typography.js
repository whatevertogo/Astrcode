
const fs = require('fs');

let css = fs.readFileSync('src/index.css', 'utf8');
css = css.replace(/font-family: [^;]+;/, 'font-family: Söhne, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, \'Segoe UI\', Roboto, Ubuntu, \'Helvetica Neue\', Arial, sans-serif;');
css = css.replace(/font-size: 14px;/, 'font-size: 16px;');
fs.writeFileSync('src/index.css', css);

let amCss = fs.readFileSync('src/components/Chat/AssistantMessage.module.css', 'utf8');
if (!amCss.includes('.content h2 {')) {
  amCss += \

.content h1, .content h2, .content h3, .content h4 {
  font-weight: 600;
  margin-top: 1.5em;
  margin-bottom: 0.75em;
  color: #0d0d0d;
}
.content h1 { font-size: 1.5em; }
.content h2 { 
  font-size: 1.25em; 
  padding-bottom: 0.3em;
  border-bottom: 1px solid #e5e5e5;
  margin-top: 2em;
}
.content h3 { font-size: 1.1em; }
.content hr {
  height: 1px;
  background-color: #e5e5e5;
  border: none;
  margin: 2em 0;
}
.content blockquote {
  border-left: 3px solid #e5e5e5;
  padding-left: 1em;
  color: #666;
  margin: 1em 0;
}
.content table {
  width: 100%;
  border-collapse: collapse;
  margin: 1em 0;
}
.content th, .content td {
  border: 1px solid #e5e5e5;
  padding: 8px 12px;
}
.content th {
  background: #f9f9f9;
  font-weight: 600;
}
\;
  fs.writeFileSync('src/components/Chat/AssistantMessage.module.css', amCss);
}
console.log('Typography format applied');

