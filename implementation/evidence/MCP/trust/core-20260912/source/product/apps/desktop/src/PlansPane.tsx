import { useEffect, useState } from "react";
import type { Command, JsonValue } from "./generated/protocol";
import { Button } from "./components/ui/button";
type Plan = { planId: string; revision: number; status: string; goal: string; summary?: string; steps: string[]; openQuestions: string[]; clarificationOptionSets?: string[][]; completedStepIndexes?: number[] };
export function PlansPane({ run }: { run: (command: Command) => Promise<JsonValue> }) {
  const [plans, setPlans] = useState<Plan[]>([]), [goal, setGoal] = useState(""), [answers, setAnswers] = useState<Record<string, string>>({}), [error, setError] = useState("");
  const refresh = async () => setPlans(await run({ kind: "plan", action: "view", args: {} }) as unknown as Plan[]);
  useEffect(() => { void refresh().catch(e => setError(String(e))); }, [run]);
  const execute = async (action: string, args: JsonValue) => { try { setError(""); await run({ kind: "plan", action, args }); await refresh(); } catch (e) { setError(String(e)); } };
  return <section aria-label="Plans"><h3>Plans</h3>{error && <p role="alert">{error}</p>}<form onSubmit={e => { e.preventDefault(); void execute("draft", { input: { kind: "inline", text: goal }, previous: null, feedback: null }); }}><label>Plan objective<input value={goal} onChange={e => setGoal(e.target.value)} /></label><Button disabled={!goal} type="submit">Draft a plan</Button><Button type="button" onClick={() => void refresh().catch(e => setError(String(e)))}>Refresh plans</Button></form>
    {!plans.length && <p>No plans recorded.</p>}
    {plans.map(plan => <article key={plan.planId}><h4>{plan.goal} · {plan.status}</h4><p>{plan.summary}</p><ol>{plan.steps.map((step, stepIndex) => <li key={stepIndex}>{step} <Button onClick={() => void execute(plan.completedStepIndexes?.includes(stepIndex) ? "reopen_step" : "complete_step", { planId: plan.planId, revision: plan.revision, stepIndex })}>{plan.completedStepIndexes?.includes(stepIndex) ? "Reopen" : "Mark step complete"}</Button></li>)}</ol>
      {plan.openQuestions.map((question, questionIndex) => <form key={questionIndex} onSubmit={e => { e.preventDefault(); void execute("answer", { planId: plan.planId, revision: plan.revision, questionIndex, answer: answers[`${plan.planId}:${questionIndex}`] ?? "" }); }}><label>{question}<input value={answers[`${plan.planId}:${questionIndex}`] ?? ""} onChange={e => setAnswers({ ...answers, [`${plan.planId}:${questionIndex}`]: e.target.value })} list={`options-${plan.planId}-${questionIndex}`} /></label><datalist id={`options-${plan.planId}-${questionIndex}`}>{plan.clarificationOptionSets?.[questionIndex]?.map(option => <option key={option} value={option} />)}</datalist><Button type="submit">Answer question</Button></form>)}
      <details><summary>Complete plan record</summary><pre>{JSON.stringify(plan, null, 2)}</pre></details>
      {["accept", "reject", "clear", "execute"].map(action => <Button key={action} onClick={() => void execute(action, { planId: plan.planId, revision: plan.revision })}>{action}</Button>)}</article>)}
  </section>;
}
