import * as TabsPrimitive from "@radix-ui/react-tabs";
import { cn } from "@/lib/utils";
import type { DiffLine } from "@/types";

export const Tabs = TabsPrimitive.Root;

export function TabsList({ className, ...props }: TabsPrimitive.TabsListProps) {
  return (
    <TabsPrimitive.List
      className={cn("inline-flex items-center gap-1 rounded-lg bg-muted p-1", className)}
      {...props}
    />
  );
}

export function TabsTrigger({ className, ...props }: TabsPrimitive.TabsTriggerProps) {
  return (
    <TabsPrimitive.Trigger
      className={cn(
        "inline-flex items-center gap-1.5 rounded-md px-3 py-1 text-xs font-medium text-muted-foreground transition-colors data-[state=active]:bg-card data-[state=active]:text-foreground data-[state=active]:shadow-sm",
        className,
      )}
      {...props}
    />
  );
}

export const TabsContent = TabsPrimitive.Content;

/* -------------------------------------------------------------------------- */
/*                              JSON + diff views                             */
/* -------------------------------------------------------------------------- */

function escapeHtml(value: string): string {
  return value.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}

/** Very small JSON syntax highlighter: safe because everything is escaped first. */
export function highlightJson(source: string): string {
  return escapeHtml(source).replace(
    /("(?:\\.|[^"\\])*"\s*:)|("(?:\\.|[^"\\])*")|\b(true|false)\b|\b(null)\b|(-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?)/g,
    (match, key, str, bool, nul, num) => {
      if (key) return `<span class="tok-key">${key}</span>`;
      if (str) return `<span class="tok-string">${str}</span>`;
      if (bool) return `<span class="tok-bool">${bool}</span>`;
      if (nul) return `<span class="tok-null">${nul}</span>`;
      if (num) return `<span class="tok-number">${num}</span>`;
      return match;
    },
  );
}

export function JsonPreview({
  value,
  className,
  emptyLabel = "No content",
}: {
  value: string | null | undefined;
  className?: string;
  emptyLabel?: string;
}) {
  if (!value) {
    return <p className="text-xs text-muted-foreground">{emptyLabel}</p>;
  }
  return (
    <pre
      className={cn(
        "json-view max-h-[28rem] overflow-auto rounded-lg border border-border bg-muted/40 p-3",
        className,
      )}
      // Content is escaped by highlightJson before any markup is produced.
      dangerouslySetInnerHTML={{ __html: highlightJson(value) }}
    />
  );
}

export function DiffViewer({ lines, className }: { lines: DiffLine[]; className?: string }) {
  if (lines.length === 0) {
    return <p className="text-xs text-muted-foreground">No differences.</p>;
  }
  return (
    <div
      className={cn(
        "json-view max-h-[28rem] overflow-auto rounded-lg border border-border bg-muted/40 p-0",
        className,
      )}
    >
      {lines.map((line, index) => (
        <div
          key={index}
          className={cn(
            "flex gap-2 px-3",
            line.kind === "add" && "bg-emerald-500/10",
            line.kind === "del" && "bg-red-500/10",
          )}
        >
          <span className="w-3 shrink-0 select-none text-muted-foreground">
            {line.kind === "add" ? "+" : line.kind === "del" ? "-" : " "}
          </span>
          <span
            className={cn(
              "whitespace-pre-wrap break-all",
              line.kind === "add" && "text-emerald-700 dark:text-emerald-300",
              line.kind === "del" && "text-red-700 dark:text-red-300",
            )}
          >
            {line.text}
          </span>
        </div>
      ))}
    </div>
  );
}
