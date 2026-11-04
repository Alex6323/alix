export function createExam({
  api,
  post,
  rememberLaunch,
  rerender,
  applyStudy,
  updateBusy,
  workingText,
  timers,
  ui,
}) {
  const {
    appendUnits,
    chip,
    deckEl,
    document: doc,
    el,
    headerBreadcrumb,
    histEl,
    legend,
    menuWrap,
    scoreEl,
    stage,
  } = ui;
  let data = null;
  let confirmingQuit = false;
  let poll = null;

  function numberedCode(lines) {
    const code = el("div", "source-code exam-code");
    lines.forEach((line, index) => {
      const row = el("div", "source-line");
      row.appendChild(el("span", "source-number", String(index + 1)));
      row.appendChild(el("span", "source-text", line));
      code.appendChild(row);
    });
    return code;
  }

  function appendExamText(parent, units, text) {
    if (!units) {
      parent.appendChild(doc.createTextNode(text || ""));
      return parent;
    }
    for (const unit of units) {
      if (unit.kind === "code") parent.appendChild(numberedCode(unit.lines || []));
      else appendUnits(parent, [unit]);
    }
    return parent;
  }

  function isOpen() {
    return data !== null;
  }

  function isPolling() {
    return poll !== null;
  }

  function start(deck, needsWorkOnly = false) {
    rememberLaunch(deck);
    return api("/api/exam/start", post({ deck, needs_work_only: needsWorkOnly })).then((next) => {
      if (next && next.phase) {
        data = next;
        rerender();
        if (next.thinking) startPoll();
      }
      return next;
    });
  }

  function close() {
    confirmingQuit = false;
    stopPoll();
    return api("/api/exam/close", post({})).then((state) => {
      data = null;
      applyStudy(state);
      return state;
    });
  }

  function answerButtons(current) {
    legend.innerHTML = "";
    if (current.current > 0) chip("Back", "", () => navigate(current.current - 1));
    if (current.on_last) chip("Submit for grading", "primary", submit, "shift+enter");
    else chip("Next", "primary", () => navigate(current.current + 1), "shift+enter");
    chip("Quit", "", quit, "esc");
  }

  function quit() {
    if (!data || data.phase !== "answering") return close();
    confirmingQuit = true;
    legend.innerHTML = "";
    legend.appendChild(el("span", "leave-msg", "Quit the exam? Your answers won't be graded."));
    chip("Quit anyway", "again", close, "enter");
    chip("Keep going", "primary", cancelQuit, "esc");
    return undefined;
  }

  function cancelQuit() {
    confirmingQuit = false;
    if (data) answerButtons(data);
  }

  function startPoll() {
    stopPoll();
    poll = timers.setInterval(() => {
      api("/api/exam").then((next) => {
        const previous = data;
        data = next;
        if (!next.thinking) stopPoll();
        if (!previous || previous.phase !== next.phase || previous.error !== next.error) {
          rerender();
        } else if (next.thinking) {
          const elapsed = doc.querySelector(".exam-elapsed");
          if (elapsed) {
            elapsed.innerHTML = "";
            elapsed.appendChild(el("span", "dot"));
            elapsed.appendChild(doc.createTextNode(elapsedText(next)));
          }
        }
      });
    }, 600);
    updateBusy();
  }

  function stopPoll() {
    if (poll) {
      timers.clearInterval(poll);
      poll = null;
    }
    updateBusy();
  }

  function elapsedText(current) {
    return workingText(current.elapsed || 0);
  }

  function countText(verdicts) {
    const points = verdicts.pass + verdicts.partial / 2;
    return `${Number.isInteger(points) ? points : points.toFixed(1)} / ${verdicts.total}`;
  }

  function navigate(goto) {
    const input = doc.querySelector(".exam-input");
    return api("/api/exam/answer", post({ text: input ? input.value : "", goto })).then((next) => {
      data = next;
      rerender();
      return next;
    });
  }

  function submit() {
    const input = doc.querySelector(".exam-input");
    return api("/api/exam/grade", post({ text: input ? input.value : "" })).then((next) => {
      data = next;
      rerender();
      if (next.thinking) startPoll();
      return next;
    });
  }

  function remediate() {
    return api("/api/exam/remediate", post({})).then((next) => {
      data = next;
      rerender();
      if (next.thinking) startPoll();
      return next;
    });
  }

  function render() {
    const current = data;
    headerBreadcrumb();
    deckEl.textContent = `exam · ${current.deck}`;
    histEl.textContent = current.strictness;
    scoreEl.innerHTML = "";
    menuWrap.style.display = "none";
    const wrap = el("div", "exam");
    if (current.error) wrap.appendChild(el("div", "exam-error", `⚠ ${current.error}`));

    if (current.phase === "generating" && current.error && !current.thinking) {
      stage.appendChild(wrap);
      chip("Close", "", close, "esc");
      return;
    }

    if (["generating", "grading", "remediating"].includes(current.phase)) {
      const message = {
        generating: "Preparing your exam…",
        grading: "Grading your answers…",
        remediating: "Writing remediation cards…",
      }[current.phase];
      wrap.appendChild(el("div", "exam-wait", message));
      const progress = el("div", "exam-elapsed");
      progress.appendChild(el("span", "dot"));
      progress.appendChild(doc.createTextNode(elapsedText(current)));
      wrap.appendChild(progress);
      stage.appendChild(wrap);
      if (current.error || current.phase === "generating") chip("Close", "", close, "esc");
      return;
    }

    if (current.phase === "answering") {
      wrap.appendChild(el("div", "exam-progress", `Question ${current.current + 1} / ${current.total}`));
      wrap.appendChild(appendExamText(el("div", "exam-q"), current.question_units, current.question));
      const input = el("textarea", "exam-input");
      input.placeholder = "Type your answer… (Shift+Enter to continue)";
      input.value = current.answer || "";
      input.rows = 6;
      input.addEventListener("keydown", (event) => {
        if (event.key === "Enter" && event.shiftKey && !confirmingQuit) {
          event.preventDefault();
          if (data.on_last) submit();
          else navigate(data.current + 1);
        }
      });
      wrap.appendChild(input);
      stage.appendChild(wrap);
      input.focus();
      answerButtons(current);
      return;
    }

    if (current.phase === "results") {
      const summary = el("div", "exam-summary");
      summary.appendChild(el("div", "exam-score", countText(current.verdicts)));
      if (current.mastered) summary.appendChild(el("div", "exam-mastered", "mastered"));
      wrap.appendChild(summary);
      const sections = el("div", "exam-sections");
      current.sections.forEach((section) => {
        const sectionEl = el("div", "exam-section");
        sectionEl.appendChild(el("span", "exam-section-label", section.label));
        sectionEl.appendChild(el("span", "exam-section-count", countText(section.verdicts)));
        sectionEl.appendChild(el(
          "span",
          "exam-section-state " + (section.pass ? "pass" : "needs-work"),
          section.pass ? "✓ pass" : "needs work",
        ));
        sections.appendChild(sectionEl);
      });
      wrap.appendChild(sections);
      if (current.unlocks && current.unlocks.length) {
        wrap.appendChild(el("div", "exam-unlocks", `Unlocks: ${current.unlocks.join(", ")}`));
      }
      current.grades.forEach((grade, index) => {
        const question = el("div", "exam-result");
        const asked = el("div", "exam-rq");
        const questionMeta = el("div", "exam-question-meta");
        questionMeta.appendChild(el("span", "exam-qn", `Q${index + 1}.`));
        questionMeta.appendChild(el("span", "exam-shape", grade.shape));
        asked.appendChild(questionMeta);
        asked.appendChild(appendExamText(el("div", "exam-rq-text"), grade.question_units, grade.question));
        question.appendChild(asked);
        const verdict = el("div", "exam-verdict");
        verdict.appendChild(el("span", `v-pill v-${grade.verdict.toLowerCase()}`, grade.verdict));
        verdict.appendChild(appendExamText(el("div", "vfb"), grade.feedback_units, grade.feedback));
        question.appendChild(verdict);
        if (grade.verdict !== "PASS") {
          if (grade.points.length) {
            question.appendChild(el("div", "exam-label", "A complete answer covers:"));
            const points = el("ul", "exam-points");
            grade.points.forEach((point, i) => {
              points.appendChild(appendExamText(el("li"), grade.points_units?.[i], point));
            });
            question.appendChild(points);
          }
          if (grade.missed.length) {
            question.appendChild(el("div", "exam-label", "You missed:"));
            const missed = el("ul", "exam-missed");
            grade.missed.forEach((point, i) => {
              missed.appendChild(appendExamText(el("li"), grade.missed_units?.[i], point));
            });
            question.appendChild(missed);
          }
        }
        wrap.appendChild(question);
      });
      stage.appendChild(wrap);
      if (current.can_remediate) {
        chip(
          current.error ? "Try remediation again" : "Add remediation cards",
          "primary",
          remediate,
        );
      }
      chip("Close", "", close, "esc");
      return;
    }

    if (current.phase === "remediated") {
      const count = current.remediated_count || 0;
      const headline = count === 0
        ? "No new remediation cards needed ✓"
        : `Created ${count} remediation card${count === 1 ? "" : "s"} ✓`;
      wrap.appendChild(el("div", "exam-pass", headline));
      wrap.appendChild(el("div", "exam-wait", "Re-drill the deck, then re-sit the exam."));
      stage.appendChild(wrap);
      chip("Close", "", close, "esc");
    }
  }

  function handleKey(event) {
    if (!data) return false;
    if (confirmingQuit) {
      if (event.key === "Enter") {
        event.preventDefault();
        close();
      } else if (event.key === "Escape") {
        event.preventDefault();
        cancelQuit();
      }
      return true;
    }
    if (event.key === "Escape") {
      event.preventDefault();
      quit();
    }
    return true;
  }

  return {
    cancelQuit,
    close,
    data: () => data,
    handleKey,
    isOpen,
    isPolling,
    quit,
    render,
    start,
  };
}
