import { useEffect, useMemo, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  Download,
  ExternalLink,
  FolderDown,
  RefreshCw,
  Search,
  Settings2,
  Sparkles,
  Store as StoreIcon,
  Trash2,
  Wrench,
} from "lucide-react";
import {
  Badge,
  Button,
  Card,
  CardContent,
  CardHeader,
  CardTitle,
  EmptyState,
  Input,
  Select,
  Spinner,
} from "@/components/ui/primitives";
import { ConfirmDialog } from "@/components/ui/dialog";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/data-display";
import { useToast } from "@/components/ui/toast";
import { ServerFormDialog } from "@/components/servers/ServerFormDialog";
import { StoreSettingsDialog } from "@/components/store/StoreSettingsDialog";
import { InstallSkillDialog } from "@/components/store/InstallSkillDialog";
import { api } from "@/services/api";
import { describeError, type RegistryState } from "@/hooks/useRegistry";
import { formatDateTime, relativeTime } from "@/lib/utils";
import type {
  InstalledSkill,
  McpServerInput,
  SkillCatalogPage,
  SkillEntry,
  StoreCatalogPage,
  StoreEntry,
  StoreSettings,
} from "@/types";

function entryKind(entry: StoreEntry): string {
  if (entry.packages.length > 0) {
    return entry.packages[0].registryType || "package";
  }
  if (entry.remotes.length > 0) {
    return entry.remotes[0].transportType || "remote";
  }
  return "manual";
}

function skillFolderLabel(count: number): string {
  return `${count} file${count === 1 ? "" : "s"}`;
}

