interface Props {
  done: number;
  total: number;
  // 继续 = 仅关闭弹窗（引擎已自动续传缺失块）；放弃 = 取消对应传输
  onContinue: () => void;
  onReject: () => void;
}

// 续传确认弹窗（resume 事件触发），接收侧与共享侧共用。
// 超时自动关闭（规格 §九：用户不响应默认继续——引擎已自动续传）由调用方持有定时器。
export default function ResumeDialog({ done, total, onContinue, onReject }: Props) {
  const pct = total > 0 ? Math.round((done / total) * 100) : 0;
  return (
    <div className="modal" onClick={onContinue}>
      <div className="modal-body" onClick={(e) => e.stopPropagation()}>
        <h3>续传确认</h3>
        <p>
          发现未完成传输，已收 {done}/{total}（{pct}%），是否续传？
        </p>
        <div className="modal-actions">
          <button className="primary" onClick={onContinue}>
            继续续传
          </button>
          <button className="secondary" onClick={onReject}>
            放弃并取消
          </button>
        </div>
      </div>
    </div>
  );
}
