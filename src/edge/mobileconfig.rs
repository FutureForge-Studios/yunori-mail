//! Apple configuration profiles (`.mobileconfig`) for iPhone, iPad and Mac.
//!
//! iOS Mail ignores Thunderbird autoconfig and Autodiscover for third-party
//! domains, so a profile is the one way to set a phone up without typing
//! server names. The portal makes one per device around a fresh app password
//! (revocable on its own), keeps the bytes in memory only, and serves them
//! once through a short-lived link (`/account/profile/<token>`) — the link a QR
//! code carries to the phone. The profile is unsigned: iOS labels it
//! "Unverified", which the portal says up front.

use rand::RngCore;

pub struct Profile<'a> {
    /// The mail server's hostname (IMAP, SMTP and DAV all live there).
    pub hostname: &'a str,
    pub address: &'a str,
    pub display_name: Option<&'a str>,
    /// The app password made for this device.
    pub password: &'a str,
    /// What the person called the device ("Ana's iPhone").
    pub device: &'a str,
    /// Also add calendars and contacts (CalDAV/CardDAV).
    pub dav: bool,
}

fn esc(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control() || matches!(c, '\t' | '\n'))
        .collect::<String>()
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Random version-4 UUID, upper-case as Apple's tools write them.
pub fn uuid_v4() -> String {
    let mut b = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut b);
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h = hex::encode_upper(b);
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

/// `mail.example.com` + `ana@example.com` → `com.example.mail.yunori.ana-example-com`.
/// Stable per address, so installing a newer profile for the same mailbox
/// replaces the older one instead of adding a second account.
pub fn identifier(hostname: &str, address: &str) -> String {
    let reversed: Vec<&str> = hostname
        .split('.')
        .filter(|p| !p.is_empty())
        .rev()
        .collect();
    let who: String = address
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    format!("{}.yunori.{who}", reversed.join("."))
}

/// `Yunori-ana.mobileconfig`.
pub fn filename(address: &str) -> String {
    let local: String = address
        .split('@')
        .next()
        .unwrap_or("mail")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        .take(40)
        .collect();
    format!(
        "Yunori-{}.mobileconfig",
        if local.is_empty() { "mail" } else { &local }
    )
}

fn key_str(k: &str, v: &str) -> String {
    format!("      <key>{k}</key>\n      <string>{}</string>\n", esc(v))
}
fn key_int(k: &str, v: u16) -> String {
    format!("      <key>{k}</key>\n      <integer>{v}</integer>\n")
}
fn key_bool(k: &str, v: bool) -> String {
    format!(
        "      <key>{k}</key>\n      <{}/>\n",
        if v { "true" } else { "false" }
    )
}

fn payload(kind: &str, id: &str, name: &str, body: String) -> String {
    format!(
        "    <dict>\n{}{}{}{}{}{body}    </dict>\n",
        key_str("PayloadType", kind),
        key_int("PayloadVersion", 1),
        key_str("PayloadIdentifier", id),
        key_str("PayloadUUID", &uuid_v4()),
        key_str("PayloadDisplayName", name),
    )
}

