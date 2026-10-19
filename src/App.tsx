import { useMemo, useState } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ToastProvider } from "@/components/ui/toast";
import { AppShell, type PageKey } from "@/components/layout/AppShell";
import { Dashboard } from "@/pages/Dashboard";
import { Servers } from "@/pages/Servers";
import { Agents } from "@/pages/Agents";
import { History } from "@/pages/History";
import { SettingsPage } from "@/pages/Settings";
import { useRegistry } from "@/hooks/useRegistry";
import { useTheme } from "@/hooks/useTheme";

export default function App() {
  const queryClient = useMemo(
    () =>
      new QueryClient({
        defaultOptions: {
          queries: { retry: false, refetchOnWindowFocus: false },
        },
      }),
    [],
  );
  const [page, setPage] = useState<PageKey>("dashboard");
  const registry = useRegistry();
  const { theme, setTheme } = useTheme();

  return (
    <QueryClientProvider client={queryClient}>
      <ToastProvider>
        <AppShell
          page={page}
          onNavigate={setPage}
          registry={registry}
          theme={theme}
          onThemeChange={setTheme}
        >
          {page === "dashboard" ? (
            <Dashboard registry={registry} onNavigate={setPage} />
          ) : null}
          {page === "servers" ? <Servers registry={registry} /> : null}
          {page === "agents" ? <Agents registry={registry} /> : null}
          {page === "history" ? <History registry={registry} /> : null}
          {page === "settings" ? (
            <SettingsPage registry={registry} theme={theme} onThemeChange={setTheme} />
          ) : null}
        </AppShell>
      </ToastProvider>
    </QueryClientProvider>
  );
}
