// The badge names what is on screen right now, provenance ("new",
// "remediation") first, then the interaction. It never names the *scheduled*
// check: choices on screen are a pick-one whatever the card's schedule will
// use once introduced. An introduction card runs its own ungraded on-ramp, so it
// names that on-ramp (pick one, draw, or reveal) rather than the depth's
// check, which does not run until the card has been met.
export function modeTag({ introducing, choices, mode, draw }) {
  const parts = [];
  if (introducing) parts.push("new");
  if (introducing) {
    parts.push(choices ? "choice" : draw ? "draw" : "reveal");
  } else {
    parts.push(choices ? "choice" : mode === "typeline" ? "typing · line" : mode);
  }
  return parts.join(" · ");
}

export function createStudy({
  api,
  post,
  storage,
  lastDeck,
  openAugment,
  model,
  rerender,
  walkData,
  replaceWalk,
  openTutor,
  startExam,
  closeMenu,
  notice,
  timers,
  ui,
}) {
  const {
    appendChecklist,
    appendChoiceOptions,
    appendKeypointList,
    appendReveal,
    appendRuns,
    appendTable,
    chip,
    diagramImage,
    maskedImage,
    clearLegendSides,
    appendContext,
    computedStyle,
    crumbStrip,
    deckEl,
    document: doc,
    el,
    frontEl,
    headerBreadcrumb,
    histEl,
    hit,
    label,
    legend,
    legendLeft,
    legendRight,
    menuWrap,
    overflowHints,
    renderNote,
    scoreEl,
    setMenuContext,
    stage,
    window: win,
  } = ui;
  let clientModel = model.create(storage);
  let state = clientModel.state;
  let revealed = clientModel.revealed;
  let citationView = clientModel.citationView;
  let sectionView = clientModel.sectionView;
  let answerConcealed = clientModel.answerConcealed;
  let feedback = clientModel.feedback;
  let selectedChoices = new Set();
  let typelineChecked = clientModel.typelineChecked;
  let confirmingLeave = clientModel.confirmingLeave;
  let explainInput = clientModel.explainInput;
  let marks = clientModel.marks;
  let kpCur = clientModel.keypointCursor;
  let drawStrokes = clientModel.drawStrokes;
  let drawSnapshot = clientModel.drawSnapshot;
  let drawTool = clientModel.drawTool;
  let drawCanvasEl = clientModel.drawCanvas;
  let drawToggle = clientModel.drawToggle;
  let duePoll = null;
  // The open summary's countdown repaint, and whether the poll has found work
  // waiting behind it.
  let summaryPaint = null;
  let summaryReady = false;
  let browsing = null;
  let keys = {};
  let browseKeys = {
    next: [{ k: "l", ctrl: false }],
    prev: [{ k: "h", ctrl: false }],
  };

  function currentState() {
    return state;
  }

  function replaceState(next) {
    state = next;
    clientModel = { ...clientModel, state: next };
  }

  function isBrowsing() {
    return browsing !== null;
  }

  function screen() {
    return model.currentScreen({ ...clientModel, state, browsing, walk: walkData() });
  }

  function setKeys(next) {
    keys = next || {};
  }

  function setBrowseKeys(next) {
    if (next) browseKeys = next;
  }

  function load() { return api("/api/state").then(s => { if (s.phase === "browse") { browsing = { cards: s.cards, label: s.label, i: 0 }; state = s; rerender(); } else apply(s); }); }
  // A rejected mutation (a stale revision after a lost reply, or a transport
  // failure) refetches the state, so the next click carries a fresh echo
  // instead of silently doing nothing forever.
  function grade(g)  { api("/api/grade", post({ grade: g })).then(apply).catch(() => load()); }
  function skip()    { api("/api/skip", post({})).then(apply).catch(() => load()); }
  function introduce() { api("/api/introduce", post({})).then(apply).catch(() => load()); }
  function remove()  { api("/api/remove", post({})).then(apply).catch(() => load()); }
  function restart() { api("/api/restart", post({})).then(apply).catch(() => load()); }

  // One heatmap cell's fill: the lib's per-card tier, colored by the CSS
  // class of the same name.
  function paintHeatCell(cellEl, { tier, locked }) {
    cellEl.classList.add(tier === "unseen" ? "empty" : tier);
    if (locked) cellEl.classList.add("locked");
  }
  function syncSaveAlert() {
    let a = doc.getElementById("save-alert");
    const msg = state && state.save_error;
    if (!msg) { if (a) a.remove(); return; }
    if (!a) {
      a = doc.createElement("div");
      a.id = "save-alert";
      doc.body.appendChild(a);
    }
    a.title = msg;
    a.textContent = "Progress is not being saved. Reopen the deck; recent grades may be lost.";
  }

  // Browse a deck read-only: the server builds the card list and returns it; open
  // the in-page browse overlay (no page nav). The picker owns the return target.
  function openBrowse(it) {
    return api("/api/browse", post({ deck: it.name })).then(d => { browsing = { cards: d.cards, label: d.label, i: 0 }; rerender(); return d; });
  }
  function closeBrowse() {
    clientModel = model.enterPicker({ ...clientModel, state, browsing, walk: walkData() });
    state = clientModel.state;
    browsing = clientModel.browsing;
    replaceWalk(clientModel.walk);
    api("/api/deselect", post({})).then(apply);
  }
  function browseGo(delta) { if (!browsing) return; const n = browsing.i + delta; if (n >= 0 && n < browsing.cards.length) { browsing.i = n; rerender(); } }
  function deselect() { confirmingLeave = false; closeMenu(); api("/api/deselect", post({})).then(apply); }

  // Returning to the picker mid-session abandons the cards still queued, so warn
  // first; a finished session (or the select phase) leaves straight away.
  function leaveSession() {
    if (!state || state.phase !== "review") { deselect(); return; }
    confirmingLeave = true;
    renderLeaveConfirm();
  }
  function cancelLeave() { confirmingLeave = false; rerender(); }
  function renderLeaveConfirm() {
    legend.innerHTML = "";
    legend.appendChild(el("span", "leave-msg", `Session not finished: ${state.remaining} card${state.remaining === 1 ? "" : "s"} left.`));
    chip("Leave anyway", "again", deselect, "enter");
    chip("Stay", "primary", cancelLeave, "esc");
  }

  function apply(s) {
    clientModel = model.applyStudyState({
      ...clientModel,
      state,
      browsing,
      walk: walkData(),
      revealed,
      citationView,
      sectionView,
      answerConcealed,
      feedback,
      typelineChecked,
      confirmingLeave,
      explainInput,
      marks,
      keypointCursor: kpCur,
      drawStrokes,
      drawSnapshot,
      drawTool,
      drawCanvas: drawCanvasEl,
    }, s);
    state = clientModel.state;
    replaceWalk(clientModel.walk);
    revealed = clientModel.revealed;
    citationView = clientModel.citationView;
    sectionView = clientModel.sectionView;
    answerConcealed = clientModel.answerConcealed;
    feedback = clientModel.feedback;
    selectedChoices = new Set();
    choiceFocus = -1;
    typelineChecked = clientModel.typelineChecked;
    confirmingLeave = clientModel.confirmingLeave;
    explainInput = clientModel.explainInput;
    marks = clientModel.marks;
    kpCur = clientModel.keypointCursor;
    drawStrokes = clientModel.drawStrokes;
    drawSnapshot = clientModel.drawSnapshot;
    drawTool = clientModel.drawTool;
    drawCanvasEl = clientModel.drawCanvas;
    rerender();
  }
  function hasKeypoints() { return isExplain() && state.keypoints && state.keypoints.length > 0; }
  // A never-seen card: an attempt then reveal, acknowledged with one key (not graded).
  function isIntroducing() { return !!(state && state.introducing); }
  // First encounter as a recognition question (strictly-augmented atomic card).
  function isIntroChoice() { return isIntroducing() && !!state.choices; }
  function isChoice() { return state.mode === "choice" && state.choices; }
  function isMultiChoice() { return isChoice() && state.choices_multiple === true; }
  function isInput() { return state.mode === "typing"; }
  function isTypeLine() { return state.mode === "typeline"; }
  function isExplain() { return state.mode === "explain"; }
  // A genuine Recognize-session MC pick (never true for the introduction on-ramp,
  // which shows its own recognition question regardless of depth).
  function isRecognizeMc() { return !isIntroducing() && isChoice(); }
  // The Recognize fallback: the session is Recognize but no MC could be built
  // (too few distractors) — attempt→reveal with a plain Knew-it/Not-yet call,
  // not the generic three-way grade.
  function isRecognizeFallback() { return !isIntroducing() && state.depth === "recognize" && !state.choices; }
  // Draw is effective when the card is authored draw-only OR the per-device toggle
  // is on — but only for the self-graded modes L1 supports, and never while introducing.
  function effectiveDraw() {
    if (!state || !state.card) return false;
    if (state.card.context && state.card.context.length) return false; // cloze cards don't draw in L1 (a mode-less cloze resolves to flip)
    if (state.mode !== "flip" && state.mode !== "explain") return false;
    return state.input === "draw" || drawToggle;
  }
  function modeLabel() {
    return modeTag({
      introducing: isIntroducing(),
      choices: !!state.choices,
      mode: state.mode,
      draw: effectiveDraw(),
    });
  }
  // A pick's result is pure evidence (the grade is separate, via /api/grade).
  // Every pick shows its feedback screen (chosen + correct options highlighted).
  // The introduction on-ramp's is ungraded — any pick just leads to "Seen". A genuine
  // Recognize pick pauses on Continue: "Next" plus the quiet "I guessed"
  // override when correct, a plain "Continue" (grades failed) when wrong — so a
  // miss shows the right answer before the card moves on, same as any other check.
  function choose(i) {
    // An introduction-card pick reveals its answer feedback: same encounter rule.
    api("/api/choose", post({ index: i, card: state.card.id })).catch(() => { load(); return Promise.reject(); }).then(f => {
      feedback = f;
      // Only the answer changed. A full rerender() rebuilds the question too, which
      // reads as a flicker (and re-rasterises any math in it).
      fillBottom();
      renderLegend();
    });
  }
  function toggleChoice(i) {
    if (selectedChoices.has(i)) selectedChoices.delete(i);
    else selectedChoices.add(i);
    choiceFocus = i;
    fillBottom();
    renderLegend();
  }
  function submitMultiChoice() {
    if (!selectedChoices.size) return;
    const indices = Array.from(selectedChoices).sort((a, b) => a - b);
    api("/api/choose", post({ indices, card: state.card.id })).catch(() => { load(); return Promise.reject(); }).then(f => {
      feedback = f;
      fillBottom();
      renderLegend();
    });
  }
  function submitCheck() {
    const lines = Array.from(doc.querySelectorAll("#ansRegion input.field")).map(i => i.value);
    api("/api/check", post({ lines })).catch(() => { load(); return Promise.reject(); }).then(f => { feedback = f; fillBottom(); renderLegend(); }).catch(() => load());
  }
  // TypeLine: one line at a time; the server derives the position-paired check
  // from the card's own mode. Resubmits every line checked so far (the
  // previously-graded inputs plus the new one) so the server always pairs by
  // true position; the last request's response IS the full result set, so once
  // it covers every back line it doubles as `feedback` for the closing
  // three-way grade.
  function submitTypeLine(value) {
    const lines = typelineChecked.map(r => r.input).concat([value]);
    api("/api/check", post({ lines })).then(f => {
      typelineChecked = f.results;
      if (typelineChecked.length >= backCount()) feedback = f;
      rerender();
    });
  }
  // The "Check" legend chip and the field's own Enter both submit the current
  // (next-unchecked) line's typed value.
  function submitCurrentTypeLine() {
    const inp = doc.querySelector("#ansRegion input.field");
    submitTypeLine(inp ? inp.value : "");
  }

  // The drawer's open/close height animation: quick.
  function backCount() { return state.card ? state.card.back.length : 0; }
  function fullyRevealed() { return backCount() === 0 || revealed >= backCount(); }
  function stopDuePoll() { if (duePoll) { timers.clearInterval(duePoll); duePoll = null; } }
  // The summary is a stopping point: when the poll finds the cooldown spent it
  // arms the Continue chip and repaints the countdown, and the learner decides
  // when to face the next card. Navigating on a timer would drop someone into
  // a graded card they never asked for.
  function startDuePoll() {
    if (duePoll) return;
    duePoll = timers.setInterval(() => {
      if (summaryPaint) summaryPaint();
      api("/api/state").then(s => {
        if (s.phase === "review" && s.card) {
          stopDuePoll();
          summaryReady = true;
          if (summaryPaint) summaryPaint();
        }
      }).catch(() => {});
    }, 3000);
  }

  function isAnswered() {
    if (!state || state.phase !== "review" || !state.card) return false;
    if (feedback) return true;
    return !isChoice() && !isInput() && fullyRevealed();
  }
  function buildCardShell({ pile, cardId = null, ansId = null, withNote = false, moreHint = true, leftAlign = false } = {}) {
    const stack = el("div", "stack");
    stack.dataset.pile = Math.min(3, Math.max(1, pile));
    stack.appendChild(el("div", "peek p2"));
    stack.appendChild(el("div", "peek p1"));
    const card = el("div", "card");
    if (cardId) card.id = cardId;
    const q = el("div", "region q");
    if (leftAlign) q.style.textAlign = "left";
    card.appendChild(q);
    card.appendChild(el("div", "divider"));
    const a = el("div", "region a" + (withNote ? " withnote" : ""));
    if (ansId) a.id = ansId;
    if (leftAlign) a.style.textAlign = "left";
    a.addEventListener("scroll", () => updateFade(a));
    card.appendChild(a);
    if (moreHint) {
      card.appendChild(el("div", "more-hint answer-hint"));      // "more below", pinned to the answer's bottom edge
      card.appendChild(el("div", "more-hint answer-hint top"));  // "more above", pinned to the answer's top edge
    }
    stack.appendChild(card);
    stage.appendChild(stack);
    return { stack, card, q, a };
  }

  function renderCard() {
    const c = state.card;
    const hasNote = c.note && c.note.length > 0;
    const { card, q } = buildCardShell({
      pile: state.remaining,
      cardId: "card",
      ansId: "ansRegion",
      withNote: hasNote,
    });
    // The type badge heads the card (top, centered), above the question — it names the
    // present check (modeLabel), doesn't change within a card, and survives the
    // answer region's re-renders because it lives on the card, not inside it.
    card.insertBefore(el("span", "mode-tag", modeLabel()), card.firstChild);

    // question region
    // Orientation breadcrumb — rendered into the #crumbStrip pinned just below the
    // header hairline, centered: each region is its name (bold = where you are) over a
    // thin per-card heatmap bar, every card a tier cell (see paintHeatCell), so the
    // line doubles as a progress map that greens up as a region is learned.
    if (c.crumb && c.crumb.regions.length) {
      const cr = c.crumb;
      const bc = el("div", "crumb");
      for (let i = 0; i < cr.regions.length; i++) {
        const reg = el("div", "crumb-region" + (i === cr.current ? " cur" : ""));
        reg.appendChild(el("div", "crumb-name", cr.regions[i]));
        const bar = el("div", "crumb-bar");
        for (const s of (cr.cells && cr.cells[i]) || []) {
          const cell = el("span", "crumb-cell");
          paintHeatCell(cell, s);
          bar.appendChild(cell);
        }
        reg.appendChild(bar);
        bc.appendChild(reg);
      }
      crumbStrip.appendChild(bc);
    }
    const frontNode = frontEl(c.front, c.front_runs, c.front_units);
    // Where context is the question (a cloze sentence) it leads and the front
    // steps back to a topic; where it only labels the front (a table title) the
    // front keeps the lead and the label sits above it.
    if (c.context.length && c.context_leads) frontNode.classList.add("topic");
    if (!c.context_leads) {
      appendContext(q, c.context, c.context_runs, c.context_units, "context label", contextDiagram);
    }
    q.appendChild(frontNode);
    if (c.context_leads) {
      appendContext(q, c.context, c.context_runs, c.context_units, undefined, contextDiagram);
    }
    appendImages(q, c.images);

    // The answer region (id="ansRegion") is filled by fillBottom(); the note region
    // and its divider are added only once the note is shown (see setNote); the answer
    // is capped shorter only when the card has a note, to leave room for it.
    fillBottom();
    renderLegend();
  }

  // Read-only browse: step through every card in a deck — front, the revealed
  // answer (with the format reshape's bullets + notes), no grading. An in-page
  // overlay reached from the picker's Browse action or `alix browse --serve`;
  // there is no separate /browse page.
  function renderBrowse() {
    headerBreadcrumb();
    deckEl.textContent = browsing.label;
    histEl.textContent = "";
    scoreEl.innerHTML = `<span class="left">${browsing.i + 1} / ${browsing.cards.length}</span>`;
    menuWrap.style.display = "none";
    const c = browsing.cards[browsing.i];
    const hasNote = c.note && c.note.length > 0;

    const { card, q, a } = buildCardShell({
      pile: browsing.cards.length - browsing.i,
      withNote: hasNote,
      moreHint: false, // browse has no overflow marker
    });

    const frontNode = frontEl(c.front, c.front_runs, c.front_units);
    // Where context is the question (a cloze sentence) it leads and the front
    // steps back to a topic; where it only labels the front (a table title) the
    // front keeps the lead and the label sits above it.
    if (c.context.length && c.context_leads) frontNode.classList.add("topic");
    if (!c.context_leads) {
      appendContext(q, c.context, c.context_runs, c.context_units, "context label", browseDiagram);
    }
    q.appendChild(frontNode);
    if (c.context_leads) {
      appendContext(q, c.context, c.context_runs, c.context_units, undefined, browseDiagram);
    }
    appendImages(q, c.images);

    const sec = el("div", "reveal" + (leftAlignAnswer(c) ? " list" : ""));
    if (isReshapedList(c)) appendReveal(sec, c.back, c.back_runs, true, c.back_units);
    else appendAnswerUnits(sec, c.back_units);
    a.appendChild(sec);
    appendImages(a, c.images_back);
    a.classList.add("has-body"); // browse always shows the full answer
    card.classList.add("answered"); // and reveal-on-answer masks lift with it

    if (hasNote) {
      card.appendChild(el("div", "divider"));
      const n = el("div", "region n");
      renderNote(n, c.note);
      card.appendChild(n);
    }

    updateFade(a); // content-aware placement: center a short answer, top-align a long one

    chip("Prev", "", () => browseGo(-1), label(browseKeys.prev)).disabled = browsing.i === 0;
    chip("Next", "primary", () => browseGo(1), label(browseKeys.next)).disabled = browsing.i >= browsing.cards.length - 1;
    chip("Leave", "", closeBrowse, "esc");
  }

  function appendRegionToggle(parent, className, title, icon, key, action) {
    const toggle = el("button", `region-toggle ${className}`);
    toggle.type = "button";
    toggle.title = title;
    toggle.setAttribute("aria-label", title);
    toggle.appendChild(el("span", "ci", icon));
    toggle.appendChild(el("span", "k", key));
    toggle.addEventListener("click", event => {
      event.stopPropagation();
      action();
    });
    parent.appendChild(toggle);
  }

  function noteVisibleForCurrentCard() {
    if (isIntroducing()) {
      if (effectiveDraw()) return revealed > 0;
      if (isIntroChoice()) return !!feedback;
      return revealed > 0;
    }
    if (feedback) return true;
    if (isChoice() || isInput() || isTypeLine()) return false;
    return fullyRevealed();
  }

  function fillBottom() {
    const a = doc.getElementById("ansRegion");
    if (!a) return;
    a.innerHTML = "";
    const citations = state.card.citations || [];
    const citable = citations.length > 0 && isAnswered();
    const showingSection = hasSection() && sectionView;
    if (showingSection) {
      appendContext(
        a,
        state.card.section_context,
        state.card.section_context_runs,
        state.card.section_context_units,
        "context section",
        contextDiagram,
      );
      setNote(noteVisibleForCurrentCard());
    } else if (citable && citationView) {
      // Source view: all cited excerpts take the answer's place in authored order.
      renderSourceCitations(a, citations);
      setNote(true);
    } else if (isIntroducing()) {
      if (effectiveDraw()) {
        // Attempt-first, ungraded: draw your answer, then reveal it to compare.
        if (revealed === 0) { renderDrawCanvas(a); setNote(false); }
        else {
          renderDrawComparison(a, fillIntroduction);
          setNote(true);
        }
      } else if (isIntroChoice()) {
        if (feedback) renderChoiceFeedback(a); else renderChoices(a);
        setNote(!!feedback);
      } else if (revealed > 0) {
        fillIntroduction(a); setNote(true);          // recall: answer shown after reveal
      } else {
        a.appendChild(el("div", "introduction-hint", "new card: try to recall it, then reveal."));
        setNote(false);                          // recall: front only until revealed
      }
    }
    else if (feedback) { (isChoice() ? renderChoiceFeedback : renderCheckFeedback)(a); setNote(true); }
    else if (isChoice()) { renderChoices(a); setNote(false); }
    else if (isInput()) { renderInput(a); setNote(false); }
    else if (isTypeLine()) { renderTypeLine(a); setNote(false); }
    else if (effectiveDraw()) {
      if (revealed === 0) { renderDrawCanvas(a); setNote(false); }
      else {
        renderDrawComparison(a, isExplain() ? renderExplain : fillAnswer);
        setNote(true);
      }
    }
    else if (isExplain()) { renderExplain(a); setNote(fullyRevealed()); }
    else { fillAnswer(a); setNote(fullyRevealed()); }
    const cardEl = doc.getElementById("card");
    if (cardEl) {
      cardEl.classList.toggle("answered", isAnswered());
      syncDiagramAnswerState(cardEl);
    }
    const toggles = el("div", "region-toggles");
    const citationActive = citable && !showingSection;
    a.classList.toggle("citable", citationActive);
    a.classList.toggle("sectioned", showingSection);
    a.onclick = showingSection ? onSectionClick : citationActive ? onCiteClick : null;
    if (hasSection()) {
      appendRegionToggle(
        toggles,
        "section-toggle",
        showingSection ? "hide section context" : "show section context",
        "§",
        "c",
        toggleSection,
      );
    }
    if (citationActive) {
      const title = citationView
        ? "show answer"
        : citations.length === 1
          ? "show source " + citations[0].locator
          : `show ${citations.length} sources`;
      appendRegionToggle(toggles, "cite-toggle", title, citationView ? "¶" : "</>", "s", toggleCitation);
    }
    // Introduction recall: the same corner-cue mechanism as the source swap, here hiding /
    // un-hiding the revealed answer in place so you can self-test the encoding. `h` (or
    // a tap on the region) flips it both ways. Shown only once the answer is revealed
    // (nothing to hide before then), and never on a cited card — citation owns the corner.
    const hidable = !showingSection && isIntroducing() && !effectiveDraw() && !isIntroChoice()
      && citations.length === 0 && revealed > 0;
    a.classList.toggle("hidable", hidable);
    a.classList.toggle("concealed", hidable && answerConcealed);
    if (hidable) {
      a.onclick = onIntroToggleClick;
      appendRegionToggle(
        toggles,
        "cite-toggle",
        answerConcealed ? "show answer" : "hide the answer to self-test",
        answerConcealed ? "⊙" : "⊘",
        "h",
        introToggle,
      );
    }
    if (toggles.childElementCount) a.appendChild(toggles);
    // keep the newest visible line in view without scrolling into the hidden
    // footprint reserved for later lines
    if (state.mode === "line" && revealed > 0) {
      const visible = a.querySelectorAll(".reveal.line .answer:not(.pending):not(.line-reserve)");
      const newest = visible[visible.length - 1];
      if (newest) {
        const bottom = newest.offsetTop + newest.offsetHeight;
        if (bottom > a.scrollTop + a.clientHeight) a.scrollTop = bottom - a.clientHeight;
      }
    }
    // Content-aware placement (applied by updateFade): a short answer centers below
    // the midline once there's real body content, sitting clearly separated from the
    // prompt. A line card reserves its final footprint, so the block centers once
    // and revealed lines grow downward. If it overflows into `filled`, the per-line
    // auto-scroll (above) keeps the newest line reachable. A short cited
    // source follows the same centering rule; a long one overflows into `filled`
    // and stays top-aligned and scrollable. The pre-reveal badge/hint alone isn't
    // body to center.
    a.classList.toggle("has-body", !!a.querySelector(
      ".reveal, .options, .inputs, .source-excerpt, .context.section, .kp-list, .explain-answer, img.card-img, .cite-err"));
    updateFade(a);
  }

  function hasSection() {
    return !!(state && state.card && (state.card.section_context || []).length);
  }

  function toggleSection() {
    if (!hasSection()) return;
    sectionView = !sectionView;
    fillBottom();
  }

  function onSectionClick() {
    if (win.getSelection && String(win.getSelection())) return;
    toggleSection();
  }

  // Swap the answer region between the worded answer and the cited source excerpt.
  function toggleCitation() {
    if (!state || !state.card || !(state.card.citations || []).length || !isAnswered()) return;
    citationView = !citationView;
    fillBottom();
  }
  // Click anywhere in the answer/excerpt swaps it — but don't hijack a drag that's
  // selecting text (e.g. copying a line of the excerpt).
  function onCiteClick() {
    if (win.getSelection && String(win.getSelection())) return;
    toggleCitation();
  }

  // Show a soft fade on whichever edge of the answer region hides content,
  // instead of a scrollbar.
  // Count source-excerpt lines fully below the region's visible bottom, so the
  // marker can say how much more there is.
  function hiddenLineCount(a) {
    const lns = a.querySelectorAll(".source-line");
    if (!lns.length) return 0;
    const foldY = a.getBoundingClientRect().bottom;
    let n = 0;
    lns.forEach(ln => { if (ln.getBoundingClientRect().top >= foldY - 4) n++; });
    return n;
  }

  function updateFade(a) {
    if (!a) return;
    // Content-aware placement: center a short answer that fits (it settles below the
    // midline), top-align one that overflows so its top stays reachable. `has-body`
    // (set by fillBottom) gates it, so a badge-only pre-reveal region isn't centered.
    const hints = overflowHints(a);
    const hasBody = a.classList.contains("has-body");
    a.classList.toggle("balanced", hasBody && !hints.overflows);
    a.classList.toggle("filled", !hasBody || hints.overflows);
    a.classList.toggle("fade-top", hints.showTop);
    a.classList.toggle("fade-bottom", hints.showBottom);
    const n = hiddenLineCount(a);
    updateHintPills(
      a,
      hints,
      "answer-hint",
      n > 0 ? `⌵ ${n} more line${n === 1 ? "" : "s"}` : "⌵ more below",
    );
  }

  function updateHintPills(region, hints, hintClass, belowText) {
    const parent = region.parentElement;
    if (!parent) return;
    const below = parent.querySelector(`.${hintClass}:not(.top)`);
    const above = parent.querySelector(`.${hintClass}.top`);
    const cardH = region.offsetParent ? region.offsetParent.clientHeight : parent.clientHeight;
    const regionTop = region.offsetTop;
    const regionBottom = region.offsetTop + region.offsetHeight;
    if (below) {
      below.style.bottom = Math.max(0, cardH - regionBottom + 8) + "px";
      if (hints.showBottom) {
        below.textContent = belowText;
        below.classList.add("show");
      } else below.classList.remove("show");
    }
    if (above) {
      above.style.top = (regionTop + 8) + "px";
      if (hints.showTop) { above.textContent = "⌃ more above"; above.classList.add("show"); }
      else above.classList.remove("show");
    }
  }

  function updateNoteFade(n) {
    if (!n) return;
    const hints = overflowHints(n);
    n.classList.toggle("fade-top", hints.showTop);
    n.classList.toggle("fade-bottom", hints.showBottom);
    updateHintPills(n, hints, "note-hint", "⌵ more below");
  }

  // Adds (or removes) the note region and a divider before it. Because content is
  // top-aligned, adding it on reveal doesn't shift the question or answer, so it
  // only needs to exist while shown — no premature empty zone or stray divider.
  function setNote(show) {
    const card = doc.getElementById("card");
    if (!card) return;
    let divider = doc.getElementById("noteDivider");
    let n = doc.getElementById("noteRegion");
    const has = state.card.note && state.card.note.length > 0;
    if (show && has) {
      if (!divider) { divider = el("div", "divider"); divider.id = "noteDivider"; card.appendChild(divider); }
      if (!n) {
        n = el("div", "region n");
        n.id = "noteRegion";
        n.addEventListener("scroll", () => updateNoteFade(n));
        card.appendChild(n);
      }
      if (!card.querySelector(".note-hint:not(.top)")) {
        card.appendChild(el("div", "more-hint note-hint"));
        card.appendChild(el("div", "more-hint note-hint top"));
      }
      n.innerHTML = "";
      renderNote(n, state.card.note);
      updateNoteFade(n);
    } else {
      if (divider) divider.remove();
      if (n) n.remove();
      card.querySelectorAll(".note-hint").forEach(hint => hint.remove());
    }
  }

  // Inline-code runs in key points are an explicit author signal: highlight those
  // exact, case-sensitive terms in the source without guessing from prose.
  function sourceTerms(runGroups) {
    const terms = new Set();
    for (const runs of runGroups || []) {
      for (const run of runs || []) {
        if (run && run.code && run.text && run.text.trim()) terms.add(run.text);
      }
    }
    return Array.from(terms).sort((a, b) => b.length - a.length || a.localeCompare(b));
  }

  function appendSourceText(parent, text, terms) {
    let cursor = 0;
    while (cursor < text.length) {
      let nextAt = -1;
      let nextTerm = "";
      for (const term of terms || []) {
        const at = text.indexOf(term, cursor);
        if (at >= 0 && (nextAt < 0 || at < nextAt || (at === nextAt && term.length > nextTerm.length))) {
          nextAt = at;
          nextTerm = term;
        }
      }
      if (nextAt < 0) {
        parent.appendChild(doc.createTextNode(text.slice(cursor)));
        return;
      }
      if (nextAt > cursor) parent.appendChild(doc.createTextNode(text.slice(cursor, nextAt)));
      parent.appendChild(el("mark", "source-term", nextTerm));
      cursor = nextAt + nextTerm.length;
    }
  }

  // One editor-style source excerpt shared by fact citations and trace reveals.
  function renderSourceExcerpt(parent, ex, terms) {
    const panel = el("div", "source-excerpt");
    panel.appendChild(el("div", "source-file", ex.path));
    const code = el("div", "source-code");
    for (const ln of ex.lines) {
      const row = el("div", "source-line");
      row.appendChild(el("span", "source-number", String(ln.n)));
      const text = el("span", "source-text");
      appendSourceText(text, ln.text, terms);
      row.appendChild(text);
      code.appendChild(row);
    }
    panel.appendChild(code);
    if (ex.truncated) panel.appendChild(el("div", "source-truncated", "… excerpt truncated"));
    parent.appendChild(panel);
  }

  function renderSourceCitations(parent, citations) {
    const stack = el("div", "source-stack");
    for (const citation of citations || []) {
      if (citation.excerpt) renderSourceExcerpt(stack, citation.excerpt);
      else {
        stack.appendChild(el(
          "div",
          "cite-err",
          `⚠ ${citation.locator}: ${citation.error || "source unavailable"}`,
        ));
      }
    }
    parent.appendChild(stack);
  }

  // Tappable multiple-choice options.
  // The keyboard-focused MC option (up/down via the configured nav keys or arrows),
  // starting on the first. Enter picks the focused one; number keys and click still work.
  let choiceFocus = -1;
  function moveChoiceFocus(delta) {
    const opts = doc.querySelectorAll(".options .option");
    if (!opts.length) return;
    choiceFocus = choiceFocus < 0
      ? (delta > 0 ? 0 : opts.length - 1)
      : (choiceFocus + delta + opts.length) % opts.length;
    opts.forEach((o, i) => o.classList.toggle("focused", i === choiceFocus));
    opts[choiceFocus].scrollIntoView({ block: "nearest" });
  }
  function renderChoices(a) {
    const previousFocus = isMultiChoice() ? choiceFocus : -1;
    choiceFocus = -1;
    const first = appendChoiceOptions(a, {
      choices: state.choices,
      choiceRuns: state.choice_runs,
      onChoose: isMultiChoice() ? toggleChoice : choose,
    });
    const options = a.querySelectorAll(".options .option");
    if (isMultiChoice()) {
      options.forEach((option, index) => {
        const selected = selectedChoices.has(index);
        option.classList.toggle("selected", selected);
        option.setAttribute("aria-pressed", String(selected));
      });
    }
    if (first) {
      choiceFocus = previousFocus >= 0 && previousFocus < options.length ? previousFocus : 0;
      options[choiceFocus].classList.add("focused");
    }
  }

  // Typing: an input per answer line, submitted with Enter or the chip.
  function renderInput(a) {
    const wrap = el("div", "inputs");
    state.card.back.forEach(() => {
      const inp = el("input", "field");
      inp.type = "text"; inp.autocomplete = "off"; inp.spellcheck = false;
      inp.addEventListener("keydown", e => { if (e.key === "Enter") { e.preventDefault(); submitCheck(); } });
      wrap.appendChild(inp);
    });
    a.appendChild(wrap);
    const first = wrap.querySelector("input");
    if (first) first.focus();
  }

  // One checked typed line: a trailing ✓ (green) or ✗ (red); a wrong line also
  // shows the expected answer beneath it (a miss — typo or genuinely wrong, the
  // learner decides which — isn't memorised silently). Shared by the plain typed
  // check's full-card feedback and TypeLine's line-by-line history.
  function checkedLine(wrap, r) {
    const line = el("div", "answer" + (r.passed ? " pass" : " miss"));
    line.appendChild(el("span", "txt", r.input || "—"));
    line.appendChild(el("span", "mark", r.passed ? "✓" : "✗"));
    wrap.appendChild(line);
    if (!r.passed) wrap.appendChild(el("div", "expected", r.expected));
  }

  // The typed answer after submitting: every line, checked.
  function renderCheckFeedback(a) {
    const wrap = el("div", "reveal");
    feedback.results.forEach(r => checkedLine(wrap, r));
    a.appendChild(wrap);
  }

  // TypeLine: the lines confirmed so far (checked, ✓/✗ + expected), then one
  // input for the next line. Progressive — never tracks more than the server's
  // own `results` tells us; the last line's check doubles as the closing
  // `feedback` (see `submitTypeLine`), so this only renders the in-progress state.
  function renderTypeLine(a) {
    const wrap = el("div", "reveal");
    typelineChecked.forEach(r => checkedLine(wrap, r));
    a.appendChild(wrap);
    const inputs = el("div", "inputs");
    const inp = el("input", "field");
    inp.type = "text"; inp.autocomplete = "off"; inp.spellcheck = false;
    inp.addEventListener("keydown", e => { if (e.key === "Enter") { e.preventDefault(); submitTypeLine(inp.value); } });
    inputs.appendChild(inp);
    a.appendChild(inputs);
    inp.focus();
  }

  // The options after answering: correct in green, a wrong pick in red.
  function renderChoiceFeedback(a) {
    a.classList.add("choices");
    const wrap = el("div", "options");
    const chosen = isMultiChoice() ? new Set(feedback.chosen) : null;
    const correct = isMultiChoice() ? new Set(feedback.correct) : null;
    state.choices.forEach((opt, i) => {
      let cls = "option";
      const wasChosen = chosen ? chosen.has(i) : false;
      const wasCorrect = correct ? correct.has(i) : false;
      if (isMultiChoice()) {
        if (wasCorrect) cls += " correct";
        else if (wasChosen) cls += " wrong";
        else cls += " dim";
      } else if (i === feedback.correct) cls += " correct";
      else if (i === feedback.chosen) cls += " wrong";
      else cls += " dim";
      const row = el("div", cls);
      row.appendChild(el("span", "num", String(i + 1)));
      const text = el("span", "opt");
      if (state.choice_runs && state.choice_runs[i]) appendRuns(text, state.choice_runs[i]);
      else text.textContent = opt;
      row.appendChild(text);
      if (isMultiChoice() && (wasChosen || wasCorrect)) {
        const status = wasChosen
          ? (wasCorrect ? "chosen · correct" : "chosen · incorrect")
          : "correct";
        row.appendChild(el("span", "choice-status", status));
      }
      wrap.appendChild(row);
    });
    a.appendChild(wrap);
  }

  // Explain (understanding) cards: before reveal a free textarea (optional); after
  // reveal your answer beside the key points (the back lines). Self-graded — the
  // typed text never leaves the client.
  function renderExplain(a) {
    if (revealed === 0) {
      const ta = el("textarea", "explain-input");
      ta.placeholder = "Type your answer… (Shift+Enter to reveal)";
      ta.rows = 3;
      ta.value = explainInput;
      ta.addEventListener("input", () => { explainInput = ta.value; });
      ta.addEventListener("keydown", e => {
        // Enter inserts a newline (compose freely); Shift+Enter reveals. Stop the
        // event here so the same keypress doesn't also reach the doc handler
        // and submit the (now visible) checklist in one go.
        if (e.key === "Enter" && e.shiftKey) { e.preventDefault(); e.stopPropagation(); explainReveal(); }
      });
      a.appendChild(ta);
      timers.setTimeout(() => ta.focus(), 0);
      return;
    }
    if (explainInput.trim()) {
      a.appendChild(el("div", "explain-label", "your answer"));
      a.appendChild(el("div", "explain-answer", explainInput));
    }
    // With cached key points, the reveal is the same green ▸ list a trace walk
    // shows — but you walk it top to bottom marking each yes/no, and the coverage
    // derives the grade. Otherwise show the plain back lines.
    if (hasKeypoints()) {
      if (marks.length !== state.keypoints.length) marks = state.keypoints.map(() => undefined);
      // The authored answer (the ground truth) first, then the checklist of the
      // claims it makes — the key points are a decomposition, not a replacement.
      a.appendChild(el("div", "explain-label", "the answer"));
      const ans = el("div", "reveal");
      appendReveal(ans, state.card.back, state.card.back_runs, false, state.card.back_units);
      a.appendChild(ans);
      appendKeypointList(a, {
        keypoints: state.keypoints,
        keypointRuns: state.keypoint_runs,
        marks,
        cursor: kpCur,
        onClick: clickKeypoint,
      });
      return;
    }
    a.appendChild(el("div", "explain-label", "your answer should cover"));
    const pts = el("div", "reveal");
    appendReveal(pts, state.card.back, state.card.back_runs, true, state.card.back_units);
    a.appendChild(pts);
  }
  // Walk the key-point list: mark the current point yes/no and advance; move the
  // cursor; click toggles a point (and lands the cursor on it).
  function answerKeypoint(val) { marks[kpCur] = val; kpCur = Math.min(kpCur + 1, marks.length - 1); fillBottom(); renderLegend(); }
  function moveKeypoint(d) { kpCur = Math.max(0, Math.min(kpCur + d, marks.length - 1)); fillBottom(); }
  function clickKeypoint(i) { kpCur = i; marks[i] = marks[i] === true ? false : true; fillBottom(); renderLegend(); }
  function keypointsYes() { return marks.filter(m => m === true).length; }
  function keypointsAnswered() { return marks.length > 0 && marks.every(m => m !== undefined); }
  // Display-only mirror of the lib's keypoint_grade (the server stays authoritative
  // on submit) — recomputed each render, so re-toggling a point updates the verdict.
  function keypointVerdict() {
    const yes = keypointsYes(), total = state.keypoints.length;
    return (total === 0 || yes >= total) ? "passed" : yes === 0 ? "failed" : "partly";
  }
  // Submit: the server derives failed/partly/passed from how many points were
  // covered (the one keypoint_grade rule, in the lib). Unanswered = not covered.
  function submitKeypoints() {
    api("/api/grade", post({ covered: keypointsYes(), total: state.keypoints.length })).then(apply).catch(() => load());
  }
  function explainReveal() {
    const ta = doc.querySelector(".explain-input");
    if (ta) explainInput = ta.value;
    revealed = backCount();
    fillBottom();
    renderLegend();
  }

  // ── draw input ──────────────────────────────────────────────────────
  // The canvas you draw/handwrite on, with Pen · Eraser · Undo · Clear. Strokes
  // live in `drawStrokes`; the drawing is ephemeral — snapshotted on reveal for
  // side-by-side comparison, never persisted or sent to the server.
  const ERASER_WIDTH = 40;                        // eraser stroke width, and the diameter of its cursor ring
  function renderDrawCanvas(a) {
    const wrap = el("div", "draw-wrap");
    const canvas = el("canvas", "draw-canvas");
    wrap.appendChild(canvas);
    const ring = el("div", "eraser-ring");        // shows the eraser's size/position; hidden until the eraser is over the canvas
    ring.style.width = ring.style.height = ERASER_WIDTH + "px";
    wrap.appendChild(ring);
    const tools = el("div", "draw-tools");
    const pen = drawToolBtn("Pen", () => setDrawTool("pen"));
    const erase = drawToolBtn("Eraser", () => setDrawTool("erase"));
    pen.classList.toggle("on", drawTool === "pen");
    erase.classList.toggle("on", drawTool === "erase");
    tools.appendChild(pen);
    tools.appendChild(erase);
    tools.appendChild(drawToolBtn("Undo", drawUndo));
    tools.appendChild(drawToolBtn("Clear", drawClear));
    wrap.appendChild(tools);
    a.appendChild(wrap);
    setupDrawCanvas(canvas, ring);
  }
  function drawToolBtn(text, onClick) {
    const b = el("button", "draw-tool", text);
    b.type = "button";
    b.addEventListener("click", e => { e.preventDefault(); e.stopPropagation(); onClick(); });
    return b;
  }
  function setDrawTool(t) { drawTool = t; rerender(); }
  function drawUndo() { drawStrokes.pop(); redrawStrokes(); }
  function drawClear() { drawStrokes = []; redrawStrokes(); }

  // Size the canvas to its box (crisp under devicePixelRatio), wire pointer
  // drawing (pen/touch/mouse), and replay existing strokes.
  function setupDrawCanvas(canvas, ring) {
    drawCanvasEl = canvas;
    const dpr = win.devicePixelRatio || 1;
    const rect = canvas.getBoundingClientRect();
    canvas.width = Math.max(1, Math.round(rect.width * dpr));
    canvas.height = Math.max(1, Math.round(rect.height * dpr));
    const ctx = canvas.getContext("2d");
    ctx.scale(dpr, dpr);
    ctx.lineCap = "round";
    ctx.lineJoin = "round";
    canvas._ctx = ctx;
    // In eraser mode the ring stands in for the cursor; the pen keeps a crosshair.
    canvas.style.cursor = drawTool === "erase" ? "none" : "crosshair";
    redrawStrokes();
    let cur = null;
    const pos = e => { const r = canvas.getBoundingClientRect(); return { x: e.clientX - r.left, y: e.clientY - r.top }; };
    const moveRing = e => {
      if (drawTool !== "erase") { ring.style.display = "none"; return; }
      const p = pos(e);
      ring.style.left = p.x + "px";
      ring.style.top = p.y + "px";
      ring.style.display = "block";
    };
    const hideRing = () => { ring.style.display = "none"; };
    canvas.addEventListener("pointerdown", e => {
      e.preventDefault();
      try { canvas.setPointerCapture(e.pointerId); } catch (err) {}
      cur = { tool: drawTool, points: [pos(e)] };
      drawStrokes.push(cur);
      moveRing(e);
    });
    canvas.addEventListener("pointermove", e => {
      moveRing(e);
      if (!cur) return;
      cur.points.push(pos(e));
      drawSeg(ctx, cur);
    });
    canvas.addEventListener("pointerenter", moveRing);
    const end = () => { cur = null; };
    canvas.addEventListener("pointerup", end);
    canvas.addEventListener("pointercancel", () => { end(); hideRing(); });
    canvas.addEventListener("pointerleave", () => { end(); hideRing(); });
  }
  // Ink is the theme's --ink; the eraser cuts pixels with destination-out.
  function drawStyle(ctx, tool) {
    ctx.globalCompositeOperation = tool === "erase" ? "destination-out" : "source-over";
    ctx.strokeStyle = computedStyle(doc.documentElement).getPropertyValue("--ink").trim() || "#e6e6e6";
    ctx.lineWidth = tool === "erase" ? ERASER_WIDTH : 2.5;
  }
  // Draw the newest segment (incremental, so live strokes are smooth).
  function drawSeg(ctx, s) {
    const p = s.points;
    if (p.length < 2) return;
    drawStyle(ctx, s.tool);
    const a = p[p.length - 2], b = p[p.length - 1];
    ctx.beginPath(); ctx.moveTo(a.x, a.y); ctx.lineTo(b.x, b.y); ctx.stroke();
  }
  // Replay every stroke onto a cleared canvas (undo / clear).
  function redrawStrokes() {
    const canvas = drawCanvasEl;
    if (!canvas || !canvas._ctx) return;
    const ctx = canvas._ctx, dpr = win.devicePixelRatio || 1;
    ctx.globalCompositeOperation = "source-over";
    ctx.clearRect(0, 0, canvas.width / dpr, canvas.height / dpr);
    for (const s of drawStrokes) {
      for (let i = 1; i < s.points.length; i++) {
        drawStyle(ctx, s.tool);
        const a = s.points[i - 1], b = s.points[i];
        ctx.beginPath(); ctx.moveTo(a.x, a.y); ctx.lineTo(b.x, b.y); ctx.stroke();
      }
    }
  }
  // Reveal: freeze the drawing (kept on screen to self-check against the answer),
  // then reveal. Use max(1, …) so an image-only answer (no back lines) still reveals.
  function drawReveal() {
    drawSnapshot = drawCanvasEl ? drawCanvasEl.toDataURL() : null;
    revealed = Math.max(1, backCount());
    rerender();
  }
  function frozenDrawImg(dataUrl) {
    const wrap = el("div", "draw-frozen");
    const img = el("img", null);
    img.src = dataUrl;
    img.alt = "your drawing";
    wrap.appendChild(img);
    return wrap;
  }
  function renderDrawComparison(a, renderExpected) {
    const comparison = el("div", "draw-comparison");
    const attempt = el("section", "draw-comparison-pane");
    attempt.appendChild(el("div", "draw-comparison-label", "Your answer"));
    const attemptBody = el("div", "draw-comparison-body");
    if (drawSnapshot) attemptBody.appendChild(frozenDrawImg(drawSnapshot));
    attempt.appendChild(attemptBody);
    comparison.appendChild(attempt);

    const expected = el("section", "draw-comparison-pane");
    expected.appendChild(el("div", "draw-comparison-label", "Expected answer"));
    const expectedBody = el("div", "draw-comparison-body");
    renderExpected(expectedBody);
    expected.appendChild(expectedBody);
    comparison.appendChild(expected);
    a.appendChild(comparison);
  }

  // A reshaped multi-item answer (the `format` augment's list) reveals with
  // bullets. Authored physical lines remain available for line-reveal and typing,
  // but ordinary answers use `back_units`, where Markdown soft wraps are spaces.
  function isReshapedList(c) { return !!(c && c.reshaped && c.back.length > 1); }
  function appendAnswerUnits(sec, units) {
    for (const unit of units || []) {
      if (unit.kind === "sentence") {
        const answer = el("div", "answer");
        if (unit.runs) appendRuns(answer, unit.runs); else answer.textContent = unit.text || "";
        sec.appendChild(answer);
      } else if (unit.kind === "code") {
        const pre = el("pre", "code-block");
        pre.textContent = (unit.lines || []).join("\n");
        sec.appendChild(pre);
      } else if (unit.kind === "diagram") {
        const img = el("img", "diagram");
        img.src = unit.src; img.alt = unit.alt || "";
        img.width = unit.width; img.height = unit.height;
        sec.appendChild(img);
      } else if (unit.kind === "checklist") {
        appendChecklist(sec, unit.items);
      } else if (unit.kind === "table") {
        appendTable(sec, unit);
      }
    }
  }

  // Fill the answer region for reveal modes (flip / line / choice fallback).
  // The introduction view: a never-seen flip card shows its answer before it is
  // ever quizzed. An ordered card keeps its authored line-reveal contract, then
  // "Seen" records the encounter after the final line. Only a reshaped list wants
  // a flush-left block: its lines are steps or bullets.
  function leftAlignAnswer(c) { return isReshapedList(c); }
  function fillIntroduction(a) {
    if (!a) return;
    if (state.mode === "line") {
      fillAnswer(a);
      return;
    }
    const c = state.card;
    const sec = el("div", "reveal" + (leftAlignAnswer(c) ? " list" : ""));
    if (isReshapedList(c)) appendReveal(sec, c.back, c.back_runs, true, c.back_units);
    else appendAnswerUnits(sec, c.back_units);
    a.appendChild(sec);
    appendImages(a, c.images_back);
  }

  function fillAnswer(a) {
    if (!a) return;
    // fillBottom already cleared the region and added the mode badge; don't wipe it.
    if (revealed === 0) return; // stays empty until revealed

    const c = state.card;
    const shown = state.mode === "line" ? Math.min(revealed, c.back.length) : c.back.length;
    const sec = el("div", "reveal" + (state.mode === "line" ? " line" : leftAlignAnswer(c) ? " list" : ""));
    if (state.mode === "line") {
      appendReveal(sec, c.back.slice(0, shown), c.back_runs && c.back_runs.slice(0, shown), false, c.back_units);
      const pending = el("div", "answer pending" + (shown < c.back.length ? "" : " complete"), "···");
      pending.setAttribute("aria-hidden", "true");
      sec.appendChild(pending);
      const reserveStart = sec.children.length;
      appendReveal(sec, c.back.slice(shown), c.back_runs && c.back_runs.slice(shown), false, c.back_units);
      for (const child of Array.from(sec.children).slice(reserveStart)) {
        child.classList.add("line-reserve");
        child.setAttribute("aria-hidden", "true");
      }
    } else if (isReshapedList(c)) {
      appendReveal(sec, c.back, c.back_runs, true, c.back_units);
    } else {
      appendAnswerUnits(sec, c.back_units);
    }
    a.appendChild(sec);
    // Attach the answer image to the region itself (a flex column), not to the
    // `.reveal` block, so it can be bounded by the region and scaled to fit.
    appendImages(a, c.images_back);
  }

  // A card side's images render as ordered blocks; `im` is a `{ src, alt }` from
  // the card's `images` / `images_back` list, its src a server `/img/<key>` URL.
  function appendImages(parent, images) {
    for (const im of (images || [])) parent.appendChild(cardImage(im));
  }
  function cardImage(im) {
    const img = el("img", "card-img");
    img.src = im.src;
    img.alt = im.alt || "";
    const regions = im.regions || [];
    if (!regions.length && !im.crop) return img;
    if (!im.crop) return maskedCardImage(img, regions, "img");
    // Region and crop geometry live in the source image's own space (px of
    // the source, or % of it), never in crop space: the crop is a viewport,
    // so the full-image sheet shifts inside it and masks sit on the sheet.
    const wrap = el("div", "img-fit");
    const box = el("div", "img-box");
    wrap.appendChild(box);
    const sheet = el("div", "img-sheet");
    sheet.appendChild(img);
    box.appendChild(sheet);
    for (const r of regions) {
      const mask = el("div", "img-mask" + (r.reveal_on_answer ? " reveals" : ""));
      maskGlyph(mask, r, "img");
      mask.dataset.region = JSON.stringify(r);
      sheet.appendChild(mask);
    }
    box.style.display = "none"; // nothing to show until the source's size is known
    const place = () => {
      const sw = img.naturalWidth, sh = img.naturalHeight;
      if (!sw || !sh) return;
      box.style.display = "";
      const pct = (r) => r.unit === "%"
        ? { x: r.x, y: r.y, w: r.width, h: r.height }
        : { x: (r.x / sw) * 100, y: (r.y / sh) * 100, w: (r.width / sw) * 100, h: (r.height / sh) * 100 };
      const crop = im.crop ? pct(im.crop) : { x: 0, y: 0, w: 100, h: 100 };
      const vw = (crop.w / 100) * sw, vh = (crop.h / 100) * sh;
      box.style.aspectRatio = `${vw} / ${vh}`;
      // The viewport shrinks to fit its bounded region instead of scrolling:
      // the wrap's basis is the natural (uncapped) height, flex squeezes it,
      // and the box contain-fits the squeezed wrap keeping the crop's aspect,
      // which the sheet's percentage math depends on.
      const fit = () => {
        const naturalW = Math.min(wrap.clientWidth, 480);
        if (!naturalW) return;
        wrap.style.flexBasis = `${(naturalW * vh) / vw}px`;
        const w = Math.min(naturalW, (wrap.clientHeight * vw) / vh);
        box.style.width = `${w}px`;
      };
      new ResizeObserver(fit).observe(wrap);
      fit();
      sheet.style.width = `${(100 / crop.w) * 100}%`;
      sheet.style.height = `${(100 / crop.h) * 100}%`;
      sheet.style.left = `${(-crop.x / crop.w) * 100}%`;
      sheet.style.top = `${(-crop.y / crop.h) * 100}%`;
      for (const mask of sheet.querySelectorAll(".img-mask")) {
        const r = JSON.parse(mask.dataset.region);
        const g = pct(r);
        const x0 = Math.max(0, g.x), y0 = Math.max(0, g.y);
        const x1 = Math.min(100, g.x + g.w), y1 = Math.min(100, g.y + g.h);
        const visible = x1 > x0 && y1 > y0;
        if (!visible && r.role === "asked") askedGone();
        mask.style.display = visible ? "" : "none";
        mask.style.left = `${x0}%`;
        mask.style.top = `${y0}%`;
        mask.style.width = `${x1 - x0}%`;
        mask.style.height = `${y1 - y0}%`;
      }
    };
    if (img.complete) place(); else img.addEventListener("load", place);
    return wrap;
  }

  // An asked region clipping to nothing is a broken question and fails loud;
  // an empty sibling mask or cover hides nothing that exists, so those stay
  // silently dropped.
  function askedGone() {
    notice("a blank lies outside the image, so its question cannot be drawn");
  }

  // The dom-level masked image bound to this surface's loud channel.
  function maskedCardImage(img, regions, prefix) {
    return maskedImage(img, regions, prefix, askedGone);
  }

  // Review context: a masked diagram carries its overlay; the asked mask
  // lifts with the card's answered class, siblings and covers stay.
  function contextDiagram(unit) {
    const img = diagramImage(unit);
    if (!unit.regions || !unit.regions.length) return img;
    return maskedCardImage(img, unit.regions, "img");
  }

  // Browse shows every card revealed: the accessible name starts at the
  // revealed text and the browse card's standing answered class lifts the
  // asked mask; siblings and covers keep protecting their own cards.
  function browseDiagram(unit) {
    const img = diagramImage(unit);
    if (img.dataset.revealedAlt) img.alt = img.dataset.revealedAlt;
    if (!unit.regions || !unit.regions.length) return img;
    return maskedCardImage(img, unit.regions, "img");
  }

  function renderLegend() {
    legend.innerHTML = "";
    clearLegendSides();
    if (feedback) {
      if (isIntroducing()) {
        chip("Seen", "primary", introduce, label(keys.reveal)); // a pick acknowledges, never grades
        chip("Ask tutor", "ask", openTutor, label(keys.ask), legendRight); // answer is showing: tutor allowed
      } else if (isRecognizeMc()) {
        if (feedback.passed) {
          // A correct Recognize pick: Next commits it; the quiet "I guessed"
          // override (also bound to the failed key) lets an honest guess demote
          // itself instead — both map to /api/grade, never an auto-continue, so
          // the learner always has the last word.
          chip("Next", "primary", () => grade("passed"), label(keys.reveal));
          chip("I guessed", "quiet", () => grade("failed"), label(keys.failed));
          chip("Ask tutor", "ask", openTutor, label(keys.ask), legendRight);
        } else {
          // A wrong pick: the correct option is already highlighted on screen
          // (renderChoiceFeedback) — Continue is the only action, and it grades
          // the miss (there's no guess left to walk back). Ask tutor is offered
          // here too: "why is the highlighted option right, not the one I picked?"
          chip("Continue", "primary", () => grade("failed"), label(keys.reveal));
          chip("Ask tutor", "ask", openTutor, label(keys.ask), legendRight);
        }
      } else {
        // A typed check's (or TypeLine's closing) result: pure evidence — the
        // learner grades it themselves, same three-way as any other reveal.
        chip("Missed it", "failed", () => grade("failed"), label(keys.failed));
        chip("Partly", "partly", () => grade("partly"), label(keys.partly));
        chip("Got it", "passed", () => grade("passed"), label(keys.passed));
        chip("Ask tutor", "ask", openTutor, label(keys.ask), legendRight);
      }
    } else if (isIntroducing()) {
      if (effectiveDraw()) {
        if (revealed === 0) {
          chip("Reveal", "primary", drawReveal, label(keys.reveal)); // reveal freezes your attempt
          chip("Skip", "", skip, label(keys.skip));
        } else {
          chip("Seen", "primary", introduce, label(keys.reveal));      // ungraded acknowledgment
          chip("Ask tutor", "ask", openTutor, label(keys.ask), legendRight);
        }
      } else if (isIntroChoice()) {
        if (isMultiChoice()) {
          const submit = chip("Submit", "primary", submitMultiChoice, "enter");
          submit.disabled = selectedChoices.size === 0;
        }
        chip("Skip", "", skip, label(keys.skip));            // options are tappable
      } else if (revealed > 0 && state.mode === "line" && !fullyRevealed()) {
        chip("Reveal next", "primary", reveal, label(keys.reveal));
        chip("Ask tutor", "ask", openTutor, label(keys.ask), legendRight);
      } else if (revealed > 0) {
        chip("Seen", "primary", introduce, label(keys.reveal)); // hide⟷show is the corner `h` toggle, not a footer button
        chip("Ask tutor", "ask", openTutor, label(keys.ask), legendRight);
      } else {
        chip("Reveal", "primary", reveal, label(keys.reveal));
        chip("Skip", "", skip, label(keys.skip));
      }
    } else if (effectiveDraw() && revealed === 0) {
      chip("Reveal", "primary", drawReveal, label(keys.reveal));
      chip("Skip", "", skip, label(keys.skip));
    } else if (isChoice()) {
      if (isMultiChoice()) {
        const submit = chip("Submit", "primary", submitMultiChoice, "enter");
        submit.disabled = selectedChoices.size === 0;
      }
      chip("Skip", "", skip, label(keys.skip));
    } else if (isInput()) {
      chip("Submit", "primary", submitCheck, "enter");
      chip("Skip", "", skip, label(keys.skip));
    } else if (isTypeLine()) {
      chip("Check", "primary", submitCurrentTypeLine, "enter");
      chip("Skip", "", skip, label(keys.skip));
    } else if (isExplain() && !fullyRevealed()) {
      chip("Reveal", "primary", explainReveal, "shift+enter");
      chip("Skip", "", skip, label(keys.skip));
    } else if (!fullyRevealed()) {
      chip(state.mode === "line" && revealed > 0 ? "Reveal next" : "Reveal", "primary", reveal, label(keys.reveal));
      chip("Skip", "", skip, label(keys.skip));
    } else if (hasKeypoints()) {
      chip("Yes", "passed", () => answerKeypoint(true), "y");
      chip("No", "failed", () => answerKeypoint(false), "n");
      if (keypointsAnswered()) {
        // Every point judged: the submit button becomes the derived verdict
        // (re-derived each render, so changing a point updates it).
        const v = keypointVerdict();
        chip(v[0].toUpperCase() + v.slice(1), "v-" + v, submitKeypoints, "enter");
      } else {
        const answered = marks.filter(m => m !== undefined).length;
        chip(`Done ${answered}/${state.keypoints.length}`, "primary", submitKeypoints, "enter").disabled = true;
      }
      chip("Ask tutor", "ask", openTutor, label(keys.ask), legendRight);
    } else if (isRecognizeFallback()) {
      // No MC could be built (too few distractors): attempt→reveal, boolean call.
      chip("Knew it", "passed", () => grade("passed"), label(keys.passed));
      chip("Not yet", "failed", () => grade("failed"), label(keys.failed));
      chip("Ask tutor", "ask", openTutor, label(keys.ask), legendRight);
    } else {
      chip("Missed it", "failed", () => grade("failed"), label(keys.failed));
      chip("Partly", "partly", () => grade("partly"), label(keys.partly));
      chip("Got it", "passed", () => grade("passed"), label(keys.passed));
      chip("Ask tutor", "ask", openTutor, label(keys.ask), legendRight);
    }
    chip("Leave", "", leaveSession, "esc", legendLeft); // pinned bottom-left; return to the deck picker
  }

  // Terse, approximate phrase for when the next scheduled card comes due, shown
  // on an empty session so "Nothing due." says when to return. No seconds, no
  // ticking; null when there is no instant or it has already passed.
  function nextDueNote(ms) {
    if (ms == null) return null;
    const delta = Number(ms) - Date.now();
    if (delta <= 0) return null;
    const min = Math.round(delta / 60000);
    if (min < 60) return `Next due in ${Math.max(1, min)} min.`;
    const hr = Math.round(delta / 3600000);
    if (hr < 24) return `Next due in ${hr} h.`;
    const days = Math.round(delta / 86400000);
    return days <= 1 ? "Next due tomorrow." : `Next due in ${days} days.`;
  }

  function renderSummary() {
    const acc = state.reviews > 0 ? Math.round(100 * state.passed / state.reviews) + "%" : "–";
    const wrap = el("div", "summary");
    // A first pass over a fresh deck is introduction-only: reviews stay 0 while
    // every card was introduced. Say what actually happened — and when nothing
    // happened (an instant-empty select), don't call it a session at all, and
    // never print a zero-valued row (a done screen must not read as "you did
    // nothing"; user rule).
    const introduced = state.introduced || 0;
    const partial = state.partial || 0;
    const didSomething = state.reviews > 0 || introduced > 0;
    wrap.appendChild(el("div", "lede", didSomething ? "session complete" : "nothing to do here"));
    const gap = state.recognize_gap;
    const headline = state.reviews > 0 ? "Nicely charged."
      : introduced > 0 ? "New cards planted."
      : gap ? "Recognize is drained."
      : "Nothing due.";
    wrap.appendChild(el("h2", null, headline));
    const row = (label, value) => {
      const r = el("div", "row");
      r.appendChild(el("span", null, label));
      r.appendChild(el("b", null, value));
      wrap.appendChild(r);
    };
    // The sitting's count is the value; the deck's lifetime standing rides the
    // label, so the numeric column stays one number per row.
    const deckTotal = state.deck_total || 0;
    const standing = deckTotal > 0 ? ` (${state.met_total || 0} of ${deckTotal} in the deck)` : "";
    if (introduced > 0) row("introduced" + standing, `${introduced}`);
    if (state.reviews > 0) {
      row("reviewed", `${state.reviews}`);
      row("passed / failed", `${state.passed} / ${state.failed}`);
      row("accuracy", acc);
    }
    if (partial > 0) row("almost", `${partial}`);
    const dueLeft = state.due_left || 0;
    const newLeft = state.new_left || 0;
    // Every countdown here is read from the clock, so the note is a function of
    // the moment, not of the render: the poll repaints it while the summary
    // sits open, and a "4 min" printed once would be wrong a minute later.
    const noteText = () => {
      if (summaryReady) return "Ready when you are.";
      // After a sitting that only introduced cards, "N new waiting" reads as an
      // endless queue: it never says the cards just met come back shortly.
      const nextDue = !gap ? nextDueNote(state.next_due_ms) : null;
      const introducedReturn = introduced > 0 && nextDue
        ? `${introduced} card${introduced === 1 ? "" : "s"} met. ${nextDue}`
        : null;
      // "N still due" beside a disabled Continue is a contradiction: when
      // nothing is servable the cards are cooling, so say when one opens.
      const cooling = dueLeft > 0 && !state.can_restart ? nextDueNote(state.next_due_ms) : null;
      const dueSegment = dueLeft > 0
        ? `${dueLeft} still due${cooling ? ` (cooling, ${cooling.charAt(0).toLowerCase() + cooling.slice(1, -1)})` : ""}`
        : null;
      if (gap) {
        // Point at the two real exits: the cards this depth can't serve.
        const parts = [];
        if (dueSegment) parts.push(dueSegment);
        if (gap.recall > 0) parts.push(`${gap.recall} card${gap.recall === 1 ? "" : "s"} wait at Recall`);
        if (gap.unaugmented > 0) parts.push(`${gap.unaugmented} need answer choices first (the Augment screen builds them)`);
        return parts.join(" · ") + ".";
      }
      if (introducedReturn) return introducedReturn + (newLeft > 0 ? ` ${newLeft} new waiting.` : "");
      if (nextDue && !didSomething) return nextDue;
      if (dueSegment) return `${dueSegment}.`;
      if (newLeft > 0) return `${newLeft} new waiting.`;
      if (newLeft === 0 && !state.can_restart) return "Nothing due right now. Come back later.";
      return null;
    };
    const noteEl = el("div", "note", "");
    wrap.appendChild(noteEl);
    const examDue = state.exam_due || [];
    examDue.forEach(name => {
      wrap.appendChild(el("div", "exam-ready", `✦ ${name} is ready for its exam.`));
    });
    stage.appendChild(wrap);
    // An exam-due deck takes the primary action; otherwise the restart chip does.
    examDue.forEach((name, i) => {
      chip(examDue.length === 1 ? "Take the exam" : `Exam: ${name}`, i === 0 ? "primary" : "", () => startExam(name));
    });
    // Continue the drain when due cards remain; otherwise say the next sitting
    // starts new cards, never an unlabeled button. The waiting count lives in
    // the note: the sitting plants only the capped share, not all of them.
    // A drained Recognize sitting cannot restart into anything, so its
    // primary action IS the pointed exit: reopen this deck at Recall (Enter),
    // with the Augment screen one chip away for the choice-less cards.
    const deck = lastDeck ? lastDeck() : null;
    if (gap && gap.recall > 0 && deck) {
      chip("Continue at Recall", examDue.length ? "" : "primary", () => {
        api("/api/select", post({ deck, depth: "recall" })).then(apply).catch(() => load());
      }, "enter");
    }
    if (gap && gap.unaugmented > 0 && deck && openAugment) {
      chip("Augment", "ask", () => openAugment(deck), "a");
    }
    const restartLabel = dueLeft > 0 ? "Continue"
      : newLeft > 0 ? "Start new"
      : "New session";
    const newSession = chip(restartLabel, examDue.length || gap ? "" : "primary", restart, label(keys.restart));
    summaryPaint = () => {
      const text = noteText();
      noteEl.textContent = text || "";
      noteEl.hidden = !text;
      newSession.disabled = !state.can_restart && !summaryReady;
    };
    summaryPaint();
    chip("Leave", "", deselect, "esc");
  }

  function reveal() {
    const firstLook = revealed === 0;
    revealed = state.mode === "line" ? Math.min(revealed + 1, backCount()) : backCount();
    // Seeing a new card's answer IS the encounter: record it server-side so
    // abandoning the session here does not re-introduce the card as new.
    // Fire-and-forget: a lost mark degrades to the old behavior, nothing worse.
    fillBottom();
    renderLegend();
  }
  // Introduction only: hide / un-hide the revealed answer so you can self-test the fresh
  // encoding (conceal it, try to recall, show it to check) before acknowledging with
  // "Seen". Deliberately does ONE thing — flips the answer text's visibility in place.
  // It does NOT re-render: the card stays fully revealed, so the note, the footer, the
  // answer's own box, everything holds its exact position. Nothing reflows or jumps.
  // A first-encounter aid: there's no spaced schedule to lean on yet, so an ordinary
  // review has no such toggle — it drills a card by failing it, which brings it back spaced.
  function introToggle() {
    if (revealed === 0) { reveal(); return; } // first look: reveal (same as the Reveal key)
    answerConcealed = !answerConcealed;
    const a = doc.getElementById("ansRegion");
    if (!a) return;
    a.classList.toggle("concealed", answerConcealed); // visibility only — no reflow, no movement
    const cardEl = doc.getElementById("card");
    if (cardEl) syncDiagramAnswerState(cardEl);
    paintIntroCue(a);
  }

  // The self-test conceal owns the diagram answer too: re-masking rides a
  // class (visibility only, no reflow) and the accessible name follows.
  function syncDiagramAnswerState(cardEl) {
    const showAnswer = isAnswered() && !answerConcealed;
    cardEl.classList.toggle("concealing", isAnswered() && answerConcealed);
    for (const img of cardEl.querySelectorAll("img.diagram[data-revealed-alt]")) {
      img.alt = showAnswer ? img.dataset.revealedAlt : img.dataset.maskedAlt;
    }
  }
  // Point the corner cue's glyph/title at what the next press does. A textContent swap
  // only — no layout change.
  function paintIntroCue(a) {
    const cue = a.querySelector(".cite-toggle");
    if (!cue) return;
    cue.title = answerConcealed ? "show answer" : "hide the answer to self-test";
    cue.setAttribute("aria-label", cue.title);
    const ci = cue.querySelector(".ci");
    if (ci) ci.textContent = answerConcealed ? "⊙" : "⊘";
  }
  // A tap on the answer region toggles too, but don't hijack a text-selection drag.
  function onIntroToggleClick() {
    if (win.getSelection && String(win.getSelection())) return;
    introToggle();
  }


  let noticedLoadWarnings = "";
  function syncLoadNotice() {
    const msg = (state && state.load_warnings || []).join(" ");
    // A warning-free state (the picker, a healthy deck) clears the memo:
    // the dedup is per session occurrence, never per identical sentence, or
    // deck B's warning would vanish because deck A said the same words.
    if (!msg) { noticedLoadWarnings = ""; return; }
    if (msg === noticedLoadWarnings) return;
    noticedLoadWarnings = msg;
    notice(msg);
  }

  function prepareRender() {
    stopDuePoll();
    summaryPaint = null;
    summaryReady = false;
    syncSaveAlert();
    syncLoadNotice();
  }

  function prepareSurface() {
    headerBreadcrumb();
    deckEl.textContent = state.label;
    histEl.textContent = "";
    if (state.phase === "review") {
      histEl.appendChild(el("span", "left-token", `${state.remaining} left`));
    }
    scoreEl.innerHTML = "";
    menuWrap.style.display = state.phase === "done" ? "none" : "";
    setMenuContext("review");
  }

  function render() {
    if (browsing) {
      renderBrowse();
      return;
    }
    prepareSurface();
    if (screen() === "summary") {
      renderSummary();
      startDuePoll();
    } else {
      renderCard();
    }
  }

  function handleKey(event) {
    const e = event;
      // The browse overlay: step cards (configurable next/prev + arrows/space/g/G),
      // Esc/Backspace leaves. Read-only — no grading.
      if (browsing) {
        if (e.key === "Escape" || e.key === "Backspace") { e.preventDefault(); closeBrowse(); return; }
        if (e.key === "ArrowRight" || e.key === " " || hit(e, browseKeys.next)) { e.preventDefault(); browseGo(1); return; }
        if (e.key === "ArrowLeft" || hit(e, browseKeys.prev)) { e.preventDefault(); browseGo(-1); return; }
        if (e.key === "g" || e.key === "Home") { e.preventDefault(); browsing.i = 0; rerender(); return; }
        if (e.key === "G" || e.key === "End") { e.preventDefault(); browsing.i = browsing.cards.length - 1; rerender(); return; }
        return;
      }
      // While the leave prompt is up: Enter confirms leaving, Esc stays; other keys
      // are inert (so a stray Esc can never blow through the guard).
      if (confirmingLeave) {
        if (e.key === "Enter") { e.preventDefault(); deselect(); }
        else if (e.key === "Escape") { e.preventDefault(); cancelLeave(); }
        return;
      }
      // Esc returns to the deck picker (with a confirm when the session isn't done).
      if (e.key === "Escape") { e.preventDefault(); leaveSession(); return; }
      if (state.phase === "done") {
        // Enter takes the primary action (the exam when one is due, else the
        // pointed exit of a drained sitting, else an enabled New session).
        if (e.key === "Enter") {
          const b = legend.querySelector(".chip.primary");
          if (b && !b.disabled) { e.preventDefault(); b.click(); }
          return;
        }
        if ((state.can_restart || summaryReady) && hit(e, keys.restart)) { e.preventDefault(); restart(); }
        if (e.key === "a" && state.recognize_gap && lastDeck && lastDeck() && openAugment) {
          e.preventDefault(); openAugment(lastDeck());
        }
        return;
      }
      // `c` swaps the answer content for the card's section and back locally.
      if (hasSection() && hit(e, keys.context)) { e.preventDefault(); toggleSection(); return; }
      // `s` swaps a cited card between its answer and its source, once answered.
      if ((state.card.citations || []).length && isAnswered() && !e.ctrlKey && e.key.toLowerCase() === "s") {
        e.preventDefault(); toggleCitation(); return;
      }
      // A never-seen card (introduction): recognition pick or recall reveal, then "Seen".
      // Handled before the feedback/grade paths so a pick never grades the card.
      if (isIntroducing()) {
        if (hit(e, keys.remove)) { e.preventDefault(); remove(); return; }
        // Post-reveal only: once the answer shows (revealed, or a pick's feedback),
        // the tutor is allowed here too, matching review's after-reveal rule.
        if ((revealed > 0 || feedback) && hit(e, keys.ask)) { e.preventDefault(); openTutor(); return; }
        if (effectiveDraw()) {
          if (revealed === 0) {
            if (hit(e, keys.skip)) { e.preventDefault(); skip(); return; }
            if (hit(e, keys.reveal)) { e.preventDefault(); drawReveal(); return; }
          } else if (hit(e, keys.reveal) || e.key === "Enter" || e.key === " ") {
            e.preventDefault(); introduce();
          }
          return;
        }
        if (isIntroChoice()) {
          if (!feedback) {
            if (hit(e, keys.skip)) { e.preventDefault(); skip(); return; }
            if (hit(e, keys.up) || e.key === "ArrowUp") { e.preventDefault(); moveChoiceFocus(-1); return; }
            if (hit(e, keys.down) || e.key === "ArrowDown") { e.preventDefault(); moveChoiceFocus(1); return; }
            if (isMultiChoice()) {
              if (e.key === " " && choiceFocus >= 0 && choiceFocus < state.choices.length) { e.preventDefault(); toggleChoice(choiceFocus); return; }
              if (e.key === "Enter") { e.preventDefault(); submitMultiChoice(); return; }
              if (e.key >= "1" && e.key <= "9") {
                const i = +e.key - 1;
                if (i < state.choices.length) { e.preventDefault(); toggleChoice(i); }
              }
              return;
            }
            if (e.key === "Enter" && choiceFocus >= 0 && choiceFocus < state.choices.length) { e.preventDefault(); choose(choiceFocus); return; }
            if (e.key >= "1" && e.key <= "9") {
              const i = +e.key - 1;
              if (i < state.choices.length) { e.preventDefault(); choose(i); }
            }
          } else if (hit(e, keys.reveal) || e.key === "Enter" || e.key === " ") {
            e.preventDefault(); introduce();
          }
          return;
        }
        // `h` toggles the answer hidden ⟷ shown, both directions on one key (the
        // source⟷answer swap's principle). Space reveals every ordered line before
        // it acknowledges the completed introduction with "Seen".
        if (e.key.toLowerCase() === "h" && !e.ctrlKey) { e.preventDefault(); introToggle(); return; }
        if (revealed === 0) {
          if (hit(e, keys.skip)) { e.preventDefault(); skip(); return; }
          if (hit(e, keys.reveal)) { e.preventDefault(); reveal(); return; }
        } else if (hit(e, keys.reveal) || e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          if (state.mode === "line" && !fullyRevealed()) reveal();
          else introduce();
        }
        return;
      }
      if (feedback) {
        if (hit(e, keys.ask)) { e.preventDefault(); openTutor(); return; }
        if (hit(e, keys.remove)) { e.preventDefault(); remove(); return; }
        if (isRecognizeMc()) {
          // Correct pick: reveal/Enter takes the primary "Next" (passed), and the
          // failed key is the quiet "I guessed" override (demote to failed). Wrong
          // pick: reveal/Enter is "Continue", which just grades the miss — there's
          // no guess to walk back on a pick that was already wrong.
          if (feedback.passed && hit(e, keys.failed)) { e.preventDefault(); grade("failed"); return; }
          if (hit(e, keys.reveal) || e.key === "Enter") { e.preventDefault(); grade(feedback.passed ? "passed" : "failed"); }
          return;
        }
        // A typed check's (or TypeLine's closing) result: the learner grades it,
        // same three-way keys as any other reveal — no auto-continue.
        if (hit(e, keys.failed)) { e.preventDefault(); grade("failed"); }
        else if (hit(e, keys.partly)) { e.preventDefault(); grade("partly"); }
        else if (hit(e, keys.passed)) { e.preventDefault(); grade("passed"); }
        return;
      }
      // While typing in a field, only Ctrl shortcuts act so plain keys stay text;
      // Enter (submit / check-line) is handled by the field itself.
      if (isInput() || isTypeLine()) {
        if (e.ctrlKey && hit(e, keys.remove)) { e.preventDefault(); remove(); }
        else if (e.ctrlKey && hit(e, keys.skip)) { e.preventDefault(); skip(); }
        return;
      }
      // Draw before reveal: the canvas takes pointer input, not keys. Enter reveals
      // (freezing the drawing) to match the "Reveal" chip; placed before the explain
      // and generic reveal branches so a flip/explain draw card reveals via drawReveal().
      if (effectiveDraw() && revealed === 0) {
        if (hit(e, keys.reveal)) { e.preventDefault(); drawReveal(); }
        else if (hit(e, keys.skip)) { e.preventDefault(); skip(); }
        else if (hit(e, keys.remove)) { e.preventDefault(); remove(); }
        return;
      }
      // Explain before reveal: the textarea takes plain keys (Enter = newline) and
      // Shift+Enter reveals. The textarea handles Shift+Enter when focused (and stops
      // it there); this covers the case where focus has left the textarea.
      if (isExplain() && !fullyRevealed()) {
        if (e.key === "Enter" && e.shiftKey) { e.preventDefault(); explainReveal(); }
        else if (e.ctrlKey && hit(e, keys.remove)) { e.preventDefault(); remove(); }
        else if (e.ctrlKey && hit(e, keys.skip)) { e.preventDefault(); skip(); }
        return;
      }
      if (hit(e, keys.remove)) { e.preventDefault(); remove(); return; }
      if (isChoice()) {
        if (hit(e, keys.skip)) { e.preventDefault(); skip(); return; }
        if (hit(e, keys.up) || e.key === "ArrowUp") { e.preventDefault(); moveChoiceFocus(-1); return; }
        if (hit(e, keys.down) || e.key === "ArrowDown") { e.preventDefault(); moveChoiceFocus(1); return; }
        if (isMultiChoice()) {
          if (e.key === " " && choiceFocus >= 0 && choiceFocus < state.choices.length) { e.preventDefault(); toggleChoice(choiceFocus); return; }
          if (e.key === "Enter") { e.preventDefault(); submitMultiChoice(); return; }
          if (e.key >= "1" && e.key <= "9") {
            const i = +e.key - 1;
            if (i < state.choices.length) { e.preventDefault(); toggleChoice(i); }
          }
          return;
        }
        if (e.key === "Enter" && choiceFocus >= 0 && choiceFocus < state.choices.length) { e.preventDefault(); choose(choiceFocus); return; }
        if (e.key >= "1" && e.key <= "9") {
          const i = +e.key - 1;
          if (i < state.choices.length) { e.preventDefault(); choose(i); }
        }
        return;
      }
      if (!fullyRevealed()) {
        if (hit(e, keys.skip)) { e.preventDefault(); skip(); return; }
        if (hit(e, keys.reveal)) { e.preventDefault(); reveal(); return; }
        return;
      }
      if (hit(e, keys.ask)) { e.preventDefault(); openTutor(); return; }
      // The key-point checklist replaces the grade buttons: walk the list top to
      // bottom with y/n (auto-advancing), the review up/down keys or arrows to move, Enter to submit once
      // every point is answered (the server derives the grade from coverage).
      if (hasKeypoints()) {
        if (e.key === "y" || e.key === "Y") { e.preventDefault(); answerKeypoint(true); }
        else if (e.key === "n" || e.key === "N") { e.preventDefault(); answerKeypoint(false); }
        else if (hit(e, keys.down) || e.key === "ArrowDown") { e.preventDefault(); moveKeypoint(1); }
        else if (hit(e, keys.up) || e.key === "ArrowUp") { e.preventDefault(); moveKeypoint(-1); }
        else if (e.key === "Enter" && keypointsAnswered()) { e.preventDefault(); submitKeypoints(); }
        return;
      }
      // The Recognize fallback (no MC) only ever shows two chips (Knew it / Not
      // yet) — the partly key has no matching chip there, so it's a no-op.
      if (hit(e, keys.failed)) { e.preventDefault(); grade("failed"); }
      else if (hit(e, keys.partly) && !isRecognizeFallback()) { e.preventDefault(); grade("partly"); }
      else if (hit(e, keys.passed)) { e.preventDefault(); grade("passed"); }

    return true;
  }

  function syncDrawMenu() {
    const authored = effectiveDraw() && state.input === "draw";
    ui.drawState.textContent = (authored || drawToggle) ? "on" : "off";
    ui.drawButton.disabled = authored;
  }

  function toggleDraw() {
    drawToggle = !drawToggle;
    try {
      storage.setItem("alix-draw", drawToggle ? "1" : "0");
    } catch (error) {}
    syncDrawMenu();
    closeMenu();
    rerender();
  }

  return {
    apply,
    buildCardShell,
    handleKey,
    isAnswered,
    isBrowsing,
    keys: () => keys,
    load,
    openBrowse,
    prepareRender,
    prepareSurface,
    remove,
    render,
    renderSourceExcerpt,
    replaceState,
    screen,
    setBrowseKeys,
    setKeys,
    sourceTerms,
    state: currentState,
    syncDrawMenu,
    toggleDraw,
    updateFade,
  };
}
