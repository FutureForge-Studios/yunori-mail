//! The preview: which expected records a provider already holds, and which
//! Yunori would create or update. Pure functions, no I/O.
//!
//! Rules that keep it safe:
//! - Nothing is ever deleted. Where fixing a record would need a deletion
//!   (several conflicting records, a CNAME in the way), the item is `manual`
//!   and says what to remove.
//! - An update rewrites exactly one record of the same name and type.
//! - What the public-resolver check accepts, the plan leaves alone (an SPF
//!   that already authorises this server, any single DMARC record whose
//!   policy is the one Yunori is on).
//! - SPF updates keep the other senders already listed; DMARC updates keep
//!   the report addresses and change only `p=`.

use super::ProviderRecord;
use crate::dns::norm_host;
use crate::dns::records::{Expected, RecordContext, adjust_dmarc, dkim_key, squash};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Action {
    Create,
    Update,
    Unchanged,
    /// Needs a person (usually a deletion Yunori will not make).
    Manual,
    /// Not published automatically (and nothing is wrong with that).
    Skip,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanItem {
    pub id: String,
    #[serde(rename = "type")]
    pub rtype: String,
    pub name: String,
    pub required: bool,
    pub action: Action,
    /// The value Yunori writes (create/update) or expects.
    pub value: String,
    /// What the provider holds now at this name and type.
    pub current: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Provider record id an update rewrites.
    #[serde(skip)]
    pub target: Option<String>,
}

impl PlanItem {
    pub fn is_change(&self) -> bool {
        matches!(self.action, Action::Create | Action::Update)
    }
}

pub fn in_zone(name: &str, zone: &str) -> bool {
    name == zone || name.ends_with(&format!(".{zone}"))
}

/// The first `;`-separated tag of a TXT value (`v=spf1`, `v=DMARC1`, …),
/// which says which records at a name are "the same kind" as ours.
fn txt_tag(v: &str) -> Option<String> {
    let lower = v.trim().to_ascii_lowercase();
    if lower.starts_with("v=spf1") {
        return Some("v=spf1".into());
    }
    let first = lower
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_string();
    first.starts_with("v=").then_some(first)
}

fn dmarc_p(v: &str) -> Option<String> {
    v.split(';')
        .map(|p| p.trim().to_ascii_lowercase())
        .find_map(|p| p.strip_prefix("p=").map(str::to_string))
}

/// Our SPF with the other senders of an existing record kept (its `include:`,
/// `a`, `mx`, `ip4:`, `ip6:`, `exists:` mechanisms; a `redirect=` becomes an
/// `include:`). The `all` term is ours.
pub fn merge_spf(ours: &str, existing: &str) -> String {
    let mut terms: Vec<String> = ours.split_whitespace().map(str::to_string).collect();
    let all_pos = terms
        .iter()
        .position(|t| t.to_ascii_lowercase().ends_with("all"))
        .unwrap_or(terms.len());
    let mut extra = Vec::new();
    for t in existing.split_whitespace().skip(1) {
        let lower = t.to_ascii_lowercase();
        let bare = lower.trim_start_matches(['+', '-', '~', '?']);
        let keep = if let Some(target) = bare.strip_prefix("redirect=") {
            Some(format!("include:{target}"))
        } else if bare.ends_with("all") || bare.contains('=') {
            None
        } else {
            Some(t.to_string())
        };
        if let Some(k) = keep
            && !terms.iter().any(|x| x.eq_ignore_ascii_case(&k))
            && !extra.iter().any(|x: &String| x.eq_ignore_ascii_case(&k))
        {
            extra.push(k);
        }
    }
    for (i, e) in extra.into_iter().enumerate() {
        terms.insert(all_pos + i, e);
    }
    terms.join(" ")
}

/// Does an existing SPF already do the job (what `records::evaluate` passes)?
fn spf_ok(existing: &str, want: &str, ctx: &RecordContext) -> bool {
    let a = squash(existing);
    if a == squash(want) {
        return true;
    }
    let authorises = a.split_whitespace().any(|t| t == "mx" || t == "+mx")
        || ctx
            .ipv4
            .as_ref()
            .is_some_and(|ip| a.contains(&format!("ip4:{ip}")));
    let has_include = ctx
        .relay_spf_include
        .as_ref()
        .is_none_or(|i| a.contains(&format!("include:{i}")));
    authorises && has_include
}

struct Item<'a> {
    e: &'a Expected,
    name: String,
}

