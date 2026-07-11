/* alix theme picker.
 *
 * A gallery of named colour themes — the alix originals plus popular editor /
 * slide palettes (Dracula, Nord, Solarized, …). A theme is just a set of CSS
 * variables in theme.css selected by `data-theme` on <html>; the choice is
 * remembered in localStorage. The initial theme is applied by a tiny inline
 * <head> script (before first paint, so no flash); this file builds the picker
 * popover, opened from the page's "Theme…" trigger (`#theme-open`):
 *   hover a swatch  → preview it in the sample card inside the dialog
 *   click a swatch  → commit it to the whole app + remember
 *   close without clicking → the app theme is unchanged (preview was local). */
(function () {
  "use strict";
  var KEY = "alix-theme";
  var DEFAULT = "dark";

  // id · display name · mode (light|dark) · preview colours [background, accent, green]
  var THEMES = [
    { id: "dark",             name: "alix",             mode: "dark",  c: ["#0f1016", "#5fd7e0", "#86c986"] },
    { id: "light",            name: "alix Light",       mode: "light", c: ["#f4f4fa", "#0e7c86", "#138a5b"] },
    { id: "kid",              name: "Fun",              mode: "kids",  c: ["#fff2db", "#7a2ff5", "#00b86b"] },
    { id: "sunrise",          name: "Sunrise",          mode: "kids",  c: ["#fff5ec", "#a84400", "#177b47"] },
    { id: "ocean",            name: "Ocean",            mode: "kids",  c: ["#eafaf7", "#0a6f77", "#0f7a55"] },
    { id: "berry",            name: "Berry",            mode: "kids",  c: ["#fdeefb", "#a02fae", "#167846"] },
    { id: "github-dark",      name: "GitHub",           mode: "dark",  c: ["#0d1117", "#2f81f7", "#3fb950"] },
    { id: "github-light",     name: "GitHub Light",     mode: "light", c: ["#ffffff", "#0969da", "#1a7f37"] },
    { id: "one-dark",         name: "One Dark",         mode: "dark",  c: ["#282c34", "#61afef", "#98c379"] },
    { id: "dracula",          name: "Dracula",          mode: "dark",  c: ["#282a36", "#bd93f9", "#50fa7b"] },
    { id: "monokai",          name: "Monokai",          mode: "dark",  c: ["#272822", "#f92672", "#a6e22e"] },
    { id: "catppuccin-mocha", name: "Catppuccin Mocha", mode: "dark",  c: ["#1e1e2e", "#cba6f7", "#a6e3a1"] },
    { id: "catppuccin-latte", name: "Catppuccin Latte", mode: "light", c: ["#eff1f5", "#8839ef", "#40a02b"] },
    { id: "tokyo-night",      name: "Tokyo Night",      mode: "dark",  c: ["#1a1b26", "#7aa2f7", "#9ece6a"] },
    { id: "solarized-dark",   name: "Solarized",        mode: "dark",  c: ["#002b36", "#268bd2", "#859900"] },
    { id: "solarized-light",  name: "Solarized Light",  mode: "light", c: ["#fdf6e3", "#268bd2", "#859900"] },
    { id: "gruvbox-dark",     name: "Gruvbox",          mode: "dark",  c: ["#282828", "#fabd2f", "#b8bb26"] },
    { id: "gruvbox-light",    name: "Gruvbox Light",    mode: "light", c: ["#fbf1c7", "#458588", "#98971a"] },
    { id: "nord",             name: "Nord",             mode: "dark",  c: ["#2e3440", "#88c0d0", "#a3be8c"] },
    { id: "ayu-dark",         name: "Ayu",              mode: "dark",  c: ["#0d1017", "#e6b450", "#aad94c"] },
    { id: "rose-pine",        name: "Rosé Pine",        mode: "dark",  c: ["#191724", "#c4a7e7", "#31748f"] },
    { id: "everforest-dark",  name: "Everforest",       mode: "dark",  c: ["#2d353b", "#a7c080", "#a7c080"] },
  ];

  function saved() { try { return localStorage.getItem(KEY); } catch (e) { return null; } }
  function setTheme(id) { document.documentElement.dataset.theme = id; }
  function commit(id) { setTheme(id); try { localStorage.setItem(KEY, id); } catch (e) {} }
  // Preview re-themes only the sample card in the dialog (a scoped data-theme),
  // never the whole app — the app theme changes on commit (a click) alone.
  function previewSample(id) { if (sample) sample.dataset.theme = id; }

  var panel = null, backdrop = null, sample = null, isOpen = false;

  function build() {
    backdrop = document.createElement("div");
    backdrop.className = "theme-backdrop";
    backdrop.addEventListener("click", close);

    panel = document.createElement("div");
    panel.className = "theme-panel";
    panel.setAttribute("role", "dialog");
    panel.setAttribute("aria-label", "Choose a theme");

    var head = document.createElement("div");
    head.className = "theme-head";
    head.innerHTML = '<span>Theme</span>';
    var x = document.createElement("button");
    x.type = "button"; x.className = "theme-x"; x.setAttribute("aria-label", "Close"); x.textContent = "✕";
    x.addEventListener("click", close);
    head.appendChild(x);
    panel.appendChild(head);

    // A mini card that shows the previewed theme without touching the app.
    sample = document.createElement("div");
    sample.className = "theme-sample";
    sample.dataset.theme = saved() || DEFAULT;
    sample.innerHTML =
      '<div class="ts-card">' +
        '<div class="ts-q">What guarantee does ownership give each value?</div>' +
        '<div class="ts-a">Exactly one owner at a time.</div>' +
        '<div class="ts-note">Lets Rust free memory with no garbage collector.</div>' +
      '</div>';
    panel.appendChild(sample);

    ["light", "dark", "kids"].forEach(function (mode) {
      var group = THEMES.filter(function (t) { return t.mode === mode; });
      if (!group.length) return;
      var label = document.createElement("div");
      label.className = "theme-group-label";
      label.textContent = mode === "light" ? "Light" : mode === "dark" ? "Dark" : "Kids";
      panel.appendChild(label);
      var grid = document.createElement("div");
      grid.className = "theme-grid";
      group.forEach(function (t) {
        var cell = document.createElement("button");
        cell.type = "button";
        cell.className = "theme-cell";
        cell.dataset.tid = t.id;
        cell.setAttribute("aria-label", t.name + " theme");
        var prev = document.createElement("span");
        prev.className = "theme-prev";
        prev.style.background = t.c[0];
        [t.c[1], t.c[2]].forEach(function (col) {
          var dot = document.createElement("i");
          dot.className = "theme-dot";
          dot.style.background = col;
          prev.appendChild(dot);
        });
        var name = document.createElement("span");
        name.className = "theme-name";
        name.textContent = t.name;
        cell.appendChild(prev);
        cell.appendChild(name);
        cell.addEventListener("mouseenter", function () { previewSample(t.id); }); // preview in the sample card only
        cell.addEventListener("focus", function () { previewSample(t.id); });
        cell.addEventListener("click", function () { commit(t.id); previewSample(t.id); mark(); });
        grid.appendChild(cell);
      });
      panel.appendChild(grid);
    });

    // Leaving the panel without committing resets the preview card to the saved theme.
    panel.addEventListener("mouseleave", function () { previewSample(saved() || DEFAULT); });

    document.body.appendChild(backdrop);
    document.body.appendChild(panel);
    mark();
  }

  function mark() {
    if (!panel) return;
    var active = saved() || DEFAULT;
    var cells = panel.querySelectorAll(".theme-cell");
    Array.prototype.forEach.call(cells, function (c) {
      var on = c.dataset.tid === active;
      c.classList.toggle("on", on);
      c.setAttribute("aria-pressed", on ? "true" : "false");
    });
  }

  function openPanel() {
    if (!panel) build();
    mark();
    previewSample(saved() || DEFAULT); // open showing the committed theme
    backdrop.classList.add("show");
    panel.classList.add("show");
    isOpen = true;
  }
  function close() {
    if (!isOpen) return;
    backdrop.classList.remove("show");
    panel.classList.remove("show");
    isOpen = false;
    previewSample(saved() || DEFAULT); // app theme is untouched by preview; just reset the sample
  }
  function toggle() { isOpen ? close() : openPanel(); }

  function wire() {
    var trigger = document.getElementById("theme-open");
    if (trigger) {
      trigger.addEventListener("click", function (e) {
        e.preventDefault();
        var m = document.querySelector(".menu.open"); // close the ⋮ menu if the trigger lives in it
        if (m) m.classList.remove("open");
        toggle();
      });
    }
    document.addEventListener("keydown", function (e) {
      if (e.key === "Escape" && isOpen) close();
    });
  }

  // Keep the live theme synced with the saved one (defensive), then wire the picker.
  setTheme(saved() || DEFAULT);
  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", wire);
  else wire();
})();
