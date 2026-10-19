import { useEffect, useState } from "react";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { openPath } from "@tauri-apps/plugin-opener";
import { Download, FolderOpen, Plus, RotateCcw, Save, Trash2, Upload } from "lucide-react";
import {
  Badge,
  Button,
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
  Field,
  Input,
  Select,
  Switch,
  Textarea,
} from "@/components/ui/primitives";
import { ConfirmDialog } from "@/components/ui/dialog";
import { useToast } from "@/components/ui/toast";
import { api } from "@/services/api";
import { describeError, type RegistryState } from "@/hooks/useRegistry";
import type { AppSettings, GenericProfile } from "@/types";
import type { Theme as ThemeType } from "@/hooks/useTheme";

function emptyProfile(): GenericProfile {
  return {
    id: `profile-${Math.random().toString(36).slice(2, 8)}`,
    label: "Custom harness",
    configPath: "",
    mcpPath: "mcpServers",
    serverNameKey: "name",
    transportMapping: {
      stdioUses: "type",
      stdioValue: "stdio",
      remoteUses: "type",
      streamableHttpValue: "streamableHttp",
      sseValue: "sse",
      urlField: "url",
      commandField: "command",
      argsField: "args",
      envField: "env",
      cwdField: "cwd",
      headersField: "headers",
      disabledField: "disabled",
    },
    customFields: [],
    verified: false,
  };
}

