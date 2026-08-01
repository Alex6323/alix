export function createSheets({
  api,
  fetchApi,
  post,
  withToken,
  focusedRowName,
  notice,
  refreshPicker,
  timers,
  ui,
}) {
  const {
    document: doc,
    el,
    FileReader: FileReaderCtor,
    Option: OptionCtor,
  } = ui;
  let open = false;

  // A small modal sheet (keyboard shortcuts, about). Esc or a backdrop click closes.
  const sheet = doc.getElementById("sheet");
  const sheetPanel = doc.getElementById("sheetPanel");
  // The interval id of whichever share/receive live-job poll is currently
  // running (never generate/import's jobPoll — those keep polling in the
  // background after close so a finished job still lands a toast). Set at
  // kick time, cleared here rather than left to self-clear off a stray 409.
  let liveJobTimer = null;
  function show(html) { sheetPanel.innerHTML = html; sheet.hidden = false; open = true; }
  function close() {
    // A share/receive sheet closed mid-wait must not leave the wormhole child
    // running invisibly — the job only cancels when replaced/cleared
    // server-side (see `ShareJob`'s `Drop`), so an abandoned sheet has to ask
    // for that itself.
    if (sheet.dataset.shareLive) { delete sheet.dataset.shareLive; api("/api/share/close", post({})).catch(() => {}); }
    if (sheet.dataset.receiveLive) { delete sheet.dataset.receiveLive; api("/api/receive/close", post({})).catch(() => {}); }
    if (liveJobTimer) { timers.clearInterval(liveJobTimer); liveJobTimer = null; }
    sheet.hidden = true; sheetPanel.innerHTML = ""; open = false;
  }
  sheet.addEventListener("click", (e) => { if (e.target === sheet) close(); });
  doc.addEventListener("keydown", (e) => { if (!sheet.hidden && e.key === "Escape") { e.stopPropagation(); e.preventDefault(); close(); } }, true);

  // Polls a job endpoint (~700ms) into #jobLine inside the open Add sheet.
  // done and error land IN the sheet while it's open; closed, error falls
  // back to a toast. Script-level (not a closure inside openAdd) so Task 13's
  // share/receive wiring can reuse it. The job-line element is captured once,
  // here at kick time, rather than re-resolved by id on every tick — a
  // re-resolve would find whatever sheet happens to be open by the time a
  // background job's tick lands, and write this job's progress into it.
  // `onError`, if given, runs (in addition to the rendering below) when the
  // job reaches the error phase — receive uses it to clear its cancel-on-close
  // flag; generate/import have none and simply omit it.
  function jobPoll(path, verb, onDone, onError) {
    const line = doc.getElementById("jobLine");
    const t = timers.setInterval(() => {
      api(path).then((d) => {
        if (d.phase === "done") {
          timers.clearInterval(t);
          api(path + "/close", post({})).catch(() => {});
          onDone(d);
        } else if (d.phase === "error") {
          timers.clearInterval(t);
          if (onError) onError(d);
          if (line && line.isConnected) {
            line.textContent = "";
            line.appendChild(el("span", "sheet-err", d.error || (verb + " failed")));
          } else {
            notice(d.error || (verb + " failed"));
          }
        } else if (line && line.isConnected) {
          line.textContent = verb + "… " + (d.elapsed || 0) + "s";
        }
      }).catch(() => { timers.clearInterval(t); notice(verb + " failed: the server log has details"); });
    }, 700);
    return t;
  }

  // Fills the Add sheet's destination <select> with Library root + every
  // workspace name, from GET /api/decks. Built with DOM Option nodes (never
  // innerHTML) since a workspace name is a user-chosen folder name.
  function fillDestOptions(sel, d) {
    sel.innerHTML = "";
    sel.appendChild(new OptionCtor("Library root", ""));
    (d.workspaces || []).forEach((w) => sel.appendChild(new OptionCtor(w.name, w.name)));
  }

  function addDone(msg) { close(); notice(msg); refreshPicker(); }

  // The Add-deck sheet: generate from a URL, import a file, or receive a
  // wormhole share/zip, all landing in a chosen destination (library root or a
  // workspace). Fetches the destination list before opening so the <select>
  // never flashes empty.
  function openAdd() {
    api("/api/decks").catch(() => ({})).then((d) => {
      show(
        '<h2>Add deck</h2><div class="sheet-add">' +
        '<label>Into <select id="addDest" class="bar-filter"></select></label>' +
        '<div class="add-sec"><h3>Generate from a URL</h3>' +
        '<input id="genUrl" class="bar-filter" placeholder="https://…" autocomplete="off" spellcheck="false">' +
        '<input id="genGuide" class="bar-filter" placeholder="guidance (optional)" autocomplete="off">' +
        '<button id="genGo" class="bar-chip">Generate</button></div>' +
        '<div class="add-sec"><h3>Import a file</h3>' +
        '<input id="impFile" type="file" accept=".tsv,.txt"></div>' +
        '<div class="add-sec"><h3>Receive</h3>' +
        '<input id="rcvCode" class="bar-filter" placeholder="wormhole code" autocomplete="off" spellcheck="false">' +
        '<button id="rcvGo" class="bar-chip">Receive</button>' +
        '<input id="rcvZip" type="file" accept=".zip"></div>' +
        '<p id="jobLine"></p></div>'
      );
      fillDestOptions(doc.getElementById("addDest"), d);
      const dest = () => doc.getElementById("addDest").value;
      doc.getElementById("genGo").addEventListener("click", () => {
        const url = doc.getElementById("genUrl").value.trim();
        if (!url) return;
        const guidance = doc.getElementById("genGuide").value.trim() || null;
        // Captured once, here at kick time, rather than re-resolved by id when
        // the response lands — a re-resolve could find the sheet already
        // closed (null) or a different sheet's line. See jobPoll's comment.
        const line = doc.getElementById("jobLine");
        api("/api/generate", post({ url, guidance, dest: dest() || null }))
          .then((d) => {
            if (d && d.phase === "error") {
              const msg = d.error || "could not start generating";
              if (line && line.isConnected) { line.textContent = msg; } else { notice(msg); }
              return;
            }
            jobPoll("/api/generate", "generating", (r) => addDone("deck '" + r.deck + "' added"));
          })
          .catch(() => notice("could not start generating: the server log has details"));
        if (line && line.isConnected) { line.textContent = "generating… 0s"; }
      });
      doc.getElementById("impFile").addEventListener("change", (e) => {
        const f = e.target.files[0];
        if (!f) return;
        const line = doc.getElementById("jobLine");
        const r = new FileReaderCtor();
        r.onload = () => {
          api("/api/import", post({ name: f.name, text: r.result, dest: dest() || null }))
            .then((d2) => addDone("imported " + d2.cards + " cards into '" + d2.deck + "'"))
            .catch(() => {
              const msg = "import failed: not a valid deck, or the name is taken";
              if (line && line.isConnected) line.textContent = msg;
              else notice(msg);
            });
        };
        r.readAsText(f);
      });
      doc.getElementById("rcvGo").addEventListener("click", () => {
        const code = doc.getElementById("rcvCode").value.trim();
        if (!code) return;
        // Captured once, here at kick time, rather than re-resolved by id when
        // the response lands — a re-resolve could find the sheet already
        // closed (null) or a different sheet's line. See jobPoll's comment.
        const line = doc.getElementById("jobLine");
        api("/api/receive", post({ code, dest: dest() || null }))
          .then((d) => {
            if (d && d.phase === "error") {
              const msg = d.error || "could not start receiving";
              if (line && line.isConnected) { line.textContent = msg; } else { notice(msg); }
              return;
            }
            sheet.dataset.receiveLive = "1"; // close() must cancel the job if abandoned
            liveJobTimer = jobPoll(
              "/api/receive", "receiving",
              (r) => { delete sheet.dataset.receiveLive; addDone("received '" + r.landed + "'"); },
              () => { delete sheet.dataset.receiveLive; }
            );
          })
          .catch(() => notice("could not start receiving: the server log has details"));
        if (line && line.isConnected) { line.textContent = "receiving… 0s"; }
      });
      doc.getElementById("rcvZip").addEventListener("change", (e) => {
        const f = e.target.files[0];
        if (!f) return;
        f.arrayBuffer().then((buf) =>
          fetchApi("/api/receive/zip?dest=" + encodeURIComponent(dest()), { method: "POST", body: buf })
            .then((resp) => { if (!resp.ok) throw 0; return resp.json(); })
            .then((r) => addDone("received '" + r.landed + "'"))
            .catch(() => { doc.getElementById("jobLine").textContent = "could not unpack that zip"; })
        );
      });
    });
  }

  // The Share sheet: send the focused row (or, unfocused, the whole library)
  // device-to-device via a wormhole code, or fall back to a plain zip download.
  // `deck: null` in the POST body shares the served root, matching the zip
  // link's bare (no `?deck=`) href — both resolve server-side the same way.
  function openShare() {
    const row = focusedRowName();
    const zipHref = "/api/share/zip" + (row ? "?deck=" + encodeURIComponent(row) : "");
    show(
      '<h2>Share</h2><div class="sheet-add">' +
      '<p>Share <b></b> device-to-device. Progress and personal config stay home.</p>' +
      '<button id="shareGo" class="bar-chip">Get code</button>' +
      '<p><a id="shareZip" download>or download as .zip</a></p>' +
      '<p id="jobLine"></p></div>'
    );
    sheetPanel.querySelector("b").textContent = row || "the whole library";
    doc.getElementById("shareZip").href = withToken(zipHref);
    doc.getElementById("shareGo").addEventListener("click", () => {
      // Captured once, here at kick time, rather than re-resolved by id when
      // the response lands or on every tick — a re-resolve could find the
      // sheet already closed (null) or a different sheet's line. See
      // jobPoll's comment for why that matters.
      const line = doc.getElementById("jobLine");
      api("/api/share", post({ deck: row }))
        .then((d) => {
          if (d && d.phase === "error") {
            const msg = d.error || "could not start sharing";
            if (line && line.isConnected) { line.textContent = msg; } else { notice(msg); }
            return;
          }
          if (line && line.isConnected) { line.textContent = "staging…"; }
          sheet.dataset.shareLive = "1"; // close() must cancel the job if abandoned
          liveJobTimer = timers.setInterval(() => {
            api("/api/share").then((d2) => {
              if (d2.phase === "code") {
                if (!line || !line.isConnected) return; // nothing to render, and not terminal
                // Rendered once and left alone — rebuilding this node every
                // tick would wipe the user's in-progress selection of the code.
                if (!line.querySelector(".share-code")) {
                  line.textContent = "";
                  line.appendChild(el("span", "share-code", d2.code));
                  line.appendChild(doc.createElement("br"));
                  line.appendChild(doc.createTextNode("waiting for the receiver…"));
                }
              } else if (d2.phase === "sent") {
                timers.clearInterval(liveJobTimer);
                delete sheet.dataset.shareLive; // already closing it below — don't double-close
                api("/api/share/close", post({})).catch(() => {});
                close();
                notice("sent");
              } else if (d2.phase === "error") {
                timers.clearInterval(liveJobTimer);
                if (line && line.isConnected) {
                  line.textContent = "";
                  line.appendChild(el("span", "sheet-err", d2.error || "share failed"));
                } else {
                  notice(d2.error || "share failed");
                }
              }
            }).catch(() => timers.clearInterval(liveJobTimer));
          }, 700);
        })
        .catch(() => notice("could not start sharing: the server log has details"));
    });
  }

  // The Reset sheet: wipe a row's review progress, gated on typing its exact
  // name back (a plain confirm dialog is too easy to reflex-click through for
  // something this destructive). No focused row → nothing to reset.
  function openReset() {
    const row = focusedRowName();
    if (!row) { notice("focus a deck first"); return; }
    show(
      '<h2>Reset progress</h2><div class="sheet-add">' +
      '<p>Wipes all review progress for <b></b>: schedules, history, exam state. This cannot be undone.</p>' +
      '<input id="resetConfirm" class="bar-filter" placeholder="type the name to confirm" autocomplete="off" spellcheck="false">' +
      '<button id="resetGo" class="bar-chip" disabled>Reset</button><p id="jobLine"></p></div>'
    );
    sheetPanel.querySelector("b").textContent = row;
    const input = doc.getElementById("resetConfirm");
    const go = doc.getElementById("resetGo");
    input.addEventListener("input", () => { go.disabled = input.value !== row; });
    go.addEventListener("click", () => {
      api("/api/reset", post({ deck: row }))
        .then((d) => { close(); notice("reset " + d.cards_cleared + " card(s)"); refreshPicker(); })
        .catch(() => { doc.getElementById("jobLine").textContent = "reset failed: the server log has details"; });
    });
  }

  // The Doctor sheet: one row per environment/backend check from /api/doctor
  // (config, store, decks, backend, share, wormhole — an open set), each with a
  // status glyph and, when something needs fixing, a muted remedy line.
  function doctorRow(r) {
    const glyph = r.status === "ok" ? "✓" : r.status === "warn" ? "!" : r.status === "fail" ? "✗" : "?";
    const row = el("div", "doc-row doc-" + r.status);
    row.appendChild(el("span", "doc-glyph", glyph));
    const body = el("span");
    body.appendChild(el("b", null, r.name));
    body.appendChild(el("span", "doc-detail", ": " + r.detail));
    if (r.remedy) body.appendChild(el("span", "doc-remedy", r.remedy));
    row.appendChild(body);
    return row;
  }
  function openDoctor() {
    api("/api/doctor").then((d) => {
      show('<h2>Doctor</h2><div class="sheet-doctor" id="docRows"></div>');
      const rows = doc.getElementById("docRows");
      (d.rows || []).forEach((r) => rows.appendChild(doctorRow(r)));
    }).catch(() => notice("could not run the checks"));
  }

  // The Pair sheet: a QR + URL for reaching this instance from another device.
  // Localhost-only servers get a plain hint instead (nothing to scan).
  function openPair() {
    api("/api/pair").then((d) => {
      if (!d.lan) {
        show('<h2>Pair a device</h2><p class="sheet-hint"></p>');
        sheetPanel.querySelector(".sheet-hint").textContent =
          "This server is localhost-only. Start alix with --lan to pair another device.";
        return;
      }
      show(
        '<h2>Pair a device</h2><div class="sheet-pair">' +
        '<div class="pair-qr"></div><p class="pair-url"></p>' +
        '<p class="sheet-hint">Scan, or open the link on the other device.</p></div>'
      );
      if (d.svg) {
        // d.svg is a complete, self-contained <svg> rendered server-side by our
        // own qr::svg (same-origin, trusted, documented in docs/API.md as safe
        // to inject) — the one sanctioned innerHTML use for API data, scoped to
        // this dedicated container and nothing else concatenated into it.
        sheetPanel.querySelector(".pair-qr").innerHTML = d.svg;
      }
      sheetPanel.querySelector(".pair-url").textContent = d.url;
    }).catch(() => notice("could not fetch the pairing info"));
  }

  function openShortcuts() {
    show(
      '<h2>Picker shortcuts</h2><div class="sheet-keys">' +
      '<kbd>/</kbd><span>filter the list</span>' +
      '<kbd>↑ ↓</kbd><span>move</span>' +
      '<kbd>enter</kbd><span>open / start</span>' +
      '<kbd>v</kbd><span>choose a depth, then <kbd>1</kbd> <kbd>2</kbd> <kbd>3</kbd> (<kbd>c</kbd> crams)</span>' +
      '<kbd>b</kbd><span>browse the deck</span>' +
      '<kbd>x</kbd><span>take the exam</span>' +
      '<kbd>m</kbd><span>mastered decks</span>' +
      '<kbd>g / G</kbd><span>top / bottom</span>' +
      '<kbd>← →</kbd><span>step regions (in the focus drawer)</span>' +
      '<kbd>r</kbd><span>refresh the deck list</span>' +
      '<kbd>esc / ⌫</kbd><span>back</span>' +
      '</div>'
    );
  }


  function openAbout() {
    return api("/api/version").catch(() => ({})).then((d) => {
      const version = d && d.version ? "v" + d.version : "";
      show(
        '<h2>About</h2><div class="sheet-about">' +
        '<p class="about-name">alix <b>' + version + '</b></p>' +
        '<p class="about-tag">Spaced repetition with an AI exam that checks understanding. Early and changing fast.</p>' +
        '<p><a href="https://alix.study" target="_blank" rel="noopener">alix.study</a></p>' +
        '<p class="about-support">Free and open source. Telling someone who studies is the best support. ' +
        '<a href="https://github.com/sponsors/Alex6323" target="_blank" rel="noopener">Sponsor</a></p>' +
        '</div>'
      );
      return d;
    });
  }

  return {
    close,
    isOpen: () => open,
    openAbout,
    openAdd,
    openDoctor,
    openPair,
    openReset,
    openShare,
    openShortcuts,
  };
}
