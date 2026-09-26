import { useEffect, useRef, useState } from "react";
import { api, errorMessage } from "./bridge";
import { Source } from "./GitHubWorkbench";
import { Markdown } from "./Markdown";
import { GitHubGraphActions, type GraphActionTarget } from "./GitHubGraphActions";

export type GraphTarget = { kind: "project"; id: string } | { kind: "discussion"; repo: string; number: number };
type Selection = { operation: string; id?: string; fieldId?: string; fieldName?: string };
const fieldOperations: Record<string, string> = { ProjectV2ItemFieldUserValue: "projects.field_users", ProjectV2ItemFieldLabelValue: "projects.field_labels", ProjectV2ItemFieldPullRequestValue: "projects.field_pull_requests", ProjectV2ItemFieldReviewerValue: "projects.field_reviewers" };
const labels: Record<string, string> = { "projects.field_users": "Field people", "projects.field_labels": "Field labels", "projects.field_pull_requests": "Field pull requests", "projects.field_reviewers": "Field reviewers", "projects.get": "Project overview", "projects.items": "Project items", "projects.fields": "Project fields", "projects.item_values": "Item field values", "discussions.get": "Discussion and comments", "discussions.categories": "Discussion categories", "discussions.replies": "Comment replies" };
const keys: Record<string, string> = { "projects.field_users": "values", "projects.field_labels": "values", "projects.field_pull_requests": "values", "projects.field_reviewers": "values", "projects.items": "items", "projects.fields": "fields", "projects.item_values": "fieldValues", "discussions.get": "comments", "discussions.categories": "discussionCategories", "discussions.replies": "replies" };
function initial(target: GraphTarget): Selection { return target.kind === "project" ? { operation: "projects.get", id: target.id } : { operation: "discussions.get" }; }
function rowTitle(row: any) { return String(row?.name ?? row?.login ?? row?.title ?? row?.content?.title ?? row?.field?.name ?? row?.author?.login ?? row?.__typename ?? row?.id ?? "Unavailable item").slice(0, 300); }
function TextPreview({ text }: { text: string }) { return <><Markdown text={text.slice(0, 32000)} />{text.length > 32000 && <p>Text preview limited to 32,000 characters. Complete text is available in the paged source below.</p>}</>; }

