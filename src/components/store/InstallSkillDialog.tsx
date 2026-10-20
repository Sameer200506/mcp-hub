import { useEffect, useState } from "react";
import { Download } from "lucide-react";
import { Button, Label, Switch } from "@/components/ui/primitives";
import { Dialog, DialogContent } from "@/components/ui/dialog";
import type { SkillEntry, StoreSettings } from "@/types";

export function InstallSkillDialog({
  skill,
  settings,
  onClose,
  onInstall,
}: {
  skill: SkillEntry | null;
  settings: StoreSettings | null;
  onClose: () => void;
  onInstall: (targetIds: string[], overwrite: boolean) => Promise<void>;
}) {
  const [targetIds, setTargetIds] = useState<string[]>([]);
  const [overwrite, setOverwrite] = useState(false);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!skill) return;
    setTargetIds(settings?.defaultTargetIds ?? []);
    setOverwrite(false);
    setBusy(false);
  }, [skill, settings]);

  if (!skill) return null;

  const toggleTarget = (id: string) => {
    setTargetIds((current) =>
      current.includes(id) ? current.filter((value) => value !== id) : [...current, id],
    );
  };

  const install = async () => {
    setBusy(true);
    try {
      await onInstall(targetIds, overwrite);
    } catch {
      // The page shows a toast; keep the dialog open so the user can retry.
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog open={Boolean(skill)} onOpenChange={(next) => (next ? undefined : onClose())}>
      <DialogContent
        title={`Install ${skill.name}`}
        description={`${skill.sourceLabel} · ${skill.files.length} file${skill.files.length === 1 ? "" : "s"} · SKILL.md`}
        size="md"
        footer={
          <>
            <Button variant="outline" onClick={onClose} disabled={busy}>
              Cancel
            </Button>
            <Button onClick={() => void install()} loading={busy}>
              <Download />
              Install skill
            </Button>
          </>
        }
      >
        <div className="space-y-4">
          <p className="text-sm text-muted-foreground">
            {skill.description || "This skill has no description in its SKILL.md frontmatter."}
          </p>

          <div className="rounded-md border border-border bg-muted/30 p-3">
            <p className="text-xs font-medium">Always installed into the app library</p>
            <p className="mt-1 text-[11px] text-muted-foreground">
              The skill is downloaded first and only then copied to the selected harness folders.
              Downloaded paths are validated so a skill cannot escape its folder.
            </p>
          </div>

          <div className="space-y-2">
            <Label>Deploy to harness skill folders (optional)</Label>
            {!settings || settings.skillTargets.length === 0 ? (
              <p className="text-xs text-muted-foreground">
                No targets configured. The skill will only be stored in the app library; add targets
                in Store settings.
              </p>
            ) : (
              <div className="space-y-1.5">
                {settings.skillTargets.map((target) => {
                  const selected = targetIds.includes(target.id);
                  return (
                    <label
                      key={target.id}
                      className="flex cursor-pointer items-start gap-3 rounded-md border border-border px-3 py-2"
                    >
                      <Switch
                        checked={selected}
                        onCheckedChange={() => toggleTarget(target.id)}
                        aria-label={`Deploy to ${target.label}`}
                      />
                      <span className="min-w-0">
                        <span className="block text-sm">{target.label}</span>
                        <span className="block truncate font-mono text-[11px] text-muted-foreground">
                          {target.path}
                        </span>
                      </span>
                    </label>
                  );
                })}
                <p className="text-[11px] text-muted-foreground">
                  Leave every target unchecked to keep the skill in the app library only.
                </p>
              </div>
            )}
          </div>

          <label className="flex cursor-pointer items-center justify-between gap-3 rounded-md border border-border px-3 py-2">
            <span>
              <span className="block text-sm">Overwrite existing copies</span>
              <span className="block text-[11px] text-muted-foreground">
                Required when the skill or a deployed copy already exists.
              </span>
            </span>
            <Switch
              checked={overwrite}
              onCheckedChange={setOverwrite}
              aria-label="Overwrite existing skill copies"
            />
          </label>
        </div>
      </DialogContent>
    </Dialog>
  );
}
