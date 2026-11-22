use super::*;

fn keyboard() -> Keyboard {
    let mut keyboard = Keyboard::default();
    keyboard.configure(400, 160);
    keyboard
}

fn center(keyboard: &Keyboard, label: &str) -> (f64, f64) {
    let face = keyboard.faces().find(|face| face.label == label).unwrap();
    (face.x + face.width / 2.0, face.y + face.height / 2.0)
}

fn down(keyboard: &mut Keyboard, id: i32, label: &str) {
    let (x, y) = center(keyboard, label);
    keyboard.down(id, x, y);
}

fn tap(keyboard: &mut Keyboard, label: &str) -> Option<Action> {
    down(keyboard, 0, label);
    keyboard.up(0);
    keyboard.frame()
}

#[test]
fn emits_only_after_release_and_frame() {
    let mut keyboard = keyboard();
    down(&mut keyboard, 7, "a");
    assert!(keyboard.frame().is_none());
    keyboard.up(1);
    assert!(keyboard.frame().is_none());
    keyboard.up(7);
    assert!(keyboard.frame() == Some(Action::Character('a')));
    keyboard.up(7);
    assert!(keyboard.frame().is_none());
    assert!(tap(&mut keyboard, "⌫") == Some(Action::Backspace));
    assert!(tap(&mut keyboard, "Unlock") == Some(Action::Submit));
    assert!(tap(&mut keyboard, "Cancel") == Some(Action::Cancel));
}

#[test]
fn layout_covers_printable_ascii() {
    let mut keyboard = keyboard();
    let mut characters = BTreeSet::new();
    for (next, shift) in [
        ("123", false),
        ("123", true),
        ("#+=", false),
        ("ABC", false),
    ] {
        if shift {
            // Toggle shift before each letter; it resets after the character.
            let labels: Vec<_> = keyboard
                .faces()
                .filter(|f| f.label.len() == 1)
                .map(|f| f.label.to_ascii_uppercase())
                .collect();
            for label in labels {
                assert!(tap(&mut keyboard, "⇧").is_none());
                let Some(Action::Character(c)) = tap(&mut keyboard, &label) else {
                    panic!("shifted letter did not emit a character");
                };
                characters.insert(c);
            }
        } else {
            let labels: Vec<_> = keyboard
                .faces()
                .map(|f| f.label)
                .filter(|label| label.len() == 1 || label == "Space")
                .collect();
            for label in labels {
                if let Some(Action::Character(c)) = tap(&mut keyboard, &label) {
                    characters.insert(c);
                }
            }
            if next == "123" {
                continue;
            }
        }
        assert!(tap(&mut keyboard, next).is_none());
    }
    assert_eq!(characters, (' '..='~').collect());
}

#[test]
fn gaps_and_drags_never_submit() {
    let mut keyboard = keyboard();
    let (x, y) = center(&keyboard, "Unlock");
    keyboard.down(0, 0.0, y);
    keyboard.motion(0, x, y);
    keyboard.up(0);
    assert!(keyboard.frame().is_none());

    keyboard.down(0, x, y);
    keyboard.motion(0, x, f64::NAN);
    keyboard.motion(0, x, y);
    keyboard.up(0);
    assert!(keyboard.frame().is_none());
}

#[test]
fn overlapping_contacts_invalidate_even_if_one_is_outside() {
    let mut keyboard = keyboard();
    down(&mut keyboard, 0, "Unlock");
    keyboard.down(1, -1.0, -1.0);
    keyboard.up(0);
    down(&mut keyboard, 2, "Unlock");
    keyboard.up(1);
    keyboard.up(2);
    assert!(keyboard.frame().is_none());
    assert!(tap(&mut keyboard, "Unlock") == Some(Action::Submit));
}

#[test]
fn second_tap_before_frame_discards_both() {
    let mut keyboard = keyboard();
    down(&mut keyboard, 0, "Unlock");
    keyboard.up(0);
    down(&mut keyboard, 0, "Cancel");
    keyboard.up(0);
    assert!(keyboard.frame().is_none());
}

#[test]
fn stream_cancel_discards_pending_submit_and_resets_layout() {
    let mut keyboard = keyboard();
    assert!(tap(&mut keyboard, "123").is_none());
    down(&mut keyboard, 0, "Unlock");
    keyboard.up(0);
    keyboard.cancel_touches();
    assert!(keyboard.frame().is_none());
    assert!(tap(&mut keyboard, "a") == Some(Action::Character('a')));

    down(&mut keyboard, 3, "Unlock");
    keyboard.cancel_touches();
    keyboard.up(3);
    assert!(keyboard.frame().is_none());
}

#[test]
fn disarm_and_reconfigure_invalidate_taps_but_keep_tracking_fingers() {
    let mut keyboard = keyboard();
    down(&mut keyboard, 0, "Unlock");
    keyboard.configure(400, 160);
    down(&mut keyboard, 1, "Unlock");
    keyboard.up(0);
    keyboard.up(1);
    assert!(keyboard.frame().is_none());

    down(&mut keyboard, 2, "Unlock");
    keyboard.up(2);
    keyboard.disarm();
    assert!(keyboard.frame().is_none());
}

#[test]
fn tiny_surfaces_are_inactive_until_valid_configure() {
    let mut keyboard = Keyboard::default();
    let (x, y) = center(&self::keyboard(), "Unlock");
    for (width, height) in [(0, 0), (299, 160), (400, 127)] {
        keyboard.configure(width, height);
        assert_eq!(keyboard.faces().count(), 0);
        keyboard.down(0, x, y);
        keyboard.up(0);
        assert!(keyboard.frame().is_none());
    }
    keyboard.configure(400, 160);
    assert!(tap(&mut keyboard, "Unlock") == Some(Action::Submit));
}
