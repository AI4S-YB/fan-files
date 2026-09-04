import { useEffect, useState } from "react";
import { createPortal } from "react-dom";

export type ToastKind = "success" | "error" | "info";

export interface Toast {
  id: number;
  kind: ToastKind;
  message: string;
  ttl?: number;
}

let nextId = 1;
const listeners = new Set<(t: Toast) => void>();

export function pushToast(t: Omit<Toast, "id">) {
  const toast: Toast = { id: nextId++, ...t };
  listeners.forEach((cb) => cb(toast));
}

const ICONS: Record<ToastKind, string> = {
  success: "✓",
  error:   "✕",
  info:    "ℹ",
};

const KIND_VAR: Record<ToastKind, string> = {
  success: "success",
  error: "destructive",
  info: "primary",
};

function ToastItem({ toast, onClose }: { toast: Toast; onClose: () => void }) {
  const ttl = toast.ttl ?? 4000;
  useEffect(() => {
    const id = setTimeout(onClose, ttl);
    return () => clearTimeout(id);
  }, [ttl, onClose]);
  const kindVar = KIND_VAR[toast.kind];
  return (
    <div
      role="status"
      data-kind={toast.kind}
      className="anim-slide-in"
      style={{
        display: "flex",
        alignItems: "center",
        gap: 10,
        background: "hsl(var(--bg-elevated))",
        border: "1px solid hsl(var(--border))",
        borderLeft: `4px solid hsl(var(--${kindVar}))`,
        borderRadius: "var(--radius-lg)",
        padding: "10px 14px",
        minWidth: 220,
        maxWidth: 380,
        boxShadow: "var(--shadow)",
        fontSize: 13,
        color: "hsl(var(--fg))",
      }}
    >
      <span
        aria-hidden="true"
        style={{
          width: 22,
          height: 22,
          borderRadius: "50%",
          background: `hsl(var(--${kindVar}) / .15)`,
          color: `hsl(var(--${kindVar}-fg))`,
          display: "grid",
          placeItems: "center",
          fontSize: 12,
          fontWeight: 700,
          flexShrink: 0,
        }}
      >
        {ICONS[toast.kind]}
      </span>
      <span style={{ flex: 1 }}>{toast.message}</span>
      <button
        type="button"
        aria-label="关闭"
        onClick={onClose}
        className="transition-base"
        style={{
          border: "none",
          background: "transparent",
          cursor: "pointer",
          color: "hsl(var(--fg-muted))",
          fontSize: 16,
          lineHeight: 1,
          padding: 0,
        }}
        onMouseEnter={(e) => { e.currentTarget.style.color = "hsl(var(--fg))"; }}
        onMouseLeave={(e) => { e.currentTarget.style.color = "hsl(var(--fg-muted))"; }}
      >
        ×
      </button>
    </div>
  );
}

export default function ToastProvider({ children }: { children: React.ReactNode }) {
  const [toasts, setToasts] = useState<Toast[]>([]);
  useEffect(() => {
    const cb = (t: Toast) => {
      setToasts((prev) => [...prev, t]);
    };
    listeners.add(cb);
    return () => { listeners.delete(cb); };
  }, []);
  const close = (id: number) => setToasts((prev) => prev.filter((t) => t.id !== id));
  return (
    <>
      {children}
      {createPortal(
        <div
          style={{
            position: "fixed",
            top: 16,
            right: 16,
            display: "flex",
            flexDirection: "column",
            gap: 8,
            zIndex: 1000,
            pointerEvents: "none",
          }}
        >
          {toasts.map((t) => (
            <div key={t.id} style={{ pointerEvents: "auto" }}>
              <ToastItem toast={t} onClose={() => close(t.id)} />
            </div>
          ))}
        </div>,
        document.body
      )}
    </>
  );
}
