export function createKidsSettings({ theme, ui }) {
const {
  backdrop,
  button,
  document: doc,
  el,
  host,
  popup,
} = ui;

function buildSettings() {
  host.innerHTML = "";
  for (const name of theme.names()) {
    const value = theme.palette(name);
    const swatch = el("button", "swatch");
    swatch.type = "button";
    swatch.dataset.theme = name;
    swatch.title = name;
    swatch.setAttribute("aria-label", name);
    swatch.style.background = "linear-gradient(168deg, " + value.bgTop + ", " + value.bgBot + ")";
    const dot = el("span", "swatch-dot");
    dot.style.background = value.accent;
    swatch.appendChild(dot);
    swatch.addEventListener("click", () => {
      theme.set(name);
      updatePressedState();
      closeSettings();
    });
    host.appendChild(swatch);
  }
  updatePressedState();
}

function updatePressedState() {
  const swatches = doc.querySelectorAll(".swatch");
  for (const swatch of swatches) {
    swatch.setAttribute("aria-pressed", String(swatch.dataset.theme === theme.current()));
  }
}

function openSettings() {
  popup.hidden = false;
  backdrop.hidden = false;
  button.setAttribute("aria-expanded", "true");
  updatePressedState();
}

function closeSettings() {
  popup.hidden = true;
  backdrop.hidden = true;
  button.setAttribute("aria-expanded", "false");
}

function handleSettingsKey(event) {
  if (event.key !== "Escape") return false;
  const acted = !popup.hidden;
  closeSettings();
  return acted;
}

function isSettingsOpen() {
  return !popup.hidden;
}

function toggleSettings() {
  if (popup.hidden) openSettings();
  else closeSettings();
}

return {
  build: buildSettings,
  close: closeSettings,
  handleKey: handleSettingsKey,
  isOpen: isSettingsOpen,
  open: openSettings,
  toggle: toggleSettings,
};
}