export function SettingsPage({
  registry,
  theme,
  onThemeChange,
}: {
  registry: RegistryState;
  theme: ThemeType;
  onThemeChange: (theme: ThemeType) => void;
}) {
  const { toast } = useToast();
  const { settings, refresh } = registry;
  const [draft, setDraft] = useState<AppSettings | null>(settings);
  const [saving, setSaving] = useState(false);
  const [resetting, setResetting] = useState(false);
  const [logPath, setLogPath] = useState<string | null>(null);
  const [pendingProfileDelete, setPendingProfileDelete] = useState<string | null>(null);

  useEffect(() => {
    setDraft(settings);
  }, [settings]);

  useEffect(() => {
    void api
      .getLogFilePath()
      .then(setLogPath)
      .catch(() => setLogPath(null));
  }, []);

  if (!draft) {
    return <p className="text-sm text-muted-foreground">Loading settings…</p>;
  }

  const patch = (partial: Partial<AppSettings>) => setDraft({ ...draft, ...partial });

  const patchProfile = (id: string, partial: Partial<GenericProfile>) =>
    patch({
      genericProfiles: draft.genericProfiles.map((profile) =>
        profile.id === id ? { ...profile, ...partial } : profile,
      ),
    });

  const save = async () => {
    setSaving(true);
    try {
      const saved = await api.updateAppSettings(draft);
      setDraft(saved);
      await refresh();
      toast({ title: "Settings saved", variant: "success" });
    } catch (err) {
      toast({ title: "Could not save settings", description: describeError(err), variant: "error" });
    } finally {
      setSaving(false);
    }
  };

  const reset = async () => {
    setResetting(true);
    try {
      const restored = await api.resetAppSettings();
      setDraft(restored);
      await refresh();
      toast({ title: "Settings reset to defaults", variant: "success" });
    } catch (err) {
      toast({ title: "Reset failed", description: describeError(err), variant: "error" });
    } finally {
      setResetting(false);
    }
  };

  const exportRegistry = async () => {
    try {
      const path = await saveDialog({
        title: "Export registry (secrets omitted)",
        defaultPath: "mcp-registry.json",
        filters: [{ name: "JSON", extensions: ["json"] }],
      });
      if (!path) return;
      const written = await api.exportRegistry({ path, includeDisabled: true, includeLiteralEnvValues: false });
      toast({ title: "Registry exported", description: written, variant: "success" });
    } catch (err) {
      toast({ title: "Export failed", description: describeError(err), variant: "error" });
    }
  };

  const importRegistry = async () => {
    try {
      const path = await openDialog({ title: "Import registry", multiple: false, filters: [{ name: "JSON", extensions: ["json"] }] });
      if (!path || Array.isArray(path)) return;
      const result = await api.importRegistry({ path, mode: "merge" });
      await refresh();
      toast({
        title: `Imported ${result.imported} server(s)`,
        description: `${result.updated} updated · ${result.skipped} skipped`,
        variant: "success",
      });
    } catch (err) {
      toast({ title: "Import failed", description: describeError(err), variant: "error" });
    }
  };

  return (
    <div className="space-y-4">
      <div className="grid grid-cols-1 gap-4 xl:grid-cols-2">
        <Card>
          <CardHeader>
            <CardTitle>Application</CardTitle>
            <CardDescription>Paths are resolved by the Rust backend; the frontend cannot write arbitrary locations.</CardDescription>
          </CardHeader>
          <CardContent className="space-y-4">
            <Field label="Application data directory" hint="Contains the SQLite registry, logs and backups.">
              <Input className="font-mono" value={draft.dataDir} readOnly />
            </Field>
            <Field label="Backup directory" hint="Timestamped backups are created here before every write.">
              <Input
                className="font-mono"
                value={draft.backupDir}
                onChange={(event) => patch({ backupDir: event.target.value })}
              />
            </Field>
            <div className="flex flex-wrap gap-2">
              <Button
                variant="outline"
                size="sm"
                onClick={() => void openPath(draft.dataDir).catch(() => undefined)}
              >
                <FolderOpen />
                Open data directory
              </Button>
              <Button
                variant="outline"
                size="sm"
                onClick={() => void openPath(draft.backupDir).catch(() => undefined)}
              >
                <FolderOpen />
                Open backup directory
              </Button>
              {logPath ? (
                <Button variant="outline" size="sm" onClick={() => void openPath(logPath).catch(() => undefined)}>
                  <FolderOpen />
                  Open log file
                </Button>
              ) : null}
            </div>
            {logPath ? <p className="break-all text-[11px] text-muted-foreground">Log: {logPath}</p> : null}
            <Field label="Theme">
              <Select value={theme} onChange={(event) => onThemeChange(event.target.value as ThemeType)}>
                <option value="dark">Dark</option>
                <option value="light">Light</option>
                <option value="system">Follow system</option>
              </Select>
            </Field>
            <Field label="Logging level" hint="Applies after restart. Logs are rotated locally.">
              <Select
                value={draft.loggingLevel}
                onChange={(event) => patch({ loggingLevel: event.target.value as AppSettings["loggingLevel"] })}
              >
                <option value="error">error</option>
                <option value="warn">warn</option>
                <option value="info">info</option>
                <option value="debug">debug</option>
                <option value="trace">trace</option>
              </Select>
            </Field>
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>Synchronization behavior</CardTitle>
            <CardDescription>Manual staging is the default; automatic sync still validates first.</CardDescription>
          </CardHeader>
          <CardContent className="space-y-4">
            <Field label="Sync mode">
              <Select
                value={draft.syncMode}
                onChange={(event) => patch({ syncMode: event.target.value as AppSettings["syncMode"] })}
              >
                <option value="manual">Manual — stage changes until Sync is pressed</option>
                <option value="automatic">Automatic — validate then synchronize after edits</option>
              </Select>
            </Field>
            <label className="flex items-center justify-between gap-3 text-sm">
              <span>
                Sync on change
                <span className="block text-xs text-muted-foreground">
                  Applies to create/edit/enable changes when sync mode is automatic.
                </span>
              </span>
              <Switch
                checked={draft.autoSyncOnChange}
                onCheckedChange={(checked) => patch({ autoSyncOnChange: checked })}
                aria-label="Sync on change"
              />
            </label>
            <label className="flex items-center justify-between gap-3 text-sm">
              <span>
                Create backups before writes
                <span className="block text-xs text-muted-foreground">
                  Strongly recommended; disabling is recorded as reason “backups-disabled”.
                </span>
              </span>
              <Switch
                checked={draft.createBackups}
                onCheckedChange={(checked) => patch({ createBackups: checked })}
                aria-label="Create backups"
              />
            </label>
            <label className="flex items-center justify-between gap-3 text-sm">
              <span>
                Watch configuration files
                <span className="block text-xs text-muted-foreground">
                  Marks harnesses as externally modified instead of overwriting them.
                </span>
              </span>
              <Switch
                checked={draft.watchConfigFiles}
                onCheckedChange={(checked) => patch({ watchConfigFiles: checked })}
                aria-label="Watch configuration files"
              />
            </label>
            <label className="flex items-center justify-between gap-3 text-sm">
              <span>
                Detect project-level configs
                <span className="block text-xs text-muted-foreground">
                  Project files are only modified after you select the project explicitly.
                </span>
              </span>
              <Switch
                checked={draft.detectProjectConfigs}
                onCheckedChange={(checked) => patch({ detectProjectConfigs: checked })}
                aria-label="Detect project configs"
              />
            </label>
            <label className="flex items-center justify-between gap-3 text-sm">
              <span>
                Prefer environment references
                <span className="block text-xs text-muted-foreground">
                  Warns when a literal value looks like a credential.
                </span>
              </span>
              <Switch
                checked={draft.preferEnvRefs}
                onCheckedChange={(checked) => patch({ preferEnvRefs: checked })}
                aria-label="Prefer environment references"
              />
            </label>
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>Registry import / export</CardTitle>
            <CardDescription>Exports omit secret values by default and are safe to share.</CardDescription>
          </CardHeader>
          <CardContent className="space-y-3">
            <div className="flex flex-wrap gap-2">
              <Button variant="outline" onClick={() => void exportRegistry()}>
                <Download />
                Export registry
              </Button>
              <Button variant="outline" onClick={() => void importRegistry()}>
                <Upload />
                Import registry (merge)
              </Button>
            </div>
            <p className="text-xs text-muted-foreground">
              Import never deletes existing definitions. Secret values are never exported — entries that
              reference secrets keep their reference metadata and must be re-entered locally.
            </p>
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>Danger zone</CardTitle>
            <CardDescription>Reset restores defaults for paths, sync behaviour and detection options.</CardDescription>
          </CardHeader>
          <CardContent>
            <Button variant="destructive" onClick={() => void reset()} loading={resetting}>
              <RotateCcw />
              Reset application settings
            </Button>
          </CardContent>
        </Card>
      </div>

      <Card>
        <CardHeader>
          <div className="flex flex-wrap items-start justify-between gap-2">
            <div>
              <CardTitle>Generic JSON integrations</CardTitle>
              <CardDescription>
                Configure additional harnesses with an explicit schema. Unverified profiles require confirmation
                in the preview before anything is written.
              </CardDescription>
            </div>
            <Button
              variant="outline"
              size="sm"
              onClick={() => patch({ genericProfiles: [...draft.genericProfiles, emptyProfile()] })}
            >
              <Plus />
              Add profile
            </Button>
          </div>
        </CardHeader>
        <CardContent className="space-y-4">
          {draft.genericProfiles.length === 0 ? (
            <p className="text-xs text-muted-foreground">
              No generic profiles configured. A profile maps a registry entry onto any JSON configuration file
              with a configurable MCP object path and field names.
            </p>
          ) : null}
          {draft.genericProfiles.map((profile) => (
            <div key={profile.id} className="space-y-3 rounded-lg border border-border p-3">
              <div className="flex items-center justify-between gap-2">
                <div className="flex items-center gap-2">
                  <Badge variant={profile.verified ? "success" : "warning"}>
                    {profile.verified ? "verified by user" : "unverified — preview confirmation required"}
                  </Badge>
                </div>
                <Button
                  variant="ghost"
                  size="icon"
                  aria-label={`Delete profile ${profile.label}`}
                  onClick={() => setPendingProfileDelete(profile.id)}
                >
                  <Trash2 />
                </Button>
              </div>
              <div className="grid grid-cols-1 gap-3 md:grid-cols-2">
                <Field label="Label">
                  <Input value={profile.label} onChange={(event) => patchProfile(profile.id, { label: event.target.value })} />
                </Field>
                <Field label="Configuration file path" hint="Absolute path. Restricted to files you explicitly name here.">
                  <Input
                    className="font-mono"
                    value={profile.configPath}
                    onChange={(event) => patchProfile(profile.id, { configPath: event.target.value })}
                    placeholder="C:\\Users\\me\\.myagent\\mcp.json"
                  />
                </Field>
                <Field label="MCP object path" hint="Dot path of the object that holds server entries.">
                  <Input
                    className="font-mono"
                    value={profile.mcpPath}
                    onChange={(event) => patchProfile(profile.id, { mcpPath: event.target.value })}
                    placeholder="mcpServers"
                  />
                </Field>
                <Field label="URL field name">
                  <Input
                    className="font-mono"
                    value={profile.transportMapping.urlField}
                    onChange={(event) =>
                      patchProfile(profile.id, {
                        transportMapping: { ...profile.transportMapping, urlField: event.target.value },
                      })
                    }
                  />
                </Field>
                <Field label="Command field name">
                  <Input
                    className="font-mono"
                    value={profile.transportMapping.commandField}
                    onChange={(event) =>
                      patchProfile(profile.id, {
                        transportMapping: { ...profile.transportMapping, commandField: event.target.value },
                      })
                    }
                  />
                </Field>
                <Field label="Args field name">
                  <Input
                    className="font-mono"
                    value={profile.transportMapping.argsField}
                    onChange={(event) =>
                      patchProfile(profile.id, {
                        transportMapping: { ...profile.transportMapping, argsField: event.target.value },
                      })
                    }
                  />
                </Field>
                <Field label="Env field name">
                  <Input
                    className="font-mono"
                    value={profile.transportMapping.envField}
                    onChange={(event) =>
                      patchProfile(profile.id, {
                        transportMapping: { ...profile.transportMapping, envField: event.target.value },
                      })
                    }
                  />
                </Field>
                <Field label="Headers field name">
                  <Input
                    className="font-mono"
                    value={profile.transportMapping.headersField}
                    onChange={(event) =>
                      patchProfile(profile.id, {
                        transportMapping: { ...profile.transportMapping, headersField: event.target.value },
                      })
                    }
                  />
                </Field>
              </div>
              <Field label="Custom fields (one KEY=value per line)" hint="Merged into every entry this profile writes.">
                <Textarea
                  className="font-mono"
                  value={profile.customFields.map((field) => `${field.key}=${field.value}`).join("\n")}
                  onChange={(event) =>
                    patchProfile(profile.id, {
                      customFields: event.target.value
                        .split("\n")
                        .map((line) => line.trim())
                        .filter(Boolean)
                        .map((line) => {
                          const index = line.indexOf("=");
                          return index === -1
                            ? { key: line, value: "" }
                            : { key: line.slice(0, index).trim(), value: line.slice(index + 1).trim() };
                        }),
                    })
                  }
                />
              </Field>
              <label className="flex items-center gap-2 text-xs">
                <input
                  type="checkbox"
                  className="accent-[var(--primary)]"
                  checked={profile.verified}
                  onChange={(event) => patchProfile(profile.id, { verified: event.target.checked })}
                />
                I verified this schema against the target application's documentation or an existing file.
              </label>
            </div>
          ))}
        </CardContent>
      </Card>

      <div className="flex justify-end gap-2">
        <Button variant="outline" onClick={() => setDraft(settings)} disabled={saving}>
          Discard changes
        </Button>
        <Button onClick={() => void save()} loading={saving}>
          <Save />
          Save settings
        </Button>
      </div>

      <ConfirmDialog
        open={pendingProfileDelete !== null}
        onOpenChange={(open) => {
          if (!open) setPendingProfileDelete(null);
        }}
        title="Delete generic profile?"
        description="Existing files written with this profile are not modified."
        confirmLabel="Delete profile"
        destructive
        onConfirm={() => {
          patch({ genericProfiles: draft.genericProfiles.filter((item) => item.id !== pendingProfileDelete) });
          setPendingProfileDelete(null);
        }}
      />
    </div>
  );
}
