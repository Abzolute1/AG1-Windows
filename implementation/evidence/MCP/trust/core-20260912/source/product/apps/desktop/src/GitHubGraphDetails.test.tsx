import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import { GitHubGraphDetails } from "./GitHubGraphDetails";
const invoke=vi.hoisted(()=>vi.fn());
vi.mock("@tauri-apps/api/core",()=>({invoke}));
afterEach(()=>{cleanup();vi.clearAllMocks();});
function page(nodes:unknown[],cursor:string|null=null){return{nodes,pageInfo:{hasNextPage:!!cursor,endCursor:cursor}};}
function result(root:unknown,repository=false){return{ok:true,value:{data:{data:{[repository?"repository":"node"]:root,rateLimit:{cost:1,remaining:40,resetAt:"later"}}}}};}
const calls=()=>invoke.mock.calls.map(([,v])=>v);
test("project fields, items and item values use separate native cursors and node targets",async()=>{
  invoke.mockImplementation(async(_,{args:r})=>result(r.operation==="projects.get"?{id:r.parameters.id,title:"Owned project",readme:"Literal <script>project</script>"}:r.operation==="projects.fields"?{id:"P1",fields:page([{id:"F1",name:r.parameters.after?"Second field":"First field",options:[{id:"option",name:"Done"}]}],r.parameters.after?null:"field-cursor")}:r.operation==="projects.items"?{id:"P1",items:page([{id:"I1",content:{title:"Owned task"}}],"item-cursor")}: {id:"I1",fieldValues:page([{id:"V1",field:{name:"Status"},name:"Done"}],"value-cursor")}));
  render(<GitHubGraphDetails target={{kind:"project",id:"P1"}} scope="one" close={vi.fn()}/>);
  await screen.findByText("Owned project");fireEvent.click(screen.getByText("Project fields"));await screen.findByText("First field");fireEvent.click(screen.getByText("Next graph page"));await screen.findByText("Second field");
  expect(calls().at(-1)).toEqual({action:"api_read",args:{operation:"projects.fields",parameters:{id:"P1",after:"field-cursor"}}});
  fireEvent.click(screen.getByText("Previous graph page"));await screen.findByText("First field");expect(calls().at(-1)!.args.parameters).toEqual({id:"P1"});
  fireEvent.click(screen.getByText("Project items"));fireEvent.click(await screen.findByText("Owned task"));fireEvent.click(screen.getByText("Inspect item field values"));await screen.findByText("Done");
  expect(calls().at(-1)!.args).toEqual({operation:"projects.item_values",parameters:{id:"I1"}});expect(calls().every(c=>c.action==="api_read"&&!c.args.repo)).toBe(true);
  expect(screen.getByText(/browse its values on separate pages/)).toBeTruthy();
});
test("discussion comments, categories and replies bind repo or explicit account-wide IDs",async()=>{
  invoke.mockImplementation(async(_,{args:r})=>r.operation==="discussions.get"?result({nameWithOwner:"owned/repo",discussion:{id:"D1",title:"Owned discussion",number:7,comments:page([{id:"C1",body:"Literal <script>comment</script>",author:{login:"owned"}}],"comment-cursor")}},true):r.operation==="discussions.categories"?result({nameWithOwner:"owned/repo",discussionCategories:page([{id:"Cat1",name:"Ideas"}],"category-cursor")},true):result({id:"C1",replies:page([{id:"R1",body:"Owned reply"}],"reply-cursor")}));
  render(<GitHubGraphDetails target={{kind:"discussion",repo:"owned/repo",number:7}} scope="one" close={vi.fn()}/>);
  await screen.findByText("Owned discussion");expect(calls()[0].args).toEqual({operation:"discussions.get",repo:"owned/repo",parameters:{owner:"owned",name:"repo",number:7}});
  fireEvent.click(screen.getByText("owned",{selector:"strong"}));fireEvent.click(screen.getByText("Inspect comment replies"));await screen.findByText("R1",{selector:"strong"});expect(calls().at(-1)!.args).toEqual({operation:"discussions.replies",parameters:{id:"C1"}});
  fireEvent.click(screen.getByText("Discussion categories"));await screen.findByText("Ideas");expect(calls().at(-1)!.args.parameters).toEqual({owner:"owned",name:"repo"});
  fireEvent.click(screen.getByText("Next graph page"));await waitFor(()=>expect(calls().at(-1)!.args.parameters.after).toBe("category-cursor"));
  expect(document.querySelector('[aria-label="GitHub graph details"] script')).toBeNull();
});
test("late private graph results are discarded on scope change and unmount",async()=>{
  let resolve!:(v:unknown)=>void;invoke.mockImplementationOnce(()=>new Promise(r=>{resolve=r;})).mockResolvedValue(result({id:"P2",title:"Second project"}));
  const ui=render(<GitHubGraphDetails target={{kind:"project",id:"P1"}} scope="one" close={vi.fn()}/>);
  ui.rerender(<GitHubGraphDetails target={{kind:"project",id:"P2"}} scope="two" close={vi.fn()}/>);await screen.findByText("Second project");await act(async()=>resolve(result({id:"P1",title:"Old private project"})));expect(screen.queryByText("Old private project")).toBeNull();ui.unmount();
});
test("repeated or malformed cursors and permission errors clear pages without auto retry",async()=>{
  invoke.mockResolvedValueOnce(result({id:"P1",title:"Owned project"})).mockResolvedValueOnce(result({id:"P1",fields:page([{name:"First"}],"repeat")})).mockResolvedValueOnce(result({id:"P1",fields:page([{name:"Discard me"}],"repeat")}));
  render(<GitHubGraphDetails target={{kind:"project",id:"P1"}} scope="one" close={vi.fn()}/>);await screen.findByText("Owned project");fireEvent.click(screen.getByText("Project fields"));await screen.findByText("First");fireEvent.click(screen.getByText("Next graph page"));await screen.findByText(/repeated page cursor/);expect(screen.queryByText("First")).toBeNull();expect(screen.queryByText("Discard me")).toBeNull();expect(calls()).toHaveLength(3);
  invoke.mockResolvedValueOnce({ok:false,error:{message:"Missing project permission."}});fireEvent.click(screen.getByText("Refresh graph view"));await screen.findByText("Missing project permission.");expect(calls()).toHaveLength(4);
});
test("large descriptions use bounded literal previews with full paged source",async()=>{
  invoke.mockResolvedValue(result({id:"P1",title:"Owned project",readme:"<script>literal</script>"+"x".repeat(40000)}));
  render(<GitHubGraphDetails target={{kind:"project",id:"P1"}} scope="one" close={vi.fn()}/>);await screen.findByText(/Text preview limited/);expect(document.querySelector('[aria-label="GitHub graph details"] script')).toBeNull();expect(screen.getByLabelText("Graph detail source").textContent!.length).toBeLessThanOrEqual(16000);
});

