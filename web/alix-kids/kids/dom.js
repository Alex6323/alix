export function kidsOverflowHints({ scrollTop, clientHeight, scrollHeight }, tolerance = 4) {
  return {
    showTop: scrollTop > tolerance,
    showBottom: scrollTop + clientHeight < scrollHeight - tolerance,
  };
}

export function createKidsDom({ document }) {
function el(tag, cls, text) {
  const n = document.createElement(tag);
  if (cls) n.className = cls;
  if (text != null) n.textContent = text;
  return n;
}

function styledRunNode(run, text) {
  let node = document.createTextNode(text);
  if (run.code) { const code = document.createElement("code"); code.appendChild(node); node = code; }
  if (run.italic) { const italic = document.createElement("em"); italic.appendChild(node); node = italic; }
  if (run.link) { const link = document.createElement("a"); link.className = "autolink"; link.appendChild(node); node = link; }
  if (run.sub || run.sup || run.ins) { const tag = document.createElement(run.sub ? "sub" : run.sup ? "sup" : "ins"); tag.appendChild(node); node = tag; }
  if (run.strike) { const strike = document.createElement("del"); strike.appendChild(node); node = strike; }
  if (run.bold) { const bold = document.createElement("strong"); bold.appendChild(node); node = bold; }
  return node;
}

function mathErrorNode(run, display) {
  const wrap = el("span", "math-run math-error " + (display ? "math-display" : "math-inline"));
  wrap.setAttribute("role", "img");
  wrap.setAttribute("aria-label", run.text);
  wrap.appendChild(el("code", "math-error-source", run.text));
  wrap.appendChild(el("span", "math-error-label", "math could not render"));
  return wrap;
}

function mathRunNode(run) {
  const math = run.math || {};
  const display = !!math.display;
  if (!math.svg || math.error) return mathErrorNode(run, display);
  const parsed = new document.defaultView.DOMParser().parseFromString(math.svg, "image/svg+xml");
  const root = parsed.documentElement;
  if (root.localName !== "svg" || root.namespaceURI !== "http://www.w3.org/2000/svg" ||
      parsed.querySelector("parsererror")) {
    return mathErrorNode(run, display);
  }
  const wrap = el("span", "math-run " + (display ? "math-display" : "math-inline"));
  wrap.setAttribute("role", "img");
  wrap.setAttribute("aria-label", run.text);
  const svg = document.importNode(root, true);
  svg.setAttribute("aria-hidden", "true");
  svg.setAttribute("focusable", "false");
  wrap.appendChild(svg);
  return wrap;
}

function appendRuns(parent, runs, blanks = false) {
  const standaloneMath = isStandaloneInlineMath(runs);
  for (const run of runs || []) {
    if (!run.math) {
      appendContextText(parent, run, blanks);
      continue;
    }
    const node = mathRunNode(run);
    if (standaloneMath) node.classList.add("math-standalone");
    parent.appendChild(node);
  }
}
function isStandaloneInlineMath(runs) {
  return Array.isArray(runs) && runs.length === 1 &&
    !!runs[0].math && !runs[0].math.display;
}

function blankPill(marker) {
  const pill = el("span", marker === "⍰" ? "blank-pill" : "blank-pill muted");
  pill.appendChild(el("span", "blank-glyph", marker));
  return pill;
}

function appendContextText(parent, run, blanks = false) {
  if (run.answer) {
    const filled = el("span", "blank-filled");
    filled.appendChild(styledRunNode(run, String(run.text)));
    parent.appendChild(filled);
    return;
  }
  const parts = blanks ? String(run.text).split(/(⍰|⬚)/) : [String(run.text)];
  for (const part of parts) {
    if (!part) continue;
    if (part === "⍰" || part === "⬚") parent.appendChild(blankPill(part));
    else parent.appendChild(styledRunNode(run, part));
  }
}

function codeWithBlanks(lines) {
  const code = el("code");
  for (const part of (lines || []).join("\n").split(/(⍰|⬚)/)) {
    if (!part) continue;
    if (part === "⍰" || part === "⬚") code.appendChild(blankPill(part));
    else code.appendChild(document.createTextNode(part));
  }
  return code;
}

function appendChecklist(parent, items, blanks = false) {
  const checklist = el("div", "checklist");
  for (const item of items || []) {
    const row = el("div", "checklist-row");
    row.appendChild(el("span", "checklist-box", item.checked ? "☑" : "☐"));
    const text = el("span", "checklist-text");
    if (item.runs) appendRuns(text, item.runs, blanks); else text.textContent = item.text || "";
    row.appendChild(text);
    checklist.appendChild(row);
  }
  parent.appendChild(checklist);
}

function appendTable(parent, unit, blanks = false) {
  const scroll = el("div", "table-scroll");
  const table = el("table", "unit-table");
  const aligns = unit.aligns || [];
  const alignCell = (cell, index) => {
    const align = aligns[index];
    if (align && align !== "none") cell.style.textAlign = align;
  };
  const head = el("thead");
  const headRow = el("tr");
  (unit.header || []).forEach((runs, index) => {
    const cell = el("th");
    alignCell(cell, index);
    appendRuns(cell, runs, blanks);
    headRow.appendChild(cell);
  });
  head.appendChild(headRow);
  table.appendChild(head);
  const body = el("tbody");
  for (const row of unit.rows || []) {
    const tr = el("tr");
    row.forEach((runs, index) => {
      const cell = el("td");
      alignCell(cell, index);
      appendRuns(cell, runs, blanks);
      tr.appendChild(cell);
    });
    body.appendChild(tr);
  }
  table.appendChild(body);
  scroll.appendChild(table);
  parent.appendChild(scroll);
}

function runLines(runs) {
  const lines = [[]];
  for (const run of runs) {
    const parts = String(run.text).split("\n");
    for (let i = 0; i < parts.length; i++) {
      if (parts[i]) lines[lines.length - 1].push({ ...run, text: parts[i] });
      if (i < parts.length - 1) lines.push([]);
    }
  }
  return lines;
}

function frontPrompt(card, makeDiagram) {
  const prompt = el("div", "rev-prompt");
  if (!card.front_units) {
    if (!card.front_runs) {
      prompt.textContent = card.front || "";
      return prompt;
    }
    const lines = runLines(card.front_runs);
    if (lines.length === 1) {
      appendRuns(prompt, lines[0], true);
      return prompt;
    }
    for (const line of lines) {
      const node = el("div", "rev-prompt-line");
      appendRuns(node, line, true);
      prompt.appendChild(node);
    }
    return prompt;
  }
  for (const unit of card.front_units) {
    if (unit.kind === "sentence") {
      const line = el("div", "rev-prompt-line");
      if (unit.runs) appendRuns(line, unit.runs, true); else line.textContent = unit.text || "";
      prompt.appendChild(line);
    } else if (unit.kind === "code") {
      const pre = el("pre", "why-code");
      pre.appendChild(codeWithBlanks(unit.lines));
      prompt.appendChild(pre);
    } else if (unit.kind === "diagram") {
      if (makeDiagram) {
        prompt.appendChild(makeDiagram(unit));
      } else {
        const img = el("img", "diagram");
        img.src = unit.src; img.alt = unit.alt || "";
        img.width = unit.width; img.height = unit.height;
        prompt.appendChild(img);
      }
    } else if (unit.kind === "checklist") {
      appendChecklist(prompt, unit.items, true);
    } else if (unit.kind === "table") {
      appendTable(prompt, unit, true);
    } else if (unit.kind === "quote") {
      appendQuote(prompt, unit.units, true);
    }
  }
  return prompt;
}

// The CSS-drawn Alix face. `extra` adds a modifier class (e.g. "mascot-sm" for
// the smaller face in the reveal row).
function mascotEl(extra) {
  const m = el("div", "mascot" + (extra ? " " + extra : ""));
  m.setAttribute("aria-hidden", "true");
  m.appendChild(el("span", "eye l"));
  m.appendChild(el("span", "eye r"));
  m.appendChild(el("span", "smile"));
  return m;
}

// A quoted block renders as its own units inside a blockquote, so a quote
// never shows its `>` marker as text.
function appendQuote(parent, units, blanks = false) {
  const quote = el("blockquote", "quote");
  for (const unit of units || []) {
    if (unit.kind === "sentence") {
      const paragraph = el("p");
      if (unit.runs) appendRuns(paragraph, unit.runs, blanks); else paragraph.textContent = unit.text || "";
      quote.appendChild(paragraph);
    } else if (unit.kind === "code") {
      const pre = el("pre", "why-code");
      pre.appendChild(el("code", null, (unit.lines || []).join("\n")));
      quote.appendChild(pre);
    } else if (unit.kind === "checklist") {
      appendChecklist(quote, unit.items, blanks);
    } else if (unit.kind === "table") {
      appendTable(quote, unit, blanks);
    } else if (unit.kind === "quote") {
      appendQuote(quote, unit.units, blanks);
    }
  }
  parent.appendChild(quote);
}

return { appendChecklist, appendQuote, appendRuns, appendTable, el, frontPrompt, mascot: mascotEl };
}
