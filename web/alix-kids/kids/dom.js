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

function appendRuns(parent, runs) {
  const standaloneMath = isStandaloneInlineMath(runs);
  for (const run of runs || []) {
    const node = run.math ? mathRunNode(run) : styledRunNode(run, run.text);
    if (standaloneMath && run.math) node.classList.add("math-standalone");
    parent.appendChild(node);
  }
}
function isStandaloneInlineMath(runs) {
  return Array.isArray(runs) && runs.length === 1 &&
    !!runs[0].math && !runs[0].math.display;
}

function appendContextText(parent, run) {
  const parts = String(run.text).split(/(⍰|⬚)/);
  for (const part of parts) {
    if (!part) continue;
    if (part === "⍰" || part === "⬚") {
      const marker = el("span", part === "⍰" ? "hole" : "muted-hole");
      marker.appendChild(styledRunNode(run, part));
      parent.appendChild(marker);
    } else {
      parent.appendChild(styledRunNode(run, part));
    }
  }
}

function contextLine(text, runs) {
  const line = el("div", "rev-context");
  if (!runs) {
    appendContextText(line, { text: text || "" });
    return line;
  }
  const standaloneMath = isStandaloneInlineMath(runs);
  for (const run of runs) {
    if (run.math) {
      const node = mathRunNode(run);
      if (standaloneMath) node.classList.add("math-standalone");
      line.appendChild(node);
    }
    else appendContextText(line, run);
  }
  return line;
}

function appendChecklist(parent, items) {
  const checklist = el("div", "checklist");
  for (const item of items || []) {
    const row = el("div", "checklist-row");
    row.appendChild(el("span", "checklist-box", item.checked ? "☑" : "☐"));
    const text = el("span", "checklist-text");
    if (item.runs) appendRuns(text, item.runs); else text.textContent = item.text || "";
    row.appendChild(text);
    checklist.appendChild(row);
  }
  parent.appendChild(checklist);
}

function appendTable(parent, unit) {
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
    appendRuns(cell, runs);
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
      appendRuns(cell, runs);
      tr.appendChild(cell);
    });
    body.appendChild(tr);
  }
  table.appendChild(body);
  scroll.appendChild(table);
  parent.appendChild(scroll);
}

function frontPrompt(card) {
  const prompt = el("div", "rev-prompt");
  if (!card.front_units) {
    if (card.front_runs) appendRuns(prompt, card.front_runs);
    else prompt.textContent = card.front || "";
    return prompt;
  }
  for (const unit of card.front_units) {
    if (unit.kind === "sentence") {
      const line = el("div", "rev-prompt-line");
      if (unit.runs) appendRuns(line, unit.runs); else line.textContent = unit.text || "";
      prompt.appendChild(line);
    } else if (unit.kind === "code") {
      const pre = el("pre", "why-code");
      pre.appendChild(el("code", null, (unit.lines || []).join("\n")));
      prompt.appendChild(pre);
    } else if (unit.kind === "diagram") {
      const img = el("img", "diagram");
      img.src = unit.src; img.alt = unit.alt || "";
      img.width = unit.width; img.height = unit.height;
      prompt.appendChild(img);
    } else if (unit.kind === "checklist") {
      appendChecklist(prompt, unit.items);
    } else if (unit.kind === "table") {
      appendTable(prompt, unit);
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

return { appendChecklist, appendRuns, appendTable, contextLine, el, frontPrompt, mascot: mascotEl };
}
