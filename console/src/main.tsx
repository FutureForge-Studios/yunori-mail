import { StrictMode, Suspense, lazy } from "react";
import { createRoot } from "react-dom/client";
import { RouterProvider } from "react-router-dom";
import "./design/vendor/globals.css";
import "./styles/fonts.css";
import { router } from "./router";
import { Toaster } from "./lib/toast";

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
