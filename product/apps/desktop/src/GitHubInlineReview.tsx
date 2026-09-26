import { useState } from "react";
import { api } from "./bridge";
import { GitHubAction } from "./GitHubAction";
import { githubFailure } from "./GitHubControls";
import { pullPatchLines } from "./GitHubPullReview";
/** Admit single-line anchors only from complete, count-matched unified hunks. */
export function completeReviewPatch(file: any): boolean {
  if (typeof file?.patch !== "string" || !file.patch || !Number.isSafeInteger(file.additions) || !Number.isSafeInteger(file.deletions)) return false;
  let oldLeft = 0, newLeft = 0, hunks = 0, additions = 0, deletions = 0;
  for (const line of file.patch.split("\n")) {
    const h = /^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@/.exec(line);
    if (h) { if (oldLeft || newLeft) return false; oldLeft = Number(h[2] ?? 1); newLeft = Number(h[4] ?? 1); ++hunks; continue; }
    if (line.startsWith("\\ No newline") || !line) continue;
    if (!hunks) return false;
    if (line.startsWith("+")) { --newLeft; ++additions; }
    else if (line.startsWith("-")) { --oldLeft; ++deletions; }
    else if (line.startsWith(" ")) { --oldLeft; --newLeft; }
    else return false;
    if (oldLeft < 0 || newLeft < 0) return false;
  }
  return hunks > 0 && oldLeft === 0 && newLeft === 0 && additions === file.additions && deletions === file.deletions;
}
export function GitHubInlineReview({ repo, number, head, base, file }: { repo: string; number: number; head: string; base: string; file: any }) {
  const [anchor, setAnchor] = useState(""), [body, setBody] = useState("");
  if (!completeReviewPatch(file)) return <p className="settings-footnote">Inline comments require a complete text patch. Review binary, omitted or truncated files on GitHub.</p>;
  const anchors = pullPatchLines(file.patch).flatMap(line => line.kind === "removed" && line.old ? [{ side: "LEFT", line: line.old, text: line.text }] : line.next ? [{ side: "RIGHT", line: line.next, text: line.text }] : []);
  return <details><summary>Comment on a diff line</summary><GitHubAction title="Submit inline review comment" scope={`${repo}:${number}:${head}:${base}:${file.filename}`} request={async () => {
    const selected = anchors.find(a => `${a.side}:${a.line}` === anchor); if (!selected || !body.trim()) throw new Error("Choose a displayed diff line and enter a comment.");
    const reply = await api.github("api_read", { operation: "pulls.get", repo, parameters: { number } });
    if (!reply.ok) throw new Error(githubFailure(reply));
    if (reply.value?.data?.head?.sha !== head || reply.value?.data?.base?.sha !== base) throw new Error("The PR head or base changed. Refresh the review; this draft must be recreated against the new diff.");
    return { operation: "pulls.review", repo, parameters: { number }, body: { event: "COMMENT", commit_id: head, comments: [{ path: file.filename, side: selected.side, line: selected.line, body }] } };
  }}><label>Diff line<select required value={anchor} onChange={e => setAnchor(e.target.value)}><option value="">Choose an exact line…</option>{anchors.map(a => <option key={`${a.side}:${a.line}`} value={`${a.side}:${a.line}`}>{a.side === "LEFT" ? "Old" : "New"} {a.line}: {a.text.slice(0, 100)}</option>)}</select></label><label>Inline comment<textarea required rows={3} maxLength={10000} value={body} onChange={e => setBody(e.target.value)} /></label><p>One comment per submitted review, bound to {head}. Refreshing, paging files or changing the selected file discards this draft. Multi-line ranges and review threads remain available through Advanced or GitHub.</p></GitHubAction></details>;
}
