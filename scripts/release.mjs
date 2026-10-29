// Release helpers for the GitHub workflow.
//   node scripts/release.mjs config    writes release.conf.json for `tauri build --config`
//   node scripts/release.mjs manifest  writes latest.json for the in-app updater
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const env = process.env;
const tag = env.GITHUB_REF_TYPE === "tag" ? env.GITHUB_REF_NAME : null;
const conf = JSON.parse(readFileSync(join("src-tauri", "tauri.conf.json"), "utf8"));

/** The version comes from the tag (v1.2.3), so the tag is the only place to change it. */
function version() {
  return tag?.startsWith("v") ? tag.slice(1) : conf.version;
}

function config() {
  const out = { version: version() };
  if (env.TAURI_SIGNING_PRIVATE_KEY) {
    out.bundle = { createUpdaterArtifacts: true };
  } else {
    console.log("::warning::TAURI_SIGNING_PRIVATE_KEY is not set. This build cannot update itself in the app.");
  }
  writeFileSync("release.conf.json", JSON.stringify(out, null, 2));
  console.log(`Building version ${out.version}`);
}

function manifest() {
  const v = version();
  const name = `${conf.productName}_${v}_x64-setup.exe`;
  const dir = join("src-tauri", "target", "release", "bundle", "nsis");
  const sig = join(dir, `${name}.sig`);
  if (!existsSync(sig)) {
    console.log("::warning::No updater signature. The release has no latest.json.");
    return;
  }
  const latest = {
    version: v,
    pub_date: new Date().toISOString(),
    platforms: {
      "windows-x86_64": {
        signature: readFileSync(sig, "utf8").trim(),
        url: `https://github.com/${env.GITHUB_REPOSITORY}/releases/download/${tag}/${encodeURIComponent(name)}`,
      },
    },
  };
  writeFileSync(join(dir, "latest.json"), JSON.stringify(latest, null, 2));
  console.log(`latest.json points to ${latest.platforms["windows-x86_64"].url}`);
}

function fail(msg) {
  console.log(`::error::${msg}`);
  process.exit(1);
}

const cmd = process.argv[2];
if (cmd === "config") config();
else if (cmd === "manifest") manifest();
else fail(`Unknown command: ${cmd}`);
