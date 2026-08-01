export function createKidsTutor({ api, post, resyncStudy, timers, ui }) {
const {
  mascot,
  input: askInput,
  log: askLog,
  overlay: askOverlay,
  sendButton: askSendBtn,
  el,
} = ui;
let askOpen = false;
let askData = { transcript: [], thinking: false, status: null, error: null };
let askPoll = null;

// ── Ask-Alix tutor overlay ─────────────────────────────────────────────────
// A card-scoped chat, mirroring review.html's ask wiring (openAsk / sendAsk /
// startPoll / sync) but as a static modal shell -- like the settings menu
// or the pairing gate -- instead of a stage takeover, since kids.html never
// tears down the review screen behind it. The client sends only {question};
// the server derives the current card as context and applies the kid-safe
// system prompt (Task 2). No note-saving / model display in kids v1.
askOverlay.querySelector("#askMascotSlot").appendChild(mascot("mascot-sm"));

function openTutor() {
  askOpen = true;
  askOverlay.hidden = false;
  sync();
  return api("/api/ask").then((d) => {
    askData = d;
    if (askOpen) sync();
    if (d.thinking) startPoll();
  });
}
function closeTutor() {
  askOpen = false;
  stopPoll();
  askOverlay.hidden = true;
}
function sendTutor() {
  const q = askInput.value.trim();
  if (!q || askData.thinking) return;
  askInput.value = "";
  api("/api/ask", post({ question: q })).then((d) => { askData = d; if (askOpen) sync(); startPoll(); }).catch(resyncStudy);
}
function startPoll() {
  stopPoll();
  askPoll = timers.setInterval(() => {
    api("/api/ask").then((d) => { askData = d; if (!d.thinking) stopPoll(); if (askOpen) sync(); });
  }, 400);
}
function stopPoll() { if (askPoll) { timers.clearInterval(askPoll); askPoll = null; } }

// (Re)fill the bubble log + input/send disabled state from askData. A
// greeting bubble stands in until the first exchange; a raw askData.error
// never reaches the child -- just a warm, generic fallback line.
function sync() {
  askLog.innerHTML = "";
  if (!askData.transcript.length && !askData.thinking && !askData.error) {
    askLog.appendChild(el("div", "ask-bubble ask-bubble-a", "Hi! I'm Alix 🦊 Ask me anything about this card and I'll explain it in a fun way."));
  }
  for (const ex of askData.transcript) {
    askLog.appendChild(el("div", "ask-bubble ask-bubble-q", ex.q));
    askLog.appendChild(el("div", "ask-bubble ask-bubble-a", ex.a));
  }
  if (askData.thinking) askLog.appendChild(el("div", "ask-bubble ask-bubble-a ask-bubble-think", "Alix is thinking… 🤔"));
  if (askData.error) askLog.appendChild(el("div", "ask-bubble ask-bubble-a", "Oops, I couldn't think just now. Try asking again! 🦊"));
  askLog.scrollTop = askLog.scrollHeight;

  askInput.disabled = askData.thinking;
  askSendBtn.disabled = askData.thinking;
  if (!askData.thinking && askOpen) askInput.focus();
}

function handleTutorKey(event, studyCardOpen) {
  if (event.key === "Escape" && askOpen) {
    closeTutor();
    return true;
  }
  if (event.key === "?" && !askOpen && studyCardOpen) {
    openTutor();
    return true;
  }
  return false;
}

function isTutorOpen() {
  return askOpen;
}

return {
  close: closeTutor,
  handleKey: handleTutorKey,
  isOpen: isTutorOpen,
  open: openTutor,
  send: sendTutor,
};
}
