import { existsSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";

export type Slot = "1" | "2" | "3";
export type LayoutTarget = { label: string; command: string };
export type Layout = { firstUsesCurrentTab: boolean; tabs: LayoutTarget[] };
export type LayoutMap = Partial<Record<Slot, Layout>>;

export type TabInfo = {
  tab_id: string;
  workspace_id: string;
  label: string;
  focused: boolean;
};

export type PaneInfo = {
  pane_id: string;
  workspace_id: string;
  tab_id: string;
  focused: boolean;
  cwd?: string;
  foreground_cwd?: string;
  agent?: string;
};

export type PaneProcess = { name: string; argv0?: string };
export type PaneProcessInfo = { pane_id: string; foreground_processes: PaneProcess[] };
type PluginContext = {
  workspace_id?: string;
  tab_id?: string;
  focused_pane_id?: string;
  focused_pane_cwd?: string;
  workspace_cwd?: string;
};

export type Snapshot = {
  tabs: TabInfo[];
  panes: PaneInfo[];
  processes: Map<string, PaneProcessInfo>;
  currentPane?: PaneInfo;
  originalTabId?: string;
  workspaceId?: string;
  activeCwd: string;
};

export type Operation =
  | { type: "already-running"; label: string; paneId: string; tabId: string }
  | { type: "run-existing"; target: LayoutTarget; paneId: string; tabId: string }
  | { type: "claim-current"; target: LayoutTarget; paneId: string; tabId: string }
  | { type: "create-tab"; target: LayoutTarget };

type LoadedConfig = {
  globalPath: string;
  overridePath?: string;
  layouts: LayoutMap;
};

const SHELLS = new Set(["pwsh", "powershell", "cmd", "zsh", "bash", "fish", "nu", "sh"]);

export function parseSlot(value: string | undefined): Slot | undefined {
  return value === "1" || value === "2" || value === "3" ? value : undefined;
}

export function validateConfigRoot(raw: unknown, source: string): LayoutMap {
  if (!isRecord(raw)) throw new Error(`${source}: YAML root must be a map`);
  const layouts = raw.layouts;
  if (!isRecord(layouts)) throw new Error(`${source}: layouts must be a map`);

  const out: LayoutMap = {};
  for (const [rawSlot, rawLayout] of Object.entries(layouts)) {
    const slot = parseSlot(rawSlot);
    if (!slot) throw new Error(`${source}: invalid layout slot ${rawSlot}`);
    out[slot] = validateLayout(rawLayout, `${source}: layouts.${rawSlot}`);
  }
  return out;
}

export function selectLayout(globalLayouts: LayoutMap, overrideLayouts: LayoutMap, slot: Slot): Layout {
  const layout = { ...globalLayouts, ...overrideLayouts }[slot];
  if (!layout) throw new Error(`Layout slot ${slot} is missing or invalid`);
  return layout;
}

async function main(argv: string[]): Promise<number> {
  const mode = parseArgs(argv);
  if (!mode) {
    await notifyUser("usage: herdr-layout <1|2|3> | --dry-run <1|2|3> | --check-config <1|2|3>");
    return 2;
  }

  const context = readPluginContext();
  const invokingCwd = context.focused_pane_cwd ?? context.workspace_cwd ?? process.cwd();

  if (mode.kind === "check-config") {
    try {
      const loaded = loadConfig(invokingCwd);
      const layout = selectLayout(loaded.layouts, {}, mode.slot);
      console.log(JSON.stringify({ ok: true, slot: mode.slot, targets: layout.tabs.length, files: configFiles(loaded) }));
      return 0;
    } catch (error) {
      await notifyUser(messageOf(error));
      return 1;
    }
  }

  let loaded: LoadedConfig;
  let layout: Layout;
  try {
    loaded = loadConfig(invokingCwd);
    layout = selectLayout(loaded.layouts, {}, mode.slot);
  } catch (error) {
    await notifyUser(messageOf(error));
    return 1;
  }

  let snapshot: Snapshot;
  let operations: Operation[];
  try {
    snapshot = await readSnapshot(context);
    operations = planLayout(layout, snapshot);
  } catch (error) {
    await notifyUser(messageOf(error));
    return 1;
  }

  if (mode.kind === "dry-run") {
    console.log(JSON.stringify({ ok: true, slot: mode.slot, files: configFiles(loaded), operations }, null, 2));
    return 0;
  }

  try {
    await applyOperations(operations, snapshot);
    if (snapshot.originalTabId) await herdrOk(["tab", "focus", snapshot.originalTabId]);
    return 0;
  } catch (error) {
    await notifyUser(messageOf(error));
    return 1;
  }
}

function parseArgs(argv: string[]): { kind: "apply" | "dry-run" | "check-config"; slot: Slot } | undefined {
  if (argv.length === 1) {
    const slot = parseSlot(argv[0]);
    return slot ? { kind: "apply", slot } : undefined;
  }
  if (argv.length === 2 && argv[0] === "--dry-run") {
    const slot = parseSlot(argv[1]);
    return slot ? { kind: "dry-run", slot } : undefined;
  }
  if (argv.length === 2 && argv[0] === "--check-config") {
    const slot = parseSlot(argv[1]);
    return slot ? { kind: "check-config", slot } : undefined;
  }
  return undefined;
}

function loadConfig(invokingCwd: string): LoadedConfig {
  const configDir = process.env.HERDR_PLUGIN_CONFIG_DIR;
  if (!configDir) throw new Error("HERDR_PLUGIN_CONFIG_DIR is not set");

  const globalPath = join(configDir, "config.yaml");
  if (!existsSync(globalPath)) throw new Error(`${globalPath}: config.yaml missing`);

  const globalLayouts = readConfigFile(globalPath);
  const overridePath = findRepoOverride(invokingCwd);
  const overrideLayouts = overridePath ? readConfigFile(overridePath) : {};
  return { globalPath, overridePath, layouts: { ...globalLayouts, ...overrideLayouts } };
}

function readConfigFile(path: string): LayoutMap {
  try {
    return validateConfigRoot(Bun.YAML.parse(readFileSync(path, "utf8")), path);
  } catch (error) {
    if (error instanceof Error && error.message.startsWith(`${path}:`)) throw error;
    throw new Error(`${path}: ${messageOf(error)}`);
  }
}

function validateLayout(raw: unknown, path: string): Layout {
  if (!isRecord(raw)) throw new Error(`${path} must be a map`);
  const firstUsesCurrentTab = raw.firstUsesCurrentTab;
  if (firstUsesCurrentTab !== undefined && typeof firstUsesCurrentTab !== "boolean") {
    throw new Error(`${path}.firstUsesCurrentTab must be boolean`);
  }
  if (!Array.isArray(raw.tabs) || raw.tabs.length === 0) throw new Error(`${path}.tabs must be a non-empty list`);

  const seen = new Set<string>();
  const tabs = raw.tabs.map((target, index) => validateTarget(target, `${path}.tabs[${index}]`));
  for (const target of tabs) {
    if (seen.has(target.label)) throw new Error(`${path}: duplicate tab label ${target.label}`);
    seen.add(target.label);
  }
  return { firstUsesCurrentTab: firstUsesCurrentTab === true, tabs };
}

function validateTarget(raw: unknown, path: string): LayoutTarget {
  if (!isRecord(raw)) throw new Error(`${path} must be a map`);
  if (typeof raw.label !== "string" || raw.label.trim() === "") throw new Error(`${path}.label must be a non-empty string`);
  if (typeof raw.command !== "string" || raw.command.trim() === "") throw new Error(`${path}.command must be a non-empty string`);
  return { label: raw.label, command: raw.command };
}

function findRepoOverride(start: string): string | undefined {
  let dir = resolve(start);
  for (;;) {
    const yaml = join(dir, ".herdr-layout.yaml");
    const yml = join(dir, ".herdr-layout.yml");
    const hasYaml = existsSync(yaml);
    const hasYml = existsSync(yml);
    if (hasYaml && hasYml) throw new Error(`${dir}: both .herdr-layout.yaml and .herdr-layout.yml exist`);
    if (hasYaml) return yaml;
    if (hasYml) return yml;
    const parent = dirname(dir);
    if (parent === dir) return undefined;
    dir = parent;
  }
}

async function readSnapshot(context: PluginContext): Promise<Snapshot> {
  const workspaceArg = context.workspace_id ? ["--workspace", context.workspace_id] : [];
  const [tabs, panes] = await Promise.all([
    herdrJson(["tab", "list", ...workspaceArg]).then(parseTabList),
    herdrJson(["pane", "list", ...workspaceArg]).then(parsePaneList),
  ]);
  if (tabs.length === 0) throw new Error("No Herdr tabs found");
  if (panes.length === 0) throw new Error("No Herdr panes found");

  const processPairs = await Promise.all(
    panes.map(async (pane) => [pane.pane_id, parseProcessInfo(await herdrJson(["pane", "process-info", "--pane", pane.pane_id]))] as const),
  );
  const processes = new Map(processPairs);
  const currentPane = panes.find((pane) => pane.pane_id === context.focused_pane_id) ?? panes.find((pane) => pane.focused);
  const originalTabId = context.tab_id ?? tabs.find((tab) => tab.focused)?.tab_id ?? currentPane?.tab_id;
  const activeCwd = context.focused_pane_cwd ?? currentPane?.foreground_cwd ?? currentPane?.cwd ?? process.cwd();
  const workspaceId = context.workspace_id ?? currentPane?.workspace_id ?? tabs.find((tab) => tab.tab_id === originalTabId)?.workspace_id;

  return { tabs, panes, processes, currentPane, originalTabId, workspaceId, activeCwd };
}

export function planLayout(layout: Layout, snapshot: Snapshot): Operation[] {
  const tabs = snapshot.tabs.map((tab) => ({ ...tab }));
  const assigned = new Set<string>();
  const operations: Operation[] = [];

  for (const [index, target] of layout.tabs.entries()) {
    if (index === 0 && layout.firstUsesCurrentTab && snapshot.currentPane && isIdle(snapshot.currentPane, snapshot.processes)) {
      const planned = planTarget(target, tabs, snapshot.panes, snapshot.processes, assigned);
      if (planned.type !== "create-tab") {
        operations.push(planned);
        continue;
      }

      assigned.add(snapshot.currentPane.pane_id);
      const tab = tabs.find((candidate) => candidate.tab_id === snapshot.currentPane?.tab_id);
      if (tab) tab.label = target.label;
      operations.push({ type: "claim-current", target, paneId: snapshot.currentPane.pane_id, tabId: snapshot.currentPane.tab_id });
      continue;
    }

    operations.push(planTarget(target, tabs, snapshot.panes, snapshot.processes, assigned));
  }

  return operations;
}

function planTarget(
  target: LayoutTarget,
  tabs: TabInfo[],
  panes: PaneInfo[],
  processes: Map<string, PaneProcessInfo>,
  assigned: Set<string>,
): Operation {
  const matchingTabs = tabs.filter((tab) => tab.label === target.label);
  if (matchingTabs.length === 0) return { type: "create-tab", target };

  for (const tab of matchingTabs) {
    const pane = panes.find((candidate) => candidate.tab_id === tab.tab_id && !assigned.has(candidate.pane_id) && isTargetRunning(candidate, target, processes));
    if (pane) {
      assigned.add(pane.pane_id);
      return { type: "already-running", label: target.label, paneId: pane.pane_id, tabId: tab.tab_id };
    }
  }

  for (const tab of matchingTabs) {
    const pane = panes.find((candidate) => candidate.tab_id === tab.tab_id && !assigned.has(candidate.pane_id) && isIdle(candidate, processes));
    if (pane) {
      assigned.add(pane.pane_id);
      return { type: "run-existing", target, paneId: pane.pane_id, tabId: tab.tab_id };
    }
  }

  throw new Error(`Tab "${target.label}" exists, but no matching or idle pane is available`);
}

async function applyOperations(operations: Operation[], snapshot: Snapshot): Promise<void> {
  for (const operation of operations) {
    if (operation.type === "already-running") continue;
    if (operation.type === "claim-current") {
      await herdrOk(["tab", "rename", operation.tabId, operation.target.label]);
      await herdrOk(["pane", "run", operation.paneId, operation.target.command]);
      continue;
    }
    if (operation.type === "run-existing") {
      await herdrOk(["pane", "run", operation.paneId, operation.target.command]);
      continue;
    }

    const args = ["tab", "create", "--cwd", snapshot.activeCwd, "--label", operation.target.label, "--no-focus"];
    if (snapshot.workspaceId) args.splice(2, 0, "--workspace", snapshot.workspaceId);
    const rootPane = parseCreatedRootPane(await herdrJson(args));
    await herdrOk(["pane", "run", rootPane.pane_id, operation.target.command]);
  }
}

export function isTargetRunning(pane: PaneInfo, target: LayoutTarget, processes: Map<string, PaneProcessInfo>): boolean {
  const targetName = commandName(target.command);
  if (normalizeName(pane.agent) === targetName) return true;
  const info = processes.get(pane.pane_id);
  return info?.foreground_processes.some((processInfo) => normalizeName(processInfo.argv0 ?? processInfo.name) === targetName) === true;
}

export function isIdle(pane: PaneInfo, processes: Map<string, PaneProcessInfo>): boolean {
  const foreground = processes.get(pane.pane_id)?.foreground_processes ?? [];
  return foreground.length > 0 && foreground.every((processInfo) => SHELLS.has(normalizeName(processInfo.argv0 ?? processInfo.name)));
}

function commandName(command: string): string {
  const trimmed = command.trimStart();
  if (trimmed === "") return "";
  const quote = trimmed[0];
  if (quote === "\"" || quote === "'") {
    const end = trimmed.indexOf(quote, 1);
    return normalizeName(end === -1 ? trimmed.slice(1) : trimmed.slice(1, end));
  }
  return normalizeName(trimmed.split(/\s+/, 1)[0]);
}

function normalizeName(value: string | undefined): string {
  if (!value) return "";
  const leaf = value.replace(/^['\"]|['\"]$/g, "").split(/[\\/]/).pop() ?? value;
  return leaf.toLowerCase().replace(/\.exe$/, "");
}

async function herdrJson(args: string[]): Promise<unknown> {
  const stdout = await herdr(args, true);
  if (stdout.trim() === "") throw new Error(`herdr ${args.join(" ")} returned no JSON`);
  return JSON.parse(stdout) as unknown;
}

async function herdrOk(args: string[]): Promise<void> {
  await herdr(args, false);
}

async function herdr(args: string[], expectJson: boolean): Promise<string> {
  const bin = process.env.HERDR_BIN_PATH ?? "herdr";

  const proc = Bun.spawn([bin, ...args], { stdout: "pipe", stderr: "pipe" });
  const [stdout, stderr, code] = await Promise.all([
    new Response(proc.stdout).text(),
    new Response(proc.stderr).text(),
    proc.exited,
  ]);
  if (code !== 0) throw new Error((stderr || stdout || `herdr ${args.join(" ")} failed with exit ${code}`).trim());
  if (expectJson && stdout.trim() === "") throw new Error(`herdr ${args.join(" ")} returned empty output`);
  return stdout;
}

async function notifyUser(message: string): Promise<void> {
  try {
    await herdrOk(["notification", "show", "Herdr Layout", "--body", message, "--position", "top-right", "--sound", "request"]);
  } catch {
    console.error(message);
  }
}

function parseTabList(response: unknown): TabInfo[] {
  const result = expectResult(response, "tab_list");
  if (!Array.isArray(result.tabs)) throw new Error("herdr tab list: tabs missing");
  return result.tabs.map(parseTabInfo);
}

function parsePaneList(response: unknown): PaneInfo[] {
  const result = expectResult(response, "pane_list");
  if (!Array.isArray(result.panes)) throw new Error("herdr pane list: panes missing");
  return result.panes.map(parsePaneInfo);
}

function parseProcessInfo(response: unknown): PaneProcessInfo {
  const result = expectResult(response, "pane_process_info");
  if (!isRecord(result.process_info)) throw new Error("herdr pane process-info: process_info missing");
  const info = result.process_info;
  const foreground = Array.isArray(info.foreground_processes) ? info.foreground_processes : [];
  return {
    pane_id: readString(info, "pane_id", "process_info"),
    foreground_processes: foreground.map(parsePaneProcess),
  };
}

function parseCreatedRootPane(response: unknown): PaneInfo {
  const result = expectResult(response, "tab_created");
  if (!isRecord(result.root_pane)) throw new Error("herdr tab create: root_pane missing");
  return parsePaneInfo(result.root_pane);
}

function parseTabInfo(raw: unknown): TabInfo {
  if (!isRecord(raw)) throw new Error("tab info must be object");
  return {
    tab_id: readString(raw, "tab_id", "tab"),
    workspace_id: readString(raw, "workspace_id", "tab"),
    label: readString(raw, "label", "tab"),
    focused: raw.focused === true,
  };
}

function parsePaneInfo(raw: unknown): PaneInfo {
  if (!isRecord(raw)) throw new Error("pane info must be object");
  return {
    pane_id: readString(raw, "pane_id", "pane"),
    workspace_id: readString(raw, "workspace_id", "pane"),
    tab_id: readString(raw, "tab_id", "pane"),
    focused: raw.focused === true,
    cwd: optionalString(raw.cwd),
    foreground_cwd: optionalString(raw.foreground_cwd),
    agent: optionalString(raw.agent),
  };
}

function parsePaneProcess(raw: unknown): PaneProcess {
  if (!isRecord(raw)) throw new Error("process info must be object");
  return {
    name: readString(raw, "name", "process"),
    argv0: optionalString(raw.argv0),
  };
}

function expectResult(response: unknown, type: string): Record<string, unknown> {
  if (!isRecord(response)) throw new Error("Herdr response must be object");
  if (isRecord(response.error)) {
    throw new Error(optionalString(response.error.message) ?? JSON.stringify(response.error));
  }
  if (!isRecord(response.result)) throw new Error("Herdr response missing result");
  if (response.result.type !== type) throw new Error(`Expected Herdr result ${type}, got ${String(response.result.type)}`);
  return response.result;
}

function readPluginContext(): PluginContext {
  const parsed = parsePluginContext();
  return {
    workspace_id: pickString(parsed, "workspace_id") ?? nestedString(parsed, "workspace", "workspace_id", "id") ?? optionalString(process.env.HERDR_WORKSPACE_ID),
    tab_id: pickString(parsed, "tab_id") ?? nestedString(parsed, "tab", "tab_id", "id") ?? optionalString(process.env.HERDR_TAB_ID),
    focused_pane_id:
      pickString(parsed, "focused_pane_id", "pane_id") ?? nestedString(parsed, "focused_pane", "pane_id", "id") ?? optionalString(process.env.HERDR_PANE_ID),
    focused_pane_cwd: pickString(parsed, "focused_pane_cwd") ?? nestedString(parsed, "focused_pane", "foreground_cwd", "cwd"),
    workspace_cwd: pickString(parsed, "workspace_cwd") ?? nestedString(parsed, "workspace", "cwd"),
  };
}

function parsePluginContext(): Record<string, unknown> {
  const raw = process.env.HERDR_PLUGIN_CONTEXT_JSON;
  if (!raw) return {};
  try {
    const parsed: unknown = JSON.parse(raw);
    return isRecord(parsed) ? parsed : {};
  } catch {
    return {};
  }
}

function pickString(record: Record<string, unknown>, ...keys: string[]): string | undefined {
  for (const key of keys) {
    const value = optionalString(record[key]);
    if (value) return value;
  }
  return undefined;
}

function nestedString(record: Record<string, unknown>, key: string, ...keys: string[]): string | undefined {
  const nested = record[key];
  return isRecord(nested) ? pickString(nested, ...keys) : undefined;
}

function readString(record: Record<string, unknown>, key: string, owner: string): string {
  const value = record[key];
  if (typeof value !== "string") throw new Error(`${owner}.${key} must be string`);
  return value;
}

function optionalString(value: unknown): string | undefined {
  return typeof value === "string" && value !== "" ? value : undefined;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function configFiles(loaded: LoadedConfig): string[] {
  return loaded.overridePath ? [loaded.globalPath, loaded.overridePath] : [loaded.globalPath];
}

if (import.meta.main) {
  process.exit(await main(Bun.argv.slice(2)));
}
