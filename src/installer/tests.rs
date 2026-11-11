//! The installer's state machine: the terminal/browser split, the browser
//! onboarding's steps against the in-memory engine, and installs started by
//! the older all-in-the-terminal flow resuming where they were.

use super::*;
use crate::edge::certs;

fn inputs() -> InstallInputs {
    InstallInputs {
        domain: "example.com".into(),
        hostname: "mail.example.com".into(),
        admin_email: "me@gmail.com".into(),
        first_mailbox: "hello".into(),
        sending: SendingMode::Direct,
        relay: None,
        client_origin: crate::app::DEFAULT_CLIENT_ORIGIN.into(),
        sending_confirmed: true,
    }
}

fn status(app: &App, id: &str) -> StepStatus {
    app.db.step_status(id).unwrap()
}

/// What the terminal leaves behind: every root step done.
fn terminal_finished(app: &App) {
    app.db.init_steps().unwrap();
    for id in TERMINAL_STEPS {
        app.db.set_step(id, StepStatus::Done, None).unwrap();
    }
}

/// Gates that need the internet (public DNS, the CA) marked as passed, so a
/// run exercises everything else.
fn gates_passed(app: &App, ids: &[&str]) {
    for id in ids {
        app.db.set_step(id, StepStatus::Done, None).unwrap();
    }
}

#[test]
fn input_validation() {
    assert!(inputs().validate().is_ok());
    let mut i = inputs();
    i.admin_email = "me@example.com".into();
    assert!(
        i.validate()
            .unwrap_err()
            .to_string()
            .contains("does not depend on this server")
    );
    // The console names the field.
    let e = check_identity("example.com", "mail.example.com", "me@example.com").unwrap_err();
    assert_eq!(e.field, "adminEmail");
    assert_eq!(
        check_identity("not a domain", "mail.example.com", "me@gmail.com")
            .unwrap_err()
            .field,
        "domain"
    );
    let (d, h, e) = check_identity(" Example.COM ", "MAIL.example.com.", " Me@Gmail.com").unwrap();
    assert_eq!(
        (d.as_str(), h.as_str(), e.as_str()),
        ("example.com", "mail.example.com", "me@gmail.com")
    );
    // The console creates the first mailbox: no local part needed.
    let mut i = inputs();
    i.first_mailbox = String::new();
    assert!(i.validate().is_ok());
    let mut i = inputs();
    i.sending = SendingMode::Relay;
    assert!(i.validate().is_err());
    i.relay = Some(RelayInput {
        provider: "postmark".into(),
        host: "smtp.postmarkapp.com".into(),
        port: 587,
        username: "u".into(),
        region: None,
        password: "p".into(),
    });
    assert!(i.validate().is_ok());
    // Amazon SES: the host comes from the region, never typed.
    i.relay = Some(RelayInput {
        provider: "ses".into(),
        host: String::new(),
        port: 587,
        username: "AKIAEXAMPLE".into(),
        region: Some("eu-west-1".into()),
        password: "p".into(),
    });
    assert!(i.validate().is_ok());
    assert_eq!(
        i.relay.as_ref().unwrap().settings().unwrap().host,
        "email-smtp.eu-west-1.amazonaws.com"
    );
    i.relay.as_mut().unwrap().region = Some("nowhere".into());
    assert!(i.validate().is_err());
}

#[test]
fn answers_saved_by_an_older_release_still_read() {
    // Verbatim shape of `install.inputs` before the browser onboarding.
    let old = serde_json::json!({
        "domain": "example.com", "hostname": "mail.example.com",
        "adminEmail": "me@gmail.com", "firstMailbox": "hello",
        "sending": "relay",
        "relay": {"provider": "postmark", "host": "smtp.postmarkapp.com", "port": 587, "username": "u", "region": null},
        "clientOrigin": "https://app.yunori.email"
    });
    let i: InstallInputs = serde_json::from_value(old).unwrap();
    assert!(
        i.sending_confirmed,
        "the old terminal always decided sending"
    );
    assert_eq!(i.first_mailbox, "hello");
    assert_eq!(i.relay.unwrap().provider, "postmark");
}

