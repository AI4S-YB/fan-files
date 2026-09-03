import {
  createContext,
  useCallback,
  useContext,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";

export type ToastType = "success" | "error" | "info";

export interface ToastContextValue {
  showToast: (msg: string, type?: ToastType) => void;
}

// 默认 no-op：未包 Provider（如单组件测试）时调用不报错
const ToastContext = createContext<ToastContextValue>({ showToast: () => {} });

export function useToast() {
  return useContext(ToastContext);
}

interface ToastItem {
  id: number;
  msg: string;
  type: ToastType;
}

// 轻量无依赖 toast：右上角堆叠，3s 自动消失 + 手动关闭按钮。
// SF-T2：扫描完成/失败通知、预检提示等全局轻提示走这里。
const AUTO_DISMISS_MS = 3000;

export default function ToastProvider({ children }: { children: ReactNode }) {
  const [toasts, setToasts] = useState<ToastItem[]>([]);
  const nextId = useRef(1);

  const dismiss = useCallback((id: number) => {
    setToasts((ts) => ts.filter((t) => t.id !== id));
  }, []);

  const showToast = useCallback(
    (msg: string, type: ToastType = "info") => {
      const id = nextId.current++;
      setToasts((ts) => [...ts, { id, msg, type }]);
      setTimeout(() => dismiss(id), AUTO_DISMISS_MS);
    },
    [dismiss]
  );

  // value 身份稳定：toasts 变化时只重渲染 Provider，不波及仅用 showToast 的消费者
  const value = useMemo(() => ({ showToast }), [showToast]);

  return (
    <ToastContext.Provider value={value}>
      {children}
      {/* role=status + aria-live：新增 toast 时读屏播报 */}
      <div className="toast-container" role="status" aria-live="polite">
        {toasts.map((t) => (
          <div key={t.id} className={`toast-item toast-${t.type}`}>
            <span className="toast-msg">{t.msg}</span>
            <button
              className="toast-close"
              aria-label="关闭"
              onClick={() => dismiss(t.id)}
            >
              ×
            </button>
          </div>
        ))}
      </div>
    </ToastContext.Provider>
  );
}