impl Item<'_> {
    fn out(
        &self,
        action: Action,
        value: String,
        current: &[&ProviderRecord],
        note: Option<String>,
        target: Option<&ProviderRecord>,
    ) -> PlanItem {
        PlanItem {
            id: self.e.id.clone(),
            rtype: self.e.rtype.clone(),
            name: self.name.clone(),
            required: self.e.required,
            action,
            value,
            current: current.iter().map(|r| r.value.clone()).collect(),
            note,
            target: target.map(|r| r.id.clone()),
        }
    }

    fn skip(&self, note: impl Into<String>) -> PlanItem {
        self.out(
            Action::Skip,
            self.e.value.clone(),
            &[],
            Some(note.into()),
            None,
        )
    }

    /// One record of this kind should exist: create it, rewrite the single
    /// wrong one, or ask a person to remove extras.
    fn single_slot(
        &self,
        related: &[&ProviderRecord],
        value: String,
        note: Option<String>,
    ) -> PlanItem {
        match related {
            [] => self.out(Action::Create, value, related, note, None),
            [one] => self.out(Action::Update, value, related, note, Some(one)),
            many => self.out(
                Action::Manual,
                value,
                related,
                Some(format!(
                    "There are {} {} records named {} and none is the one this server needs. Remove the ones you no longer use at your DNS provider, then publish again.",
                    many.len(),
                    self.e.rtype,
                    self.name
                )),
                None,
            ),
        }
    }

    fn unchanged_or_unproxy(&self, rec: &ProviderRecord, current: &[&ProviderRecord]) -> PlanItem {
        if rec.proxied {
            self.out(
                Action::Update,
                self.e.value.clone(),
                current,
                Some("Turns off Cloudflare's proxy (the orange cloud): mail apps and other servers must reach this server directly.".into()),
                Some(rec),
            )
        } else {
            self.out(Action::Unchanged, self.e.value.clone(), current, None, None)
        }
    }
}

