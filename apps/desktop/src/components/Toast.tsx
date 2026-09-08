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

function ToastItem({ toast, onClose }: { toast: Toast; onClose: () => void }) {
  const ttl = toast.ttl ?? 4000;
  useEffect(() => {
    const id = setTimeout(onClose, ttl);
    return () => clearTimeout(id);
  }, [ttl, onClose]);
  return (
    <div
      role="status"
      data-kind={toast.kind}
      className="toast-item anim-slide-in"
    >
      <span aria-hidden="true" className={`toast-icon toast-icon-${toast.kind}`}>
        {ICONS[toast.kind]}
      </span>
      <span className="toast-msg">{toast.message}</span>
      <button
        type="button"
        aria-label="关闭"
        onClick={onClose}
        className="toast-close"
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
          className="toast-stack"
          role="status"
          aria-live="polite"
        >
          {toasts.map((t) => (
            <ToastItem key={t.id} toast={t} onClose={() => close(t.id)} />
          ))}
        </div>,
        document.body
      )}
    </>
  );
}
