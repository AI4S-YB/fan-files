export default function EngineBanner({ error, onRetry }: { error: string | null; onRetry: () => void }) {
  if (!error) return null;
  return (
    <div className="engine-banner">
      <span>⚠️ {error}</span>
      <button onClick={onRetry}>重试</button>
    </div>
  );
}