fn plan_one(
    e: &Expected,
    existing: &[ProviderRecord],
    zone: &str,
    ctx: &RecordContext,
) -> PlanItem {
    let it = Item {
        e,
        name: norm_host(&e.name),
    };
    let name = it.name.as_str();
    if e.value.is_empty() || name.starts_with('*') {
        return it
            .skip("The mail engine is still generating DKIM keys. Publish again in a minute.");
    }
    if !in_zone(name, zone) {
        return it.skip(format!(
            "{name} is not in the {zone} zone. Publish it where that name is hosted."
        ));
    }
    if e.id == "aaaa" && !e.required {
        return it.skip(
            "Optional, and left to you: publish an IPv6 address only once reverse DNS for IPv6 is set too.",
        );
    }
    let at_name: Vec<&ProviderRecord> = existing.iter().filter(|r| r.name == name).collect();
    let same: Vec<&ProviderRecord> = at_name
        .iter()
        .copied()
        .filter(|r| r.rtype == e.rtype)
        .collect();

    if e.rtype != "CNAME"
        && let Some(c) = at_name.iter().find(|r| r.rtype == "CNAME")
    {
        return it.out(
            Action::Manual,
            e.value.clone(),
            &[*c],
            Some(format!(
                "{name} is a CNAME (to {}), which cannot sit beside other records. Remove it at your DNS provider, then publish again.",
                c.value
            )),
            None,
        );
    }

    match e.rtype.as_str() {
        "A" | "AAAA" => {
            if let Some(m) = same.iter().find(|r| r.value == e.value) {
                let mut item = it.unchanged_or_unproxy(m, &same);
                if same.len() > 1 && item.action == Action::Unchanged {
                    item.note = Some("Other addresses are listed at this name too; remove them at your DNS provider so mail always reaches this server.".into());
                }
                return item;
            }
            it.single_slot(&same, e.value.clone(), None)
        }
        "CNAME" => {
            let want = norm_host(&e.value);
            if let Some(m) = same.iter().find(|r| r.value == want) {
                return it.unchanged_or_unproxy(m, &same);
            }
            let others: Vec<&ProviderRecord> = at_name
                .iter()
                .copied()
                .filter(|r| r.rtype != "CNAME")
                .collect();
            if !e.id.starts_with("relay:")
                && let Some(ip) = &ctx.ipv4
                && others
                    .iter()
                    .any(|r| r.rtype == "A" && &r.value == ip && !r.proxied)
            {
                return it.out(
                    Action::Unchanged,
                    e.value.clone(),
                    &others,
                    Some("An A record pointing at this server does the same job.".into()),
                    None,
                );
            }
            if !others.is_empty() {
                let types: Vec<&str> = others.iter().map(|r| r.rtype.as_str()).collect();
                return it.out(
                    Action::Manual,
                    e.value.clone(),
                    &others,
                    Some(format!(
                        "{name} already has {} record{}, and a CNAME cannot sit beside other records. Remove {} at your DNS provider if you no longer need {}, then publish again.",
                        types.join(", "),
                        if types.len() == 1 { "" } else { "s" },
                        if types.len() == 1 { "it" } else { "them" },
                        if types.len() == 1 { "it" } else { "them" },
                    )),
                    None,
                );
            }
            it.single_slot(&same, want, None)
        }
        "MX" => {
            let want = e
                .value
                .split_whitespace()
                .last()
                .map(norm_host)
                .unwrap_or_default();
            let host = |r: &ProviderRecord| {
                r.value
                    .split_whitespace()
                    .last()
                    .map(norm_host)
                    .unwrap_or_default()
            };
            if same.iter().any(|r| host(r) == want) {
                let others = same.iter().any(|r| host(r) != want);
                return it.out(
                    Action::Unchanged,
                    e.value.clone(),
                    &same,
                    others.then(|| "Other MX hosts are listed too; mail may be delivered to them instead. Remove them at your DNS provider when you are ready.".into()),
                    None,
                );
            }
            let note = (same.len() == 1).then(|| {
                format!(
                    "Mail for {name} currently goes to {}. After this change it comes to this server.",
                    host(same[0])
                )
            });
            it.single_slot(&same, e.value.clone(), note)
        }
        "TXT" => plan_txt(&it, &same, ctx),
        "SRV" => {
            let parts: Vec<&str> = e.value.split_whitespace().collect();
            let (port, target) = (parts.get(2).copied(), parts.get(3).map(|t| norm_host(t)));
            let matches = |r: &ProviderRecord| {
                let p: Vec<&str> = r.value.split_whitespace().collect();
                p.get(2).copied() == port && p.get(3).map(|t| norm_host(t)) == target
            };
            if same.iter().any(|r| matches(r)) {
                return it.out(Action::Unchanged, e.value.clone(), &same, None, None);
            }
            it.single_slot(&same, e.value.clone(), None)
        }
        "CAA" => {
            let norm = |v: &str| squash(&v.replace('"', ""));
            if same.iter().any(|r| norm(&r.value) == norm(&e.value)) {
                return it.out(Action::Unchanged, e.value.clone(), &same, None, None);
            }
            if same.is_empty() {
                return it.skip("No CAA records exist, so every certificate authority may already issue for this domain. Nothing to add.");
            }
            it.out(
                Action::Create,
                e.value.clone(),
                &same,
                Some(
                    "Adds this server's certificate authority to the ones already allowed.".into(),
                ),
                None,
            )
        }
        other => it.skip(format!(
            "Yunori does not publish {other} records automatically."
        )),
    }
}

