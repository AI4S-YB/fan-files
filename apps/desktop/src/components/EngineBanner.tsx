export default function EngineBanner({
  error,
  onRetry,
}: {
  error: string | null;
  onRetry: () => void;
}) {
  if (!error) return null;
  return (
    <div
      role="alert"
      className="anim-fade-down"
      style={{
        display: "flex",
        alignItems: "center",
        gap: 12,
        background: "hsl(var(--destructive) / .08)",
        border: "1px solid hsl(var(--destructive) / .3)",
        color: "hsl(var(--destructive-fg))",
        borderRadius: "var(--radius-lg)",
        padding: "12px 16px",
        marginBottom: 16,
        animation: "pulse-bg 2.4s ease-in-out infinite",
      }}
    >
      <div
        style={{
          width: 36,
          height: 36,
          borderRadius: "50%",
          background: "hsl(var(--destructive) / .12)",
          display: "grid",
          placeItems: "center",
          fontSize: 18,
          flexShrink: 0,
        }}
        aria-hidden="true"
      >
        ⚠
      </div>
      <div style={{ flex: 1, minWidth: 0 }}>
        <div style={{ fontWeight: 600, fontSize: 13 }}>引擎启动失败</div>
        <div
          style={{
            fontSize: 12,
            opacity: 0.8,
            marginTop: 2,
            overflow: "hidden",
            textOverflow: "ellipsis",
            whiteSpace: "nowrap",
          }}
        >
          {error}
        </div>
      </div>
      <button
        type="button"
        onClick={onRetry}
        className="transition-base"
        style={{
          flexShrink: 0,
          padding: "6px 14px",
          fontSize: 13,
          fontWeight: 500,
          background: "hsl(var(--destructive))",
          color: "hsl(var(--primary-fg))",
          border: "none",
          borderRadius: "var(--radius)",
          cursor: "pointer",
        }}
        onMouseEnter={(e) => {
          e.currentTarget.style.background = "hsl(var(--destructive) / .85)";
        }}
        onMouseLeave={(e) => {
          e.currentTarget.style.background = "hsl(var(--destructive))";
        }}
      >
        重试
      </button>
    </div>
  );
}
