import { errorMessage } from "./bridge";

export function reviewError(error: unknown): string {
  const message = errorMessage(error);
  if (/native Git helper/i.test(message)) return "Workspace review is unavailable because the Git helper is not configured. Artifact review is still available.";
  if (/not a git repository|unborn|unknown revision.*HEAD/i.test(message)) return "This folder has no Git history to compare. Open a repository with a commit to review tracked changes.";
  if (/integrity mismatch|invalid artifact|invalid.*byte page/i.test(message)) return "This output could not be verified. Refresh the artifact list and try again.";
  return message.replace(/^Error:\s*/, "");
}
