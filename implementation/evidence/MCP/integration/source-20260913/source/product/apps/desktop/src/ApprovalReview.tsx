import { useEffect, useState, type ReactNode } from "react";
import { LocalPreview } from "./GitHubLocal";

export function approvalExpired(expires: string | undefined, now = Date.now()) {
  const deadline = Number(expires);
  return !expires || !Number.isFinite(deadline) || now >= deadline;
}

export function ApprovalReview({ title, expires, children, actions }: {
  title: string;
  expires?: string;
  children?: (expired: boolean, checkExpired: () => boolean) => ReactNode;
  actions?: (expired: boolean, checkExpired: () => boolean) => ReactNode;
}) {
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | undefined;
    const update = () => {
      clearTimeout(timer);
      const current = Date.now();
      setNow(current);
      if (!approvalExpired(expires, current)) {
        timer = setTimeout(update, Math.min(1000, Number(expires) - current));
      }
    };
    update();
    window.addEventListener("focus", update);
    document.addEventListener("visibilitychange", update);
    return () => {
      clearTimeout(timer);
      window.removeEventListener("focus", update);
      document.removeEventListener("visibilitychange", update);
    };
  }, [expires]);
  let review: { summary?: string; details?: string; changes?: string; risk?: string; kind?: string } | null = null;
  if (title.length <= 32768) {
    try { const value = JSON.parse(title); if (typeof value?.summary === "string") review = value; } catch { /* Literal fallback. */ }
  }
  let localGit: any = null;
  if (review?.kind === "native_local_git" && typeof review.details === "string") {
    try {
      const value = JSON.parse(review.details);
      if (value && typeof value.action === "string" && typeof value.repository === "string" && typeof value.folder === "string"
        && (!value.files || Array.isArray(value.files) && value.files.length <= 64 && value.files.every((f: any) => f && typeof f.path === "string" && (f.operation === undefined || typeof f.operation === "string")))
        && (value.file_count === undefined || Number.isSafeInteger(value.file_count) && value.file_count >= 0)
        && (!value.account || typeof value.account.login === "string" && typeof value.account.host === "string")
        && (!value.retained_locks || Array.isArray(value.retained_locks) && value.retained_locks.every((p: any) => typeof p === "string"))
        && ["network_warning", "recovery_warning", "display_limits"].every(k => value[k] === undefined || typeof value[k] === "string")) localGit = value;
    } catch { /* Incomplete or older previews keep their literal exact details. */ }
  }
  const expired = approvalExpired(expires, now);
  const checkExpired = () => {
    const current = Date.now();
    setNow(current);
    return approvalExpired(expires, current);
  };
  const seconds = expires ? expired ? 0 : Math.ceil((Number(expires) - now) / 1000) : null;
  return <><div className="approval-body"><div className="approval-review">
    <p className="approval-summary">{review?.summary ?? title}</p>
    {review?.risk === "critical" && <p className="approval-warning">Outside the current project. Check the target carefully.</p>}
    {localGit ? <LocalPreview preview={localGit} /> : typeof review?.details === "string" && review.details && <pre tabIndex={0} aria-label="Request details">{review.details}</pre>}
    {typeof review?.changes === "string" && review.changes && <section className="approval-changes"><h3>Proposed change</h3><pre tabIndex={0} aria-label="Proposed change">{review.changes}</pre></section>}
    <details className="approval-help" key={title}>
      <summary>About this approval</summary>
      <p>Allows only this operation, not future requests. Execution time starts after approval.</p>
      <p>To allow, click the button or focus it and press Space. Enter does not approve.</p>
    </details>
  </div>{children?.(expired, checkExpired)}</div>
    <div className="approval-footer">
      {expired ? <p className="approval-expired" role="status">This request expired. No approval can be applied; ask AG1 to try again.</p>
        : seconds !== null && <p className="approval-expiry" role="timer" aria-live="off">Expires in {Math.floor(seconds / 60)}m {seconds % 60}s</p>}
      <p className="approval-close-note">Closing denies this request.</p>
      {actions?.(expired, checkExpired)}
    </div>
  </>;
}
