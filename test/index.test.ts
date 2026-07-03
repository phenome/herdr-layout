import { describe, expect, test } from "bun:test";
import { isIdle, isTargetRunning, planLayout, selectLayout, validateConfigRoot } from "../src/index.ts";
import type { Layout, LayoutTarget, PaneInfo, PaneProcess, PaneProcessInfo, Snapshot, TabInfo } from "../src/index.ts";

const target = (label = "api", command = "api --serve"): LayoutTarget => ({ label, command });
const layout = (tabs: LayoutTarget[] = [target()]): Layout => ({ firstUsesCurrentTab: false, tabs });
const tab = (tab_id: string, label: string): TabInfo => ({ tab_id, workspace_id: "w", label, focused: false });
const pane = (pane_id: string, tab_id: string, agent?: string): PaneInfo => ({
  pane_id,
  workspace_id: "w",
  tab_id,
  focused: false,
  ...(agent ? { agent } : {}),
});
const processes = (...entries: [string, PaneProcess[]][]): Map<string, PaneProcessInfo> =>
  new Map(entries.map(([pane_id, foreground_processes]) => [pane_id, { pane_id, foreground_processes }]));
const snapshot = (tabs: TabInfo[], panes: PaneInfo[], processMap: Map<string, PaneProcessInfo>, currentPane?: PaneInfo): Snapshot => ({
  tabs,
  panes,
  processes: processMap,
  currentPane,
  activeCwd: "/repo",
});

describe("config validation", () => {
  test("accepts a YAML root layouts map keyed only by slots 1/2/3", () => {
    const slots = { "1": layout([target("one")]), "2": layout([target("two")]), "3": layout([target("three")]) };

    expect(validateConfigRoot({ layouts: slots }, "cfg")).toEqual(slots);
  });

  test("rejects invalid config shape, slots, tab fields, and duplicate labels", () => {
    const cases: [string, unknown, RegExp][] = [
      ["root", null, /YAML root must be a map/],
      ["layouts", {}, /layouts must be a map/],
      ["slot", { layouts: { "4": layout() } }, /invalid layout slot 4/],
      ["tabs", { layouts: { "1": { firstUsesCurrentTab: false, tabs: [] } } }, /tabs must be a non-empty list/],
      ["label", { layouts: { "1": layout([{ label: "", command: "api" }]) } }, /label must be a non-empty string/],
      ["command", { layouts: { "1": layout([{ label: "api", command: "" }]) } }, /command must be a non-empty string/],
      ["duplicate", { layouts: { "1": layout([target("api"), target("api", "worker")]) } }, /duplicate tab label api/],
    ];

    for (const [name, raw, error] of cases) {
      expect(() => validateConfigRoot(raw, `cfg-${name}`)).toThrow(error);
    }
  });

  test("repo override replaces the whole slot layout", () => {
    const global = { "1": layout([target("global-a"), target("global-b")]) };
    const override = { "1": { firstUsesCurrentTab: true, tabs: [target("repo-only")] } };

    expect(selectLayout(global, override, "1")).toEqual(override["1"]);
  });
});

describe("runtime planning helpers", () => {
  test("matches tab labels exactly", () => {
    const desired = target("api");

    expect(
      planLayout(
        { firstUsesCurrentTab: false, tabs: [desired] },
        snapshot([tab("t1", "API")], [pane("p1", "t1")], processes(["p1", [{ name: "bash" }]])),
      ),
    ).toEqual([{ type: "create-tab", target: desired }]);
  });

  test("recognizes running targets by command basename without .exe or pane agent", () => {
    expect(
      isTargetRunning(
        pane("p1", "t1"),
        target("api", "C:\\tools\\api.exe --serve"),
        processes(["p1", [{ name: "ignored", argv0: "/usr/local/bin/api.exe" }]]),
      ),
    ).toBe(true);

    expect(isTargetRunning(pane("p2", "t2", "api.exe"), target("api", "api --serve"), processes())).toBe(true);
  });

  test("treats only known foreground shells as idle", () => {
    for (const shell of ["pwsh", "powershell.exe", "cmd", "zsh", "bash", "fish", "nu", "sh"]) {
      expect(isIdle(pane(`p-${shell}`, "t1"), processes([`p-${shell}`, [{ name: shell }]]))).toBe(true);
    }

    expect(isIdle(pane("p-node", "t1"), processes(["p-node", [{ name: "node" }]]))).toBe(false);
    expect(isIdle(pane("p-empty", "t1"), processes(["p-empty", []]))).toBe(false);
  });

  test("firstUsesCurrentTab does not claim current idle pane when first target already runs elsewhere", () => {
    const desired = target("api", "api.exe --serve");
    const current = pane("p-current", "t-current");

    expect(
      planLayout(
        { firstUsesCurrentTab: true, tabs: [desired] },
        snapshot(
          [tab("t-current", "scratch"), tab("t-existing", "api")],
          [current, pane("p-existing", "t-existing")],
          processes(["p-current", [{ name: "pwsh" }]], ["p-existing", [{ name: "api.exe" }]]),
          current,
        ),
      ),
    ).toEqual([{ type: "already-running", label: "api", paneId: "p-existing", tabId: "t-existing" }]);
  });

  test("firstUsesCurrentTab renames and reuses current pane when first target is already running there", () => {
    const desired = target("agent", "omp");
    const current = pane("p-current", "t-current");

    expect(
      planLayout(
        { firstUsesCurrentTab: true, tabs: [desired] },
        snapshot([tab("t-current", "1")], [current], processes(["p-current", [{ name: "omp" }]]), current),
      ),
    ).toEqual([{ type: "rename-current", target: desired, paneId: "p-current", tabId: "t-current" }]);
  });

  test("duplicate target tabs prefer a running pane, then first idle pane, and throw when neither exists", () => {
    const desired = target("api", "api --serve");
    const tabs = [tab("t-idle", "api"), tab("t-running", "api")];
    const panes = [pane("p-idle", "t-idle"), pane("p-running", "t-running")];

    expect(
      planLayout(
        { firstUsesCurrentTab: false, tabs: [desired] },
        snapshot(tabs, panes, processes(["p-idle", [{ name: "bash" }]], ["p-running", [{ name: "api" }]])),
      ),
    ).toEqual([{ type: "already-running", label: "api", paneId: "p-running", tabId: "t-running" }]);

    expect(
      planLayout(
        { firstUsesCurrentTab: false, tabs: [desired] },
        snapshot(tabs, panes, processes(["p-idle", [{ name: "bash" }]], ["p-running", [{ name: "zsh" }]])),
      ),
    ).toEqual([{ type: "run-existing", target: desired, paneId: "p-idle", tabId: "t-idle" }]);

    expect(() =>
      planLayout(
        { firstUsesCurrentTab: false, tabs: [desired] },
        snapshot(tabs, panes, processes(["p-idle", [{ name: "node" }]], ["p-running", [{ name: "python" }]])),
      ),
    ).toThrow(/no matching or idle pane is available/);
  });
});
