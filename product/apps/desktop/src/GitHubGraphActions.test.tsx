import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import { GitHubGraphActions, type GraphActionTarget } from "./GitHubGraphActions";
const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
afterEach(() => { cleanup(); vi.clearAllMocks(); });
function preview(request: unknown, risk = "write", expires = 300) { return { ok: true, value: { confirmation: "ticket", account: { login: "owned", host: "github.com" }, request, risk, expires_in_seconds: expires } }; }
const cases: [GraphActionTarget, string, Record<string, string>, string, Record<string, unknown>][] = [
  [{ kind: "project", id: "P", title: "Before" }, "edit", { "Graph title": "After", "Graph text": "Readme" }, "projects.update", { projectId: "P", title: "After", readme: "Readme" }],
  [{ kind: "project", id: "P" }, "state", {}, "projects.update", { projectId: "P", closed: true }],
  [{ kind: "project", id: "P" }, "add", { "Issue or pull request node ID": "ISSUE" }, "projects.add_item", { projectId: "P", contentId: "ISSUE" }],
  [{ kind: "project", id: "P" }, "draft", { "Graph title": "Draft", "Graph text": "Body" }, "projects.add_draft", { projectId: "P", title: "Draft", body: "Body" }],
  [{ kind: "discussion", id: "D", title: "Before" }, "edit", { "Graph title": "After", "Graph text": "Body" }, "discussions.update", { discussionId: "D", title: "After", body: "Body" }],
  [{ kind: "discussion", id: "D" }, "state", {}, "discussions.close", { discussionId: "D", reason: "RESOLVED" }],
  [{ kind: "discussion", id: "D", closed: true }, "state", {}, "discussions.reopen", { discussionId: "D" }],
  [{ kind: "discussion", id: "D" }, "comment", { "Graph text": "Comment" }, "discussions.comment", { discussionId: "D", body: "Comment" }],
  [{ kind: "comment", id: "C", discussionId: "D" }, "reply", { "Graph text": "Reply" }, "discussions.comment", { discussionId: "D", replyToId: "C", body: "Reply" }],
  [{ kind: "comment", id: "C" }, "edit", { "Graph text": "Edit" }, "discussions.edit_comment", { commentId: "C", body: "Edit" }],
  [{ kind: "comment", id: "C" }, "answer", {}, "discussions.answer", { id: "C" }],
  [{ kind: "comment", id: "C", isAnswer: true }, "answer", {}, "discussions.unanswer", { id: "C" }],
  [{ kind: "item", id: "I", projectId: "P" }, "archive", {}, "projects.archive", { projectId: "P", itemId: "I" }],
  [{ kind: "item", id: "I", projectId: "P", archived: true }, "archive", {}, "projects.unarchive", { projectId: "P", itemId: "I" }],
  [{ kind: "field", id: "F", projectId: "P", itemId: "I" }, "clear", {}, "projects.clear_field", { projectId: "P", itemId: "I", fieldId: "F" }],
  [{ kind: "field", id: "F", projectId: "P", itemId: "I" }, "set", { "Field value": "Literal <script>text</script>" }, "projects.update_field", { projectId: "P", itemId: "I", fieldId: "F", value: { text: "Literal <script>text</script>" } }],
];
test.each(cases)("%j %s creates exact native arguments and confirms separately", async (target, action, values, operation, input) => {
  invoke.mockImplementation(async (_, { action: command, args }) => command === "api_preview" ? preview(args) : { ok: true, value: { outcome: "applied", data: {} } });
  render(<GitHubGraphActions target={target} scope="one" />);
  fireEvent.change(screen.getByLabelText("Graph action"), { target: { value: action } });
  for (const [label, value] of Object.entries(values)) fireEvent.change(screen.getByLabelText(label), { target: { value } });
  fireEvent.click(screen.getByText("Review graph change")); await screen.findByLabelText("Exact native graph change");
  expect(invoke).toHaveBeenCalledTimes(1); expect(invoke.mock.calls[0][1]).toEqual({ action: "api_preview", args: { operation, parameters: { input } } });
  fireEvent.click(screen.getByText("Confirm graph change")); await screen.findByText(/GitHub responded/);
  expect(invoke.mock.calls[1][1]).toEqual({ action: "api_confirm", args: { confirmation: "ticket" } });
});
test("field types reject invalid numbers and preserve array/date/number values", async () => {
  invoke.mockImplementation(async (_, {args}) => preview(args));
  render(<GitHubGraphActions target={{kind:"field",id:"F",projectId:"P",itemId:"I"}} scope="one" />);
  for (const [type, value, expected] of [["number", "3.5", 3.5], ["date", "2026-09-10", "2026-09-10"], ["multiSelectOptionIds", "a\nb", ["a","b"]]] as const) {
    fireEvent.change(screen.getByLabelText("Field value type"),{target:{value:type}});fireEvent.change(screen.getByLabelText("Field value"),{target:{value}});fireEvent.click(screen.getByText("Review graph change"));await screen.findByLabelText("Exact native graph change");
    expect(invoke.mock.calls.filter(([,v])=>v.action==="api_preview").at(-1)![1].args.parameters.input.value).toEqual({[type]:expected});fireEvent.click(screen.getByText("Back to graph form"));
  }
  fireEvent.change(screen.getByLabelText("Field value type"),{target:{value:"number"}});fireEvent.change(screen.getByLabelText("Field value"),{target:{value:"NaN"}});const before=invoke.mock.calls.length;fireEvent.click(screen.getByText("Review graph change"));await screen.findByText("Enter a finite number.");expect(invoke).toHaveBeenCalledTimes(before);
});
test("scope change dismisses late tickets and ignores private results", async () => {
  let resolve!:(v:unknown)=>void;invoke.mockImplementationOnce(()=>new Promise(r=>resolve=r)).mockResolvedValue({ok:true});
  const ui=render(<GitHubGraphActions target={{kind:"comment",id:"C",body:"Old private"}} scope="one"/>);fireEvent.click(screen.getByText("Review graph change"));
  ui.rerender(<GitHubGraphActions target={{kind:"comment",id:"OTHER",body:"New"}} scope="two"/>);await act(async()=>resolve(preview({body:"Old private"})));expect(screen.queryByLabelText("Exact native graph change")).toBeNull();expect(screen.getByLabelText("Graph text")).toHaveProperty("value","New");expect(invoke).toHaveBeenCalledWith("github_command",{action:"api_dismiss",args:{confirmation:"ticket"}});
});
test("consequential acknowledgement, expiry and uncertain results never auto retry", async () => {
  invoke.mockImplementation(async (_, {action,args}) => action==="api_preview"?preview(args,"consequential",1):{ok:false,error:{message:"Interrupted",outcome:"unknown"}});
  render(<GitHubGraphActions target={{kind:"project",id:"P",title:"Title"}} scope="one"/>);fireEvent.click(screen.getByText("Review graph change"));await screen.findByLabelText("Exact native graph change");expect(screen.getByText("Confirm graph change")).toHaveProperty("disabled",true);fireEvent.click(screen.getByRole("checkbox"));expect(screen.getByText("Confirm graph change")).toHaveProperty("disabled",false);await screen.findByText(/preview expired/,{}, {timeout:2000});expect(screen.getByText("Confirm graph change")).toHaveProperty("disabled",true);
  fireEvent.click(screen.getByText("Back to graph form"));fireEvent.click(screen.getByText("Review graph change"));await screen.findByLabelText("Exact native graph change");fireEvent.click(screen.getByRole("checkbox"));fireEvent.click(screen.getByText("Confirm graph change"));await screen.findByText(/change may have applied/);expect(invoke.mock.calls.filter(([,v])=>v.action==="api_confirm")).toHaveLength(1);expect(screen.queryByText("Confirm graph change")).toBeNull();
});
