export function createTutor({
  api,
  post,
  rerender,
  updateBusy,
  timers,
  walk,
  study,
  ui,
}) {
  const {
    appendRuns,
    appendRunsOrText,
    chip,
    contextLine,
    document: doc,
    el,
    keys,
    label,
    legend,
    stage,
  } = ui;
  let open = false;
  let data = { transcript: [], thinking: false, status: null, error: null };
  let info = { backend: "claude", model: "default", effort: "default" };
  let poll = null;
  let confirmingClose = false;
  let needsStateRefresh = false;

  function isOpen() {
    return open;
  }

  function isPolling() {
    return poll !== null;
  }

  function setInfo(next) {
    if (next) info = next;
  }

  function backendName() {
    return info.backend.charAt(0).toUpperCase() + info.backend.slice(1);
  }

  function endpoint(suffix = "") {
    return walk.isOpen() ? `/api/walk/ask${suffix}` : `/api/ask${suffix}`;
  }

  function show() {
    open = true;
    confirmingClose = false;
    needsStateRefresh = false;
    rerender();
    return api(endpoint()).then((next) => {
      data = next;
      if (open) sync();
      return next;
    });
  }

  function close() {
    if (!confirmingClose && data && data.transcript && data.transcript.length) {
      confirmingClose = true;
      renderLeaveConfirm();
      return Promise.resolve(false);
    }
    confirmingClose = false;
    open = false;
    stopPoll();
    if (walk.isOpen() && needsStateRefresh) {
      return api("/api/walk").then((next) => {
        walk.replace(next);
        rerender();
        return true;
      }).catch(() => rerender());
    }
    if (!walk.isOpen() && needsStateRefresh) {
      return api("/api/state").then((next) => {
        study.replaceState(next);
        rerender();
        return true;
      }).catch(() => rerender());
    }
    rerender();
    return Promise.resolve(true);
  }

  function send(text) {
    const question = (text || "").trim();
    if (!question || data.thinking) return Promise.resolve(data);
    const input = doc.querySelector(".ask-input");
    if (input) input.value = "";
    return api(endpoint(), post({ question })).then((next) => {
      data = next;
      if (open) sync();
      startPoll();
      return next;
    }).catch(() => study.load());
  }

  function canDistill() {
    return !data.thinking && data.transcript.length > 0;
  }

  function saveNote() {
    if (!canDistill()) return Promise.resolve(data);
    needsStateRefresh = true;
    return api(endpoint("/note"), post({})).then((next) => {
      data = next;
      if (open) sync();
      startPoll();
      return next;
    }).catch(() => study.load());
  }

  function draftCard() {
    if (!canDistill()) return Promise.resolve(data);
    return api("/api/ask/card/draft", post({})).then((next) => {
      data = next;
      if (open) sync();
      startPoll();
      return next;
    }).catch(() => {
      data = { ...data, status: "couldn't draft a card" };
      if (open) sync();
      return data;
    });
  }

  function createCard(front, back) {
    return api("/api/ask/card/create", post({ front, back })).then(() => {
      data = { ...data, draft: null, status: "card added" };
      if (open) sync();
      return data;
    }).catch(() => {
      data = { ...data, status: "couldn't add that card" };
      if (open) sync();
      return data;
    });
  }

  function cancelDraft() {
    data = { ...data, draft: null };
    if (open) sync();
  }

  function startPoll() {
    stopPoll();
    poll = timers.setInterval(() => {
      api(endpoint()).then((next) => {
        data = next;
        if (!next.thinking) {
          stopPoll();
          refreshInfo();
        }
        if (open) sync();
      });
    }, 400);
    updateBusy();
  }

  function stopPoll() {
    if (poll) {
      timers.clearInterval(poll);
      poll = null;
    }
    updateBusy();
  }

  function refreshInfo() {
    if (info.model !== "default") return;
    api("/api/ask-info").then((next) => {
      if (next) {
        info = next;
        if (open) sync();
      }
    }).catch(() => {});
  }

  function fillLog(log) {
    const signature = JSON.stringify([
      data.transcript.length,
      data.thinking,
      data.status,
      data.error,
    ]);
    if (log._sig === signature) return;
    log._sig = signature;
    log.innerHTML = "";
    for (const exchange of data.transcript) {
      log.appendChild(el("div", "ask-q", exchange.q));
      log.appendChild(el("div", "ask-a", exchange.a));
    }
    if (data.thinking) {
      const thinking = el("div", "ask-thinking");
      const logo = doc.createElement("alix-logo");
      logo.setAttribute("height", "18");
      logo.setAttribute("loop", "");
      thinking.appendChild(logo);
      thinking.appendChild(doc.createTextNode("Thinking…"));
      log.appendChild(thinking);
    }
    if (data.status) log.appendChild(el("div", "ask-status", data.status));
    if (data.error) log.appendChild(el("div", "ask-error", data.error));
    if (!data.transcript.length && !data.thinking && !data.status && !data.error) {
      log.appendChild(el(
        "div",
        "ask-hint",
        walk.isOpen() ? "Ask the tutor about this step." : "Ask the tutor about this card.",
      ));
    }
    const scroll = log.closest(".ask-scroll") || log;
    scroll.scrollTop = scroll.scrollHeight;
  }

  function buildDraftBox() {
    if (!data.draft) return null;
    const box = el("div", "draft-box");
    box.appendChild(el("div", "draft-label", "New card (edit, then Add):"));
    const front = el("input", "draft-front");
    front.value = data.draft.front;
    box.appendChild(front);
    const back = el("textarea", "draft-back");
    back.value = data.draft.back.join("\n");
    back.rows = Math.max(2, data.draft.back.length);
    box.appendChild(back);
    const actions = el("div", "draft-actions");
    chip("Add", "primary", () => {
      const lines = back.value.split("\n").map((line) => line.trim()).filter(Boolean);
      createCard(front.value.trim(), lines);
    }, "", actions);
    chip("Cancel", "", cancelDraft, "", actions);
    box.appendChild(actions);
    return box;
  }

  function sync() {
    const wrap = doc.querySelector(".ask-panel");
    if (!wrap) {
      rerender();
      return;
    }
    fillLog(wrap.querySelector(".ask-log"));
    const previousDraft = wrap.querySelector(".draft-box");
    if (previousDraft) previousDraft.remove();
    const draft = buildDraftBox();
    if (draft) wrap.insertBefore(draft, wrap.querySelector(".ask-input"));
    const input = wrap.querySelector(".ask-input");
    if (input) {
      input.disabled = data.thinking;
      if (!data.thinking) input.focus();
    }
    const sendButton = legend.querySelector(".chip.primary");
    if (sendButton) sendButton.disabled = data.thinking;
    legend.querySelectorAll(".chip.distill").forEach((button) => {
      button.disabled = !canDistill();
    });
  }

  function render(subject) {
    const wrap = el("div", "ask-panel");
    const head = el("div", "ask-head");
    const title = el("span");
    title.appendChild(el("span", "ask-eyebrow", "ASK TUTOR"));
    title.appendChild(el("span", "ask-scope", subject ? "· step-scoped" : "· card-scoped"));
    head.appendChild(title);
    head.appendChild(el(
      "span",
      "ask-model",
      `${info.backend} · model: ${info.model} · effort: ${info.effort}`,
    ));
    wrap.appendChild(head);

    const scroll = el("div", "ask-scroll");
    if (subject) {
      const reference = el("div", "ask-card");
      const prompt = el("div", "ask-card-q");
      appendRunsOrText(prompt, subject.q, subject.qRuns);
      reference.appendChild(prompt);
      for (let index = 0; index < (subject.items || []).length; index++) {
        const answer = el("div", "ask-card-a");
        answer.appendChild(doc.createTextNode("▸ "));
        appendRunsOrText(answer, subject.items[index], subject.itemRuns && subject.itemRuns[index]);
        reference.appendChild(answer);
      }
      scroll.appendChild(reference);
    } else {
      const card = study.state().card;
      if (card) {
        const reference = el("div", "ask-card");
        const front = el("div", "ask-card-q");
        if (card.front_runs) appendRuns(front, card.front_runs);
        else front.textContent = card.front;
        reference.appendChild(front);
        for (let index = 0; index < (card.context || []).length; index++) {
          reference.appendChild(contextLine(
            card.context[index],
            card.context_runs && card.context_runs[index],
            "ask-card-ctx",
          ));
        }
        for (let index = 0; index < card.back.length; index++) {
          const answer = el("div", "ask-card-a");
          if (card.back_runs && card.back_runs[index]) appendRuns(answer, card.back_runs[index]);
          else answer.textContent = card.back[index];
          reference.appendChild(answer);
        }
        scroll.appendChild(reference);
      }
    }
    const log = el("div", "ask-log");
    fillLog(log);
    scroll.appendChild(log);
    wrap.appendChild(scroll);

    const draft = buildDraftBox();
    if (draft) wrap.appendChild(draft);

    const input = el("textarea", "ask-input");
    input.placeholder = subject
      ? "Ask about this step… (Shift+Enter to send)"
      : "Ask about this card… (Shift+Enter to send)";
    input.rows = 2;
    input.disabled = data.thinking;
    input.addEventListener("keydown", (event) => {
      if (event.key === "Enter" && event.shiftKey) {
        event.preventDefault();
        send(input.value);
      }
    });
    wrap.appendChild(input);
    stage.appendChild(wrap);

    renderFooter(input);
    if (!data.thinking) input.focus();
  }

  function renderFooter(input) {
    legend.innerHTML = "";
    const sendButton = chip("Send", "primary", () => send(input.value), "shift+enter");
    sendButton.disabled = data.thinking;
    chip("Make this a note", "distill", saveNote, label(keys().make_note)).disabled = !canDistill();
    if (!walk.isOpen()) {
      chip("Make this a card", "distill", draftCard, label(keys().make_card)).disabled = !canDistill();
    }
    chip("Close", "", close, "esc");
  }

  function renderLeaveConfirm() {
    legend.innerHTML = "";
    legend.appendChild(el(
      "span",
      "leave-msg",
      "Leave the tutor? Moving on to the next card drops this conversation. Making a note or a card keeps it.",
    ));
    chip("Leave anyway", "again", close);
    chip("Stay", "primary", cancelClose, "esc");
  }

  function cancelClose() {
    confirmingClose = false;
    const input = doc.querySelector(".ask-input");
    if (input) {
      renderFooter(input);
      input.focus();
    }
  }

  function handleKey(event) {
    if (!open) return false;
    if (confirmingClose) {
      if (event.key === "Escape") {
        event.preventDefault();
        cancelClose();
      }
      return true;
    }
    if (event.key === "Escape") {
      event.preventDefault();
      close();
      return true;
    }
    if (ui.hit(event, keys().make_note)) {
      event.preventDefault();
      saveNote();
      return true;
    }
    if (ui.hit(event, keys().make_card)) {
      event.preventDefault();
      draftCard();
    }
    return true;
  }

  return {
    backendName,
    close,
    data: () => data,
    draftCard,
    handleKey,
    isOpen,
    isPolling,
    render,
    saveNote,
    send,
    setInfo,
    show,
  };
}
