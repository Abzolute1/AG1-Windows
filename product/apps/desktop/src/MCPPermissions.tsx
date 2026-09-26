import { useEffect, useRef, useState } from "react";

type Props = {
  scope: string;
  server: { id: string; revision: number };
  policy: any;
  detail: any;
  connection?: string;
  kind: string;
  busy: boolean;
  run: (args: any) => Promise<any>;
  perform: (work: () => Promise<void>) => Promise<void>;
  notice: (text: string) => void;
};
type Field = { name: string; type: string; value: string };
const blankField = (): Field => ({ name: "", type: "string", value: "" });
const source = (value: unknown) => <pre className="mcp-source">{JSON.stringify(value, null, 2)}</pre>;

export function MCPPermissions({ scope, server, policy, detail, connection, kind, busy, run, perform, notice }: Props) {
  const [mode, setMode] = useState(policy?.mode ?? "restricted");
  const [effect, setEffect] = useState("unknown"), [permission, setPermission] = useState("ask");
  const [match, setMatch] = useState("equals"), [fields, setFields] = useState<Field[]>([blankField()]);
  const [replace, setReplace] = useState(false), [review, setReview] = useState<any>(null), [ack, setAck] = useState(false);
  const pending = useRef<string | null>(null), generation = useRef(0);
  useEffect(() => { setMode(policy?.mode ?? "restricted"); }, [policy?.mode]);
  useEffect(() => {
    ++generation.current;
    return () => { ++generation.current; const id = pending.current; pending.current = null; if (id) void run({ action: "trust_dismiss", review: id }).catch(() => {}); };
  }, [scope, server.id]);
  const capability = detail?.identity?.originalName;
  const grants: any[] = policy?.grants ?? [];
  const saved = grants.find(g => g.identity.kind === kind && g.identity.originalName === capability
    || kind === "tool" && !g.identity.kind && g.identity.originalName === capability);
  const knownFields = Object.keys(detail?.parameters?.properties ?? detail?.original?.inputSchema?.properties ?? {});
  useEffect(() => {
    setEffect("unknown"); setPermission("ask"); setMatch("equals"); setFields([blankField()]); setReplace(false);
  }, [capability, kind]);
  function updateField(index: number, value: Partial<Field>) {
    setFields(fields.map((field, i) => i === index ? { ...field, ...value } : field));
  }
  function argumentsRule() {
    if (match === "allArguments") return { kind: "allArguments" };
    const values: { [key: string]: string | number | boolean | null } = Object.create(null);
    for (const field of fields) {
      if (!field.name || Object.hasOwn(values, field.name)) throw new Error("Choose a distinct argument name for each condition.");
      let value: string | number | boolean | null = field.value;
      if (field.type === "number") {
        value = Number(field.value);
        if (!field.value.trim() || !Number.isFinite(value) || Number.isInteger(value) && !Number.isSafeInteger(value)) throw new Error("Enter a finite number within the exact supported range.");
      } else if (field.type === "boolean") value = field.value === "true";
      else if (field.type === "null") value = null;
      values[field.name] = value;
    }
    return { kind: "equals", fields: values };
  }
  async function dismiss() {
    const id = pending.current; pending.current = null; setReview(null); setAck(false);
    if (id) await run({ action: "trust_dismiss", review: id });
  }
  async function prepare() {
    if (!connection || !capability) throw new Error("Reconnect and inspect the capability before reviewing a rule.");
    await dismiss();
    const rule = { arguments: argumentsRule(), effect, permission };
    const rules = [rule];
    if (!replace && (saved?.rules.length ?? 0) >= 32) throw new Error("This capability already has 32 rules. Replace or revoke its existing rules first.");
    const ticket = generation.current;
    const result = await run({ action: "trust_review", connection, revision: server.revision, kind, key: capability, rules, append: !replace });
    if (ticket !== generation.current) { await run({ action: "trust_dismiss", review: result.review }); return; }
    pending.current = result.review; setReview(result); setAck(false);
  }
  async function confirm() {
    const id = pending.current; pending.current = null; setReview(null); setAck(false);
    if (id) { const result = await run({ action: "trust_confirm", review: id }); notice(result.message); }
  }
  return <section className="mcp-permissions" aria-label="MCP permissions">
    <h3>Permissions for this scope</h3>
    <p>Restricted asks by default. Disable writes admits only matching reviewed read rules. Destructive, administrative, credential, paid and other consequential operations still require exact confirmation, including with chat approval bypass.</p>
    <div className="mcp-actions">
      <label>Scope policy<select disabled={busy || !!review} value={mode} onChange={e => setMode(e.target.value)}><option value="restricted">Restricted</option><option value="disableWrites">Disable writes</option></select></label>
      <button disabled={busy || !!review || mode === (policy?.mode ?? "restricted")} onClick={() => void perform(async () => {
        const result = await run({ action: "trust_mode", id: server.id, revision: server.revision, mode }); notice(result.message);
      })}>Save scope policy</button>
    </div>
    <p>{grants.length ? `${grants.length} capability rule set${grants.length === 1 ? "" : "s"} saved for this scope.` : "No capability rules saved for this scope."} Trust comes from these explicit grants. Rules apply only while the exact configuration, credentials and capability schema match.</p>
    <p>Changing rules disconnects this scope so pending calls cannot use the old policy. Other scopes keep their connections.</p>
    {grants.map(grant => <div className="mcp-saved-rule" key={`${grant.identity.kind ?? "tool"}:${grant.identity.originalName}`}>
      <strong>{grant.identity.originalName}</strong>
      <details><summary>Saved conditions and bindings</summary>{source({ targetDigest: grant.targetDigest })}<pre className="mcp-source">{grant.identityJson ?? JSON.stringify(grant.identity, null, 2)}</pre><pre className="mcp-source">{grant.rulesJson ?? JSON.stringify(grant.rules, null, 2)}</pre></details>
      <button disabled={busy || !!review} onClick={() => void perform(async () => {
        const result = await run({ action: "trust_revoke", id: server.id, revision: server.revision, wire_name: grant.wireName }); notice(result.message);
      })}>Revoke rules for {grant.identity.originalName}</button>
    </div>)}
    {!detail && <p>Inspect a connected capability below to review or change its rules.</p>}
    {detail && <div className="mcp-rule-editor">
      <h4>Review policy for {capability ?? "this capability"}</h4>
      <p>AG1 classification: {detail.classification ?? "unknown"}. {detail.classificationBasis}</p>
      <details><summary>Server assertions · unverified</summary>{source(detail.original?.annotations ?? "No annotations supplied")}</details>
      <p>Your reviewed effect is a policy decision, not a guarantee of server behavior. Reads may disclose data or incur cost. Local tool rules do not sandbox server code.</p>
      {detail.schemaSupported && connection && capability ? <fieldset disabled={busy || !!review}>
        <legend>User-reviewed rule</legend>
        <label>Reviewed effect<select value={effect} onChange={e => { setEffect(e.target.value); if (!["read", "write"].includes(e.target.value)) setPermission("ask"); }}>
          <option value="unknown">Unknown or mixed</option><option value="read">Read</option><option value="write">Non-consequential write</option><option value="consequential">Consequential</option>
        </select></label>
        <label>Approval behavior<select value={permission} onChange={e => setPermission(e.target.value)}><option value="ask">Ask each time</option><option value="allow" disabled={!["read", "write"].includes(effect)}>Allow matching reviewed cases</option><option value="deny">Deny matching cases</option></select></label>
        <label>Argument scope<select value={match} onChange={e => setMatch(e.target.value)}><option value="equals">Exact argument conditions</option><option value="allArguments">All arguments for this capability</option></select></label>
        {match === "allArguments" ? <p>This rule covers every valid argument combination for the exact reviewed capability schema.</p> : <>
          <p>All conditions must match exactly. Nested paths and pattern matching are not supported.</p>
          <datalist id={`mcp-argument-fields-${server.id}`}>{knownFields.map(name => <option key={name} value={name} />)}</datalist>
          {fields.map((field, index) => <div className="mcp-rule-condition" key={index}>
            <label>Argument name {index + 1}<input maxLength={256} list={`mcp-argument-fields-${server.id}`} value={field.name} onChange={e => updateField(index, { name: e.target.value })} /></label>
            <label>Value type {index + 1}<select value={field.type} onChange={e => updateField(index, { type: e.target.value, value: e.target.value === "boolean" ? "false" : "" })}><option value="string">Text</option><option value="number">Number</option><option value="boolean">Boolean</option><option value="null">Null</option></select></label>
            {field.type === "boolean" ? <label>Exact value {index + 1}<select value={field.value || "false"} onChange={e => updateField(index, { value: e.target.value })}><option value="false">False</option><option value="true">True</option></select></label> : field.type !== "null" && <label>Exact value {index + 1}<input maxLength={4096} value={field.value} onChange={e => updateField(index, { value: e.target.value })} /></label>}
            {fields.length > 1 && <button onClick={() => setFields(fields.filter((_, i) => i !== index))}>Remove condition {index + 1}</button>}
          </div>)}
          <button disabled={fields.length >= 16} onClick={() => setFields([...fields, blankField()])}>Add condition</button>
        </>}
        {saved && <label className="checkbox-label"><input type="checkbox" checked={replace} onChange={e => setReplace(e.target.checked)} />Replace this capability’s saved rules. Otherwise add this rule; existing denials and confirmations stay stronger.</label>}
        <button onClick={() => void perform(prepare)}>Review rule</button>
      </fieldset> : <p>Connect and inspect a supported capability to change its rules.</p>}
    </div>}
    {review && <section className="mcp-review" aria-label="Review MCP rule">
      <h4>Confirm the native rule review</h4>
      <p>{review.details.basis}</p>{review.details.localAccess && <p>{review.details.localAccess}</p>}
      {source({ server: review.details.serverName, configuration: review.details.configuration, scope: review.details.scope, targetDigest: review.details.targetDigest })}
      <pre className="mcp-source">{review.details.identityJson ?? JSON.stringify(review.details.identity, null, 2)}</pre>
      <pre className="mcp-source">{review.details.rulesJson ?? JSON.stringify(review.details.rules, null, 2)}</pre>
      {review.details.existingRulesRetained != null && <p>{review.details.existingRulesRetained} existing rules retained after native target and schema checks.</p>}
      <details><summary>Complete reviewed capability metadata</summary>{source(review.details.metadata)}</details>
      <label className="checkbox-label"><input type="checkbox" checked={ack} onChange={e => setAck(e.target.checked)} />I reviewed the target, scope, effects and exact argument conditions.</label>
      <div className="mcp-actions"><button disabled={busy} onClick={() => void perform(dismiss)}>Cancel rule review</button><button disabled={busy || !ack} onClick={() => void perform(confirm)}>Save reviewed rules</button></div>
    </section>}
  </section>;
}