test.each([
  ["ProjectV2ItemFieldUserValue", "projects.field_users"],
  ["ProjectV2ItemFieldLabelValue", "projects.field_labels"],
  ["ProjectV2ItemFieldPullRequestValue", "projects.field_pull_requests"],
  ["ProjectV2ItemFieldReviewerValue", "projects.field_reviewers"],
])("nested %s pages bind item, field ID/name and independent cursor", async (type, operation) => {
  invoke.mockImplementation(async (_, {args:r}) => result(r.operation === "projects.get" ? {id:"P1",title:"Project"} : r.operation === "projects.items" ? {id:"P1",items:page([{id:"I1",content:{title:"Task"}}])} : r.operation === "projects.item_values" ? {id:"I1",fieldValues:page([{__typename:type,field:{id:"F1",name:"People"}}],"item-values-cursor")} : {id:"I1",fieldValueByName:{__typename:type,field:{id:r.parameters.fieldId,name:r.parameters.fieldName},values:page([{id:"U1",login:r.parameters.after?"second-person":"first-person"}],r.parameters.after?null:"people-cursor")}}));
  const original = invoke.getMockImplementation()!;
  invoke.mockImplementation(async (...args) => { const response = await original(...args); if (args[1].args.operation === operation) response.value.data.data.field={id:"F1"}; return response; });
  render(<GitHubGraphDetails target={{kind:"project",id:"P1"}} scope="one" close={vi.fn()}/>);
  await screen.findByText("Project",{selector:"h3"});fireEvent.click(screen.getByText("Project items"));fireEvent.click(await screen.findByText("Task"));fireEvent.click(screen.getByText("Inspect item field values"));fireEvent.click(await screen.findByText("People",{selector:"strong"}));fireEvent.click(screen.getByText("Browse field values"));await screen.findByText("first-person");
  expect(calls().at(-1).args).toEqual({operation,parameters:{id:"I1",fieldId:"F1",fieldName:"People"}});
  fireEvent.click(screen.getByText("Next graph page"));await screen.findByText("second-person");expect(calls().at(-1).args.parameters.after).toBe("people-cursor");
  invoke.mockResolvedValueOnce(result({id:"I1",fieldValueByName:{__typename:type,field:{id:"wrong",name:"People"},values:page([{login:"Do not show"}])}}));
  fireEvent.click(screen.getByText("Refresh graph view"));await screen.findByText(/selected field changed/);expect(screen.queryByText("Do not show")).toBeNull();expect(screen.queryByText("second-person")).toBeNull();
});
