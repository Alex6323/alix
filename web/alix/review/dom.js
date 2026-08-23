export function overflowHints({ scrollTop, clientHeight, scrollHeight }, tolerance = 2) {
  const overflows = scrollHeight > clientHeight + tolerance;
  return {
    overflows,
    showTop: overflows && scrollTop > tolerance,
    showBottom: overflows && scrollTop + clientHeight < scrollHeight - tolerance,
  };
}

export function el(tag, cls, text) {
  const node = document.createElement(tag);
  if (cls) node.className = cls;
  if (text !== undefined) node.textContent = text;
  return node;
}

function styledRunNode(run, text) {
  let node = document.createTextNode(text);
  if (run.code) {
    const code = document.createElement("code");
    code.appendChild(node);
    node = code;
  }
  if (run.italic) {
    const italic = document.createElement("em");
    italic.appendChild(node);
    node = italic;
  }
  if (run.strike) {
    const strike = document.createElement("del");
    strike.appendChild(node);
    node = strike;
  }
  if (run.bold) {
    const bold = document.createElement("strong");
    bold.appendChild(node);
    node = bold;
  }
  return node;
}

function mathErrorNode(run, display) {
  const wrap = el("span", `math-run math-error ${display ? "math-display" : "math-inline"}`);
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
  const parsed = new DOMParser().parseFromString(math.svg, "image/svg+xml");
  const root = parsed.documentElement;
  if (root.localName !== "svg" || root.namespaceURI !== "http://www.w3.org/2000/svg" ||
      parsed.querySelector("parsererror")) {
    return mathErrorNode(run, display);
  }
  const wrap = el("span", `math-run ${display ? "math-display" : "math-inline"}`);
  wrap.setAttribute("role", "img");
  wrap.setAttribute("aria-label", run.text);
  const svg = document.importNode(root, true);
  svg.setAttribute("aria-hidden", "true");
  svg.setAttribute("focusable", "false");
  wrap.appendChild(svg);
  return wrap;
}

function isStandaloneInlineMath(runs) {
  return Array.isArray(runs) && runs.length === 1 &&
    !!runs[0].math && !runs[0].math.display;
}

export function appendRuns(parent, runs) {
  const standaloneMath = isStandaloneInlineMath(runs);
  for (const run of runs || []) {
    const node = run.math ? mathRunNode(run) : styledRunNode(run, run.text);
    if (standaloneMath && run.math) node.classList.add("math-standalone");
    parent.appendChild(node);
  }
}

export function appendRunsOrText(parent, text, runs) {
  if (runs) appendRuns(parent, runs);
  else parent.textContent = text || "";
}

export function appendChecklist(parent, items) {
  const checklist = el("div", "checklist");
  for (const item of items || []) {
    const row = el("div", "checklist-row");
    row.appendChild(el("span", "checklist-box", item.checked ? "☑" : "☐"));
    const text = el("span", "checklist-text");
    if (item.runs) appendRuns(text, item.runs);
    else text.textContent = item.text || "";
    row.appendChild(text);
    checklist.appendChild(row);
  }
  parent.appendChild(checklist);
}

