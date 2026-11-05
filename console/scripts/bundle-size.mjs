#!/usr/bin/env node
/**
 * Reports the gzipped size of the built console (dist/) and fails above the
 * budget — the bundle is embedded into the yunori-mail binary.
 *   node scripts/bundle-size.mjs [distDir] [--budget-kb 400]
 */
import { readdirSync, readFileSync, statSync } from "node:fs";
import path from "node:path";
import { gzipSync } from "node:zlib";

const args = process.argv.slice(2);
const dir = path.resolve(args.find((a) => !a.startsWith("--")) ?? "dist");
const bi = args.indexOf("--budget-kb");
const budgetKb = bi >= 0 ? Number(args[bi + 1]) : 400;

const walk = (d) => readdirSync(d).flatMap((n) => (statSync(path.join(d, n)).isDirectory() ? walk(path.join(d, n)) : [path.join(d, n)]));
const files = walk(dir);
const kb = (n) => (n / 1024).toFixed(1).padStart(7);

let jsGz = 0;
let cssGz = 0;
let total = 0;
const rows = [];
for (const f of files) {
  const buf = readFileSync(f);
  total += buf.length;
  const ext = path.extname(f);
  if (ext === ".js" || ext === ".css") {
    const gz = gzipSync(buf, { level: 9 }).length;
    if (ext === ".js") jsGz += gz;
    else cssGz += gz;
    rows.push([path.relative(dir, f), buf.length, gz]);
  }
}
rows.sort((a, b) => b[2] - a[2]);
for (const [name, raw, gz] of rows) console.log(`${kb(raw)} KB raw ${kb(gz)} KB gz  ${name}`);
console.log(`\nJS  gzipped total: ${(jsGz / 1024).toFixed(1)} KB (budget ${budgetKb} KB)`);
console.log(`CSS gzipped total: ${(cssGz / 1024).toFixed(1)} KB`);
console.log(`dist/ on disk (fonts included): ${(total / 1024).toFixed(1)} KB`);
if (jsGz / 1024 > budgetKb) {
  console.error(`\nOver budget by ${(jsGz / 1024 - budgetKb).toFixed(1)} KB.`);
  process.exit(1);
}
