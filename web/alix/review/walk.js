export function createWalk({
  api,
  fetchApi,
  post,
  rerender,
  applyStudy,
  sessionStorage,
  examStart,
  tutor,
  ui,
}) {
  const {
    appendRunsOrText,
    buildCardShell,
    chip,
    deckEl,
    el,
    headerBreadcrumb,
    histEl,
    hit,
    keys,
    label,
    legend,
    legendLeft,
    legendRight,
    menuWrap,
    renderSourceExcerpt,
    requestAnimationFrame,
    scoreEl,
    setMenuContext,
    setTimeout,
    sourceTerms,
    stage,
    updateFade,
  } = ui;
  let data = null;
  let confirmingLeave = false;

  const deltas = [
    { delta: "f", cls: "failed", label: "Missed it", keys: () => keys().failed },
    { delta: "p", cls: "partly", label: "Partly", keys: () => keys().partly },
    { delta: "n", cls: "passed", label: "Got it", keys: () => keys().passed },
  ];

  function isOpen() {
    return data !== null;
  }

  function replace(next) {
    data = next;
    if (!next) confirmingLeave = false;
  }

  function open(next) {
    confirmingLeave = false;
    data = next;
    rerender();
  }

  function applyNext(next) {
    data = next;
    rerender();
    return next;
  }

  function predict(text) {
    return api("/api/walk/predict", post({ text })).then(applyNext);
  }

  function grade(delta) {
    return api("/api/walk/grade", post({ delta })).then(applyNext);
  }

  function restart() {
    return api("/api/walk/restart", post({})).then(applyNext);
  }

  function takeExam() {
    const deck = sessionStorage.getItem("alix.lastDeck") || "";
    return fetchApi("/api/walk/leave", post({})).catch(() => {}).finally(() => {
      replace(null);
      if (deck) examStart(deck);
      else api("/api/state").then(applyStudy);
    });
  }

  function leave() {
    return api("/api/walk/leave", post({})).then((state) => {
      replace(null);
      applyStudy(state);
      return state;
    }).catch(() => {
      replace(null);
      return api("/api/state").then((state) => {
        applyStudy(state);
        return state;
      });
    });
  }

  function backToDecks() {
    if (!data || data.phase === "done") return leave();
    confirmingLeave = true;
    renderLeaveConfirm();
    return undefined;
  }

  function cancelLeave() {
    confirmingLeave = false;
    rerender();
  }

  function renderLeaveConfirm() {
    legend.innerHTML = "";
    legend.appendChild(el("span", "leave-walk-msg", "Leave the walk before finishing the path?"));
    chip("Leave anyway", "again", leave, "enter");
    chip("Stay", "primary", cancelLeave, "esc");
  }

  function buildCard() {
    const current = data;
    const card = buildCardShell({
      pile: current.total - current.current + 1,
      leftAlign: true,
    });
    requestAnimationFrame(() => updateFade(card.a));
    return card;
  }

  function addPrompt(question, text, runs) {
    question.appendChild(el("div", "walk-eyebrow", `checkpoint ${data.current} / ${data.total}`));
    const front = el("div", `front-text${(text || "").length > 110 ? " long" : ""}`);
    appendRunsOrText(front, text, runs);
    question.appendChild(front);
  }

  function render() {
    const current = data;
    headerBreadcrumb();
    deckEl.textContent = "";
    appendRunsOrText(deckEl, current.description || "trace", current.description_runs);
    deckEl.title = current.description || "";
    histEl.textContent = "";
    scoreEl.innerHTML = "";
    menuWrap.style.display = current.phase === "done" ? "none" : "";
    setMenuContext("walk");

    if (confirmingLeave) {
      renderLeaveConfirm();
      return;
    }
    if (tutor.isOpen()) {
      tutor.render({
        q: current.prompt || "",
        qRuns: current.prompt_runs,
        items: current.points || [],
        itemRuns: current.point_runs,
      });
      return;
    }
    if (current.phase === "predict") renderPredict();
    else if (current.phase === "reveal") renderReveal();
    else if (current.phase === "done") renderDone();
    chip(
      "Leave",
      "",
      backToDecks,
      "esc",
      current.phase === "done" ? undefined : legendLeft,
    );
  }

  function renderPredict() {
    const { q, a } = buildCard();
    addPrompt(q, data.prompt, data.prompt_runs);
    if (data.givens && data.givens.length) {
      const givens = el("div", "givens");
      data.givens.forEach((text, index) => {
        const given = el("span", "given");
        appendRunsOrText(given, text, data.given_runs && data.given_runs[index]);
        givens.appendChild(given);
      });
      q.appendChild(givens);
    }
    const compose = el("div", "wcompose");
    const input = el("textarea", "wfield");
    input.placeholder = "Predict the next checkpoint — even a hunch beats “I don’t know.”";
    input.addEventListener("keydown", (event) => {
      if (event.key === "Enter" && (event.shiftKey || event.metaKey || event.ctrlKey)) {
        event.preventDefault();
        submitPredict(input);
      }
    });
    compose.appendChild(input);
    compose.appendChild(el(
      "div",
      "wlede",
      "The gap between your guess and the truth is the learning.  ·  Enter for a new line, Shift+Enter to reveal.",
    ));
    a.appendChild(compose);
    chip("Reveal", "primary", () => submitPredict(input), "⇧↵");
    setTimeout(() => input.focus(), 0);
  }

  function submitPredict(input) {
    const text = input.value.trim();
    if (text) predict(text);
  }

  function renderReveal() {
    const { card, q, a } = buildCard();
    addPrompt(q, data.prompt, data.prompt_runs);
    const prediction = el("div", `predicted${data.prediction ? "" : " empty"}`);
    const predictionLabel = el("span", "wlbl", "you predicted");
    predictionLabel.style.display = "block";
    predictionLabel.style.marginBottom = "4px";
    prediction.appendChild(predictionLabel);
    prediction.appendChild(el("p", null, data.prediction || "(no prediction)"));
    q.appendChild(prediction);

    const head = el("div", "reveal-head");
    const sourceLabel = el("span", "wlbl", "the source");
    sourceLabel.style.color = "var(--bolt)";
    head.appendChild(sourceLabel);
    if (data.locator) head.appendChild(el("span", "at", `at ${data.locator}`));
    a.appendChild(head);
    if (data.excerpt) renderSourceExcerpt(a, data.excerpt, sourceTerms(data.point_runs));
    else a.appendChild(el("div", "excerpt-miss", data.excerpt_error || "no excerpt for this checkpoint"));
    if (data.points && data.points.length) {
      const points = el("ul", "wpoints");
      points.appendChild(el("li", "wlbl", "key points"));
      data.points.forEach((text, index) => {
        const point = el("li", "wpt");
        appendRunsOrText(point, text, data.point_runs && data.point_runs[index]);
        points.appendChild(point);
      });
      a.appendChild(points);
    }
    if (data.note) {
      card.appendChild(el("div", "divider"));
      const noteRegion = el("div", "region n");
      noteRegion.style.textAlign = "left";
      const note = el("div", "note");
      appendRunsOrText(note, data.note, data.note_runs);
      noteRegion.appendChild(note);
      card.appendChild(noteRegion);
    }
    deltas.forEach((delta) => {
      chip(delta.label, delta.cls, () => grade(delta.delta), label(delta.keys()));
    });
    chip("Ask tutor", "ask", tutor.open, label(keys().ask) || "?", legendRight);
  }

  function renderDone() {
    const summary = data.summary || { passed: 0, partly: 0, failed: 0, weak: [], total: 0 };
    const wrap = el("div", "summary");
    wrap.appendChild(el("div", "lede", "walk complete · the drill is done"));
    const description = el("h2");
    appendRunsOrText(description, data.description || "Trace walked.", data.description_runs);
    wrap.appendChild(description);
    const row = (key, value, cls) => {
      const line = el("div", "row");
      line.appendChild(el("span", null, key));
      line.appendChild(el("b", cls || null, value));
      wrap.appendChild(line);
    };
    row("got it", String(summary.passed), "passed");
    row("partly", String(summary.partly), "partly");
    row("missed it", String(summary.failed), "failed");
    if (summary.weak && summary.weak.length) {
      row("weak (resurface sooner)", summary.weak.map((hop) => `#${hop}`).join(" · "));
    } else if (summary.total) {
      row("every checkpoint landed", "✓");
    }
    wrap.appendChild(el(
      "div",
      "lede",
      "Verify it: retrace the whole path in the exam to master this trace.",
    ));
    stage.appendChild(wrap);
    chip("Take the exam", "primary", takeExam, "↵");
    chip("Walk again", "", restart, "");
  }

  function handleKey(event) {
    if (!data) return false;
    if (tutor.isOpen()) {
      if (event.key === "Escape") {
        event.preventDefault();
        tutor.close();
      }
      return true;
    }
    if (confirmingLeave) {
      if (event.key === "Enter") {
        event.preventDefault();
        leave();
      } else if (event.key === "Escape") {
        event.preventDefault();
        cancelLeave();
      }
      return true;
    }
    if (event.key === "Escape") {
      event.preventDefault();
      backToDecks();
      return true;
    }
    if (event.target && /^(TEXTAREA|INPUT)$/.test(event.target.tagName)) return true;
    if (data.phase === "reveal") {
      if (event.key === "?" || hit(event, keys().ask)) {
        event.preventDefault();
        tutor.open();
        return true;
      }
      for (const delta of deltas) {
        if (hit(event, delta.keys())) {
          event.preventDefault();
          grade(delta.delta);
          return true;
        }
      }
    }
    if (hit(event, keys().reveal) || (event.key === "Enter" && !event.ctrlKey)) {
      const primary = legend.querySelector(".chip.primary");
      if (primary && !primary.disabled) {
        event.preventDefault();
        primary.click();
      }
    }
    return true;
  }

  return {
    backToDecks,
    data: () => data,
    grade,
    handleKey,
    isOpen,
    leave,
    open,
    predict,
    render,
    replace,
    restart,
  };
}