#[tokio::test]
async fn the_service_never_runs_terminal_steps() {
    let app = App::for_tests();
    let inst = Installer::new(app.clone(), Mode::Service);
    let r = inst.run(&mut NullUi).await.unwrap();
    assert_eq!(
        r,
        RunResult::Waiting {
            step: "preflight".into(),
            detail: RUN_IN_TERMINAL.into()
        }
    );
    // Terminal done: the browser's first step waits for its answer.
    terminal_finished(&app);
    let app = App::for_tests_fresh("panel.example.com");
    terminal_finished(&app);
    let inst = Installer::new(app.clone(), Mode::Service);
    let r = inst.run(&mut NullUi).await.unwrap();
    assert!(
        matches!(r, RunResult::Waiting { ref step, .. } if step == "inputs"),
        "{r:?}"
    );
}

#[tokio::test]
async fn the_browser_names_the_domain_and_hostname_after_the_engine_runs() {
    // The terminal bootstrapped the engine with names derived from the
    // console's address (sslip.io): domain and hostname come later.
    let app = App::for_tests_fresh("203-0-113-10.sslip.io");
    terminal_finished(&app);
    let engine = app.engine().unwrap();
    assert_eq!(engine.hostname().await.unwrap(), "203-0-113-10.sslip.io");

    let mut i = inputs();
    i.first_mailbox = String::new();
    i.sending_confirmed = false; // as the console saves it
    app.db.set_json(KV_INPUTS, &i).unwrap();
    let inst = Installer::new(app.clone(), Mode::Service);
    let r = inst.run(&mut NullUi).await.unwrap();
    assert!(
        matches!(r, RunResult::Waiting { ref step, .. } if step == "sending"),
        "{r:?}"
    );
    assert_eq!(status(&app, "inputs"), StepStatus::Done);

    // The placeholder domain was renamed in place, the hostname set, and the
    // engine administrator's sign-in follows the domain.
    let domains = engine.list_domains().await.unwrap();
    assert_eq!(domains.len(), 1);
    assert_eq!(domains[0].name, "example.com");
    assert_eq!(engine.hostname().await.unwrap(), "mail.example.com");
    let admin = app.db.get_secret("engine.admin").unwrap().unwrap();
    assert_eq!(admin.as_str(), "admin@example.com:engine-secret");
    let cfg = app.config().unwrap();
    assert_eq!(
        (cfg.hostname.as_str(), cfg.primary_domain.as_str()),
        ("mail.example.com", "example.com")
    );
    assert_eq!(cfg.admin_email, "me@gmail.com");
    assert_eq!(cfg.console_host(), "203-0-113-10.sslip.io");
    assert_eq!(
        app.db
            .get_kv("dmarc_policy:example.com")
            .unwrap()
            .as_deref(),
        Some("none")
    );

    // Applying the same answer again changes nothing.
    apply_identity(&app, &i, "test").await.unwrap();
    assert_eq!(engine.list_domains().await.unwrap().len(), 1);

    // Sending answered; the internet-facing gates pass; the first mailbox
    // waits for the console (the service never makes one up).
    i.sending_confirmed = true;
    app.db.set_json(KV_INPUTS, &i).unwrap();
    gates_passed(&app, &["host_record", "certificate", "dns", "ptr"]);
    let r = inst.run(&mut NullUi).await.unwrap();
    assert!(
        matches!(r, RunResult::Waiting { ref step, .. } if step == "first_mailbox"),
        "{r:?}"
    );
    assert_eq!(status(&app, "sending"), StepStatus::Done);
    ops::create_first_mailbox(&app, "hello", None, "a-strong-password-123", "test")
        .await
        .unwrap();
    assert_eq!(inst.run(&mut NullUi).await.unwrap(), RunResult::Complete);
    assert!(app.config().unwrap().install_complete);
}

#[tokio::test]
async fn a_domain_with_mailboxes_is_never_renamed() {
    let app = App::for_tests_fresh("panel.example.org");
    terminal_finished(&app);
    // The placeholder is the console's parent domain.
    assert_eq!(app.config().unwrap().primary_domain, "example.org");
    ops::create_first_mailbox(&app, "hello", None, "a-strong-password-123", "test")
        .await
        .unwrap();
    let err = apply_identity(&app, &inputs(), "test").await.unwrap_err();
    assert!(err.to_string().contains("already has mailboxes"), "{err}");
    assert_eq!(
        app.engine().unwrap().list_domains().await.unwrap()[0].name,
        "example.org"
    );
    // The suggested domain needs no rename at all.
    let mut same = inputs();
    same.domain = "example.org".into();
    same.hostname = "mail.example.org".into();
    apply_identity(&app, &same, "test").await.unwrap();
}

