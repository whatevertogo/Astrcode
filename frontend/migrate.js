const fs = require('fs');
const glob = require('glob');

const tsxFiles = glob.sync('src/components/**/*.tsx');
for (const file of tsxFiles) {
  let content = fs.readFileSync(file, 'utf-8');
  if(content.includes('.module.css')) {
    content = content.replace(/import styles from '\.\/[a-zA-Z0-9_\.]+\.module\.css';\n?/g, '');
    content = content.replace(/className=\{styles\.([a-zA-Z0-9_]+)\}/g, 'className="$1"');
    content = content.replace(/className=\{\\\\$\{styles\.([a-zA-Z0-9_]+)\\\}\\s+(.*?)\\}/g, 'className={\$1 \}');
    fs.writeFileSync(file, content);
  }
}

let allCss = '';
const cssFiles = glob.sync('src/components/**/*.module.css');
for (const file of cssFiles) {
  allCss += '\n' + fs.readFileSync(file, 'utf-8');
  fs.unlinkSync(file);
}

fs.appendFileSync('src/index.css', allCss);
