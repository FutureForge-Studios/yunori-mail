use super::*;

fn folder(name: &str, attrs: &[&str], messages: u64) -> SourceFolder {
    SourceFolder {
        raw: imap::encode_mutf7(name),
        name: name.to_string(),
        delimiter: Some('/'),
        attrs: attrs.iter().map(|a| a.to_ascii_lowercase()).collect(),
        messages,
    }
}

fn plan_of<'a>(plans: &'a [FolderPlan], name: &str) -> &'a FolderPlan {
    plans
        .iter()
        .find(|p| p.name == name)
        .unwrap_or_else(|| panic!("no plan for {name}"))
}

fn role(r: &str) -> Target {
    Target::Role { role: r.into() }
}

#[test]
fn gmail_folders_map_to_special_mailboxes_and_skip_virtual_ones() {
    let plans = plan_folders(&[
        folder("INBOX", &["\\HasNoChildren"], 10),
        folder("Receipts", &[], 3),
        folder("Work/Clients/Acme", &[], 2),
        folder("[Gmail]", &["\\HasChildren", "\\Noselect"], 0),
        folder("[Gmail]/All Mail", &["\\All"], 50),
        folder("[Gmail]/Drafts", &["\\Drafts"], 1),
        folder("[Gmail]/Important", &["\\Important"], 9),
        folder("[Gmail]/Sent Mail", &["\\Sent"], 7),
        folder("[Gmail]/Spam", &["\\Junk"], 4),
        folder("[Gmail]/Starred", &["\\Flagged"], 2),
        folder("[Gmail]/Trash", &["\\Trash"], 5),
    ]);
    assert!(
        plans.iter().all(|p| p.name != "[Gmail]"),
        "\\Noselect is not offered"
    );
    assert_eq!(plan_of(&plans, "INBOX").target, role("inbox"));
    assert_eq!(plan_of(&plans, "[Gmail]/Sent Mail").target, role("sent"));
    assert_eq!(plan_of(&plans, "[Gmail]/Drafts").target, role("drafts"));
    assert_eq!(plan_of(&plans, "[Gmail]/Spam").target, role("junk"));
    assert_eq!(plan_of(&plans, "[Gmail]/Trash").target, role("trash"));
    assert_eq!(plan_of(&plans, "[Gmail]/Sent Mail").destination, "Sent");
    for virt in ["[Gmail]/All Mail", "[Gmail]/Important", "[Gmail]/Starred"] {
        let p = plan_of(&plans, virt);
        assert!(!p.selected && p.note.is_some(), "{virt} is not selected");
    }
    let acme = plan_of(&plans, "Work/Clients/Acme");
    assert_eq!(
        acme.target,
        Target::Path {
            under_inbox: false,
            parts: vec!["Work".into(), "Clients".into(), "Acme".into()]
        }
    );
    assert_eq!(acme.destination, "Work / Clients / Acme");
    assert!(acme.selected);
}

#[test]
fn virtual_folders_are_recognised_without_special_use_too() {
    let plans = plan_folders(&[
        folder("INBOX", &[], 1),
        folder("[Google Mail]/All Mail", &[], 5),
        folder("[Google Mail]/Sent Mail", &[], 2),
    ]);
    assert!(!plan_of(&plans, "[Google Mail]/All Mail").selected);
    assert_eq!(
        plan_of(&plans, "[Google Mail]/Sent Mail").target,
        role("sent")
    );
}

