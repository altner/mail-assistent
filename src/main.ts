import { mount } from "svelte";
import App from "./App.svelte";
import "./app.css";

// WebView-Kontextmenü („Neu laden“ usw.) im Release-Build unterdrücken.
// Eingabefelder und markierter Text behalten es für Kopieren/Einfügen,
// im Dev-Modus bleibt es ganz, damit „Element untersuchen“ erreichbar ist.
if (!import.meta.env.DEV) {
  document.addEventListener("contextmenu", (event) => {
    const target = event.target as HTMLElement | null;
    const editable = target?.closest("input, textarea, [contenteditable='true']");
    const hasSelection = (window.getSelection()?.toString() ?? "").length > 0;
    if (!editable && !hasSelection) event.preventDefault();
  });
}

const app = mount(App, { target: document.getElementById("app")! });

export default app;
