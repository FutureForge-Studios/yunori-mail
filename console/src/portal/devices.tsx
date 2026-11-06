import { useState } from "react";
import { Download, ExternalLink, RefreshCw, Smartphone } from "lucide-react";
import { Button } from "@/design/vendor/ui/button";
import { Input } from "@/design/vendor/ui/input";
import { Switch } from "@/design/vendor/ui/switch";
import { errorMessage, portalApi } from "@/api/client";
import type { AppleProfileResponse, PortalMe } from "@/api/types";
import { FieldError, FilterPills } from "@/components/kit";
import { QrCode } from "@/components/qr-code";
import { openInYunoriUrl } from "@/lib/format";

type Target = "yunori" | "apple" | "other";

/** What the visitor is holding, from the user agent: it picks the first tab and the device name. */
export function guessDevice(ua: string = typeof navigator === "undefined" ? "" : navigator.userAgent): {
  target: Target;
  name: string;
  isApple: boolean;
} {
  if (/iPad/.test(ua)) return { target: "apple", name: "iPad", isApple: true };
  if (/iPhone|iPod/.test(ua)) return { target: "apple", name: "iPhone", isApple: true };
  // iPadOS asks for the desktop site and says "Macintosh"; either way a profile fits.
  if (/Macintosh/.test(ua)) return { target: "yunori", name: "Mac", isApple: true };
  if (/Android/.test(ua)) return { target: "other", name: "Phone", isApple: false };
  return { target: "yunori", name: "iPhone", isApple: false };
}

/**
 * "Connect your devices": Yunori (a QR code that opens the client with the
 * address filled in), Apple devices (a one-time configuration profile around
 * an app password made for that device) and everything else (apps find the
 * settings themselves through autoconfig).
 */
export function ConnectDevices({ me, onChange }: { me: PortalMe; onChange: (me: PortalMe) => void }) {
  const [guess] = useState(() => guessDevice());
  const [target, setTarget] = useState<Target>(guess.target);
  return (
    <div className="rounded-xl border border-border/60 bg-card/40">
      <div className="border-b border-border/50 px-5 py-3">
        <FilterPills
          label="Device"
          value={target}
          onChange={setTarget}
          options={[
            { value: "yunori", label: "Yunori" },
            { value: "apple", label: "iPhone, iPad & Mac" },
            { value: "other", label: "Android & other apps" },
          ]}
        />
      </div>
      <div className="p-5">
        {target === "yunori" && <YunoriPanel me={me} />}
        {target === "apple" && (
          <ApplePanel me={me} onChange={onChange} defaultName={guess.name} onThisDevice={guess.isApple} handheld={guess.isApple && guess.name !== "Mac"} />
        )}
        {target === "other" && <OtherPanel me={me} />}
      </div>
    </div>
  );
}

function YunoriPanel({ me }: { me: PortalMe }) {
  const url = openInYunoriUrl(me.clientOrigin, me.address);
  return (
    <div className="flex flex-col gap-5 sm:flex-row sm:items-center">
      <div className="shrink-0 self-center rounded-lg border border-border/60 bg-[#faf7f2] p-2">
        <QrCode value={url} label="QR code that opens Yunori with your address filled in" size={148} />
      </div>
      <div className="min-w-0">
        <p className="text-sm font-medium">A calm inbox on the web and on your phone</p>
        <p className="mt-1 text-xs leading-relaxed text-muted-foreground">
          Scan the code with your phone's camera, or open Yunori here. It recognises this server and fills in every setting — you
          only type your password.
        </p>
        <Button asChild variant="torii" size="sm" className="mt-4">
          <a href={url} target="_blank" rel="noopener noreferrer">
            Open in Yunori
            <ExternalLink className="h-3.5 w-3.5" />
          </a>
        </Button>
      </div>
    </div>
  );
}

