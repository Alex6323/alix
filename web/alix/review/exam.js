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
    alert: alertUser,
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

  function isOpen() {
    return data !== null;
  }

  function isPolling() {
    return poll !== null;
  }

  function start(deck) {
    rememberLaunch(deck);
    return api("/api/exam/start", post({ deck })).then((next) => {
      if (next && next.phase === "cooldown") {
        const mins = Math.max(1, Math.round(next.cooldown_ms / 60000));
        alertUser(
          "This trace exam is cooling down after a recent fail.\n" +
          `Re-walk it and try again in about ${mins} min.`,
        );
        return next;
      }
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
      wrap.appendChild(el("div", "exam-q", current.question || ""));
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
      wrap.appendChild(el(
        "div",
        current.passed ? "exam-pass" : "exam-fail",
        current.passed ? "PASSED: deck mastered ✓" : "FAILED",
      ));
      if (current.passed && current.unlocks.length) {
        wrap.appendChild(el("div", "exam-unlocks", `Unlocks: ${current.unlocks.join(", ")}`));
      }
      current.grades.forEach((grade, index) => {
        const question = el("div", "exam-result");
        question.appendChild(el("div", "exam-rq", `Q${index + 1}. ${grade.question}`));
        const verdict = el("div", "exam-verdict");
        verdict.appendChild(el("span", `v-pill v-${grade.verdict.toLowerCase()}`, grade.verdict));
        verdict.appendChild(el("span", "vfb", grade.feedback));
        question.appendChild(verdict);
        if (grade.verdict !== "PASS") {
          if (grade.points.length) {
            question.appendChild(el("div", "exam-label", "A complete answer covers:"));
            const points = el("ul", "exam-points");
            grade.points.forEach((point) => points.appendChild(el("li", null, point)));
            question.appendChild(points);
          }
          if (grade.missed.length) {
            question.appendChild(el("div", "exam-label", "You missed:"));
            const missed = el("ul", "exam-missed");
            grade.missed.forEach((point) => missed.appendChild(el("li", null, point)));
            question.appendChild(missed);
          }
        }
        wrap.appendChild(question);
      });
      if (!current.passed && current.is_trace) {
        wrap.appendChild(el(
          "div",
          "exam-wait",
          "Re-walk the trace to strengthen the weak hops, then re-sit.",
        ));
      }
      stage.appendChild(wrap);
      if (!current.passed && current.can_remediate) {
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
