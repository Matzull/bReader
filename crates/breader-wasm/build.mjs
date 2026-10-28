// Builds the JavaScript packages with wasm-pack: `pkg/` (ES module for browsers, --target web) and
// `pkg-node/` (CommonJS for Node, --target nodejs), then makes their package.json private.
//
//   node crates/breader-wasm/build.mjs [web|nodejs|all]    (default: all)
import { spawnSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const crate = dirname(fileURLToPath(import.meta.url));
const which = process.argv[2] ?? "all";
const targets = { web: "pkg", nodejs: "pkg-node" };
const wanted = which === "all" ? Object.keys(targets) : [which];

for (const target of wanted) {
  const outDir = targets[target];
  if (!outDir) {
    console.error(`unknown target ${which}, expected web, nodejs or all`);
    process.exit(2);
  }
  const run = spawnSync(
    "wasm-pack",
    ["build", crate, "--release", "--target", target, "--out-dir", outDir, "--out-name", "breader"],
    { stdio: "inherit" },
  );
  if (run.status !== 0) process.exit(run.status ?? 1);

  // wasm-pack names the package after the crate and leaves it publishable; the project is
  // private, so the package is called "breader" and carries "private": true.
  const file = join(crate, outDir, "package.json");
  const pkg = JSON.parse(readFileSync(file, "utf8"));
  pkg.name = "breader";
  pkg.private = true;
  pkg.license = "AGPL-3.0-only";
  if (target === "web") pkg.type = "module";
  writeFileSync(file, `${JSON.stringify(pkg, null, 2)}\n`);
  console.log(`${outDir}/package.json: name=${pkg.name} private=${pkg.private}`);
}
