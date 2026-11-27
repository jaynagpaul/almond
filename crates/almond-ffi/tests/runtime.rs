//! The runtime end to end: real SQLite, a fake Swift shell.

use std::sync::{Arc, Mutex};

use almond_core::{Event, Haptic, PlatformCmd, Rating, ViewModel};
use almond_ffi::{Core, Shell};

#[derive(Default)]
struct FakeShell {
    changes: Mutex<u32>,
    performed: Mutex<Vec<PlatformCmd>>,
}

impl Shell for FakeShell {
    fn changed(&self) {
        *self.changes.lock().unwrap() += 1;
    }
    fn perform(&self, cmd: PlatformCmd) {
        self.performed.lock().unwrap().push(cmd);
    }
}

fn temp_db(name: &str) -> String {
    let dir = std::env::temp_dir().join(format!("almond-ffi-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("almond.db").to_string_lossy().into_owned()
}

#[test]
fn a_card_added_today_is_reviewed_and_remembered_across_launches() {
    let db = temp_db("launches");
    let shell = Arc::new(FakeShell::default());
    let core = Core::new(&db, "Asia/Kolkata", shell.clone()).unwrap();
    core.flush();
    assert_eq!(core.view(), ViewModel::Home { due: 0 });

    core.dispatch(Event::AddCard {
        front: "gato".into(),
        back: "cat".into(),
    });
    assert_eq!(core.view(), ViewModel::Home { due: 1 });

    core.dispatch(Event::StartReview);
    core.dispatch(Event::Reveal);
    core.dispatch(Event::Rate {
        rating: Rating::Good,
    });
    assert_eq!(
        core.view(),
        ViewModel::Done {
            reviewed: 1,
            can_undo: true
        }
    );
    assert!(
        shell
            .performed
            .lock()
            .unwrap()
            .contains(&PlatformCmd::Haptic {
                kind: Haptic::Finished
            })
    );
    assert!(*shell.changes.lock().unwrap() >= 5);
    core.flush();
    drop(core);

    // Relaunch: the card was rated Good, so it is not due again today.
    let core = Core::new(&db, "Asia/Kolkata", Arc::new(FakeShell::default())).unwrap();
    core.flush();
    assert_eq!(core.view(), ViewModel::Home { due: 0 });
}

#[test]
fn an_undone_review_leaves_the_card_due_after_relaunch() {
    let db = temp_db("undo");
    let core = Core::new(&db, "Asia/Kolkata", Arc::new(FakeShell::default())).unwrap();
    core.flush();
    core.dispatch(Event::AddCard {
        front: "perro".into(),
        back: "dog".into(),
    });
    core.dispatch(Event::StartReview);
    core.dispatch(Event::Reveal);
    core.dispatch(Event::Rate {
        rating: Rating::Easy,
    });
    core.dispatch(Event::Undo);
    core.flush();
    drop(core);

    let core = Core::new(&db, "Asia/Kolkata", Arc::new(FakeShell::default())).unwrap();
    core.flush();
    assert_eq!(core.view(), ViewModel::Home { due: 1 });
}

#[test]
fn a_bad_path_is_an_error_not_a_crash() {
    let result = Core::new(
        "/nonexistent/dir/almond.db",
        "UTC",
        Arc::new(FakeShell::default()),
    );
    assert!(result.is_err());
}

#[test]
fn a_card_added_while_loading_still_shows_up() {
    let core = Core::new(&temp_db("loading"), "UTC", Arc::new(FakeShell::default())).unwrap();
    // No flush: the first load may still be in flight.
    core.dispatch(Event::AddCard {
        front: "uno".into(),
        back: "one".into(),
    });
    core.flush();
    assert_eq!(core.view(), ViewModel::Home { due: 1 });
}

#[test]
fn an_unknown_time_zone_falls_back_instead_of_blocking_launch() {
    let db = temp_db("tz");
    let core = Core::new(&db, "Mars/Olympus_Mons", Arc::new(FakeShell::default())).unwrap();
    core.flush();
    assert_eq!(core.view(), ViewModel::Home { due: 0 });
    core.set_time_zone("Europe/Paris");
}
