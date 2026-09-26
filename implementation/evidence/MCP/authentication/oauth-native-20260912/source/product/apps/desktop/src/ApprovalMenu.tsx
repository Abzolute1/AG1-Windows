import { Check, Hand, ListChecks, Zap } from "lucide-react";
export const approvalModes = [
  { id: "interactive", label: "Ask before changes", description: "Review writes and shell commands before they run.", icon: Hand },
  { id: "autonomous", label: "Allow routine changes", description: "Allow routine work; block risky or destructive commands.", icon: ListChecks },
  { id: "disabled", label: "Bypass approvals", description: "Run tools without approval prompts. Only use with work you trust.", icon: Zap },
] as const;
export type ApprovalMode = typeof approvalModes[number]["id"];
export function ApprovalChoices({ value, onChange }: { value: string; onChange: (mode: ApprovalMode) => void }) {
  return <div className="approval-choices" role="radiogroup" aria-label="Tool approval policy">{approvalModes.map(({ id, label, description, icon: Icon }) => <button type="button" role="radio" aria-label={label} aria-checked={value === id} key={id} onClick={() => onChange(id)}><Icon size={17} /><span>{label}<small>{description}</small></span>{value === id && <Check size={15} />}</button>)}</div>;
}

export function ApprovalIcon({ value, size = 15 }: { value: string; size?: number }) { const Icon = approvalModes.find(m => m.id === value)?.icon ?? Hand; return <Icon size={size} />; }
