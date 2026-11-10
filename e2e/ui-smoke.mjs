// Browser smoke test of the console and portal against a REAL installed server,
// run ON the server after e2e/vps-e2e.py (it reuses that script's console owner).
//
//   npm i --prefix /root/ui playwright-core && \
//   UI_HOST=mail.example.com UI_USER=x@example.com UI_PASS=… UI_FORWARD=y@example.com \
//   NODE_PATH=/root/ui/node_modules node e2e/ui-smoke.mjs
//
// Drives the real pages over https://127.0.0.1 (the mail hostname resolved to
// loopback; certificate errors ignored before Let's Encrypt has issued one).
import { createRequire } from "node:module";
import { createHmac } from "node:crypto";
import { readFileSync, mkdirSync } from "node:fs";

const require = createRequire(import.meta.url);
const { chromium } = require("playwright-core");
const host = process.env.UI_HOST;
const shots = process.env.UI_SHOTS || "/root/ui-shots";
mkdirSync(shots, { recursive: true });
let passed = 0;
let failed = 0;
const check = (name, ok, detail = "") => {
  ok ? passed++ : failed++;
  console.log(`  [${ok ? "ok" : "FAIL"}] ${name}${detail ? " - " + detail : ""}`);
};

function totp(secret) {
  const alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
  let bits = "";
  for (const c of secret.replace(/=+$/, "").toUpperCase()) bits += alphabet.indexOf(c).toString(2).padStart(5, "0");
  const key = Buffer.from(bits.match(/.{8}/g).map((b) => parseInt(b, 2)));
  const counter = Buffer.alloc(8);
  counter.writeBigUInt64BE(BigInt(Math.floor(Date.now() / 1000 / 30)));
  const mac = createHmac("sha1", key).update(counter).digest();
  const o = mac[mac.length - 1] & 15;
  return String((mac.readUInt32BE(o) & 0x7fffffff) % 1e6).padStart(6, "0");
}

const browser = await chromium.launch({
  executablePath: process.env.CHROMIUM || "/usr/bin/chromium",
  args: ["--no-sandbox", `--host-resolver-rules=MAP ${host} 127.0.0.1`],
});
const ctx = await browser.newContext({ ignoreHTTPSErrors: true, viewport: { width: 1280, height: 900 } });
const page = await ctx.newPage();
const errors = [];
page.on("pageerror", (e) => errors.push(e.message));

// ── portal: auto-reply & forwarding ─────────────────────────────────────────
console.log("== portal (real server)");
await page.goto(`https://${host}/account/login`);
await page.fill("#p-email", process.env.UI_USER);
await page.locator('input[type="password"]').first().fill(process.env.UI_PASS);
await page.getByRole("button", { name: "Sign in" }).click();
await page.locator("#vr-toggle").waitFor({ timeout: 20000 });
check("portal sign-in shows Auto-reply & forwarding", true);
await page.locator("#vr-toggle").click();
await page.getByRole("button", { name: "Save" }).click();
await page.locator("#vr-body-err").waitFor({ timeout: 10000 }).catch(() => {});
check("empty auto-reply message is refused under the field", (await page.locator("#vr-body-err").textContent())?.length > 0);
await page.fill("#vr-body", "Away until Monday (UI smoke).");
await page.locator("#fw-toggle").click();
await page.fill("#fw-address", process.env.UI_FORWARD);
await page.getByRole("button", { name: "Save" }).click();
await page.getByText(/Forwarding to/).first().waitFor({ timeout: 15000 });
check("saved: status line shows forwarding", true);
await page.screenshot({ path: `${shots}/portal-rules-real.png`, fullPage: true });
await page.reload();
await page.locator("#fw-address").waitFor({ timeout: 15000 });
check("rules persist after reload (read back from the engine)",
  (await page.inputValue("#fw-address")) === process.env.UI_FORWARD.toLowerCase());
await page.locator("#vr-toggle").click();
await page.locator("#fw-toggle").click();
await page.getByRole("button", { name: "Save" }).click();
await page.waitForTimeout(2000);

// ── console: mailbox import & export ────────────────────────────────────────
console.log("== console (real server)");
const owner = JSON.parse(readFileSync("/root/.yunori-e2e-owner.json", "utf8"));
await new Promise((r) => setTimeout(r, (31 - ((Date.now() / 1000) % 30)) * 1000)); // a fresh TOTP step
await page.goto(`https://${host}/console/login`);
await page.fill("#email", owner.owner);
await page.fill("#password", owner.password);
await page.fill("#code", totp(owner.secret));
await page.getByRole("button", { name: /Sign in/ }).click();
await page.waitForURL(/\/console(?!\/login)/, { timeout: 20000 });
check("console sign-in with TOTP", true);
await page.goto(`https://${host}/console/mailboxes`);
const [download] = await Promise.all([
  page.waitForEvent("download", { timeout: 20000 }),
  page.getByRole("button", { name: "Export" }).click(),
]);
check("Export downloads a CSV", download.suggestedFilename().endsWith(".csv"), download.suggestedFilename());
await page.getByRole("button", { name: "Import" }).click();
const domain = process.env.UI_USER.split("@")[1];
const stamp = Date.now().toString(36);
await page.locator("textarea").fill(`address,display_name\nui${stamp}@${domain},UI Smoke\nbroken,,\n`);
await page.getByRole("button", { name: "Check" }).click();
await page.getByRole("button", { name: /Import 1 mailbox/ }).waitFor({ timeout: 15000 });
check("dry run: 1 ready, the broken row skipped", (await page.getByText("Not an email address.").count()) > 0);
await page.screenshot({ path: `${shots}/console-import-check-real.png` });
await page.getByRole("button", { name: /Import 1 mailbox/ }).click();
await page.getByText(/#invite=/).first().waitFor({ timeout: 20000 });
check("import shows the invite link", true);
await page.screenshot({ path: `${shots}/console-import-done-real.png` });

check("no page errors", errors.length === 0, errors.join(" | "));
console.log(`\n${passed} passed, ${failed} failed`);
console.log(`UI_IMPORTED=ui${stamp}@${domain}`);
await browser.close();
process.exit(failed ? 1 : 0);