#[test]
fn outlook_and_dovecot_names() {
    let mut f = vec![
        folder("Inbox", &[], 3),
        folder("Sent Items", &["\\Sent"], 2),
        folder("Deleted Items", &["\\Trash"], 1),
        folder("Junk Email", &["\\Junk"], 1),
        folder("Archive", &["\\Archive"], 1),
        folder("Drafts", &["\\Drafts"], 0),
        folder("Conversation History", &[], 0),
    ];
    // Dovecot-style: '.' delimiter, folders under INBOX, names only.
    let dot = |n: &str| SourceFolder {
        delimiter: Some('.'),
        ..folder(n, &[], 1)
    };
    f.push(dot("INBOX.Receipts"));
    f.push(dot("INBOX.Receipts.2024"));
    f.push(dot("Projects.Sent"));
    let plans = plan_folders(&f);
    assert_eq!(plan_of(&plans, "Inbox").target, role("inbox"));
    assert_eq!(plan_of(&plans, "Sent Items").target, role("sent"));
    assert_eq!(plan_of(&plans, "Deleted Items").target, role("trash"));
    assert_eq!(plan_of(&plans, "Junk Email").target, role("junk"));
    assert_eq!(plan_of(&plans, "Archive").target, role("archive"));
    assert_eq!(
        plan_of(&plans, "INBOX.Receipts.2024").target,
        Target::Path {
            under_inbox: true,
            parts: vec!["Receipts".into(), "2024".into()]
        }
    );
    assert_eq!(
        plan_of(&plans, "INBOX.Receipts.2024").destination,
        "Inbox / Receipts / 2024"
    );
    // A nested "Sent" is just a folder, not the Sent mailbox.
    assert_eq!(
        plan_of(&plans, "Projects.Sent").target,
        Target::Path {
            under_inbox: false,
            parts: vec!["Projects".into(), "Sent".into()]
        }
    );
}

#[test]
fn names_find_roles_once_and_attributes_win() {
    let plans = plan_folders(&[
        folder("INBOX", &[], 1),
        folder("Sent", &[], 1),                  // a name match…
        folder("Sent Messages", &["\\Sent"], 1), // …but this one says it is Sent
        folder("Spam", &[], 1),
        folder("Junk", &[], 1),
        folder("a/b", &[], 1),
    ]);
    assert_eq!(plan_of(&plans, "Sent Messages").target, role("sent"));
    assert!(matches!(
        plan_of(&plans, "Sent").target,
        Target::Path { .. }
    ));
    let junk: Vec<_> = plans.iter().filter(|p| p.target == role("junk")).collect();
    assert_eq!(junk.len(), 1, "one Junk");
}

#[test]
fn folder_names_are_cleaned_for_the_destination() {
    let plans = plan_folders(&[SourceFolder {
        delimiter: Some('.'),
        ..folder("Bills/Paid.\u{7}Old ", &[], 1)
    }]);
    assert_eq!(
        plan_of(&plans, "Bills/Paid.\u{7}Old ").target,
        Target::Path {
            under_inbox: false,
            parts: vec!["Bills-Paid".into(), "Old".into()]
        }
    );
}

#[test]
fn flags_become_keywords() {
    let f = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert_eq!(
        keywords(&f(&[
            "\\Seen",
            "\\Flagged",
            "\\Answered",
            "\\Draft",
            "\\Recent",
            "\\Deleted",
            "$Forwarded",
            "$label1",
            "\\SEEN"
        ])),
        vec![
            "$seen",
            "$flagged",
            "$answered",
            "$draft",
            "$forwarded",
            "$label1"
        ]
    );
    assert!(keywords(&f(&["bad(word", ""])).is_empty());
}

#[test]
fn message_ids_are_found_and_normalised() {
    assert_eq!(
        message_id(b"Message-ID: <ABC@Example.COM>\r\n\r\n").as_deref(),
        Some("abc@example.com")
    );
    assert_eq!(
        message_id(b"message-id:\r\n <folded@x>\r\n\r\n").as_deref(),
        Some("folded@x")
    );
    assert_eq!(message_id(b"\r\n"), None);
    assert_eq!(message_id(b"Message-ID: <>\r\n"), None);
}

#[test]
fn only_public_addresses_are_allowed() {
    for bad in [
        "127.0.0.1",
        "10.1.2.3",
        "172.16.0.1",
        "192.168.1.1",
        "169.254.169.254",
        "100.64.0.1",
        "0.0.0.0",
        "224.0.0.1",
        "255.255.255.255",
        "192.0.2.1",
        "198.18.0.1",
        "::1",
        "::",
        "fc00::1",
        "fd12::1",
        "fe80::1",
        "::ffff:127.0.0.1",
        "::ffff:10.0.0.1",
        "64:ff9b::a00:1",
        "2001:db8::1",
    ] {
        assert!(!is_public(bad.parse().unwrap()), "{bad} must be refused");
    }
    for good in [
        "142.250.1.109",
        "40.97.1.1",
        "2a00:1450:4001::6d",
        "::ffff:8.8.8.8",
    ] {
        assert!(is_public(good.parse().unwrap()), "{good} is public");
    }
}

