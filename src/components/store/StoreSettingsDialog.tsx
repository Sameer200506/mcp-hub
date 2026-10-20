import { useEffect, useState } from "react";
import { Plus, Trash2 } from "lucide-react";
import { Button, Card, Field, Input, Label, Separator, Switch } from "@/components/ui/primitives";
import { Dialog, DialogContent } from "@/components/ui/dialog";
import { useToast } from "@/components/ui/toast";
import { api } from "@/services/api";
import { describeError } from "@/hooks/useRegistry";
import type { SkillSource, SkillTarget, StoreSettings } from "@/types";

export function StoreSettingsDialog({
  open,
  settings,
  onClose,
  onSaved,
}: {
  open: boolean;
  settings: StoreSettings | null;
  onClose: () => void;
  onSaved: (settings: StoreSettings) => void;
}) {
  const { toast } = useToast();
  const [draft, setDraft] = useState<StoreSettings | null>(null);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    if (open && settings) {
      setDraft({
        skillSources: settings.skillSources.map((source) => ({ ...source })),
        skillTargets: settings.skillTargets.map((target) => ({ ...target })),
        defaultTargetIds: [...settings.defaultTargetIds],
      });
    }
  }, [open, settings]);

  const updateSource = (index: number, patch: Partial<SkillSource>) => {
    if (!draft) return;
    setDraft({
      ...draft,
      skillSources: draft.skillSources.map((source, position) =>
        position === index ? { ...source, ...patch } : source,
      ),
    });
  };

  const updateTarget = (index: number, patch: Partial<SkillTarget>) => {
    if (!draft) return;
    setDraft({
      ...draft,
      skillTargets: draft.skillTargets.map((target, position) =>
        position === index ? { ...target, ...patch } : target,
      ),
    });
  };

  const save = async () => {
    if (!draft) return;
    setSaving(true);
    try {
      const saved = await api.updateStoreSettings(draft);
      onSaved(saved);
      toast({ title: "Store settings saved", variant: "success" });
      onClose();
    } catch (err) {
      toast({
        title: "Could not save store settings",
        description: describeError(err),
        variant: "error",
      });
    } finally {
      setSaving(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={(next) => (next ? undefined : onClose())}>
      <DialogContent
        title="Store settings"
        description="Skill sources are GitHub repositories that contain a skills folder. Targets are the harness folders that receive installed skills."
        size="lg"
        footer={
          <>
            <Button variant="outline" onClick={onClose} disabled={saving}>
              Cancel
            </Button>
            <Button onClick={() => void save()} loading={saving} disabled={!draft}>
              Save settings
            </Button>
          </>
        }
      >
        {!draft ? (
          <p className="text-sm text-muted-foreground">Loading store settings…</p>
        ) : (
          <div className="space-y-6">
            <section className="space-y-3">
              <div className="flex items-end justify-between gap-3">
                <div>
                  <Label>Skill sources</Label>
                  <p className="text-xs text-muted-foreground">
                    Each source is a public GitHub repository. Skills are discovered by their
                    SKILL.md files.
                  </p>
                </div>
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  onClick={() =>
                    setDraft({
                      ...draft,
                      skillSources: [
                        ...draft.skillSources,
                        {
                          id: `source-${Date.now()}`,
                          label: "New source",
                          repo: "",
                          branch: "main",
                          path: "skills",
                        },
                      ],
                    })
                  }
                >
                  <Plus />
                  Add source
                </Button>
              </div>

              {draft.skillSources.length === 0 ? (
                <p className="rounded-md border border-dashed border-border p-3 text-xs text-muted-foreground">
                  No skill sources configured. The Skills tab will stay empty until you add one.
                </p>
              ) : null}

              <div className="space-y-3">
                {draft.skillSources.map((source, index) => (
                  <Card key={index} className="space-y-3 p-3">
                    <div className="flex items-start justify-between gap-2">
                      <p className="text-xs font-medium text-muted-foreground">Source {index + 1}</p>
                      <Button
                        type="button"
                        variant="ghost"
                        size="icon"
                        aria-label={`Remove skill source ${source.label}`}
                        onClick={() =>
                          setDraft({
                            ...draft,
                            skillSources: draft.skillSources.filter((_, position) => position !== index),
                          })
                        }
                      >
                        <Trash2 />
                      </Button>
                    </div>
                    <div className="grid grid-cols-1 gap-3 md:grid-cols-2">
                      <Field label="Label" hint="Shown in the skills list.">
                        <Input
                          value={source.label}
                          onChange={(event) => updateSource(index, { label: event.target.value })}
                          placeholder="OpenClaw skills"
                        />
                      </Field>
                      <Field label="Repository" hint="owner/repo">
                        <Input
                          value={source.repo}
                          onChange={(event) => updateSource(index, { repo: event.target.value })}
                          placeholder="openclaw/openclaw"
                          className="font-mono"
                        />
                      </Field>
                      <Field label="Skills folder" hint="Repository-relative path.">
                        <Input
                          value={source.path}
                          onChange={(event) => updateSource(index, { path: event.target.value })}
                          placeholder="skills"
                          className="font-mono"
                        />
                      </Field>
                      <Field label="Branch" hint="Defaults to main.">
                        <Input
                          value={source.branch ?? ""}
                          onChange={(event) => updateSource(index, { branch: event.target.value })}
                          placeholder="main"
                          className="font-mono"
                        />
                      </Field>
                    </div>
                  </Card>
                ))}
              </div>
            </section>

            <Separator />

            <section className="space-y-3">
              <div className="flex items-end justify-between gap-3">
                <div>
                  <Label>Deploy targets</Label>
                  <p className="text-xs text-muted-foreground">
                    Installed skills are always kept in the app library and can be copied to these
                    harness skill folders. Absolute paths only.
                  </p>
                </div>
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  onClick={() =>
                    setDraft({
                      ...draft,
                      skillTargets: [
                        ...draft.skillTargets,
                        {
                          id: `custom-${Date.now()}`,
                          label: "Custom folder",
                          path: "",
                        },
                      ],
                    })
                  }
                >
                  <Plus />
                  Add target
                </Button>
              </div>

              <div className="space-y-3">
                {draft.skillTargets.map((target, index) => (
                  <Card key={target.id} className="space-y-3 p-3">
                    <div className="flex items-start justify-between gap-2">
                      <div className="flex items-center gap-2">
                        <Switch
                          checked={draft.defaultTargetIds.includes(target.id)}
                          onCheckedChange={(checked) =>
                            setDraft({
                              ...draft,
                              defaultTargetIds: checked
                                ? [...draft.defaultTargetIds, target.id]
                                : draft.defaultTargetIds.filter((id) => id !== target.id),
                            })
                          }
                          aria-label={`Deploy to ${target.label} by default`}
                        />
                        <span className="text-xs text-muted-foreground">
                          Selected by default at install time
                        </span>
                      </div>
                      <Button
                        type="button"
                        variant="ghost"
                        size="icon"
                        aria-label={`Remove skill target ${target.label}`}
                        onClick={() =>
                          setDraft({
                            ...draft,
                            skillTargets: draft.skillTargets.filter((_, position) => position !== index),
                            defaultTargetIds: draft.defaultTargetIds.filter((id) => id !== target.id),
                          })
                        }
                      >
                        <Trash2 />
                      </Button>
                    </div>
                    <div className="grid grid-cols-1 gap-3 md:grid-cols-2">
                      <Field label="Label">
                        <Input
                          value={target.label}
                          onChange={(event) => updateTarget(index, { label: event.target.value })}
                          placeholder="opencode"
                        />
                      </Field>
                      <Field label="Folder" hint="Absolute directory that contains skill folders.">
                        <Input
                          value={target.path}
                          onChange={(event) => updateTarget(index, { path: event.target.value })}
                          placeholder="C:\\Users\\you\\.config\\opencode\\skills"
                          className="font-mono"
                        />
                      </Field>
                    </div>
                  </Card>
                ))}
              </div>
            </section>
          </div>
        )}
      </DialogContent>
    </Dialog>
  );
}
