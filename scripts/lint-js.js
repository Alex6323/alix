// Syntax-checks the JavaScript embedded in the served HTML assets.
//
// The pages in assets/web/*.html are shipped to the browser as static strings
// baked into the binary, so the Rust build and `cargo test` never parse the JS
// inside them — a stray syntax error there passes every cargo gate and only
// surfaces in a browser. This extracts each inline <script> block and compiles
// it with the JS engine to catch that one class of error. It is a *syntax* check
// only: nothing runs, so browser globals (document, fetch, …) are irrelevant.
//
// Run via `make lint-js` (a no-op when node isn't installed). CommonJS + bare
// module names so it works on any reasonably recent Node without a package.json.

const { readFileSync, readdirSync } = require("fs");
const { Script } = require("vm");
const { join } = require("path");

const dir = "assets/web";
let blocks = 0;
let errors = 0;

const files = readdirSync(dir)
  .filter((f) => f.endsWith(".html"))
  .sort();

for (const file of files) {
  const html = readFileSync(join(dir, file), "utf8");
  // Every <script …>…</script>; the attribute group lets us skip external
  // scripts (src=…), which have nothing inline to check.
  const re = /<script\b([^>]*)>([\s\S]*?)<\/script>/gi;
  let m;
  while ((m = re.exec(html)) !== null) {
    const [, attrs, code] = m;
    if (/\bsrc\s*=/i.test(attrs) || !code.trim()) continue;
    blocks++;
    // The HTML line the block opens on, so an error points back into the file;
    // lineOffset makes the engine's own line numbers line up too.
    const line = html.slice(0, m.index).split("\n").length;
    try {
      new Script(code, { filename: file, lineOffset: line - 1 });
    } catch (err) {
      errors++;
      console.error(`${file}: script at line ${line}: ${err.message}`);
    }
  }
}

if (errors) {
  console.error(`lint-js: ${errors} script block(s) with syntax errors`);
  process.exit(1);
}
console.log(`lint-js: ${blocks} inline script block(s) OK across ${files.length} file(s)`);