#[tokio::test]
async fn an_install_paused_by_the_old_terminal_flow_resumes() {
    // A real situation: the older release stopped at "Point the mail hostname
    // here" with the domain and hostname saved, before installing anything.
    let app = App::for_tests();
    let mut cfg = app.config().unwrap();
    cfg.install_complete = false;
    app.save_config(&cfg).unwrap();
    {
        let conn = app.db.conn();
        conn.execute("DELETE FROM install_steps", []).unwrap();
        for (i, (id, title, st)) in [
            ("preflight", "Check this server", "done"),
            ("inputs", "Your domain", "done"),
            ("host_record", "Point the mail hostname here", "waiting"),
            ("firewall", "Open mail ports", "pending"),
            ("install_engine", "Install the mail engine", "pending"),
            ("bootstrap_engine", "Configure the mail engine", "pending"),
            ("edge_tls", "Start the console and HTTPS edge", "pending"),
            ("certificate", "Get a trusted TLS certificate", "pending"),
            ("dns", "Publish DNS records", "pending"),
            ("ptr", "Set reverse DNS", "pending"),
            ("first_mailbox", "Create your first mailbox", "pending"),
            ("done", "Finish", "pending"),
        ]
        .into_iter()
        .enumerate()
        {
            conn.execute(
                "INSERT INTO install_steps (id, ord, title, status) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![id, i as i64, title, st],
            )
            .unwrap();
        }
    }
    app.db
        .set_json(
            KV_INPUTS,
            &serde_json::json!({
                "domain": "example.test", "hostname": "mail.example.test",
                "adminEmail": "owner@elsewhere.example", "firstMailbox": "hello",
                "sending": "direct", "relay": null,
                "clientOrigin": "https://app.yunori.email"
            }),
        )
        .unwrap();

    app.db.init_steps().unwrap();
    // The saved answers and verdicts are kept; the new steps are waiting.
    assert_eq!(status(&app, "inputs"), StepStatus::Done);
    assert_eq!(status(&app, "host_record"), StepStatus::Waiting);
    assert_eq!(status(&app, "panel"), StepStatus::Pending);
    let st = state::load(&app.db).unwrap();
    assert_eq!(
        st.steps.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
        STEPS.iter().map(|(id, _)| *id).collect::<Vec<_>>()
    );

    // Nothing is installed yet: the service sends the person to the terminal,
    // which asks its one question and installs.
    let inst = Installer::new(app.clone(), Mode::Service);
    assert_eq!(
        inst.run(&mut NullUi).await.unwrap(),
        RunResult::Waiting {
            step: "panel".into(),
            detail: RUN_IN_TERMINAL.into()
        }
    );
    terminal_finished(&app);
    // The browser then continues from the host record, with the answers the
    // old terminal took (sending was decided there).
    gates_passed(&app, &["certificate", "dns", "ptr"]);
    app.db
        .set_step("host_record", StepStatus::Done, None)
        .unwrap();
    assert_eq!(inst.run(&mut NullUi).await.unwrap(), RunResult::Complete);
    assert_eq!(status(&app, "sending"), StepStatus::Done);
    assert_eq!(status(&app, "first_mailbox"), StepStatus::Done);
    assert!(app.config().unwrap().install_complete);
}

#[tokio::test]
async fn an_old_install_past_its_terminal_phase_never_waits_for_the_terminal() {
    let app = App::for_tests();
    let mut cfg = app.config().unwrap();
    cfg.install_complete = false;
    app.save_config(&cfg).unwrap();
    app.db.init_steps().unwrap();
    app.db
        .conn()
        .execute(
            "DELETE FROM install_steps WHERE id IN ('panel', 'sending')",
            [],
        )
        .unwrap();
    for id in [
        "preflight",
        "inputs",
        "host_record",
        "firewall",
        "install_engine",
        "bootstrap_engine",
        "edge_tls",
        "certificate",
    ] {
        app.db.set_step(id, StepStatus::Done, None).unwrap();
    }
    app.db
        .set_step("dns", StepStatus::Waiting, Some("Publish 2 records"))
        .unwrap();
    app.db.set_json(KV_INPUTS, &inputs()).unwrap();
    app.db.init_steps().unwrap();
    assert_eq!(
        status(&app, "panel"),
        StepStatus::Done,
        "the console stays on the mail hostname"
    );
    assert_eq!(status(&app, "dns"), StepStatus::Waiting);
}

