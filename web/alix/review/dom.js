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

export function frontEl(text, runs, units) {
  if (units) {
    const wrap = el("div", "front-text multi");
    for (const unit of units) {
      if (unit.kind === "sentence") {
        const line = el("div", "front-line");
        if (unit.runs) appendRuns(line, unit.runs);
        else line.textContent = unit.text || "";
        wrap.appendChild(line);
      } else if (unit.kind === "code") {
        const pre = el("pre");
        pre.appendChild(el("code", null, (unit.lines || []).join("\n")));
        wrap.appendChild(pre);
      } else if (unit.kind === "checklist") {
        appendChecklist(wrap, unit.items);
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

export function appendReveal(parent, lines, runs, isList) {
  let index = 0;
  while (index < lines.length) {
    const fence = lines[index].trim().match(/^(```|~~~)/);
    if (fence) {
      const marker = fence[1];
      const code = [];
      index++;
      while (index < lines.length && lines[index].trim() !== marker) {
        code.push(lines[index]);
        index++;
      }
      if (index < lines.length) index++;
      const pre = el("pre", "code-block");
      pre.textContent = code.join("\n");
      parent.appendChild(pre);
    } else {
      const line = el("div", "answer");
      if (isList) line.appendChild(document.createTextNode("• "));
      if (runs && runs[index]) appendRuns(line, runs[index]);
      else line.appendChild(document.createTextNode(lines[index]));
      parent.appendChild(line);
      index++;
    }
  }
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
    } else if (unit.kind === "code") {
      const pre = el("pre");
      pre.appendChild(el("code", null, unit.lines.join("\n")));
      note.appendChild(pre);
    } else if (unit.kind === "checklist") {
      appendChecklist(note, unit.items);
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
