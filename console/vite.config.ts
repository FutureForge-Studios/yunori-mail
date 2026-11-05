import { defineConfig, loadEnv, type Plugin } from "vite";
import react from "@vitejs/plugin-react";
import path from "node:path";

/**
 * One SPA bundle, two route trees: /console/* (admin) and /account/* (portal).
 * The yunori-mail binary serves dist/index.html for every path under /console
 * and /account, and static assets under /_yunori/ — so `base` is /_yunori/ but
 * the app's routes live outside it.
 */
const BASE = "/_yunori/";

/** Dev/preview only: serve index.html for app routes that sit outside `base`. */
function spaRoutesOutsideBase(): Plugin {
  const isAppRoute = (url: string) => /^\/(?:console|account)(?:[/?#]|$)|^\/(?:[?#]|$)/.test(url);
  return {
    name: "yunori-spa-routes",
    configureServer(server) {
      server.middlewares.use((req, _res, next) => {
        if (req.url && isAppRoute(req.url) && req.headers.accept?.includes("text/html")) req.url = BASE;
        next();
      });
    },
    configurePreviewServer(server) {
      server.middlewares.use((req, _res, next) => {
        if (req.url && isAppRoute(req.url)) req.url = `${BASE}index.html`;
        next();
      });
    },
  };
}

export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, process.cwd(), "");
  const mock = env.VITE_MOCK === "1";
  return {
    base: BASE,
    plugins: [react(), spaRoutesOutsideBase()],
    resolve: {
      alias: { "@": path.resolve(__dirname, "src") },
    },
    define: {
      // A literal boolean so the mock (and its dev panel) is dead code — and
      // never emitted as a chunk — in the production build.
      __YUNORI_MOCK__: JSON.stringify(mock),
    },
    server: {
      port: 5174,
      // Real-backend dev: forward the API to a running yunori-mail.
      proxy: mock
        ? undefined
        : {
            "/api/yunori": {
              target: env.YUNORI_API_ORIGIN || "http://127.0.0.1:8787",
              changeOrigin: false,
              secure: false,
            },
          },
    },
    preview: { port: 4174 },
    build: {
      outDir: "dist",
      emptyOutDir: true,
      sourcemap: false,
      // Fonts and the favicon are tiny; keep them as files so the CSP can stay
      // `font-src 'self'` with no data: allowance.
      assetsInlineLimit: 0,
      target: "es2022",
      rollupOptions: {
        output: {
          // Long-lived vendor chunks: app releases don't invalidate React/Radix.
          manualChunks(id) {
            if (!id.includes("node_modules")) return undefined;
            if (/[\\/]node_modules[\\/](react|react-dom|scheduler|react-router|react-router-dom|cookie|set-cookie-parser)[\\/]/.test(id)) return "react";
            if (/[\\/]node_modules[\\/](@radix-ui|cmdk|@floating-ui|react-remove-scroll|react-remove-scroll-bar|react-style-singleton|use-callback-ref|use-sidecar|aria-hidden|get-nonce|tslib|use-sync-external-store)[\\/]/.test(id)) return "ui";
            return undefined;
          },
        },
      },
    },
  };
});