fn plan_txt(it: &Item, same: &[&ProviderRecord], ctx: &RecordContext) -> PlanItem {
    let e = it.e;
    let tag = txt_tag(&e.value);
    let related: Vec<&ProviderRecord> = same
        .iter()
        .copied()
        .filter(|r| match &tag {
            Some(t) => txt_tag(&r.value).as_ref() == Some(t),
            None => squash(&r.value) == squash(&e.value),
        })
        .collect();

    if e.id.starts_with("dkim") || e.id == "relay:dkim" {
        let want = dkim_key(&e.value);
        if related.iter().any(|r| dkim_key(&r.value) == want) {
            return it.out(Action::Unchanged, e.value.clone(), &related, None, None);
        }
        return it.single_slot(&related, e.value.clone(), None);
    }
    if e.id == "spf" {
        return match related.as_slice() {
            [] => it.out(Action::Create, e.value.clone(), &related, None, None),
            [one] if spf_ok(&one.value, &e.value, ctx) => {
                it.out(Action::Unchanged, e.value.clone(), &related, None, None)
            }
            [one] => {
                let merged = merge_spf(&e.value, &one.value);
                let kept = merged != e.value;
                it.out(
                    Action::Update,
                    merged,
                    &related,
                    kept.then(|| "Keeps the other senders your record already lists.".into()),
                    Some(one),
                )
            }
            _ => it.out(
                Action::Manual,
                e.value.clone(),
                &related,
                Some("There are several SPF records, so receivers ignore all of them. Merge them into one at your DNS provider (Yunori never deletes records), then publish again.".into()),
                None,
            ),
        };
    }
    if e.id == "dmarc" {
        let want_p = dmarc_p(&e.value);
        return match related.as_slice() {
            [] => it.out(Action::Create, e.value.clone(), &related, None, None),
            [one] if dmarc_p(&one.value) == want_p => {
                it.out(Action::Unchanged, e.value.clone(), &related, None, None)
            }
            [one] => {
                let policy = want_p.clone().unwrap_or_else(|| "none".into());
                it.out(
                    Action::Update,
                    adjust_dmarc(&one.value, ctx.dmarc_policy),
                    &related,
                    Some(format!(
                        "Changes the policy from p={} to p={policy}, the DMARC step this server is on. Your report addresses stay.",
                        dmarc_p(&one.value).unwrap_or_else(|| "(none)".into())
                    )),
                    Some(one),
                )
            }
            _ => it.out(
                Action::Manual,
                e.value.clone(),
                &related,
                Some("There are several DMARC records; receivers then ignore them. Keep one at your DNS provider, then publish again.".into()),
                None,
            ),
        };
    }
    if related.iter().any(|r| squash(&r.value) == squash(&e.value)) {
        return it.out(Action::Unchanged, e.value.clone(), &related, None, None);
    }
    it.single_slot(&related, e.value.clone(), None)
}

/// The preview for one zone.
pub fn plan(
    expected: &[Expected],
    existing: &[ProviderRecord],
    zone: &str,
    ctx: &RecordContext,
) -> Vec<PlanItem> {
    expected
        .iter()
        .filter(|e| e.rtype != "PTR")
        .map(|e| plan_one(e, existing, zone, ctx))
        .collect()
}

