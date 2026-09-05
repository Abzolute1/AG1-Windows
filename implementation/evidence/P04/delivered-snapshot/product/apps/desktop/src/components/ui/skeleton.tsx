/* Derived from Dyad vendor/dyad/src/components/ui/skeleton.tsx at fb17039. Apache-2.0; see product/licenses/dyad-p04-extraction.json. */
import { cn } from "@/lib/utils";

function Skeleton({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="skeleton"
      className={cn("bg-accent animate-pulse rounded-md", className)}
      {...props}
    />
  );
}

export { Skeleton };
