/* Derived from Dyad vendor/dyad/src/lib/utils.ts at fb17039. Apache-2.0; see product/licenses/dyad-p04-extraction.json. */
import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}
