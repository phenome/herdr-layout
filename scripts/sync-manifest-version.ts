import { readFile, writeFile } from "node:fs/promises";

const version = Bun.argv[2];
if (!version || !/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(version)) {
  throw new Error("Usage: bun scripts/sync-manifest-version.ts <version>");
}

const path = "herdr-plugin.toml";
const manifest = await readFile(path, "utf8");
if (!/^version\s*=\s*"[^"]+".*$/m.test(manifest)) {
  throw new Error("Missing version in herdr-plugin.toml");
}

await writeFile(path, manifest.replace(/^version\s*=\s*"[^"]+".*$/m, `version = "${version}"`));