export function GitHubGraphDetails({ target, scope, close }: { target: GraphTarget; scope: string; close: () => void }) {
  const identity = JSON.stringify(target), epoch = useRef(0);
  const [selection, setSelection] = useState<Selection>(() => initial(target)), [value, setValue] = useState<any>(null), [error, setError] = useState(""), [busy, setBusy] = useState(false);
  const [cursors, setCursors] = useState<(string | null)[]>([null]), [page, setPage] = useState(0), [selected, setSelected] = useState<any>(null);
  async function read(next: Selection, after: string | null = null, history: (string | null)[] = [null], index = 0) {
    const ticket = ++epoch.current; setSelection(next); setValue(null); setSelected(null); setError(""); setBusy(true);
    try {
      const repoBound = next.operation === "discussions.get" || next.operation === "discussions.categories";
      const [owner, name] = target.kind === "discussion" ? target.repo.split("/") : [];
      const parameters = repoBound ? { owner, name, ...(next.operation === "discussions.get" && target.kind === "discussion" ? { number: target.number } : {}), ...(after ? { after } : {}) } : { id: next.id, ...(next.fieldId ? { fieldId: next.fieldId, fieldName: next.fieldName } : {}), ...(after ? { after } : {}) };
      const result = await api.github("api_read", { operation: next.operation, ...(repoBound && target.kind === "discussion" ? { repo: target.repo } : {}), parameters });
      if (ticket !== epoch.current) return;
      if (!result.ok) throw new Error(result.error?.message ?? "Cannot read this GitHub detail.");
      const graph = result.value?.data?.data;
      const root = next.operation === "discussions.get" ? graph?.repository?.discussion : next.operation === "discussions.categories" ? graph?.repository : next.fieldId ? graph?.node?.fieldValueByName : graph?.node;
      if (next.fieldId && (graph?.node?.id !== next.id || graph?.field?.id !== next.fieldId || root?.field?.id !== next.fieldId || root?.field?.name !== next.fieldName || fieldOperations[root?.__typename] !== next.operation)) throw new Error("The selected field changed or is unavailable. Refresh the item fields.");
      if (!root || typeof root !== "object") throw new Error("The item is unavailable or this account cannot access it.");
      const connection = keys[next.operation] ? root[keys[next.operation]] : null;
      if (connection && (!Array.isArray(connection.nodes) || connection.nodes.length > 30 || typeof connection.pageInfo?.hasNextPage !== "boolean")) throw new Error("Invalid bounded GitHub detail page.");
      if (keys[next.operation] && !connection) throw new Error("Missing GitHub detail page.");
      if (connection?.pageInfo.hasNextPage && (typeof connection.pageInfo.endCursor !== "string" || !connection.pageInfo.endCursor || connection.pageInfo.endCursor.length > 4096 || history.slice(0, index + 1).includes(connection.pageInfo.endCursor))) throw new Error("GitHub returned a missing or repeated page cursor. Refresh this view.");
      setValue({ root, connection, response: result.value.data, rate: graph.rateLimit }); setCursors(history); setPage(index);
    } catch (e) { if (ticket === epoch.current) setError(errorMessage(e)); }
    finally { if (ticket === epoch.current) setBusy(false); }
  }
  useEffect(() => { setCursors([null]); setPage(0); void read(initial(target)); return () => { ++epoch.current; }; }, [identity, scope]);
  const rows: any[] = value?.connection?.nodes ?? [];
  const nextCursor = value?.connection?.pageInfo.hasNextPage ? value.connection.pageInfo.endCursor : null;
  const heading = typeof value?.root?.title === "string" ? value.root.title : labels[selection.operation];
  let actionTarget: GraphActionTarget | null = null;
  if (value) {
    const root = value.root;
    if (!selected && selection.operation === "projects.get") actionTarget = { kind: "project", id: root.id, title: root.title, body: root.readme, closed: root.closed };
    else if (!selected && selection.operation === "discussions.get") actionTarget = { kind: "discussion", id: root.id, title: root.title, body: root.body, closed: root.closed };
    else if (!selected && selection.operation === "projects.item_values" && root.project?.id) actionTarget = { kind: "item", id: root.id, projectId: root.project.id, archived: root.isArchived };
    else if (selected && ["discussions.get", "discussions.replies"].includes(selection.operation)) actionTarget = { kind: "comment", id: selected.id, body: selected.body, isAnswer: selected.isAnswer, discussionId: selection.operation === "discussions.get" ? root.id : root.discussion?.id };
    else if (selected && selection.operation === "projects.item_values" && selected.field?.id && root.project?.id) actionTarget = { kind: "field", id: selected.field.id, itemId: root.id, projectId: root.project.id };
  }
  return <section className="github-graph-detail settings-card" aria-label="GitHub graph details">
    <div className="github-heading"><h3>{heading}</h3><button onClick={close}>Close graph details</button></div>
    <p>{target.kind === "project" ? `Account-wide project ${target.id}` : `${target.repo} discussion #${target.number}`} · {labels[selection.operation]}{selection.fieldName ? ` · ${selection.fieldName}` : ""}</p>
    <p className="settings-footnote">Each list has its own cursor. Pages are current GitHub reads, not an atomic snapshot or permission to edit. Node-ID reads are account-wide; selecting a repository does not restrict them.</p>
    <div className="github-actions">{(target.kind === "project" ? ["projects.get", "projects.items", "projects.fields"] : ["discussions.get", "discussions.categories"]).map(operation => <button disabled={busy} key={operation} aria-pressed={selection.operation === operation} onClick={() => void read({ operation, ...(target.kind === "project" ? { id: target.id } : {}) })}>{labels[operation]}</button>)}<button disabled={busy} onClick={() => void read(selection)}>Refresh graph view</button></div>
    {busy && <p role="status">Loading GitHub detail…</p>}{error && <p className="inline-error" role="alert">{error}</p>}
    {value && <>
      {typeof value.root.body === "string" && <TextPreview text={value.root.body} />}{typeof value.root.readme === "string" && <TextPreview text={value.root.readme} />}{typeof value.root.shortDescription === "string" && <p>{value.root.shortDescription.slice(0, 32000)}</p>}
      {typeof value.root.closed === "boolean" && <p>{value.root.closed ? "Closed" : "Open"}</p>}
      {value.connection && <><div className="github-items">{rows.map((row, index) => <button key={index} disabled={!row} onClick={() => setSelected(row)}><span><strong>{rowTitle(row)}</strong><small>{typeof row?.body === "string" ? row.body.slice(0, 160) : String(row?.dataType ?? row?.__typename ?? row?.id ?? "Unavailable node")}</small></span></button>)}</div>{!rows.length && <p>No entries on this page.</p>}<div className="github-pagination"><button disabled={busy || !page} onClick={() => void read(selection, cursors[page - 1], cursors, page - 1)}>Previous graph page</button><span>Page {page + 1}</span><button disabled={busy || !nextCursor || page >= 99} onClick={() => void read(selection, nextCursor, [...cursors.slice(0, page + 1), nextCursor], page + 1)}>Next graph page</button></div></>}
      {selection.operation === "projects.item_values" && <p className="settings-footnote">Scalar values, milestone and repository fields are shown. Select a people, labels, linked pull requests or reviewers field to browse its values on separate pages.</p>}
      {selection.fieldId && <button disabled={busy} onClick={() => void read({ operation: "projects.item_values", id: selection.id })}>Back to item field values</button>}
      {selected && <div className="github-browser-detail"><h4>{rowTitle(selected)}</h4>{typeof selected.body === "string" && <TextPreview text={selected.body} />}{selection.operation === "projects.items" && typeof selected.id === "string" && <button onClick={() => void read({ operation: "projects.item_values", id: selected.id })}>Inspect item field values</button>}{["discussions.get", "discussions.replies"].includes(selection.operation) && typeof selected.id === "string" && <button onClick={() => void read({ operation: "discussions.replies", id: selected.id })}>Inspect comment replies</button>}{selection.operation === "projects.item_values" && fieldOperations[selected.__typename] && typeof selected.field?.id === "string" && typeof selected.field?.name === "string" && <button onClick={() => void read({ operation: fieldOperations[selected.__typename], id: selection.id, fieldId: selected.field.id, fieldName: selected.field.name })}>Browse field values</button>}<Source value={selected} label="Selected graph item" /><button onClick={() => setSelected(null)}>Close selected graph item</button></div>}
      {actionTarget && typeof actionTarget.id === "string" && <details><summary>Project or discussion changes</summary><GitHubGraphActions key={JSON.stringify(actionTarget)} target={actionTarget} scope={scope} /></details>}
      {value.rate && <p className="settings-footnote">Query cost: {String(value.rate.cost ?? "unknown")} · Remaining: {String(value.rate.remaining ?? "unknown")} · Reset: {String(value.rate.resetAt ?? "unknown")}</p>}
      <details><summary>Complete graph detail response</summary><Source value={value.response} label="Graph detail source" /></details>
    </>}
  </section>;
}
