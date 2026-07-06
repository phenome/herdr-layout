import { readFile, writeFile } from "node:fs/promises";

const version = Bun.argv[2];
if (!version || !/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(version)) {
  throw new Error("Usage: bun scripts/sync-manifest-version.ts <version>");
}

await replaceVersion("herdr-plugin.toml");
await replaceVersion("Cargo.toml");

async function replaceVersion(path: string) {
  const text = await readFile(path, "utf8");
  if (!/^version\s*=\s*"[^"]+".*$/m.test(text)) {
    throw new Error(`Missing version in ${path}`);
  }
  await writeFile(path, text.replace(/^version\s*=\s*"[^"]+".*$/m, `version = "${version}"`));
}
