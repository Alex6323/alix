import assert from "node:assert/strict";
import test from "node:test";

import { createKidsTutor } from "../../web/alix-kids/kids/tutor.js";

function element(text = "") {
  return {
    textContent: text,
    children: [],
    hidden: true,
    appendChild(child) { this.children.push(child); return child; },
    focus() {},
    set innerHTML(_) { this.children = []; },
  };
}

test("closing kids tutor stops polling but preserves transcript", async () => {
  let resolveSecond;
  const second = new Promise((resolve) => { resolveSecond = resolve; });
  const transcript = {
    transcript: [{ q: "Why?", a: "Because." }],
    thinking: true,
    status: null,
    error: null,
  };
  const responses = [Promise.resolve(transcript), second];
  const cleared = [];
  const log = element();
  const overlay = element();
  const mascotSlot = element();
  overlay.querySelector = () => mascotSlot;
  const tutor = createKidsTutor({
    api: async () => responses.shift(),
    post: (body) => ({ method: "POST", body }),
    resyncStudy: () => {},
    timers: {
      setInterval: () => 17,
      clearInterval: (id) => cleared.push(id),
    },
    ui: {
      mascot: () => element(),
      input: { ...element(), value: "", disabled: false },
      log,
      overlay,
      sendButton: { ...element(), disabled: false },
      el: (_tag, _className, text) => element(text),
    },
  });

  await tutor.open();
  assert.equal(tutor.isOpen(), true);

  tutor.close();
  assert.equal(tutor.isOpen(), false);
  assert.deepEqual(cleared, [17]);

  const reopening = tutor.open();
  assert.deepEqual(
    log.children.map((child) => child.textContent),
    ["Why?", "Because.", "Alix is thinking… 🤔"],
  );

  resolveSecond({ ...transcript, thinking: false });
  await reopening;
});
