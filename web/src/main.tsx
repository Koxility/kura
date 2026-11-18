import "@fontsource-variable/geist";
import "@fontsource-variable/geist-mono";
import "./styles/tokens.css";
import "./styles/base.css";

import { QueryCache, QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { BrowserRouter } from "react-router";
import { ApiError } from "./api";
import { App } from "./App";

const client: QueryClient = new QueryClient({
  // Kura's own data is read again whenever a page shows it, so switching pages never shows
  // an old state; the cached one shows meanwhile. Lists from the sites set a longer time.
  defaultOptions: { queries: { retry: false, staleTime: 0, refetchOnWindowFocus: true } },
  queryCache: new QueryCache({
    // The session ended: asking for the status again brings up the login page.
    onError: (error, query) => {
      if (error instanceof ApiError && error.status === 401 && query.queryKey[0] !== "status") {
        client.invalidateQueries({ queryKey: ["status"] });
      }
    },
  }),
});

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <QueryClientProvider client={client}>
      <BrowserRouter>
        <App />
      </BrowserRouter>
    </QueryClientProvider>
  </StrictMode>,
);
