import React from "react";
import ReactDOM from "react-dom/client";
import { ThemeProvider } from "next-themes";

import "../index.css";
import { Launcher } from "./Launcher";

// The macro launcher's own module root. Like the main window it mounts a dark
// ThemeProvider around a single page component; unlike the main window it renders
// one palette, not the tabbed app shell.
ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <ThemeProvider attribute="class" defaultTheme="dark" enableSystem disableTransitionOnChange>
      <Launcher />
    </ThemeProvider>
  </React.StrictMode>,
);
