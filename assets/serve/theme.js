/* alix theme picker.
 *
 * Builds the swatch control in #theme-picker and switches the `data-theme` on
 * <html>, remembering the choice in localStorage. The *initial* theme is set by
 * a tiny inline script in each page's <head> (before first paint, so there is no
 * flash of the wrong theme); this script only builds the control and handles
 * clicks. */
(function () {
  "use strict";
  var KEY = "alix-theme";
  var THEMES = [
    { id: "dark", label: "Dark" },
    { id: "light", label: "Light" },
    { id: "kid", label: "Fun" },
  ];

  function saved() {
    try { return localStorage.getItem(KEY); } catch (e) { return null; }
  }
  function setTheme(id) {
    document.documentElement.dataset.theme = id;
  }
  function choose(id) {
    setTheme(id);
    try { localStorage.setItem(KEY, id); } catch (e) {}
    render();
  }
  function render() {
    var mount = document.getElementById("theme-picker");
    if (!mount) return;
    var active = saved() || "dark";
    mount.textContent = "";
    mount.setAttribute("role", "group");
    mount.setAttribute("aria-label", "Theme");
    THEMES.forEach(function (t) {
      var b = document.createElement("button");
      b.type = "button";
      b.className = "theme-swatch" + (t.id === active ? " on" : "");
      b.dataset.theme = t.id;
      b.title = t.label + " theme";
      b.setAttribute("aria-label", t.label + " theme");
      b.setAttribute("aria-pressed", t.id === active ? "true" : "false");
      b.addEventListener("click", function () { choose(t.id); });
      mount.appendChild(b);
    });
  }

  // Defensive: keep the live theme in sync with the saved one even if a page's
  // inline head script is missing, then build the control once the DOM is ready.
  setTheme(saved() || "dark");
  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", render);
  } else {
    render();
  }
})();