#[tokio::test]
async fn private_hosts_are_refused_before_connecting() {
    let err = resolve("localhost", 993, &NetPolicy::default())
        .await
        .unwrap_err();
    assert!(err.to_string().contains("private or local"), "{err}");
    let err = resolve("127.0.0.1", 993, &NetPolicy::default())
        .await
        .unwrap_err();
    assert!(err.to_string().contains("private or local"), "{err}");
    let ok = resolve(
        "127.0.0.1",
        993,
        &NetPolicy {
            allow_private: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(ok.port(), 993);
}

#[test]
fn source_input_is_validated() {
    let ok = validate_source(" IMAP.Gmail.com. ", 993, "me@gmail.com", "pw").unwrap();
    assert_eq!(ok, ("imap.gmail.com".to_string(), Security::Implicit));
    assert_eq!(
        validate_source("imap.example.com", 143, "me", "pw")
            .unwrap()
            .1,
        Security::StartTls
    );
    for port in [25, 110, 465, 587, 8080, 0] {
        assert_eq!(
            validate_source("imap.example.com", port, "me", "pw")
                .unwrap_err()
                .field,
            "port"
        );
    }
    for host in [
        "",
        "localhost",
        "bad host.com",
        "-x.com",
        "a..b",
        "http://x.com",
    ] {
        assert_eq!(
            validate_source(host, 993, "me", "pw").unwrap_err().field,
            "host",
            "{host}"
        );
    }
    assert_eq!(
        validate_source("imap.example.com", 993, " ", "pw")
            .unwrap_err()
            .field,
        "username"
    );
    assert_eq!(
        validate_source("imap.example.com", 993, "me\r\nA1 LOGOUT", "pw")
            .unwrap_err()
            .field,
        "username"
    );
    assert_eq!(
        validate_source("imap.example.com", 993, "me", "")
            .unwrap_err()
            .field,
        "password"
    );
}

// ── the runner, against an in-memory source and the mock engine ──────────

#[derive(Clone)]
struct MemFolder {
    folder: SourceFolder,
    uidvalidity: u64,
    messages: Vec<(SourceMessage, Vec<u8>)>,
}

#[derive(Default)]
struct MemState {
    folders: Vec<MemFolder>,
    /// Drop the connection after this many bodies (once).
    fail_after_bodies: Option<usize>,
    bodies_served: usize,
    connects: usize,
    wrong_password: bool,
}

#[derive(Clone, Default)]
struct MemConnector(Arc<Mutex<MemState>>);

struct MemSource {
    state: Arc<Mutex<MemState>>,
    open: Option<usize>,
}

#[async_trait]
impl SourceConnector for MemConnector {
    async fn connect(&self) -> Result<Box<dyn MailSource>> {
        let mut s = self.0.lock().unwrap();
        if s.wrong_password {
            return Err(anyhow!("The old server refused the username or password."));
        }
        s.connects += 1;
        Ok(Box::new(MemSource {
            state: self.0.clone(),
            open: None,
        }))
    }
}

#[async_trait]
impl MailSource for MemSource {
    async fn folders(&mut self) -> Result<Vec<SourceFolder>> {
        let s = self.state.lock().unwrap();
        Ok(s.folders
            .iter()
            .map(|f| SourceFolder {
                messages: f.messages.len() as u64,
                ..f.folder.clone()
            })
            .collect())
    }
    async fn open(&mut self, raw: &[u8]) -> Result<(u64, u64)> {
        let s = self.state.lock().unwrap();
        let i = s
            .folders
            .iter()
            .position(|f| f.folder.raw == raw)
            .ok_or_else(|| anyhow::Error::from(imap::Refused("no such folder".into())))?;
        self.open = Some(i);
        Ok((s.folders[i].messages.len() as u64, s.folders[i].uidvalidity))
    }
    async fn uids(&mut self) -> Result<Vec<u32>> {
        let s = self.state.lock().unwrap();
        Ok(s.folders[self.open.unwrap()]
            .messages
            .iter()
            .map(|m| m.0.uid)
            .collect())
    }
    async fn metadata(&mut self, uids: &[u32]) -> Result<Vec<SourceMessage>> {
        let s = self.state.lock().unwrap();
        Ok(s.folders[self.open.unwrap()]
            .messages
            .iter()
            .filter(|m| uids.contains(&m.0.uid))
            .map(|m| m.0.clone())
            .collect())
    }
    async fn body(&mut self, uid: u32) -> Result<Option<Vec<u8>>> {
        let mut s = self.state.lock().unwrap();
        if let Some(n) = s.fail_after_bodies
            && s.bodies_served >= n
        {
            s.fail_after_bodies = None;
            return Err(anyhow!("connection reset"));
        }
        s.bodies_served += 1;
        Ok(s.folders[self.open.unwrap()]
            .messages
            .iter()
            .find(|m| m.0.uid == uid)
            .map(|m| m.1.clone()))
    }
    async fn close(self: Box<Self>) {}
}

fn msg(uid: u32, flags: &[&str], id: &str, size: usize) -> (SourceMessage, Vec<u8>) {
    let body = format!(
        "Message-ID: <{id}>\r\nSubject: {uid}\r\n\r\n{}",
        "x".repeat(size)
    );
    (
        SourceMessage {
            uid,
            flags: flags.iter().map(|f| f.to_string()).collect(),
            internal_date: Some(format!("2021-0{}-01T10:00:00Z", (uid % 9) + 1)),
            size: body.len() as u64,
            message_id: Some(id.to_ascii_lowercase()),
        },
        body.into_bytes(),
    )
}

fn source() -> MemConnector {
    let c = MemConnector::default();
    c.0.lock().unwrap().folders = vec![
        MemFolder {
            folder: folder("INBOX", &[], 0),
            uidvalidity: 1,
            messages: vec![
                msg(1, &["\\Seen"], "a@x", 10),
                msg(2, &["\\Seen", "\\Flagged"], "b@x", 10),
                msg(3, &[], "c@x", 10),
            ],
        },
        MemFolder {
            folder: folder("Sent Items", &["\\Sent"], 0),
            uidvalidity: 5,
            messages: vec![msg(1, &["\\Seen", "\\Answered"], "s1@x", 10)],
        },
        MemFolder {
            folder: folder("Projects/2024", &[], 0),
            uidvalidity: 9,
            messages: vec![msg(10, &["\\Draft"], "p@x", 10), msg(11, &[], "q@x", 10)],
        },
        MemFolder {
            folder: folder("[Gmail]/All Mail", &["\\All"], 0),
            uidvalidity: 2,
            messages: vec![msg(1, &[], "a@x", 10)],
        },
    ];
    c
}

fn spec() -> SourceSpec {
    SourceSpec {
        host: "imap.example.com".into(),
        port: 993,
        username: "me@example.com".into(),
        password: Zeroizing::new("old-secret".into()),
    }
}

fn request(account: &str, folders: Option<Vec<String>>, dry_run: bool) -> JobRequest {
    JobRequest {
        account: account.into(),
        actor: account.into(),
        source: spec(),
        folders,
        dry_run,
    }
}

fn direct(account: &str) -> DestAuth {
    DestAuth::Direct(UserCreds {
        email: account.into(),
        password: Zeroizing::new("unused".into()),
    })
}

async fn run_to_end(app: &Arc<App>, c: &MemConnector, req: JobRequest, auth: DestAuth) -> Progress {
    let job = start_with(app, req, auth, Arc::new(c.clone()))
        .await
        .unwrap_or_else(|e| panic!("start: {e:?}"));
    tokio::time::timeout(Duration::from_secs(10), job.wait())
        .await
        .expect("job finishes");
    job.progress()
}

#[tokio::test]
async fn copies_folders_flags_and_dates_then_resumes_without_duplicates() {
    let app = App::for_tests();
    let mock = app.mock().unwrap();
    let account = "runner1@example.test";
    let c = source();

    let p = run_to_end(&app, &c, request(account, None, false), direct(account)).await;
    assert_eq!(p.state, JobState::Done, "{p:?}");
    assert_eq!(p.messages_copied, 6, "All Mail is not selected by default");
    assert_eq!(p.messages_skipped, 0);
    assert_eq!(p.folders_total, 3);
    assert_eq!(p.folders_done, 3);
    assert!(p.bytes_copied > 0);

    let stored = mock.mail().messages(account);
    let boxes = mock.mail().mailbox_list(account);
    let id_of = |role: &str| {
        boxes
            .iter()
            .find(|b| b.role.as_deref() == Some(role))
            .unwrap()
            .id
            .clone()
    };
    let projects = boxes
        .iter()
        .find(|b| b.name == "Projects" && b.parent_id.is_none())
        .unwrap();
    let y2024 = boxes
        .iter()
        .find(|b| b.name == "2024" && b.parent_id.as_deref() == Some(&projects.id))
        .expect("nested folder created under its parent");
    let in_box = |id: &str| stored.iter().filter(|m| m.mailbox_id == id).count();
    assert_eq!(in_box(&id_of("inbox")), 3);
    assert_eq!(in_box(&id_of("sent")), 1);
    assert_eq!(in_box(&y2024.id), 2);
    let sent = stored
        .iter()
        .find(|m| m.mailbox_id == id_of("sent"))
        .unwrap();
    assert_eq!(sent.keywords, vec!["$seen", "$answered"]);
    assert_eq!(sent.received_at.as_deref(), Some("2021-02-01T10:00:00Z"));
    assert!(
        stored
            .iter()
            .any(|m| m.keywords == vec!["$seen", "$flagged"])
    );

    // Again: nothing new, nothing copied twice.
    let p = run_to_end(&app, &c, request(account, None, false), direct(account)).await;
    assert_eq!(p.state, JobState::Done);
    assert_eq!((p.messages_copied, p.messages_skipped), (0, 6));
    assert_eq!(mock.mail().messages(account).len(), 6);

    // A new message arrives on the old server; a renumbered folder
    // (new UIDVALIDITY) is recognised by Message-ID.
    {
        let mut s = c.0.lock().unwrap();
        s.folders[0].messages.push(msg(4, &[], "d@x", 10));
        s.folders[2].uidvalidity = 99;
        for (i, m) in s.folders[2].messages.iter_mut().enumerate() {
            m.0.uid = 500 + i as u32;
        }
    }
    let p = run_to_end(&app, &c, request(account, None, false), direct(account)).await;
    assert_eq!((p.messages_copied, p.messages_skipped), (1, 6), "{p:?}");
    assert_eq!(mock.mail().messages(account).len(), 7);
    // The last run is remembered for the portal.
    let last = status(&app.db, account).unwrap().unwrap();
    assert_eq!(last.state, JobState::Done);
}

#[tokio::test]
async fn a_dropped_connection_is_resumed_in_the_same_run() {
    let app = App::for_tests();
    let mock = app.mock().unwrap();
    let account = "runner2@example.test";
    let c = source();
    c.0.lock().unwrap().fail_after_bodies = Some(2);
    let p = run_to_end(&app, &c, request(account, None, false), direct(account)).await;
    assert_eq!(p.state, JobState::Done, "{p:?}");
    assert_eq!(
        (p.messages_copied, p.messages_skipped, p.messages_failed),
        (6, 0, 0)
    );
    assert_eq!(mock.mail().messages(account).len(), 6);
    assert_eq!(c.0.lock().unwrap().connects, 2);
}

#[tokio::test]
async fn chosen_folders_only_and_dry_run_writes_nothing() {
    let app = App::for_tests();
    let mock = app.mock().unwrap();
    let account = "runner3@example.test";
    let c = source();
    let p = run_to_end(
        &app,
        &c,
        request(
            account,
            Some(vec!["Sent Items".into(), "[Gmail]/All Mail".into()]),
            true,
        ),
        direct(account),
    )
    .await;
    assert_eq!(p.state, JobState::Done);
    assert!(p.dry_run);
    assert_eq!(p.messages_copied, 2, "both chosen folders, even All Mail");
    assert!(mock.mail().messages(account).is_empty());
    assert_eq!(
        mock.mail().mailbox_list(account).len(),
        5,
        "no folder created"
    );
    assert!(p.summary.unwrap().starts_with("Would copy 2 messages"));
    // A dry run leaves no record: a real run then copies them.
    let p = run_to_end(
        &app,
        &c,
        request(account, Some(vec!["Sent Items".into()]), false),
        direct(account),
    )
    .await;
    assert_eq!(p.messages_copied, 1);
}

#[tokio::test]
async fn oversized_messages_are_reported_and_left_behind() {
    let app = App::for_tests();
    let account = "runner4@example.test";
    let c = source();
    c.0.lock().unwrap().folders[0].messages.push(msg(
        9,
        &[],
        "huge@x",
        (MAX_MESSAGE_BYTES + 1) as usize,
    ));
    let p = run_to_end(
        &app,
        &c,
        request(account, Some(vec!["INBOX".into()]), false),
        direct(account),
    )
    .await;
    assert_eq!(p.state, JobState::Done);
    assert_eq!((p.messages_copied, p.messages_failed), (3, 1));
    assert!(p.errors[0].message.contains("over the"), "{:?}", p.errors);
}

#[tokio::test]
async fn one_job_per_mailbox_and_wrong_passwords_fail_fast() {
    let app = App::for_tests();
    let account = "runner5@example.test";
    let c = source();
    // Hold the lease as another process would.
    let ledger = Ledger::open(&app.db).unwrap();
    assert!(ledger.acquire(account, "someone-else", 60).unwrap());
    let err = start_with(
        &app,
        request(account, None, false),
        direct(account),
        Arc::new(c.clone()),
    )
    .await
    .err()
    .unwrap();
    assert!(matches!(err, StartError::Busy), "{err:?}");
    ledger.release(account, "someone-else").unwrap();

    let bad = source();
    bad.0.lock().unwrap().wrong_password = true;
    let err = start_with(
        &app,
        request(account, None, false),
        direct(account),
        Arc::new(bad),
    )
    .await
    .err()
    .unwrap();
    assert!(matches!(err, StartError::Source(_)), "{err:?}");
    assert!(
        !ledger.leased(account).unwrap(),
        "a failed start releases the lease"
    );
}

#[tokio::test]
async fn portal_jobs_use_a_temporary_app_password_and_revoke_it() {
    let app = App::for_tests();
    let mock = app.mock().unwrap();
    let engine = app.engine().unwrap();
    let user = UserCreds {
        email: "ren@example.test".into(),
        password: Zeroizing::new("demo-password-1234".into()),
    };
    let c = source();
    let p = run_to_end(
        &app,
        &c,
        request(&user.email, Some(vec!["INBOX".into()]), false),
        DestAuth::TemporaryAppPassword(user.clone()),
    )
    .await;
    assert_eq!(p.state, JobState::Done, "{p:?}");
    assert_eq!(mock.mail().messages(&user.email).len(), 3);
    let left = engine.user_app_passwords(&user).await.unwrap();
    assert!(
        left.iter().all(|a| a.description != TEMP_APP_PASSWORD),
        "revoked when the job ends: {left:?}"
    );
    let actions: Vec<String> = app
        .db
        .audit_list(50)
        .unwrap()
        .into_iter()
        .filter(|e| e.target.as_deref() == Some("ren@example.test"))
        .map(|e| e.action)
        .collect();
    assert!(
        actions.contains(&"mailbox.migrate_start".to_string()),
        "{actions:?}"
    );
    assert!(
        actions.contains(&"mailbox.migrate_done".to_string()),
        "{actions:?}"
    );
}

#[tokio::test]
async fn cancelling_stops_between_messages() {
    let app = App::for_tests();
    let account = "runner6@example.test";
    let c = source();
    {
        let mut s = c.0.lock().unwrap();
        s.folders[0].messages = (1..=400)
            .map(|u| msg(u, &[], &format!("m{u}@x"), 10))
            .collect();
    }
    let job = start_with(
        &app,
        request(account, None, false),
        direct(account),
        Arc::new(c.clone()),
    )
    .await
    .unwrap();
    job.cancel();
    tokio::time::timeout(Duration::from_secs(10), job.wait())
        .await
        .unwrap();
    let p = job.progress();
    assert_eq!(p.state, JobState::Cancelled, "{p:?}");
    assert!(p.messages_copied < 403);
    // It resumes from there.
    let p = run_to_end(&app, &c, request(account, None, false), direct(account)).await;
    assert_eq!(p.state, JobState::Done);
    assert_eq!(p.messages_copied + p.messages_skipped, 403);
}
