import { StrictMode, Suspense, lazy } from "react";
import { createRoot } from "react-dom/client";
import { RouterProvider } from "react-router-dom";
import "./design/vendor/globals.css";
// yunori.email tokens override the vendored client tokens (see scripts/sync-yunori-design.mjs).
import "./design/yunori/tokens.css";
import "./styles/fonts.css";
// Hand-written console/portal refinements (src/ui/README.md).
import "./styles/app.css";
import { router } from "./router";
import { Toaster } from "./lib/toast";
import { applyTheme } from "./lib/theme";

// public/theme-init.js normally does this before first paint; repeat it in case
// that script didn't load (e.g. `vite dev`, which double-prefixes %BASE_URL%).
applyTheme();

// Mock builds only: the scenario switcher. `__YUNORI_MOCK__` is a build-time
// literal, so production bundles drop this import entirely.
const MockPanel = __YUNORI_MOCK__ ? lazy(() => import("./api/mock-panel")) : null;

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <RouterProvider router={router} />
    <Toaster />
    {MockPanel && (
      <Suspense fallback={null}>
        <MockPanel />
      </Suspense>
    )}
  </StrictMode>,
);
