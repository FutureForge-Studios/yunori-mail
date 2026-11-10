import { Suspense, lazy } from "react";
import { Navigate, createBrowserRouter } from "react-router-dom";
import { ConsoleRoot, RequireAdmin, Splash } from "./console/session";
import { ConsoleShell } from "./console/shell";
import { LoginPage } from "./console/pages/login";
import { OverviewPage } from "./console/pages/overview";
import { NotFound } from "./pages/not-found";

// Heavier / less-visited screens are split out; the portal and the first-run
// setup (QR code) never load for an admin who's already signed in.
const SetupPage = lazy(() => import("./console/pages/setup"));
const DomainsPage = lazy(() => import("./console/pages/domains"));
const DomainDetailPage = lazy(() => import("./console/pages/domain-detail"));
const MailboxesPage = lazy(() => import("./console/pages/mailboxes"));
const AliasesPage = lazy(() => import("./console/pages/aliases"));
const SendingPage = lazy(() => import("./console/pages/sending"));
const DeliverabilityPage = lazy(() => import("./console/pages/deliverability"));
const QueuePage = lazy(() => import("./console/pages/queue"));
const SecurityPage = lazy(() => import("./console/pages/security"));
const SystemPage = lazy(() => import("./console/pages/system"));
const AlertsPage = lazy(() => import("./console/pages/alerts"));
const PortalApp = lazy(() => import("./portal/portal-app"));

/**
 * One SPA, two trees: /console/* (admins) and /account/* (mailbox users).
 * The server falls back to index.html for both; anything else is a 404 view.
 */
export const router = createBrowserRouter([
  { path: "/", element: <Navigate to="/console" replace /> },
  {
    path: "/console",
    element: <ConsoleRoot />,
    children: [
      {
        path: "setup",
        element: (
          <Suspense fallback={<Splash label="Setup" />}>
            <SetupPage />
          </Suspense>
        ),
      },
      { path: "login", element: <LoginPage /> },
      {
        element: (
          <RequireAdmin>
            <ConsoleShell />
          </RequireAdmin>
        ),
        children: [
          { index: true, element: <OverviewPage /> },
          { path: "domains", element: <DomainsPage /> },
          { path: "domains/:domainId", element: <DomainDetailPage /> },
          { path: "mailboxes", element: <MailboxesPage /> },
          { path: "aliases", element: <AliasesPage /> },
          { path: "sending", element: <SendingPage /> },
          { path: "deliverability", element: <DeliverabilityPage /> },
          { path: "queue", element: <QueuePage /> },
          { path: "security", element: <SecurityPage /> },
          { path: "system", element: <SystemPage /> },
          { path: "alerts", element: <AlertsPage /> },
          { path: "*", element: <NotFound inShell /> },
        ],
      },
    ],
  },
  {
    path: "/account/*",
    element: (
      <Suspense fallback={<Splash label="Yunori Mail" />}>
        <PortalApp />
      </Suspense>
    ),
  },
  { path: "*", element: <NotFound /> },
]);