export function Store({ registry }: { registry: RegistryState }) {
  const { toast } = useToast();
  const { harnesses } = registry;

  const [tab, setTab] = useState("servers");

  // --- MCP servers ---------------------------------------------------------
  const [catalog, setCatalog] = useState<StoreCatalogPage | null>(null);
  const [query, setQuery] = useState("");
  const [loadingCatalog, setLoadingCatalog] = useState(true);
  const [refreshingCatalog, setRefreshingCatalog] = useState(false);
  const [prefill, setPrefill] = useState<McpServerInput | null>(null);
  const [formOpen, setFormOpen] = useState(false);
  const [formKey, setFormKey] = useState(0);
  const [busyEntry, setBusyEntry] = useState<string | null>(null);

  // --- skills --------------------------------------------------------------
  const [storeSettings, setStoreSettings] = useState<StoreSettings | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [skillSource, setSkillSource] = useState("all");
  const [skillQuery, setSkillQuery] = useState("");
  const [skills, setSkills] = useState<SkillCatalogPage | null>(null);
  const [loadingSkills, setLoadingSkills] = useState(false);
  const [refreshingSkills, setRefreshingSkills] = useState(false);
  const [installingSkill, setInstallingSkill] = useState<SkillEntry | null>(null);
  const [installed, setInstalled] = useState<InstalledSkill[]>([]);
  const [pendingUninstall, setPendingUninstall] = useState<InstalledSkill | null>(null);
  const [uninstalling, setUninstalling] = useState(false);

  const detectedTargets = useMemo(() => {
    const detected = harnesses.filter((harness) => harness.detected).map((harness) => harness.id);
    return detected.length > 0 ? detected : harnesses.map((harness) => harness.id);
  }, [harnesses]);

  useEffect(() => {
    void (async () => {
      try {
        const [settings, installedSkills] = await Promise.all([
          api.getStoreSettings(),
          api.listInstalledSkills(),
        ]);
        setStoreSettings(settings);
        setInstalled(installedSkills);
      } catch (err) {
        toast({
          title: "Could not load store settings",
          description: describeError(err),
          variant: "error",
        });
      }
    })();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    let cancelled = false;
    setLoadingCatalog(true);
    const handle = window.setTimeout(() => {
      void (async () => {
        try {
          const page = await api.listStoreCatalog(query.trim() || undefined);
          if (!cancelled) setCatalog(page);
        } catch (err) {
          if (!cancelled) {
            toast({
              title: "Could not read the store cache",
              description: describeError(err),
              variant: "error",
            });
          }
        } finally {
          if (!cancelled) setLoadingCatalog(false);
        }
      })();
    }, 200);
    return () => {
      cancelled = true;
      window.clearTimeout(handle);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [query]);

  useEffect(() => {
    if (tab !== "skills") return;
    let cancelled = false;
    setLoadingSkills(true);
    const handle = window.setTimeout(() => {
      void (async () => {
        try {
          const page = await api.listSkillCatalog(
            skillSource === "all" ? undefined : skillSource,
            skillQuery.trim() || undefined,
          );
          if (!cancelled) setSkills(page);
        } catch (err) {
          if (!cancelled) {
            toast({
              title: "Could not read cached skills",
              description: describeError(err),
              variant: "error",
            });
          }
        } finally {
          if (!cancelled) setLoadingSkills(false);
        }
      })();
    }, 200);
    return () => {
      cancelled = true;
      window.clearTimeout(handle);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tab, skillSource, skillQuery]);

  const withTargets = (input: McpServerInput): McpServerInput => ({
    ...input,
    targets: input.targets.length > 0 ? input.targets : detectedTargets,
  });

  const refreshCatalog = async (restart: boolean) => {
    setRefreshingCatalog(true);
    try {
      const result = await api.refreshStoreCatalog(restart ? 2 : 1, restart);
      const page = await api.listStoreCatalog(query.trim() || undefined);
      setCatalog(page);
      toast({
        title: restart ? "Catalog refreshed" : "More servers loaded",
        description: `${result.added} server${result.added === 1 ? "" : "s"} cached (${result.total} total)${
          result.warnings.length > 0 ? ` · ${result.warnings.length} warning(s)` : ""
        }`,
        variant: result.warnings.length > 0 ? "warning" : "success",
      });
    } catch (err) {
      toast({
        title: "Could not reach the MCP registry",
        description: describeError(err),
        variant: "error",
      });
    } finally {
      setRefreshingCatalog(false);
    }
  };

  const reviewEntry = async (entry: StoreEntry) => {
    setBusyEntry(entry.id);
    try {
      const input = withTargets(await api.storePrefillServer(entry.id));
      setPrefill(input);
      setFormKey((value) => value + 1);
      setFormOpen(true);
    } catch (err) {
      toast({
        title: "Could not prepare this server",
        description: describeError(err),
        variant: "error",
      });
    } finally {
      setBusyEntry(null);
    }
  };

  const addEntry = async (entry: StoreEntry) => {
    setBusyEntry(entry.id);
    try {
      const input = withTargets(await api.storePrefillServer(entry.id));
      if (Object.keys(input.env).length > 0) {
        // Environment variables (often credentials) need a review step first.
        setPrefill(input);
        setFormKey((value) => value + 1);
        setFormOpen(true);
        return;
      }
      await api.createMcpServer(input);
      await registry.refresh();
      toast({
        title: `Added ${input.name}`,
        description:
          "It is in your registry. Run Preview sync to write it into the selected harnesses.",
        variant: "success",
      });
    } catch (err) {
      try {
        const input = withTargets(await api.storePrefillServer(entry.id));
        setPrefill(input);
        setFormKey((value) => value + 1);
        setFormOpen(true);
        toast({
          title: "Review the definition",
          description: describeError(err),
          variant: "warning",
        });
      } catch {
        toast({
          title: "Could not add this server",
          description: describeError(err),
          variant: "error",
        });
      }
    } finally {
      setBusyEntry(null);
    }
  };

  const refreshSkillCatalog = async () => {
    setRefreshingSkills(true);
    try {
      const sourceId = skillSource === "all" ? undefined : skillSource;
      const result = await api.refreshSkills(sourceId);
      const page = await api.listSkillCatalog(sourceId, skillQuery.trim() || undefined);
      setSkills(page);
      toast({
        title: `Loaded ${result.skills} skill${result.skills === 1 ? "" : "s"}`,
        description:
          result.warnings.length > 0
            ? result.warnings[0]
            : `From ${result.sources} GitHub source${result.sources === 1 ? "" : "s"}.`,
        variant: result.warnings.length > 0 ? "warning" : "success",
      });
    } catch (err) {
      toast({
        title: "Could not load skills",
        description: describeError(err),
        variant: "error",
      });
    } finally {
      setRefreshingSkills(false);
    }
  };

  const installSkill = async (targetIds: string[], overwrite: boolean) => {
    if (!installingSkill) return;
    try {
      const result = await api.installSkill({
        skillId: installingSkill.id,
        targetIds,
        overwrite,
      });
      setInstallingSkill(null);
      setInstalled(await api.listInstalledSkills());
      toast({
        title: `Installed ${result.name}`,
        description:
          result.deployedPaths.length > 0
            ? `Copied to ${result.deployedPaths.length} harness folder${
                result.deployedPaths.length === 1 ? "" : "s"
              } and kept in the app library.`
            : "Kept in the app library only.",
        variant: "success",
      });
      if (result.skipped.length > 0) {
        toast({
          title: "Some targets were skipped",
          description: result.skipped.join(" · "),
          variant: "warning",
        });
      }
    } catch (err) {
      toast({
        title: "Could not install the skill",
        description: describeError(err),
        variant: "error",
      });
      throw err;
    }
  };

  const uninstallSkill = async () => {
    if (!pendingUninstall) return;
    setUninstalling(true);
    try {
      await api.uninstallSkill(pendingUninstall.id, true);
      setInstalled(await api.listInstalledSkills());
      toast({ title: `Removed ${pendingUninstall.name}`, variant: "success" });
      setPendingUninstall(null);
    } catch (err) {
      toast({
        title: "Could not uninstall the skill",
        description: describeError(err),
        variant: "error",
      });
    } finally {
      setUninstalling(false);
    }
  };

  const installedIds = useMemo(() => new Set(installed.map((skill) => skill.id)), [installed]);

  return (
    <div className="space-y-4">
      <div className="flex flex-wrap items-start justify-between gap-3 rounded-lg border border-border bg-card/60 px-4 py-3">
        <div className="flex min-w-0 gap-3">
          <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-primary/15 text-primary">
            <StoreIcon className="h-4 w-4" />
          </div>
          <div className="min-w-0">
            <p className="text-sm font-medium">One-click installs, still local-first</p>
            <p className="text-xs text-muted-foreground">
              Servers come from the official MCP registry. Skills come from GitHub-hosted skill
              repositories. Everything lands in your local registry first — harness files are only
              touched by a preview + sync.
            </p>
          </div>
        </div>
        <Button variant="outline" size="sm" onClick={() => setSettingsOpen(true)}>
          <Settings2 />
          Store settings
        </Button>
      </div>

      <Tabs value={tab} onValueChange={setTab}>
        <TabsList>
          <TabsTrigger value="servers">MCP Servers</TabsTrigger>
          <TabsTrigger value="skills">Agent Skills</TabsTrigger>
        </TabsList>

        <TabsContent value="servers" className="space-y-4">
          <div className="flex flex-wrap items-center gap-2">
            <div className="relative min-w-56 flex-1">
              <Search className="pointer-events-none absolute left-2.5 top-2.5 h-4 w-4 text-muted-foreground" />
              <Input
                className="pl-8"
                placeholder="Search the cached registry (name, title, description)."
                value={query}
                onChange={(event) => setQuery(event.target.value)}
                aria-label="Search the MCP store"
              />
            </div>
            <Button
              variant="outline"
              size="sm"
              onClick={() => void refreshCatalog(false)}
              disabled={refreshingCatalog || !catalog?.hasMore}
              title={catalog?.hasMore ? "Fetch the next registry page" : "All registry pages are cached"}
            >
              <Download />
              Load more
            </Button>
            <Button
              size="sm"
              onClick={() => void refreshCatalog(true)}
              loading={refreshingCatalog}
            >
              <RefreshCw />
              Refresh catalog
            </Button>
          </div>

          <p className="text-[11px] text-muted-foreground">
            {catalog
              ? `${catalog.total} server${catalog.total === 1 ? "" : "s"} cached${
                  catalog.lastRefreshedAt
                    ? ` · refreshed ${relativeTime(catalog.lastRefreshedAt)}`
                    : ""
                } · source: registry.modelcontextprotocol.io`
              : "Reading the local cache…"}
          </p>

          {loadingCatalog && !catalog ? (
            <div className="flex items-center gap-2 p-6 text-sm text-muted-foreground">
              <Spinner />
              Reading the local store cache…
            </div>
          ) : !catalog || catalog.entries.length === 0 ? (
            <EmptyState
              icon={<StoreIcon className="h-6 w-6" />}
              title={catalog && catalog.total > 0 ? "No servers match that search" : "The store cache is empty"}
              description={
                catalog && catalog.total > 0
                  ? "Clear the search to see every cached registry entry."
                  : "Refresh to fetch the official MCP registry. This is the only time the store contacts the network."
              }
              action={
                catalog && catalog.total > 0 ? (
                  <Button variant="outline" size="sm" onClick={() => setQuery("")}>
                    Clear search
                  </Button>
                ) : (
                  <Button size="sm" onClick={() => void refreshCatalog(true)} loading={refreshingCatalog}>
                    <RefreshCw />
                    Refresh catalog
                  </Button>
                )
              }
            />
          ) : (
            <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
              {catalog.entries.map((entry) => {
                const sourceUrl = entry.repository ?? entry.homepage;
                const busy = busyEntry === entry.id;
                return (
                  <Card key={entry.id} className="flex flex-col">
                    <CardHeader className="pb-1">
                      <div className="flex items-start justify-between gap-2">
                        <div className="min-w-0">
                          <CardTitle className="truncate text-sm" title={entry.name}>
                            {entry.title || entry.name}
                          </CardTitle>
                          <p className="truncate font-mono text-[11px] text-muted-foreground">
                            {entry.name}
                            {entry.version ? ` · v${entry.version}` : ""}
                          </p>
                        </div>
                        <div className="flex shrink-0 flex-wrap justify-end gap-1">
                          <Badge variant="secondary">{entryKind(entry)}</Badge>
                          {entry.packages.some((pkg) => pkg.environmentVariables.some((v) => v.isSecret)) ? (
                            <Badge variant="warning">secrets</Badge>
                          ) : null}
                        </div>
                      </div>
                    </CardHeader>
                    <CardContent className="flex flex-1 flex-col gap-3 pt-2">
                      <p className="line-clamp-3 text-xs text-muted-foreground">
                        {entry.description || "No description provided."}
                      </p>
                      <div className="mt-auto flex flex-wrap items-center gap-2">
                        <Button size="sm" onClick={() => void addEntry(entry)} loading={busy}>
                          <Download />
                          Add
                        </Button>
                        <Button
                          size="sm"
                          variant="outline"
                          onClick={() => void reviewEntry(entry)}
                          disabled={busy}
                        >
                          <Wrench />
                          Review
                        </Button>
                        {sourceUrl ? (
                          <Button
                            size="sm"
                            variant="ghost"
                            onClick={() => void openUrl(sourceUrl).catch(() => undefined)}
                            title={sourceUrl}
                          >
                            <ExternalLink />
                            Source
                          </Button>
                        ) : null}
                      </div>
                    </CardContent>
                  </Card>
                );
              })}
            </div>
          )}
        </TabsContent>

        <TabsContent value="skills" className="space-y-4">
          <div className="flex flex-wrap items-center gap-2">
            <div className="relative min-w-56 flex-1">
              <Search className="pointer-events-none absolute left-2.5 top-2.5 h-4 w-4 text-muted-foreground" />
              <Input
                className="pl-8"
                placeholder="Search cached skills (name or description)."
                value={skillQuery}
                onChange={(event) => setSkillQuery(event.target.value)}
                aria-label="Search skills"
              />
            </div>
            <Select
              aria-label="Filter skills by source"
              className="w-48"
              value={skillSource}
              onChange={(event) => setSkillSource(event.target.value)}
            >
              <option value="all">All sources</option>
              {(storeSettings?.skillSources ?? []).map((source) => (
                <option key={source.id} value={source.id}>
                  {source.label}
                </option>
              ))}
            </Select>
            <Button
              size="sm"
              onClick={() => void refreshSkillCatalog()}
              loading={refreshingSkills}
            >
              <RefreshCw />
              Refresh skills
            </Button>
          </div>

          <p className="text-[11px] text-muted-foreground">
            {skills
              ? `${skills.total} skill${skills.total === 1 ? "" : "s"} cached${
                  skills.lastRefreshedAt ? ` · refreshed ${relativeTime(skills.lastRefreshedAt)}` : ""
                } · sources: api.github.com / raw.githubusercontent.com`
              : "Reading the local cache…"}
          </p>

          {loadingSkills && !skills ? (
            <div className="flex items-center gap-2 p-6 text-sm text-muted-foreground">
              <Spinner />
              Reading cached skills…
            </div>
          ) : !skills || skills.entries.length === 0 ? (
            <EmptyState
              icon={<Sparkles className="h-6 w-6" />}
              title={skills && skills.total > 0 ? "No skills match that search" : "No skills cached yet"}
              description={
                skills && skills.total > 0
                  ? "Clear the search or switch the source filter."
                  : "Refresh to read the skills folders of the configured GitHub sources. You can add sources in Store settings."
              }
              action={
                skills && skills.total > 0 ? (
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => {
                      setSkillQuery("");
                      setSkillSource("all");
                    }}
                  >
                    Clear filters
                  </Button>
                ) : (
                  <Button size="sm" onClick={() => void refreshSkillCatalog()} loading={refreshingSkills}>
                    <RefreshCw />
                    Refresh skills
                  </Button>
                )
              }
            />
          ) : (
            <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
              {skills.entries.map((skill) => (
                <Card key={skill.id} className="flex flex-col">
                  <CardHeader className="pb-1">
                    <div className="flex items-start justify-between gap-2">
                      <div className="min-w-0">
                        <CardTitle className="truncate text-sm" title={skill.name}>
                          {skill.name}
                        </CardTitle>
                        <p className="truncate font-mono text-[11px] text-muted-foreground">
                          {skill.path}
                        </p>
                      </div>
                      <div className="flex shrink-0 flex-wrap justify-end gap-1">
                        <Badge variant="secondary">{skill.sourceLabel}</Badge>
                        {installedIds.has(skill.id) ? <Badge variant="success">installed</Badge> : null}
                      </div>
                    </div>
                  </CardHeader>
                  <CardContent className="flex flex-1 flex-col gap-3 pt-2">
                    <p className="line-clamp-3 text-xs text-muted-foreground">
                      {skill.description || "No description in SKILL.md frontmatter."}
                    </p>
                    <div className="mt-auto flex flex-wrap items-center gap-2">
                      <Button size="sm" onClick={() => setInstallingSkill(skill)}>
                        <Download />
                        Install
                      </Button>
                      <span className="text-[11px] text-muted-foreground">
                        {skillFolderLabel(skill.files.length)}
                      </span>
                    </div>
                  </CardContent>
                </Card>
              ))}
            </div>
          )}

          {installed.length > 0 ? (
            <Card>
              <CardHeader className="pb-2">
                <CardTitle className="flex items-center gap-2 text-sm">
                  <FolderDown className="h-4 w-4" />
                  Installed skills ({installed.length})
                </CardTitle>
              </CardHeader>
              <CardContent className="space-y-2">
                {installed.map((skill) => (
                  <div
                    key={skill.id}
                    className="flex flex-wrap items-center gap-3 rounded-md border border-border px-3 py-2"
                  >
                    <div className="min-w-0 flex-1">
                      <p className="truncate text-sm font-medium">{skill.name}</p>
                      <p className="truncate text-[11px] text-muted-foreground">
                        {skill.deployedPaths.length > 0
                          ? `Deployed to ${skill.deployedPaths.length} harness folder${
                              skill.deployedPaths.length === 1 ? "" : "s"
                            }`
                          : "App library only"}{" "}
                        · installed {formatDateTime(skill.installedAt)}
                      </p>
                    </div>
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={() => setPendingUninstall(skill)}
                    >
                      <Trash2 />
                      Uninstall
                    </Button>
                  </div>
                ))}
              </CardContent>
            </Card>
          ) : null}
        </TabsContent>
      </Tabs>

      <ServerFormDialog
        key={formKey}
        open={formOpen}
        onOpenChange={setFormOpen}
        server={null}
        harnesses={harnesses}
        initialInput={prefill}
        onSaved={registry.refresh}
      />

      <StoreSettingsDialog
        open={settingsOpen}
        settings={storeSettings}
        onClose={() => setSettingsOpen(false)}
        onSaved={(settings) => {
          setStoreSettings(settings);
          setSkillSource("all");
        }}
      />

      <InstallSkillDialog
        skill={installingSkill}
        settings={storeSettings}
        onClose={() => setInstallingSkill(null)}
        onInstall={installSkill}
      />

      <ConfirmDialog
        open={pendingUninstall !== null}
        onOpenChange={(open) => {
          if (!open) setPendingUninstall(null);
        }}
        title={`Uninstall ${pendingUninstall?.name ?? "skill"}?`}
        description="The app library copy and any deployed harness copies recorded at install time are deleted. This cannot be undone."
        confirmLabel="Uninstall"
        destructive
        busy={uninstalling}
        onConfirm={() => void uninstallSkill()}
      />
    </div>
  );
}
