//! Thunderbird-style autoconfig with the Yunori marker. The Yunori client's
//! discovery maps `<displayShortName>Yunori</displayShortName>` from a domain's
//! own autoconfig to its `yunori` provider preset (see
//! `server/services/discovery.ts` in the client). Calendars and contacts use
//! the same element shape Mail-in-a-Box ships (`addressbook`/`calendar`, after
//! the provider), pointing at the well-known DAV paths the edge proxies.

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

pub fn render(domain: &str, hostname: &str) -> String {
    let d = xml_escape(domain);
    let h = xml_escape(hostname);
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<clientConfig version="1.1">
  <emailProvider id="{d}">
    <domain>{d}</domain>
    <displayName>Yunori Mail ({d})</displayName>
    <displayShortName>Yunori</displayShortName>
    <incomingServer type="imap">
      <hostname>{h}</hostname>
      <port>993</port>
      <socketType>SSL</socketType>
      <username>%EMAILADDRESS%</username>
      <authentication>password-cleartext</authentication>
    </incomingServer>
    <outgoingServer type="smtp">
      <hostname>{h}</hostname>
      <port>465</port>
      <socketType>SSL</socketType>
      <username>%EMAILADDRESS%</username>
      <authentication>password-cleartext</authentication>
    </outgoingServer>
    <outgoingServer type="smtp">
      <hostname>{h}</hostname>
      <port>587</port>
      <socketType>STARTTLS</socketType>
      <username>%EMAILADDRESS%</username>
      <authentication>password-cleartext</authentication>
    </outgoingServer>
    <documentation url="https://{h}/account">
      <descr lang="en">Yunori Mail account portal</descr>
    </documentation>
  </emailProvider>
  <addressbook type="carddav">
    <username>%EMAILADDRESS%</username>
    <authentication system="http">basic</authentication>
    <url>https://{h}/.well-known/carddav</url>
  </addressbook>
  <calendar type="caldav">
    <username>%EMAILADDRESS%</username>
    <authentication system="http">basic</authentication>
    <url>https://{h}/.well-known/caldav</url>
  </calendar>
</clientConfig>
"#
    )
}

/// Domain part of an `emailaddress` query value, lower-cased.
pub fn domain_of(email: &str) -> Option<String> {
    email
        .rsplit_once('@')
        .map(|(_, d)| d.trim().to_ascii_lowercase())
        .filter(|d| !d.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn carries_yunori_marker_and_ssl_ports() {
        let x = render("example.com", "mail.example.com");
        assert!(x.contains("<displayShortName>Yunori</displayShortName>"));
        assert!(x.contains("<port>993</port>"));
        assert!(x.contains("<port>465</port>"));
        assert!(x.contains("https://mail.example.com/account"));
        assert!(x.contains("<url>https://mail.example.com/.well-known/carddav</url>"));
        assert!(x.contains("<url>https://mail.example.com/.well-known/caldav</url>"));
        // The DAV elements sit after the provider, so a reader that only
        // knows `emailProvider` is unaffected.
        assert!(x.find("</emailProvider>") < x.find("<addressbook"));
    }

    #[test]
    fn escapes_hostile_input() {
        let x = render("ex<ample>.com", "mail.example.com");
        assert!(!x.contains("<ample>"));
    }

    #[test]
    fn extracts_domain() {
        assert_eq!(domain_of("Ana@Example.COM").as_deref(), Some("example.com"));
        assert_eq!(domain_of("nope"), None);
    }
}