#[tokio::test]
async fn first_mailbox_waits_in_service_mode_and_creates_in_cli() {
    let app = App::for_tests_fresh("panel.example.com");
    app.db.init_steps().unwrap();
    for (id, _) in STEPS.iter().take_while(|(id, _)| *id != "first_mailbox") {
        app.db.set_step(id, StepStatus::Done, None).unwrap();
    }
    let mut i = inputs();
    i.domain = "example.com".into();
    app.db.set_json(KV_INPUTS, &i).unwrap();
    let svc = Installer::new(app.clone(), Mode::Service);
    assert!(matches!(
        svc.run(&mut NullUi).await.unwrap(),
        RunResult::Waiting { ref step, .. } if step == "first_mailbox"
    ));
    // `--first-mailbox` (with --domain): the terminal creates it.
    let cli = Installer::new(app.clone(), Mode::Cli);
    assert_eq!(cli.run(&mut NullUi).await.unwrap(), RunResult::Complete);
    let accounts = app.engine().unwrap().list_accounts().await.unwrap();
    assert!(accounts.iter().any(|a| a.address == "hello@example.com"));
}

#[test]
fn the_certificate_covers_the_console_first_and_the_mail_hostname_once_it_points_here() {
    // Before the browser onboarding: the mail hostname only.
    let app = App::for_tests();
    let cfg = app.config().unwrap();
    assert_eq!(
        certs::required_names(&app, &cfg),
        vec![cfg.hostname.clone()]
    );

    // sslip.io console: its name is also the engine's placeholder hostname.
    let app = App::for_tests_fresh("203-0-113-10.sslip.io");
    app.db.init_steps().unwrap();
    let mut cfg = app.config().unwrap();
    assert_eq!(
        certs::required_names(&app, &cfg),
        vec!["203-0-113-10.sslip.io".to_string()]
    );
    // A mail hostname that does not point here yet never holds the console back…
    cfg.hostname = "mail.example.com".into();
    assert_eq!(
        certs::required_names(&app, &cfg),
        vec!["203-0-113-10.sslip.io".to_string()]
    );
    // …and joins once its host record is verified.
    app.db
        .set_step("host_record", StepStatus::Done, None)
        .unwrap();
    assert_eq!(
        certs::required_names(&app, &cfg),
        vec![
            "mail.example.com".to_string(),
            "203-0-113-10.sslip.io".to_string()
        ]
    );

    // A bare-IP console: nothing to ask a CA for until the mail hostname points here.
    let app = App::for_tests_fresh("203.0.113.10");
    app.db.init_steps().unwrap();
    let cfg = app.config().unwrap();
    assert!(certs::required_names(&app, &cfg).is_empty());

    // Required names are never left out, even after failing before.
    let mut st = certs::AcmeState::default();
    let now = chrono::Utc::now();
    st.excluded.insert(
        "203-0-113-10.sslip.io".into(),
        (now + chrono::Duration::hours(3)).to_rfc3339(),
    );
    let required = vec![
        "mail.example.com".to_string(),
        "203-0-113-10.sslip.io".to_string(),
    ];
    let plan = certs::plan_for_names(
        &required,
        &required,
        None,
        crate::edge::acme::LE_PRODUCTION,
        &st,
        now,
    );
    assert_eq!(plan.names, required);
}

#[test]
fn links_use_the_console_address_until_the_mail_hostname_has_its_certificate() {
    let app = App::for_tests_fresh("203.0.113.10");
    app.db.init_steps().unwrap();
    let mut cfg = app.config().unwrap();
    cfg.hostname = "mail.example.com".into();
    app.save_config(&cfg).unwrap();
    assert_eq!(public_host(&app).as_deref(), Some("203.0.113.10"));
    app.db
        .set_step("certificate", StepStatus::Done, None)
        .unwrap();
    assert_eq!(public_host(&app).as_deref(), Some("mail.example.com"));
    assert_eq!(
        setup_link("2a01:4f8::2", "tok"),
        "https://[2a01:4f8::2]/console/setup#token=tok"
    );
}
