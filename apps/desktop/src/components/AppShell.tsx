import type { ReactNode } from "react";
import { useScrollShadow } from "../hooks/useScrollShadow";

export function AppShell({
  title,
  actions,
  children,
}: {
  title: string;
  actions?: ReactNode;
  children: ReactNode;
}) {
  const { ref, scrolled } = useScrollShadow();
  return (
    <div className="app-shell">
      <header
        className={`app-header glass-header transition-base ${scrolled ? "scrolled" : ""}`}
      >
        <div className="app-header-content">
          <h1 className="app-title">{title}</h1>
          {actions && <div className="app-actions">{actions}</div>}
        </div>
      </header>
      <main ref={ref} className="app-main anim-fade" data-app-main>
        {children}
      </main>
    </div>
  );
}