/// A short fingerprint of the changes, so "publish" can refuse to act on a
/// preview that no longer matches what the provider holds.
pub fn fingerprint(items: &[PlanItem]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    for i in items.iter().filter(|i| i.is_change()) {
        h.update(format!(
            "{}|{:?}|{}|{}|{}\n",
            i.id,
            i.action,
            i.name,
            i.value,
            i.target.as_deref().unwrap_or("")
        ));
    }
    hex::encode(&h.finalize()[..8])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dns::records::expected_records;
    use crate::model::DmarcPolicy;

    const ZONE_FILE: &str = r#"example.com. IN MX 10 mail.example.com.
example.com. IN TXT "v=spf1 mx ra=postmaster -all"
mail.example.com. IN TXT "v=spf1 a -all"
k1._domainkey.example.com. IN TXT "v=DKIM1; k=ed25519; p=KEYONE"
_dmarc.example.com. IN TXT "v=DMARC1; p=reject; rua=mailto:postmaster@example.com"
_mta-sts.example.com. IN TXT "v=STSv1; id=1"
_imaps._tcp.example.com. IN SRV 0 1 993 mail.example.com.
autoconfig.example.com. IN CNAME mail.example.com.
example.com. IN CAA 0 issue "letsencrypt.org"
"#;

    fn ctx() -> RecordContext {
        RecordContext {
            hostname: "mail.example.com".into(),
            ipv4: Some("203.0.113.10".into()),
            ipv6: Some("2001:db8::10".into()),
            is_primary: true,
            relay_spf_include: None,
            relay_records: vec![],
            dmarc_policy: DmarcPolicy::None,
        }
    }

    fn rec(id: &str, t: &str, name: &str, value: &str) -> ProviderRecord {
        ProviderRecord {
            id: id.into(),
            rtype: t.into(),
            name: name.into(),
            value: value.into(),
            proxied: false,
        }
    }

    fn by_id<'a>(items: &'a [PlanItem], id: &str) -> &'a PlanItem {
        items
            .iter()
            .find(|i| i.id == id)
            .unwrap_or_else(|| panic!("no item {id}"))
    }

    fn run(existing: &[ProviderRecord]) -> Vec<PlanItem> {
        let c = ctx();
        plan(
            &expected_records("example.com", ZONE_FILE, &c),
            existing,
            "example.com",
            &c,
        )
    }

    #[test]
    fn empty_zone_creates_everything_it_may() {
        let items = run(&[]);
        for id in [
            "a",
            "mx",
            "spf",
            "spf-host",
            "dkim:k1",
            "dmarc",
            "mta-sts",
            "srv:_imaps._tcp",
            "autoconfig",
        ] {
            assert_eq!(by_id(&items, id).action, Action::Create, "{id}");
        }
        // Optional IPv6 (needs IPv6 reverse DNS) and CAA (nothing restricts CAs yet) are left alone.
        assert_eq!(by_id(&items, "aaaa").action, Action::Skip);
        assert_eq!(by_id(&items, "caa").action, Action::Skip);
        // The DMARC ladder starts at p=none whatever the engine proposes.
        assert!(by_id(&items, "dmarc").value.contains("p=none"));
    }

    #[test]
    fn correct_records_are_unchanged_and_wrong_ones_updated_in_place() {
        let existing = vec![
            rec("1", "A", "mail.example.com", "203.0.113.10"),
            rec("2", "MX", "example.com", "10 mail.example.com"),
            rec(
                "3",
                "TXT",
                "k1._domainkey.example.com",
                "v=DKIM1; k=ed25519; p=OLDKEY",
            ),
            rec("4", "TXT", "example.com", "google-site-verification=abc"),
            rec(
                "5",
                "SRV",
                "_imaps._tcp.example.com",
                "0 1 993 mail.example.com",
            ),
            rec("6", "TXT", "_mta-sts.example.com", "v=STSv1; id=0"),
        ];
        let items = run(&existing);
        assert_eq!(by_id(&items, "a").action, Action::Unchanged);
        assert_eq!(by_id(&items, "mx").action, Action::Unchanged);
        assert_eq!(by_id(&items, "srv:_imaps._tcp").action, Action::Unchanged);
        let dkim = by_id(&items, "dkim:k1");
        assert_eq!(
            (dkim.action, dkim.target.as_deref()),
            (Action::Update, Some("3"))
        );
        let sts = by_id(&items, "mta-sts");
        assert_eq!(
            (sts.action, sts.target.as_deref()),
            (Action::Update, Some("6"))
        );
        // An unrelated TXT at the apex is not ours to touch: SPF is created beside it.
        let spf = by_id(&items, "spf");
        assert_eq!((spf.action, spf.target.as_deref()), (Action::Create, None));
        assert!(spf.current.is_empty());
    }

    #[test]
    fn spf_keeps_other_senders_and_accepts_one_that_already_works() {
        let items = run(&[rec(
            "1",
            "TXT",
            "example.com",
            "v=spf1 include:_spf.google.com ~all",
        )]);
        let spf = by_id(&items, "spf");
        assert_eq!(spf.action, Action::Update);
        assert_eq!(
            spf.value,
            "v=spf1 mx ra=postmaster include:_spf.google.com -all"
        );
        assert!(spf.note.as_deref().unwrap().contains("other senders"));

        let items = run(&[rec(
            "1",
            "TXT",
            "example.com",
            "v=spf1 mx include:_spf.google.com ~all",
        )]);
        assert_eq!(by_id(&items, "spf").action, Action::Unchanged);

        let items = run(&[
            rec("1", "TXT", "example.com", "v=spf1 a -all"),
            rec("2", "TXT", "example.com", "v=spf1 include:x.example -all"),
        ]);
        assert_eq!(by_id(&items, "spf").action, Action::Manual);
    }

    #[test]
    fn merge_spf_cases() {
        assert_eq!(
            merge_spf("v=spf1 mx -all", "v=spf1 redirect=_spf.host.example"),
            "v=spf1 mx include:_spf.host.example -all"
        );
        assert_eq!(
            merge_spf("v=spf1 mx -all", "v=spf1 +mx ip4:192.0.2.1 ?all exp=x"),
            "v=spf1 mx +mx ip4:192.0.2.1 -all"
        );
        assert_eq!(
            merge_spf(
                "v=spf1 mx include:amazonses.com -all",
                "v=spf1 include:amazonses.com -all"
            ),
            "v=spf1 mx include:amazonses.com -all"
        );
    }

    #[test]
    fn dmarc_changes_only_the_policy() {
        let items = run(&[rec(
            "9",
            "TXT",
            "_dmarc.example.com",
            "v=DMARC1; p=reject; rua=mailto:reports@dmarc.example",
        )]);
        let d = by_id(&items, "dmarc");
        assert_eq!(d.action, Action::Update);
        assert_eq!(
            d.value,
            "v=DMARC1; p=none; rua=mailto:reports@dmarc.example"
        );
        assert!(d.note.as_deref().unwrap().contains("p=reject to p=none"));

        let items = run(&[rec(
            "9",
            "TXT",
            "_dmarc.example.com",
            "v=DMARC1; p=none; rua=mailto:x@y.example",
        )]);
        assert_eq!(by_id(&items, "dmarc").action, Action::Unchanged);
    }

    #[test]
    fn never_plans_a_deletion() {
        // Two A records, neither ours: a person decides which goes.
        let items = run(&[
            rec("1", "A", "mail.example.com", "192.0.2.1"),
            rec("2", "A", "mail.example.com", "192.0.2.2"),
        ]);
        let a = by_id(&items, "a");
        assert_eq!(a.action, Action::Manual);
        assert_eq!(a.current.len(), 2);

        // MX elsewhere (a single one) is rewritten, with the consequence spelled out.
        let items = run(&[rec("1", "MX", "example.com", "1 aspmx.l.google.com")]);
        let mx = by_id(&items, "mx");
        assert_eq!(mx.action, Action::Update);
        assert!(mx.note.as_deref().unwrap().contains("aspmx.l.google.com"));

        // Several MX elsewhere: manual.
        let items = run(&[
            rec("1", "MX", "example.com", "1 aspmx.l.google.com"),
            rec("2", "MX", "example.com", "5 alt1.aspmx.l.google.com"),
        ]);
        assert_eq!(by_id(&items, "mx").action, Action::Manual);
    }

    #[test]
    fn cname_rules() {
        // Our host already resolves via an A record: fine as it is.
        let items = run(&[rec("1", "A", "autoconfig.example.com", "203.0.113.10")]);
        assert_eq!(by_id(&items, "autoconfig").action, Action::Unchanged);
        // Something else lives there: a CNAME cannot be added beside it.
        let items = run(&[rec("1", "TXT", "autoconfig.example.com", "hello")]);
        assert_eq!(by_id(&items, "autoconfig").action, Action::Manual);
        // A CNAME in the way of a TXT.
        let items = run(&[rec(
            "1",
            "CNAME",
            "_mta-sts.example.com",
            "elsewhere.example",
        )]);
        assert_eq!(by_id(&items, "mta-sts").action, Action::Manual);
        // Wrong CNAME target: rewritten.
        let items = run(&[rec(
            "1",
            "CNAME",
            "autoconfig.example.com",
            "old.example.net",
        )]);
        assert_eq!(by_id(&items, "autoconfig").action, Action::Update);
    }

    #[test]
    fn proxied_host_record_is_switched_to_dns_only() {
        let mut r = rec("1", "A", "mail.example.com", "203.0.113.10");
        r.proxied = true;
        let items = run(&[r]);
        let a = by_id(&items, "a");
        assert_eq!((a.action, a.target.as_deref()), (Action::Update, Some("1")));
        assert!(a.note.as_deref().unwrap().contains("orange cloud"));
    }

    #[test]
    fn caa_is_added_only_beside_existing_caa() {
        let items = run(&[rec("1", "CAA", "example.com", "0 issue \"digicert.com\"")]);
        assert_eq!(by_id(&items, "caa").action, Action::Create);
        let items = run(&[rec(
            "1",
            "CAA",
            "example.com",
            "0 issue \"letsencrypt.org\"",
        )]);
        assert_eq!(by_id(&items, "caa").action, Action::Unchanged);
    }

    #[test]
    fn names_outside_the_zone_and_pending_dkim_are_skipped() {
        let c = RecordContext {
            hostname: "mail.other.net".into(),
            ..ctx()
        };
        let exp = expected_records("example.com", "example.com. IN MX 10 mail.other.net.\n", &c);
        let items = plan(&exp, &[], "example.com", &c);
        assert_eq!(by_id(&items, "a").action, Action::Skip);
        assert_eq!(by_id(&items, "dkim").action, Action::Skip);
        assert_eq!(by_id(&items, "mx").action, Action::Create);
    }

    #[test]
    fn fingerprint_tracks_changes_only() {
        let a = run(&[]);
        let b = run(&[rec(
            "1",
            "TXT",
            "example.com",
            "google-site-verification=abc",
        )]);
        assert_eq!(fingerprint(&a), fingerprint(&b));
        let c = run(&[rec("1", "MX", "example.com", "1 aspmx.l.google.com")]);
        assert_ne!(fingerprint(&a), fingerprint(&c));
    }
}