export function appendTable(parent, unit) {
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

export function frontEl(text, runs, units) {
  if (units) {
    const wrap = el("div", "front-text multi");
    for (const unit of units) {
      if (unit.kind === "sentence") {
        const line = el("div", "front-line");
        if (unit.runs) appendRuns(line, unit.runs);
        else line.textContent = unit.text || "";
        wrap.appendChild(line);
      } else if (unit.kind === "diagram") {
        const img = el("img", "diagram");
        img.src = unit.src; img.alt = unit.alt || "";
        img.width = unit.width; img.height = unit.height;
        wrap.appendChild(img);
      } else if (unit.kind === "code") {
        const pre = el("pre");
        pre.appendChild(el("code", null, (unit.lines || []).join("\n")));
        wrap.appendChild(pre);
      } else if (unit.kind === "checklist") {
        appendChecklist(wrap, unit.items);
      } else if (unit.kind === "table") {
        appendTable(wrap, unit);
      }
    }
    return wrap;
  }

  const value = text == null ? "" : String(text);
  if (!value.includes("\n")) {
    const front = el("div", "front-text");
    if (runs) appendRuns(front, runs);
    else front.textContent = value;
    return front;
  }

  const wrap = el("div", "front-text multi");
  if (!runs) {
    for (const line of value.split("\n")) wrap.appendChild(el("div", "front-line", line));
    return wrap;
  }
  const lineRuns = [[]];
  for (const run of runs) {
    const parts = String(run.text).split("\n");
    for (let i = 0; i < parts.length; i++) {
      if (parts[i]) lineRuns[lineRuns.length - 1].push({ ...run, text: parts[i] });
      if (i < parts.length - 1) lineRuns.push([]);
    }
  }
  for (const line of lineRuns) {
    const lineNode = el("div", "front-line");
    appendRuns(lineNode, line);
    wrap.appendChild(lineNode);
  }
  return wrap;
}

function closesFence(line, marker) {
  const t = line.trim();
  return t.length >= marker.length && [...t].every((c) => c === marker[0]);
}

// The ONE fence walk (docs/API.md alignment law): fence-shaped units arrive
// in the same document order as the raw fences, the nth closed fence
// consumes the nth unit, and a resolved diagram replaces its fence only
// once the closing marker is within the walked lines; a partial fence
// stays code. `onLine(index)` renders a non-fence line in the caller's
// own style.
function walkFences(parent, lines, units, onLine, makeDiagram) {
  const fenceUnits = (units || []).filter(
    (u) => u.kind === "code" || u.kind === "diagram"
  );
  let fenceIndex = 0;
  let index = 0;
  while (index < lines.length) {
    const fence = lines[index].trim().match(/^(`{3,}|~{3,})/);
    if (fence) {
      const marker = fence[1];
      const code = [];
      index++;
      while (index < lines.length && !closesFence(lines[index], marker)) {
        code.push(lines[index]);
        index++;
      }
      const closed = index < lines.length;
      if (closed) index++;
      const unit = fenceUnits[fenceIndex];
      fenceIndex++;
      if (closed && unit && unit.kind === "diagram") {
        parent.appendChild(makeDiagram ? makeDiagram(unit) : diagramImage(unit));
        continue;
      }
      const pre = el("pre", "code-block");
      pre.textContent = code.join("\n");
      parent.appendChild(pre);
    } else {
      onLine(index);
      index++;
    }
  }
}

// The bare diagram img; a caller with masking policy wraps it (the
// makeDiagram hook). Both accessible texts ride as data so answer-state
// toggles can swap `alt` without a rebuild.
export function diagramImage(unit) {
  const img = el("img", "diagram");
  img.src = unit.src; img.alt = unit.alt || "";
  img.width = unit.width; img.height = unit.height;
  if (unit.revealed_alt) {
    img.dataset.maskedAlt = unit.alt || "";
    img.dataset.revealedAlt = unit.revealed_alt;
  }
  return img;
}

// The three-role vocabulary: an asked region shows the blank glyph, a
// sibling card's mask the hidden glyph, and a cover stays a plain fill:
// it hides answer-giving content and is never a question.
export function maskGlyph(mask, r, prefix) {
  if (r.role === "cover") return;
  mask.appendChild(el("span", prefix + "-mask-glyph", r.role === "asked" ? "\u2370" : "\u2b1a"));
}

// Masks over an uncropped image ride the standard card image: the img
// keeps its shrink-to-fit layout and each mask is positioned in px over
// the PAINTED rect (object-fit contain letterboxes, so the element box
// and the picture disagree), re-synced whenever the img resizes.
export function maskedImage(img, regions, prefix, onAskedGone) {
  const wrap = el("div", prefix + "-wrap");
  wrap.appendChild(img);
  for (const r of regions) {
    const mask = el("div", prefix + "-mask" + (r.reveal_on_answer ? " reveals" : ""));
    maskGlyph(mask, r, prefix);
    mask.dataset.region = JSON.stringify(r);
    wrap.appendChild(mask);
  }
  let warned = false; // sync re-runs on every resize; the notice fires once
  const sync = () => {
    const sw = img.naturalWidth, sh = img.naturalHeight;
    const w = img.clientWidth, h = img.clientHeight;
    if (!sw || !sh || !w || !h) return;
    const scale = Math.min(w / sw, h / sh);
    const ox = img.offsetLeft + (w - sw * scale) / 2;
    const oy = img.offsetTop + (h - sh * scale) / 2;
    for (const mask of wrap.querySelectorAll("[data-region]")) {
      const r = JSON.parse(mask.dataset.region);
      const g = r.unit === "%"
        ? { x: (r.x / 100) * sw, y: (r.y / 100) * sh, w: (r.width / 100) * sw, h: (r.height / 100) * sh }
        : { x: r.x, y: r.y, w: r.width, h: r.height };
      // Partial overlap is valid geometry; the painted mask clips at the
      // source edge instead of floating over neighboring content.
      const x0 = Math.max(0, g.x), y0 = Math.max(0, g.y);
      const x1 = Math.min(sw, g.x + g.w), y1 = Math.min(sh, g.y + g.h);
      const visible = x1 > x0 && y1 > y0;
      if (!visible && r.role === "asked" && !warned) {
        warned = true;
        if (onAskedGone) onAskedGone();
      }
      mask.style.display = visible ? "" : "none";
      mask.style.left = `${ox + x0 * scale}px`;
      mask.style.top = `${oy + y0 * scale}px`;
      mask.style.width = `${(x1 - x0) * scale}px`;
      mask.style.height = `${(y1 - y0) * scale}px`;
    }
  };
  new ResizeObserver(sync).observe(img);
  if (img.complete) sync(); else img.addEventListener("load", sync);
  return wrap;
}

export function appendReveal(parent, lines, runs, isList, units, makeDiagram) {
  walkFences(parent, lines, units, (index) => {
    const line = el("div", "answer");
    if (isList) line.appendChild(document.createTextNode("• "));
    if (runs && runs[index]) appendRuns(line, runs[index]);
    else line.appendChild(document.createTextNode(lines[index]));
    parent.appendChild(line);
  }, makeDiagram);
}

// Context keeps its per-line rendering (blank/hidden glyphs, standalone
// math, the labelling style); only fence slots consume context_units.
export function appendContext(parent, lines, runs, units, cls, makeDiagram) {
  walkFences(parent, lines || [], units, (index) => {
    parent.appendChild(contextLine(lines[index], runs && runs[index], cls));
  }, makeDiagram);
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

export function contextLine(text, runs, cls) {
  const line = el("div", cls || "context");
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
    } else {
      appendContextText(line, run);
    }
  }
  return line;
}

export function renderNote(parent, units) {
  if (!units || units.length === 0) return;
  const note = el("div", "note");
  for (const unit of units) {
    if (unit.kind === "sentence") {
      const paragraph = el("p");
      if (unit.runs) appendRuns(paragraph, unit.runs);
      else paragraph.textContent = unit.text;
      note.appendChild(paragraph);
    } else if (unit.kind === "diagram") {
      const img = el("img", "diagram");
      img.src = unit.src; img.alt = unit.alt || "";
      img.width = unit.width; img.height = unit.height;
      note.appendChild(img);
    } else if (unit.kind === "code") {
      const pre = el("pre");
      pre.appendChild(el("code", null, unit.lines.join("\n")));
      note.appendChild(pre);
    } else if (unit.kind === "checklist") {
      appendChecklist(note, unit.items);
    } else if (unit.kind === "table") {
      appendTable(note, unit);
    }
  }
  parent.appendChild(note);
}

export function appendChoiceOptions(parent, { choices, choiceRuns, onChoose }) {
  parent.classList.add("choices");
  const wrap = el("div", "options");
  (choices || []).forEach((option, index) => {
    const button = el("button", "option");
    button.appendChild(el("span", "num", String(index + 1)));
    const text = el("span", "opt");
    if (choiceRuns && choiceRuns[index]) appendRuns(text, choiceRuns[index]);
    else text.textContent = option;
    button.appendChild(text);
    if (onChoose) button.addEventListener("click", () => onChoose(index));
    wrap.appendChild(button);
  });
  parent.appendChild(wrap);
  return wrap.querySelector(".option");
}

export function appendKeypointList(parent, { keypoints, keypointRuns, marks, cursor, onClick }) {
  const list = el("ul", "kp-list reveal");
  list.appendChild(el("li", "lbl", "did your answer cover these?"));
  (keypoints || []).forEach((point, index) => {
    let cls = "pt";
    if (marks[index] === true) cls += " yes";
    else if (marks[index] === false) cls += " no";
    if (index === cursor) cls += " cur";
    const item = el("li", cls);
    if (keypointRuns && keypointRuns[index]) appendRuns(item, keypointRuns[index]);
    else item.textContent = point;
    if (onClick) {
      item.addEventListener("click", (event) => {
        event.stopPropagation();
        onClick(index);
      });
    }
    list.appendChild(item);
  });
  parent.appendChild(list);
  return list;
}
