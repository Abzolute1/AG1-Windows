import { afterEach, expect, test, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { ChatFolders, type FolderSelection } from "./ChatFolders";
afterEach(cleanup);
const selection: FolderSelection = { revision: 4, primary: "/project", folders: [{ path: "/project", mode: "read_write" }, { path: "/reference", mode: "read_only" }] };
test("Browse selects a path but never grants access until Add is clicked", async () => {
  const change = vi.fn(), chooseFolder = vi.fn().mockResolvedValue("/chosen folder");
  render(<ChatFolders selection={selection} change={change} chooseFolder={chooseFolder} />);
  fireEvent.click(screen.getByRole("button", { name: "Browse…" }));
  await waitFor(() => expect((screen.getByLabelText("Add a folder") as HTMLInputElement).value).toBe("/chosen folder"));
  expect(change).not.toHaveBeenCalled();
});
test("adding a folder defaults to read-only and submits an exact revision and path", async () => {
  const change = vi.fn().mockResolvedValue({ ...selection, revision: 5 });
  render(<ChatFolders selection={selection} change={change} />);
  expect((screen.getByRole("combobox", { name: "New folder access" }) as HTMLSelectElement).value).toBe("read_only");
  fireEvent.change(screen.getByLabelText("Add a folder"), { target: { value: "/other project" } });
  fireEvent.click(screen.getByRole("button", { name: "Add folder" }));
  await waitFor(() => expect(change).toHaveBeenCalledWith({ revision: 4, action: "add", path: "/other project", mode: "read_only" }));
  await waitFor(() => expect((screen.getByLabelText("Add a folder") as HTMLInputElement).value).toBe(""));
});
test("working folder and revocation are separate, explicit actions", async () => {
  const change = vi.fn().mockResolvedValue({ ...selection, revision: 5, primary: "/reference" });
  render(<ChatFolders selection={selection} change={change} />);
  expect(screen.queryByRole("button", { name: "Remove /project" })).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "Make working folder" }));
  await waitFor(() => expect(change).toHaveBeenCalledWith({ revision: 4, action: "primary", path: "/reference" }));
  fireEvent.click(await screen.findByRole("button", { name: "Remove /project" }));
  await waitFor(() => expect(change).toHaveBeenLastCalledWith({ revision: 5, action: "remove", path: "/project" }));
});
test("read/write changes identify the exact folder and retain a rejected path for correction", async () => {
  const change = vi.fn().mockRejectedValue(new Error("That folder includes AG1's private storage."));
  render(<ChatFolders selection={selection} change={change} />);
  fireEvent.change(screen.getByRole("combobox", { name: "Access for /reference" }), { target: { value: "read_write" } });
  await waitFor(() => expect(change).toHaveBeenCalledWith({ revision: 4, action: "mode", path: "/reference", mode: "read_write" }));
  expect(await screen.findByRole("alert")).toBeTruthy();
  expect((screen.getByRole("combobox", { name: "Access for /reference" }) as HTMLSelectElement).value).toBe("read_only");
});
test("running or dirty chats cannot submit permission changes", () => {
  const change = vi.fn();
  render(<ChatFolders selection={selection} blocked="Finish the response first." change={change} />);
  expect(screen.getByRole("status").textContent).toBe("Finish the response first.");
  for (const button of screen.getAllByRole("button")) { expect((button as HTMLButtonElement).disabled).toBe(true); fireEvent.click(button); }
  expect(change).not.toHaveBeenCalled();
});
test("unavailable folders remain removable and do not disappear from the review", () => {
  render(<ChatFolders selection={{ ...selection, folders: [selection.folders[0], { ...selection.folders[1], available: false }] }} change={vi.fn()} />);
  const folder = screen.getByRole("region", { name: "Folder /reference" });
  expect(within(folder).getByText(/Unavailable/)).toBeTruthy();
  expect(within(folder).getByRole("button", { name: "Remove /reference" })).toBeTruthy();
});
