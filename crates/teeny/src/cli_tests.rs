use clap::{CommandFactory, Parser};

use super::*;

#[test]
fn the_command_line_is_well_formed() {
    Cli::command().debug_assert();
}

#[test]
fn reads_serve() {
    let cli = Cli::try_parse_from(["teeny", "serve"]).unwrap();
    assert_eq!(cli.command, Command::Serve(ServeArgs { dry_run: false }));
    let rehearsal = Cli::try_parse_from(["teeny", "serve", "--dry-run"]).unwrap();
    assert_eq!(
        rehearsal.command,
        Command::Serve(ServeArgs { dry_run: true })
    );
}

#[test]
fn reads_healthcheck() {
    let cli = Cli::try_parse_from(["teeny", "healthcheck"]).unwrap();
    assert_eq!(cli.command, Command::Healthcheck);
}

#[test]
fn refuses_to_guess_without_a_subcommand() {
    assert!(Cli::try_parse_from(["teeny"]).is_err());
}

mod operator {
    use clap::Parser;

    use crate::cli::{Cli, Command};
    use crate::cmd::cron::CronCommand;
    use crate::cmd::dry_run::DryRunJob;
    use crate::cmd::webhook::WebhookCommand;

    fn parse(args: &[&str]) -> Command {
        let mut argv = vec!["teeny"];
        argv.extend_from_slice(args);
        Cli::try_parse_from(argv)
            .unwrap_or_else(|error| panic!("{args:?}: {error}"))
            .command
    }

    #[test]
    fn reads_auth_x() {
        let Command::AuthX(args) = parse(&["auth-x", "--env-file", ".env"]) else {
            panic!("not auth-x");
        };
        assert_eq!(args.env_file.as_deref(), Some(std::path::Path::new(".env")));
    }

    #[test]
    fn reads_every_webhook_command() {
        let Command::Webhook(args) = parse(&[
            "webhook",
            "register",
            "https://a.example/webhooks/x",
            "--confirm",
        ]) else {
            panic!("not webhook");
        };
        assert_eq!(
            args.command,
            WebhookCommand::Register {
                url: "https://a.example/webhooks/x".into(),
                confirm: true
            }
        );
        for sub in [
            &["webhook", "list"][..],
            &["webhook", "subscribe"],
            &["webhook", "unsubscribe", "w1"],
            &["webhook", "delete", "w1"],
        ] {
            assert!(matches!(parse(sub), Command::Webhook(_)), "{sub:?}");
        }
    }

    #[test]
    fn reads_forget_remember_and_unsay() {
        let Command::Forget(forget) = parse(&["forget", "--ids", "a,b", "--confirm"]) else {
            panic!("not forget");
        };
        assert_eq!(forget.ids, vec!["a".to_owned(), "b".to_owned()]);
        assert!(forget.confirm);
        let Command::Remember(remember) =
            parse(&["remember", "Teeny", "is", "small", "--label", "identity"])
        else {
            panic!("not remember");
        };
        assert_eq!(remember.note.join(" "), "Teeny is small");
        assert_eq!(remember.labels, vec!["identity".to_owned()]);
        let Command::Unsay(unsay) = parse(&["unsay", "1", "2", "--confirm"]) else {
            panic!("not unsay");
        };
        assert_eq!(unsay.ids.len(), 2);
    }

    #[test]
    fn reads_dry_run_cron_and_redteam() {
        let Command::DryRun(dry) = parse(&["dry-run", "calendar", "gm"]) else {
            panic!("not dry-run");
        };
        assert_eq!(dry.job, DryRunJob::Calendar { item: "gm".into() });
        for job in [
            "morning",
            "afternoon",
            "evening",
            "moon",
            "chart",
            "lore",
            "quote",
            "narrative",
            "article",
        ] {
            assert!(
                matches!(parse(&["dry-run", job]), Command::DryRun(_)),
                "{job}"
            );
        }
        let Command::Cron(cron) = parse(&["cron", "run", "teeny_morning", "--yes"]) else {
            panic!("not cron");
        };
        assert_eq!(
            cron.command,
            CronCommand::Run {
                name: "teeny_morning".into(),
                yes: true
            }
        );
        assert_eq!(parse(&["redteam"]), Command::Redteam);
    }

    #[test]
    fn unsay_and_remember_need_something_to_act_on() {
        assert!(Cli::try_parse_from(["teeny", "unsay"]).is_err());
        assert!(Cli::try_parse_from(["teeny", "remember"]).is_err());
        assert!(Cli::try_parse_from(["teeny", "dry-run"]).is_err());
    }
}
