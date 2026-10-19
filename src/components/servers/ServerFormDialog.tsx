import { useEffect, useMemo, useState } from "react";
import { FlaskConical, Info, KeyRound, Plus, Trash2 } from "lucide-react";
import {
  Badge,
  Button,
  Field,
  Input,
  Label,
  Select,
  Switch,
  Textarea,
} from "@/components/ui/primitives";
import { Dialog, DialogContent } from "@/components/ui/dialog";
import { JsonPreview, Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/data-display";
import { useToast } from "@/components/ui/toast";
import { api } from "@/services/api";
import { describeError } from "@/hooks/useRegistry";
import { isSensitiveKey, maskValue } from "@/lib/mask";
import type {
  DefinitionPreview,
  HarnessInfo,
  McpServer,
  McpServerInput,
  ValidationIssue,
  ValidationReport,
  VarKind,
  VarValue,
} from "@/types";

interface VarRow {
  key: string;
  value: string;
  kind: VarKind;
}

function rowsFromRecord(record: Record<string, VarValue>): VarRow[] {
  return Object.entries(record).map(([key, value]) => ({ key, value: value.value, kind: value.kind }));
}

function recordFromRows(rows: VarRow[]): Record<string, VarValue> {
  const out: Record<string, VarValue> = {};
  for (const row of rows) {
    const key = row.key.trim();
    if (!key) continue;
    out[key] = { value: row.value, kind: row.kind };
  }
  return out;
}

function emptyInput(harnesses: HarnessInfo[]): McpServerInput {
  return {
    name: "",
    description: "",
    transport: "stdio",
    command: "",
    args: [],
    env: {},
    headers: {},
    url: null,
    cwd: null,
    enabled: true,
    targets: harnesses.filter((item) => item.detected).map((item) => item.id),
    tags: [],
    notes: "",
  };
}

function toInput(server: McpServer): McpServerInput {
  return {
    id: server.id,
    name: server.name,
    description: server.description,
    transport: server.transport,
    command: server.command,
    args: server.args,
    env: server.env,
    headers: server.headers,
    url: server.url,
    cwd: server.cwd,
    enabled: server.enabled,
    targets: server.targets,
    tags: server.tags,
    notes: server.notes,
  };
}

function VarEditor({
  title,
  description,
  rows,
  onChange,
  remote,
}: {
  title: string;
  description: string;
  rows: VarRow[];
  onChange: (rows: VarRow[]) => void;
  remote: boolean;
}) {
  const update = (index: number, patch: Partial<VarRow>) => {
    onChange(rows.map((row, i) => (i === index ? { ...row, ...patch } : row)));
  };
  return (
    <div className="space-y-2">
      <div className="flex items-center justify-between">
        <div>
          <Label>{title}</Label>
          <p className="text-xs text-muted-foreground">{description}</p>
        </div>
        <Button
          type="button"
          variant="outline"
          size="sm"
          onClick={() => onChange([...rows, { key: "", value: "", kind: "literal" }])}
        >
          <Plus />
          Add
        </Button>
      </div>
      {rows.length === 0 ? (
        <p className="rounded-md border border-dashed border-border p-2 text-xs text-muted-foreground">
          No {remote ? "headers" : "environment variables"} defined.
        </p>
      ) : null}
      <div className="space-y-2">
        {rows.map((row, index) => (
          <div key={index} className="flex flex-wrap items-start gap-2">
            <Input
              aria-label={`${title} key ${index + 1}`}
              className="h-8 w-40 font-mono text-xs"
              placeholder="KEY_NAME"
              value={row.key}
              onChange={(event) => update(index, { key: event.target.value })}
            />
            <Input
              aria-label={`${title} value ${index + 1}`}
              className="h-8 min-w-40 flex-1 font-mono text-xs"
              placeholder={
                row.kind === "envRef"
                  ? "{env:MY_VARIABLE}"
                  : row.kind === "secret"
                    ? "secret value (stored in OS credential store)"
                    : "value"
              }
              type={row.kind === "secret" ? "password" : "text"}
              value={row.value}
              onChange={(event) => update(index, { value: event.target.value })}
            />
            <Select
              aria-label={`${title} kind ${index + 1}`}
              className="h-8 w-32 text-xs"
              value={row.kind}
              onChange={(event) => update(index, { kind: event.target.value as VarKind })}
            >
              <option value="literal">Literal</option>
              <option value="envRef">Env reference</option>
              <option value="secret">Secret</option>
            </Select>
            <Button
              type="button"
              variant="ghost"
              size="icon"
              aria-label={`Remove ${title} row ${index + 1}`}
              onClick={() => onChange(rows.filter((_, i) => i !== index))}
            >
              <Trash2 />
            </Button>
            {row.kind === "secret" && row.value ? (
              <Badge variant="secondary" className="self-center">
                <KeyRound className="h-3 w-3" /> {maskValue(row.value)}
              </Badge>
            ) : null}
          </div>
        ))}
      </div>
    </div>
  );
}

export function ServerFormDialog({
  open,
  onOpenChange,
  server,
  harnesses,
  onSaved,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  server: McpServer | null;
  harnesses: HarnessInfo[];
  onSaved: () => Promise<void> | void;
}) {
  const { toast } = useToast();
  const [input, setInput] = useState<McpServerInput>(() => (server ? toInput(server) : emptyInput(harnesses)));
  const [envRows, setEnvRows] = useState<VarRow[]>([]);
  const [headerRows, setHeaderRows] = useState<VarRow[]>([]);
  const [argsText, setArgsText] = useState("");
  const [tagsText, setTagsText] = useState("");
  const [saving, setSaving] = useState(false);
  const [testing, setTesting] = useState(false);
  const [validation, setValidation] = useState<ValidationReport | null>(null);
  const [testResult, setTestResult] = useState<string | null>(null);
  const [definitionPreview, setDefinitionPreview] = useState<DefinitionPreview | null>(null);
  const [previewHarness, setPreviewHarness] = useState(harnesses[0]?.id ?? "opencode");

  useEffect(() => {
    if (!open) return;
    const next = server ? toInput(server) : emptyInput(harnesses);
    setInput(next);
    setEnvRows(rowsFromRecord(next.env));
    setHeaderRows(rowsFromRecord(next.headers));
    setArgsText(next.args.join("\n"));
    setTagsText(next.tags.join(", "));
    setValidation(null);
    setTestResult(null);
    setDefinitionPreview(null);
  }, [open, server, harnesses]);

  const current: McpServerInput = useMemo(
    () => ({
      ...input,
      args: argsText
        .split("\n")
        .map((line) => line.trim())
        .filter(Boolean),
      tags: tagsText
        .split(",")
        .map((tag) => tag.trim())
        .filter(Boolean),
      env: recordFromRows(envRows),
      headers: recordFromRows(headerRows),
    }),
    [input, argsText, tagsText, envRows, headerRows],
  );

  const localErrors = useMemo(() => {
    const errors: Record<string, string> = {};
    if (!current.name.trim()) errors.name = "Name is required.";
    else if (!/^[A-Za-z0-9._-]+$/.test(current.name.trim()))
      errors.name = "Use letters, digits, dot, dash or underscore (harness keys must be simple).";
    if (current.transport === "stdio") {
      if (!current.command?.trim()) errors.command = "Command is required for stdio servers.";
    } else if (!current.url?.trim()) {
      errors.url = "URL is required for remote servers.";
    } else if (!/^https?:\/\//i.test(current.url.trim())) {
      errors.url = "URL must start with http:// or https://";
    }
    const seen = new Set<string>();
    for (const row of envRows) {
      if (!row.key.trim()) continue;
      if (seen.has(row.key.trim())) errors.env = `Duplicate environment key: ${row.key}`;
      seen.add(row.key.trim());
    }
    return errors;
  }, [current, envRows]);

  const save = async () => {
    if (Object.keys(localErrors).length > 0) {
      toast({ title: "Fix validation errors first", variant: "warning" });
      return;
    }
    setSaving(true);
    try {
      const report = await api.validateMcpConfig(current);
      setValidation(report);
      if (!report.valid) {
        toast({
          title: "Definition rejected by validation",
          description: report.issues.find((issue) => issue.severity === "error")?.message,
          variant: "error",
        });
        return;
      }
      if (server) {
        await api.updateMcpServer(server.id, current);
        toast({ title: `Updated ${current.name}`, variant: "success" });
      } else {
        await api.createMcpServer(current);
        toast({ title: `Added ${current.name}`, variant: "success" });
      }
      await onSaved();
      onOpenChange(false);
    } catch (err) {
      toast({ title: "Could not save server", description: describeError(err), variant: "error" });
    } finally {
      setSaving(false);
    }
  };

  const testConfiguration = async () => {
    setTesting(true);
    try {
      const report = await api.validateMcpConfig(current);
      setValidation(report);
      setTestResult(
        report.valid
          ? "JSON/schema validation passed. This does not prove the server can start or connect."
          : "Validation failed; fix the reported errors.",
      );
    } catch (err) {
      setTestResult(describeError(err));
    } finally {
      setTesting(false);
    }
  };

  const testConnection = async () => {
    setTesting(true);
    try {
      const result = await api.testMcpConnection(current);
      setTestResult(`${result.message} ${result.disclaimer}`);
      toast({
        title: result.success ? "Probe finished" : "Probe failed",
        description: result.message,
        variant: result.success ? "success" : "error",
      });
    } catch (err) {
      setTestResult(describeError(err));
    } finally {
      setTesting(false);
    }
  };

  const loadDefinitionPreview = async () => {
    try {
      const result = await api.previewDefinition(current, previewHarness);
      setDefinitionPreview(result);
    } catch (err) {
      toast({ title: "Preview failed", description: describeError(err), variant: "error" });
    }
  };

  const issues: ValidationIssue[] = validation?.issues ?? [];

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        title={server ? `Edit ${server.name}` : "Add MCP server"}
        description="Definitions live in the local registry; selected harnesses receive a translated entry on synchronize."
        size="xl"
        footer={
          <>
            <Button variant="outline" onClick={testConfiguration} loading={testing}>
              <FlaskConical />
              Validate
            </Button>
            <Button variant="ghost" onClick={testConnection} disabled={testing}>
              Launch probe
            </Button>
            <Button variant="outline" onClick={() => onOpenChange(false)} disabled={saving}>
              Cancel
            </Button>
            <Button onClick={() => void save()} loading={saving}>
              {server ? "Save changes" : "Add server"}
            </Button>
          </>
        }
      >
        <Tabs defaultValue="basics">
          <TabsList className="mb-4">
            <TabsTrigger value="basics">Basics</TabsTrigger>
            <TabsTrigger value="transport">Transport</TabsTrigger>
            <TabsTrigger value="targets">Harnesses &amp; tags</TabsTrigger>
            <TabsTrigger value="json">JSON preview</TabsTrigger>
          </TabsList>

          <TabsContent value="basics" className="space-y-4">
            <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
              <Field label="Server name" required error={localErrors.name} hint="Unique key used in every harness config.">
                <Input
                  value={input.name}
                  onChange={(event) => setInput({ ...input, name: event.target.value })}
                  placeholder="filesystem"
                  className="font-mono"
                />
              </Field>
              <Field label="Transport" required>
                <Select
                  value={input.transport}
                  onChange={(event) =>
                    setInput({ ...input, transport: event.target.value as McpServerInput["transport"] })
                  }
                >
                  <option value="stdio">stdio (local process)</option>
                  <option value="streamableHttp">Streamable HTTP (remote)</option>
                  <option value="sse">SSE (legacy remote)</option>
                </Select>
              </Field>
            </div>
            <Field label="Description" hint="Shown in the registry only; never written to harness configs.">
              <Input
                value={input.description}
                onChange={(event) => setInput({ ...input, description: event.target.value })}
                placeholder="Reads and writes files in a sandboxed directory"
              />
            </Field>
            <Field label="Enabled">
              <div className="flex items-center gap-2">
                <Switch
                  checked={input.enabled}
                  onCheckedChange={(checked) => setInput({ ...input, enabled: checked })}
                  aria-label="Server enabled"
                />
                <span className="text-xs text-muted-foreground">
                  Disabled servers are still synchronized, using the harness' disabled flag.
                </span>
              </div>
            </Field>
            <Field label="Notes">
              <Textarea
                value={input.notes}
                onChange={(event) => setInput({ ...input, notes: event.target.value })}
                placeholder="Internal notes, never written to harness configs."
              />
            </Field>
          </TabsContent>

          <TabsContent value="transport" className="space-y-4">
            {input.transport === "stdio" ? (
              <>
                <Field label="Command" required error={localErrors.command} hint="Executable only; arguments go below.">
                  <Input
                    className="font-mono"
                    value={input.command ?? ""}
                    onChange={(event) => setInput({ ...input, command: event.target.value })}
                    placeholder="npx"
                  />
                </Field>
                <Field label="Arguments" hint="One argument per line. No shell interpolation is performed.">
                  <Textarea
                    className="font-mono"
                    value={argsText}
                    onChange={(event) => setArgsText(event.target.value)}
                    placeholder={"-y\n@modelcontextprotocol/server-filesystem\nD:\\sandbox"}
                  />
                </Field>
                <Field label="Working directory" hint="Optional. Not supported by every harness; a warning is shown when it would be dropped.">
                  <Input
                    className="font-mono"
                    value={input.cwd ?? ""}
                    onChange={(event) => setInput({ ...input, cwd: event.target.value || null })}
                    placeholder="C:\\projects\\sandbox"
                  />
                </Field>
              </>
            ) : (
              <Field label="Remote URL" required error={localErrors.url}>
                <Input
                  className="font-mono"
                  value={input.url ?? ""}
                  onChange={(event) => setInput({ ...input, url: event.target.value })}
                  placeholder="https://mcp.example.com/mcp"
                />
              </Field>
            )}

            <VarEditor
              title="Environment variables"
              description="Value can be a literal, an {env:NAME} reference, or a secret stored in the OS credential store."
              rows={envRows}
              onChange={setEnvRows}
              remote={false}
            />
            {localErrors.env ? <p className="text-xs text-destructive">{localErrors.env}</p> : null}

            <VarEditor
              title="HTTP headers"
              description="Only used by remote transports that support headers."
              rows={headerRows}
              onChange={setHeaderRows}
              remote
            />

            {input.transport !== "stdio" && headerRows.some((row) => isSensitiveKey(row.key)) ? (
              <p className="flex items-start gap-2 rounded-md border border-border bg-muted/40 p-2 text-xs text-muted-foreground">
                <Info className="mt-0.5 h-3.5 w-3.5" />
                Prefer <span className="font-mono">{"{env:NAME}"}</span> references for API keys so the raw secret
                never lands in a harness config file on disk.
              </p>
            ) : null}
          </TabsContent>

          <TabsContent value="targets" className="space-y-4">
            <div className="space-y-2">
              <Label>Target harnesses</Label>
              <p className="text-xs text-muted-foreground">
                Compatibility is re-checked at synchronize time; unsupported transports are skipped with a warning.
              </p>
              <div className="grid grid-cols-1 gap-2 md:grid-cols-2">
                {harnesses.map((harness) => {
                  const supported =
                    (input.transport === "stdio" && harness.capabilities.stdio) ||
                    (input.transport === "streamableHttp" && harness.capabilities.streamableHttp) ||
                    (input.transport === "sse" && harness.capabilities.sse);
                  const checked = input.targets.includes(harness.id);
                  return (
                    <label
                      key={harness.id}
                      className="flex items-start gap-2 rounded-md border border-border p-2 text-sm"
                    >
                      <input
                        type="checkbox"
                        className="mt-0.5 accent-[var(--primary)]"
                        checked={checked}
                        onChange={(event) =>
                          setInput({
                            ...input,
                            targets: event.target.checked
                              ? [...input.targets, harness.id]
                              : input.targets.filter((id) => id !== harness.id),
                          })
                        }
                      />
                      <span className="min-w-0">
                        <span className="flex items-center gap-2">
                          {harness.name}
                          {harness.detected ? (
                            <Badge variant="success">detected</Badge>
                          ) : (
                            <Badge variant="outline">not detected</Badge>
                          )}
                          {!supported ? <Badge variant="warning">transport unsupported</Badge> : null}
                        </span>
                        <span className="block truncate text-[11px] text-muted-foreground">
                          {harness.configPath ?? "No configuration path resolved"}
                        </span>
                      </span>
                    </label>
                  );
                })}
              </div>
            </div>
            <Field label="Tags" hint="Comma separated; registry-only metadata.">
              <Input value={tagsText} onChange={(event) => setTagsText(event.target.value)} placeholder="local, files" />
            </Field>
          </TabsContent>

          <TabsContent value="json" className="space-y-3">
            <div className="flex flex-wrap items-end gap-2">
              <Field label="Harness" className="w-52">
                <Select value={previewHarness} onChange={(event) => setPreviewHarness(event.target.value)}>
                  {harnesses.map((harness) => (
                    <option key={harness.id} value={harness.id}>
                      {harness.name}
                    </option>
                  ))}
                </Select>
              </Field>
              <Button variant="outline" onClick={() => void loadDefinitionPreview()}>
                Generate preview
              </Button>
            </div>
            {definitionPreview ? (
              <>
                {definitionPreview.warnings.map((warning) => (
                  <p key={warning} className="text-xs text-amber-500">
                    {warning}
                  </p>
                ))}
                <JsonPreview value={definitionPreview.entryJson} />
              </>
            ) : (
              <p className="text-xs text-muted-foreground">
                The entry that would be merged into the selected harness config (produced by the Rust adapter).
              </p>
            )}
          </TabsContent>
        </Tabs>

        {issues.length > 0 ? (
          <div className="mt-4 space-y-1 rounded-md border border-border bg-muted/30 p-3">
            <p className="text-xs font-medium">Validation results</p>
            {issues.map((issue, index) => (
              <p
                key={index}
                className={
                  issue.severity === "error"
                    ? "text-xs text-destructive"
                    : issue.severity === "warning"
                      ? "text-xs text-amber-500"
                      : "text-xs text-muted-foreground"
                }
              >
                [{issue.severity}] {issue.field}: {issue.message}
                {issue.remediation ? ` — ${issue.remediation}` : ""}
              </p>
            ))}
          </div>
        ) : null}
        {testResult ? (
          <p className="mt-3 rounded-md border border-border bg-muted/30 p-2 text-xs text-muted-foreground">
            {testResult}
          </p>
        ) : null}
      </DialogContent>
    </Dialog>
  );
}
