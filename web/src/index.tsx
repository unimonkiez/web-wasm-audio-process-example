// Only used once
// deno-lint-ignore no-import-prefix no-unversioned-import
import { createRoot } from "npm:react-dom/client";
import { App } from "./App.tsx";

const onDomReady = (fn: () => void) => {
  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", fn, { once: true });
  } else {
    fn();
  }
};

onDomReady(() => {
  const root = createRoot(document.body);
  root.render(<App />);
});