function ApplePanel({
  me,
  onChange,
  defaultName,
  onThisDevice,
  handheld,
}: {
  me: PortalMe;
  onChange: (me: PortalMe) => void;
  defaultName: string;
  /** An Apple device is reading this page: the download goes first. */
  onThisDevice: boolean;
  /** …and it is an iPhone or iPad, so a code to scan would point back at itself. */
  handheld: boolean;
}) {
  const [device, setDevice] = useState(defaultName);
  const [calendars, setCalendars] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [made, setMade] = useState<AppleProfileResponse | null>(null);

  async function make(e: React.FormEvent) {
    e.preventDefault();
    if (!device.trim()) {
      setError("Name the device, like “iPhone” or “Work Mac”.");
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const r = await portalApi.appleProfile({ device: device.trim(), calendars });
      setMade(r);
      onChange({ ...me, appPasswords: [...me.appPasswords, r.appPassword] });
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setBusy(false);
    }
  }

  if (made && handheld) {
    return (
      <div>
        <p className="text-sm font-medium">Profile ready for {made.appPassword.description.replace(/ \(profile\)$/, "")}</p>
        <Button asChild variant="torii" className="mt-3 w-full sm:w-auto">
          <a href={made.url} download>
            <Download className="h-4 w-4" />
            Download profile
          </a>
        </Button>
        <ol className="mt-4 list-decimal space-y-1.5 pl-4 text-xs leading-relaxed text-foreground/85">
          <li>Tap Download profile and allow the download.</li>
          <li>
            Open <span className="font-medium text-foreground">Settings</span> — <span className="font-medium text-foreground">Profile Downloaded</span> is at the top —
            and tap <span className="font-medium text-foreground">Install</span>.
          </li>
          <li>
            Mail{calendars ? ", Calendar and Contacts" : ""} start syncing. The profile is marked “Unverified” because it comes from
            your server rather than from Apple — that's expected.
          </li>
        </ol>
        <p className="mt-3 text-[11px] leading-relaxed text-muted-foreground">
          The link works once, for 10 minutes. It carries the new app password “{made.appPassword.description}”; revoke that below to
          sign the device out.
        </p>
        <Button size="sm" variant="ghost" className="mt-3" onClick={() => setMade(null)}>
          <RefreshCw className="h-3.5 w-3.5" />
          Another device
        </Button>
      </div>
    );
  }

  if (made) {
    return (
      <div className="flex flex-col gap-5 sm:flex-row sm:items-start">
        <div className="shrink-0 self-center rounded-lg border border-border/60 bg-[#faf7f2] p-2 sm:self-start">
          <QrCode value={made.url} label={`QR code for the ${made.appPassword.description} setup link`} size={148} />
        </div>
        <div className="min-w-0 flex-1">
          <p className="text-sm font-medium">Profile ready for {made.appPassword.description.replace(/ \(profile\)$/, "")}</p>
          <ol className="mt-2 list-decimal space-y-1.5 pl-4 text-xs leading-relaxed text-foreground/85">
            <li>
              On the iPhone or iPad, point the Camera at the code and open the link. On {onThisDevice ? "this device" : "a Mac"}, use{" "}
              <span className="font-medium text-foreground">Download profile</span>.
            </li>
            <li>
              Allow the download, then open <span className="font-medium text-foreground">Settings → Profile Downloaded → Install</span>.
              On a Mac: <span className="font-medium text-foreground">System Settings → General → Device Management</span>.
            </li>
            <li>
              Mail{calendars ? ", Calendar and Contacts" : ""} start syncing. The profile is marked “Unverified” because it comes from
              your server rather than from Apple — that's expected.
            </li>
          </ol>
          <p className="mt-3 text-[11px] leading-relaxed text-muted-foreground">
            The link works once, for 10 minutes. It carries the new app password “{made.appPassword.description}”; revoke that below
            to sign the device out.
          </p>
          <div className="mt-4 flex flex-wrap gap-2">
            <Button asChild size="sm" variant={onThisDevice ? "torii" : "outline"}>
              <a href={made.url} download>
                <Download className="h-3.5 w-3.5" />
                Download profile
              </a>
            </Button>
            <Button size="sm" variant="ghost" onClick={() => setMade(null)}>
              <RefreshCw className="h-3.5 w-3.5" />
              Another device
            </Button>
          </div>
        </div>
      </div>
    );
  }

  return (
    <form onSubmit={make} noValidate>
      <p className="text-sm font-medium">Set up Apple Mail without typing a server name</p>
      <p className="mt-1 text-xs leading-relaxed text-muted-foreground">
        A profile adds this mailbox to Mail — and to Calendar and Contacts if you like — using an app password made just for that
        device. {handheld ? "You download it here and install it in Settings." : "You get a code to scan with the phone, or a download for this computer."}
      </p>
      <div className="mt-4 grid gap-3 sm:grid-cols-[minmax(0,1fr)_auto] sm:items-end">
        <div className="space-y-1.5">
          <label htmlFor="dev-name" className="block text-xs font-medium text-foreground/80">
            Device name
          </label>
          <Input
            id="dev-name"
            value={device}
            onChange={(e) => setDevice(e.target.value)}
            maxLength={40}
            placeholder="iPhone"
            aria-invalid={!!error}
            aria-describedby={error ? "dev-name-err" : undefined}
          />
        </div>
        <Button type="submit" disabled={busy} className="shrink-0">
          <Smartphone className="h-4 w-4" />
          {busy ? "Making…" : "Make a profile"}
        </Button>
      </div>
      <FieldError id="dev-name-err">{error}</FieldError>
      <label className="mt-3 flex cursor-pointer items-center gap-2.5 text-xs text-foreground/85">
        <Switch checked={calendars} onCheckedChange={setCalendars} aria-label="Calendars and contacts too" />
        Calendars and contacts too
      </label>
    </form>
  );
}

function OtherPanel({ me }: { me: PortalMe }) {
  return (
    <div className="space-y-3 text-xs leading-relaxed text-foreground/85">
      <p className="text-sm font-medium text-foreground">Most apps set themselves up</p>
      <p>
        In Gmail, Outlook, Thunderbird, K-9 or FairEmail, add an account and enter <span className="font-mono">{me.address}</span> and
        your password. The app finds this server's settings by itself; if it asks, choose IMAP and use the settings below.
      </p>
      <p>
        Calendars and contacts on Android: install <span className="font-medium">DAVx⁵</span>, choose “Login with URL and user name”
        and enter <span className="font-mono">{me.settings.dav.url}</span> with your address and password.
      </p>
      <p className="text-muted-foreground">
        Rather not type your main password into every app? Make an app password for each one below — revoking it signs out just
        that app.
      </p>
      <a href="#sec-mail-settings" className="inline-block text-xs text-foreground underline underline-offset-4 hover:text-foreground/80">
        See all the settings
      </a>
    </div>
  );
}