pub fn render(p: &Profile) -> String {
    let base = identifier(p.hostname, p.address);
    let name = p.display_name.unwrap_or(p.address);
    let mut payloads = String::new();

    let mut mail = String::new();
    mail += &key_str("EmailAccountType", "EmailTypeIMAP");
    mail += &key_str("EmailAccountDescription", p.address);
    mail += &key_str("EmailAccountName", name);
    mail += &key_str("EmailAddress", p.address);
    mail += &key_str("IncomingMailServerHostName", p.hostname);
    mail += &key_int("IncomingMailServerPortNumber", 993);
    mail += &key_bool("IncomingMailServerUseSSL", true);
    mail += &key_str("IncomingMailServerUsername", p.address);
    mail += &key_str("IncomingMailServerAuthentication", "EmailAuthPassword");
    mail += &key_str("IncomingPassword", p.password);
    mail += &key_str("OutgoingMailServerHostName", p.hostname);
    mail += &key_int("OutgoingMailServerPortNumber", 465);
    mail += &key_bool("OutgoingMailServerUseSSL", true);
    mail += &key_str("OutgoingMailServerUsername", p.address);
    mail += &key_str("OutgoingMailServerAuthentication", "EmailAuthPassword");
    mail += &key_str("OutgoingPassword", p.password);
    mail += &key_bool("OutgoingPasswordSameAsIncomingPassword", true);
    mail += &key_bool("SMIMEEnabled", false);
    payloads += &payload(
        "com.apple.mail.managed",
        &format!("{base}.mail"),
        &format!("Mail · {}", p.address),
        mail,
    );

    if p.dav {
        let mut cal = String::new();
        cal += &key_str("CalDAVAccountDescription", p.address);
        cal += &key_str("CalDAVHostName", p.hostname);
        cal += &key_int("CalDAVPort", 443);
        cal += &key_bool("CalDAVUseSSL", true);
        cal += &key_str("CalDAVUsername", p.address);
        cal += &key_str("CalDAVPassword", p.password);
        payloads += &payload(
            "com.apple.caldav.account",
            &format!("{base}.caldav"),
            &format!("Calendars · {}", p.address),
            cal,
        );

        let mut card = String::new();
        card += &key_str("CardDAVAccountDescription", p.address);
        card += &key_str("CardDAVHostName", p.hostname);
        card += &key_int("CardDAVPort", 443);
        card += &key_bool("CardDAVUseSSL", true);
        card += &key_str("CardDAVUsername", p.address);
        card += &key_str("CardDAVPassword", p.password);
        payloads += &payload(
            "com.apple.carddav.account",
            &format!("{base}.carddav"),
            &format!("Contacts · {}", p.address),
            card,
        );
    }

    let what = if p.dav {
        "mail, calendars and contacts"
    } else {
        "mail"
    };
    let description = format!(
        "Sets up {what} for {} on {}, using an app password made for this device. Remove the profile, or revoke \"{}\" on https://{}/account, to sign this device out.",
        p.address, p.device, p.device, p.hostname
    );
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>PayloadType</key>
  <string>Configuration</string>
  <key>PayloadVersion</key>
  <integer>1</integer>
  <key>PayloadIdentifier</key>
  <string>{id}</string>
  <key>PayloadUUID</key>
  <string>{uuid}</string>
  <key>PayloadDisplayName</key>
  <string>{title}</string>
  <key>PayloadDescription</key>
  <string>{desc}</string>
  <key>PayloadOrganization</key>
  <string>{org}</string>
  <key>PayloadRemovalDisallowed</key>
  <false/>
  <key>PayloadContent</key>
  <array>
{payloads}  </array>
</dict>
</plist>
"#,
        id = esc(&base),
        uuid = uuid_v4(),
        title = esc(&format!("Yunori Mail · {}", p.address)),
        desc = esc(&description),
        org = esc(p.hostname),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(dav: bool) -> String {
        render(&Profile {
            hostname: "mail.example.com",
            address: "ana@example.com",
            display_name: Some("Ana <Tanaka> & co"),
            password: "abcd efgh ijkl mnop",
            device: "Ana's iPhone",
            dav,
        })
    }

    #[test]
    fn mail_payload_uses_implicit_tls_and_the_app_password() {
        let x = sample(false);
        assert!(x.contains("<string>com.apple.mail.managed</string>"));
        assert!(
            x.contains("<key>IncomingMailServerPortNumber</key>\n      <integer>993</integer>")
        );
        assert!(
            x.contains("<key>OutgoingMailServerPortNumber</key>\n      <integer>465</integer>")
        );
        assert_eq!(x.matches("<string>abcd efgh ijkl mnop</string>").count(), 2);
        assert!(!x.contains("caldav"), "no DAV payloads unless asked");
        assert!(x.contains("<string>com.example.mail.yunori.ana-example-com</string>"));
    }

    #[test]
    fn dav_payloads_and_escaping() {
        let x = sample(true);
        assert!(x.contains("<string>com.apple.caldav.account</string>"));
        assert!(x.contains("<string>com.apple.carddav.account</string>"));
        assert_eq!(x.matches("<string>abcd efgh ijkl mnop</string>").count(), 4);
        assert!(x.contains("Ana &lt;Tanaka&gt; &amp; co"));
        assert!(!x.contains("<Tanaka>"));
        // Every payload gets its own UUID.
        let uuids: std::collections::HashSet<_> = x
            .match_indices("<key>PayloadUUID</key>")
            .map(|(i, _)| &x[i..i + 80])
            .collect();
        assert_eq!(uuids.len(), 4);
    }

    #[test]
    fn uuid_and_filename_shapes() {
        let u = uuid_v4();
        assert_eq!(u.len(), 36);
        assert_eq!(&u[14..15], "4");
        assert_eq!(filename("ana.t@example.com"), "Yunori-ana.t.mobileconfig");
        assert_eq!(filename("@x"), "Yunori-mail.mobileconfig");
    }
}
