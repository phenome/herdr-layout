import { mkdir, rm } from "node:fs/promises";

const builds = [
  { id: "windows-x64", target: "bun-windows-x64", asset: "herdr-layout-windows-x64.exe" },
  { id: "darwin-x64", target: "bun-darwin-x64", asset: "herdr-layout-darwin-x64" },
  { id: "darwin-arm64", target: "bun-darwin-arm64", asset: "herdr-layout-darwin-arm64" },
  { id: "linux-x64", target: "bun-linux-x64", asset: "herdr-layout-linux-x64" },
  { id: "linux-arm64", target: "bun-linux-arm64", asset: "herdr-layout-linux-arm64" },
  { id: "linux-musl-x64", target: "bun-linux-x64-musl", asset: "herdr-layout-linux-musl-x64" },
] as const;

type BuildId = (typeof builds)[number]["id"];

const hostBuilds: Record<string, BuildId> = {
  "win32-x64": "windows-x64",
  "darwin-x64": "darwin-x64",
  "darwin-arm64": "darwin-arm64",
  "linux-x64": "linux-x64",
  "linux-arm64": "linux-arm64",
};

const usage = `Use "all" or one of: ${builds.map(({ id }) => id).join(", ")}`;

function hostBuildId() {
  const id = hostBuilds[`${process.platform}-${process.arch}`];
  if (!id) throw new Error(`Unsupported host ${process.platform}-${process.arch}. ${usage}`);
  return id;
}

const requested = Bun.argv[2]?.replace(/^--target=/, "") ?? hostBuildId();
const selected =
  requested === "all" || requested === "--all"
    ? builds
    : builds.filter(({ id, target }) => requested === id || requested === target);

if (selected.length === 0) throw new Error(`Unknown release target "${requested}". ${usage}`);

await rm("dist", { recursive: true, force: true });
await mkdir("dist", { recursive: true });

for (const { target, asset } of selected) {
  await Bun.$`bun build --compile --target=${target} --outfile=${`dist/${asset}`} src/index.ts`;
}
