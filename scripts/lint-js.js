const { readFileSync, readdirSync } = require("fs");
const { join } = require("path");
const { spawnSync } = require("child_process");
const { Script } = require("vm");

const webDir = "assets/web";
const reviewDir = join(webDir, "review");
const manifest = JSON.parse(readFileSync(join(reviewDir, "manifest.json"), "utf8"));
let blocks = 0;
let modules = 0;
let errors = 0;

function fail(message) {
  errors++;
  console.error(message);
}

function checkedSources(kind, extension) {
  const sources = manifest[kind];
  if (!Array.isArray(sources) || sources.length === 0) {
    fail(`assets/web/review/manifest.json: ${kind} must be a non-empty array`);
    return [];
  }
  for (const source of sources) {
    if (typeof source !== "string" || !/^[A-Za-z0-9._-]+$/.test(source) || !source.endsWith(extension)) {
      fail(`assets/web/review/manifest.json: invalid ${kind} source ${JSON.stringify(source)}`);
    }
  }
  return sources;
}

function checkModule(code, label) {
  const result = spawnSync(process.execPath, ["--input-type=module", "--check"], {
    input: code,
    encoding: "utf8",
  });
  if (result.status !== 0) {
    fail(`${label}: ${result.stderr.trim() || "module syntax check failed"}`);
  }
}

function topLevelNames(code) {
  const names = [];
  const patterns = [
    /^(?:export\s+)?(?:async\s+)?function(?:\s*\*)?\s+([A-Za-z_$][\w$]*)/gm,
    /^(?:export\s+)?class\s+([A-Za-z_$][\w$]*)/gm,
    /^(?:export\s+)?(?:const|let|var)\s+([A-Za-z_$][\w$]*)\s*(?==|;)/gm,
  ];
  for (const pattern of patterns) {
    for (const match of code.matchAll(pattern)) names.push(match[1]);
  }
  return names;
}

const htmlFiles = readdirSync(webDir)
  .filter((file) => file.endsWith(".html"))
  .sort()
  .map((file) => join(webDir, file));
htmlFiles.push(join(webDir, "kids", "kids.html"));

for (const file of htmlFiles) {
  const html = readFileSync(file, "utf8");
  const scripts = /<script\b([^>]*)>([\s\S]*?)<\/script>/gi;
  let match;
  while ((match = scripts.exec(html)) !== null) {
    const [, attributes, code] = match;
    if (/\bsrc\s*=/i.test(attributes) || !code.trim()) continue;
    blocks++;
    const line = html.slice(0, match.index).split("\n").length;
    try {
      new Script(code, { filename: file, lineOffset: line - 1 });
    } catch (error) {
      fail(`${file}: script at line ${line}: ${error.message}`);
    }
  }
}

const cssSources = checkedSources("css", ".css");
for (const source of cssSources) {
  const path = join(reviewDir, source);
  const css = readFileSync(path, "utf8").replace(/\/\*[\s\S]*?\*\//g, "");
  if (/@import\b/i.test(css)) fail(`${path}: @import is forbidden in composed CSS`);
}

const javascriptSources = checkedSources("javascript", ".js");
if (javascriptSources.at(-1) !== "app.js") {
  fail("assets/web/review/manifest.json: app.js must be the last JavaScript source");
}

const declarations = new Map();
const javascriptParts = [];
for (const source of javascriptSources) {
  const path = join(reviewDir, source);
  const code = readFileSync(path, "utf8");
  javascriptParts.push(code);
  modules++;
  if (/^\s*import\b/m.test(code)) fail(`${path}: import declarations are forbidden`);
  checkModule(code, path);
  for (const name of topLevelNames(code)) {
    const first = declarations.get(name);
    if (first && first !== path) {
      fail(`duplicate top-level declaration ${name}: ${first} and ${path}`);
    } else {
      declarations.set(name, path);
    }
  }
}

checkModule(`${javascriptParts.join("\n")}\n`, "composed /review.js");

if (errors) {
  console.error(`lint-js: ${errors} error(s)`);
  process.exit(1);
}
console.log(
  `lint-js: ${blocks} inline block(s), ${modules} standalone module(s), and composed /review.js OK`,
);
