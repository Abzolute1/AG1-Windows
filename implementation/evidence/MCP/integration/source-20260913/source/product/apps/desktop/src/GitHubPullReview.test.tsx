import { afterEach, expect, test, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { GitHubPullReview, GitHubReviewContext, pullPatchLines } from "./GitHubPullReview";
import { GitHubResult } from "./GitHubResult";
const invoke=vi.hoisted(()=>vi.fn());vi.mock("@tauri-apps/api/core",()=>({invoke}));
afterEach(()=>{cleanup();vi.clearAllMocks();});
const head="a".repeat(40),base="b".repeat(40),target={repo:"owned/fixture",number:7};
const item={number:7,title:"Owned PR",head:{sha:head},base:{sha:base},state:"open",mergeable:null};
function page(args:any,files:any[]=[{filename:"file.txt",status:"modified",additions:1,deletions:1,patch:"@@ -1 +1 @@\n-old\n+new"}]) {return {item,head_sha:head,base_sha:base,files,checks:[{name:"Owned check",status:"completed",conclusion:"success"}],page:args.page,check_page:args.check_page,files_next:true,checks_next:true,limits:"Not an atomic server snapshot."};}
function fixture(files?:any[]) {invoke.mockImplementation(async(_,{action,args})=>action==="api_read"?{ok:true,value:{data:item}}:page(args,files));}
test("PR review binds exact commits and independently pages files and checks without mutations",async()=>{
  fixture();render(<GitHubPullReview target={target}/>);await screen.findByText("Owned PR");
  expect(invoke).toHaveBeenCalledWith("github_command",{action:"pull_review",args:{repo:target.repo,number:7,head_sha:head,base_sha:base,page:1,check_page:1}});
  expect(screen.getByText(/not yet known/)).toBeTruthy();expect(screen.getByText(/not proof that every branch protection/)).toBeTruthy();
  fireEvent.click(screen.getByRole("button",{name:"Next PR files"}));await screen.findByText("File page 2");
  fireEvent.click(screen.getByRole("button",{name:"Next PR checks"}));await screen.findByText("Check page 2");
  expect(invoke).toHaveBeenLastCalledWith("github_command",{action:"pull_review",args:{repo:target.repo,number:7,head_sha:head,base_sha:base,page:2,check_page:2}});
  expect(invoke.mock.calls.every(([,v])=>["api_read","pull_review"].includes(v.action))).toBe(true);
});
test("patch rows are paged, numbered and literal; omitted patches are not empty diffs",async()=>{
  const patch="@@ -4,0 +9,620 @@\n"+Array.from({length:620},(_,i)=>`+line ${i} <script>literal</script>`).join("\n");fixture([{filename:"literal <script>.txt",patch},{filename:"binary.dat",patch:null}]);
  const {container}=render(<GitHubPullReview target={target}/>);fireEvent.click(await screen.findByRole("button",{name:"literal <script>.txt"}));
  expect(container.querySelectorAll('.review-diff pre > span')).toHaveLength(300);expect(container.querySelector("script")).toBeNull();
  fireEvent.click(screen.getByRole("button",{name:"Next patch page"}));expect(screen.getByText("Patch page 2")).toBeTruthy();
  fireEvent.click(screen.getByRole("button",{name:"binary.dat"}));expect(screen.getByText(/This is not an empty diff/)).toBeTruthy();
  expect(pullPatchLines("@@ -4,2 +9,2 @@\n-old\n+new\n same")).toEqual([{text:"@@ -4,2 +9,2 @@",kind:"hunk"},{text:"-old",old:4,kind:"removed"},{text:"+new",next:9,kind:"added"},{text:" same",old:5,next:10,kind:""}]);
});
test("late private responses cannot populate a different PR or a hidden review",async()=>{
  let resolve!:(v:any)=>void;invoke.mockImplementation(()=>new Promise(r=>resolve=r));const view=render(<GitHubPullReview target={target}/>);
  view.rerender(<GitHubPullReview target={{repo:"other/repo",number:8}} visible={false}/>);resolve({ok:true,value:{data:item}});
  await waitFor(()=>expect(screen.queryByRole("status")).toBeNull());expect(screen.queryByText("Owned PR")).toBeNull();expect(invoke).toHaveBeenCalledTimes(1);
});
test("changed head on later page discards the old display and does not retry automatically",async()=>{
  fixture();render(<GitHubPullReview target={target}/>);await screen.findByText("Owned PR");invoke.mockRejectedValueOnce(new Error("The pull request head changed. Refresh."));
  fireEvent.click(screen.getByRole("button",{name:"Next PR files"}));expect((await screen.findByRole("alert")).textContent).toContain("head changed");expect(screen.queryByText("Owned PR")).toBeNull();expect(invoke).toHaveBeenCalledTimes(3);
});
test("model PR result opens the shared review by explicit click, not by result text",()=>{
  const open=vi.fn();render(<GitHubReviewContext.Provider value={{open,clear:vi.fn()}}><GitHubResult call={{kind:"tool",name:"github",id:"call",arguments:{operation:"pulls.get",repo:target.repo,parameters:{number:7}}} as any} message={{id:"result",role:"tool",blocks:[{kind:"text",text:JSON.stringify({outcome:"read",data:item})}],tool:{name:"github",call_id:"call",failed:false}} as any} expand={vi.fn()}/></GitHubReviewContext.Provider>);
  expect(open).not.toHaveBeenCalled();fireEvent.click(screen.getByRole("button",{name:"Review PR files and checks"}));expect(open).toHaveBeenCalledWith(target);expect(invoke).not.toHaveBeenCalled();
});
