//! Pairing a new device with a one-time code: made once, works once, for ten minutes; wrong
//! codes are braked for everyone.

use emberhut_core::pairing::PairingRefusal;
use emberhut_server::services::pairing::CodeMaker;

use crate::common::{TestApp, device, refusal};

#[tokio::test]
async fn a_code_is_eight_characters_without_look_alikes_for_ten_minutes() {
    let app = TestApp::new().await;
    let issued = app
        .state
        .pairing()
        .new_code(CodeMaker::CommandLine)
        .await
        .unwrap();
    let shown = issued.code.to_string();
    let (first, second) = shown.split_once('-').unwrap();
    assert_eq!((first.len(), second.len()), (4, 4), "{shown}");
    assert!(
        shown
            .chars()
            .filter(|c| *c != '-')
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
    );
    assert!(!shown.contains(['0', '1', 'O', 'I', 'L']), "{shown}");
    assert_eq!(
        issued.expires_at - app.clock_now(),
        jiff::SignedDuration::from_mins(10)
    );
}

#[tokio::test]
async fn a_code_typed_in_small_letters_opens_a_session_once() {
    let app = TestApp::new().await;
    let pairing = app.state.pairing();
    let code = pairing
        .new_code(CodeMaker::CommandLine)
        .await
        .unwrap()
        .code
        .to_string();

    let typed = code.to_lowercase().replace('-', " ");
    let session = pairing.redeem(&typed, &device("phone-1")).await.unwrap();
    let signed_in = app
        .state
        .auth()
        .authenticate(session.tokens.access.as_str())
        .await
        .unwrap();
    assert_eq!(signed_in, Some(session.device));

    let again = pairing.redeem(&code, &device("phone-2")).await;
    assert_eq!(refusal(again), PairingRefusal::Used);
}

#[tokio::test]
async fn the_same_install_pairing_again_keeps_its_device() {
    let app = TestApp::new().await;
    let pairing = app.state.pairing();
    let first = pairing.new_code(CodeMaker::CommandLine).await.unwrap();
    let paired = pairing
        .redeem(&first.code.to_string(), &device("phone-1"))
        .await
        .unwrap();
    let second = pairing
        .new_code(CodeMaker::Device(paired.device))
        .await
        .unwrap();
    let repaired = pairing
        .redeem(&second.code.to_string(), &device("phone-1"))
        .await
        .unwrap();
    assert_eq!(repaired.device, paired.device);
    assert_eq!(
        app.state
            .auth()
            .authenticate(paired.tokens.access.as_str())
            .await
            .unwrap(),
        None,
        "the old session stopped"
    );
}

#[tokio::test]
async fn an_expired_or_missing_code_is_refused() {
    let app = TestApp::new().await;
    let pairing = app.state.pairing();
    let code = pairing.new_code(CodeMaker::CommandLine).await.unwrap().code;
    app.advance_minutes(10);
    let late = pairing.redeem(&code.to_string(), &device("phone-1")).await;
    assert_eq!(refusal(late), PairingRefusal::Expired);

    for blank in ["", "  ", "-"] {
        let missing = pairing.redeem(blank, &device("phone-1")).await;
        assert_eq!(refusal(missing), PairingRefusal::Missing);
    }
}

#[tokio::test]
async fn five_wrong_codes_stop_every_code_for_ten_minutes() {
    let app = TestApp::new().await;
    let pairing = app.state.pairing();
    let right = pairing.new_code(CodeMaker::CommandLine).await.unwrap().code;
    for _ in 0..5 {
        let wrong = pairing.redeem("AAAA-AAAA", &device("guesser")).await;
        assert_eq!(refusal(wrong), PairingRefusal::Unknown);
    }

    let locked = pairing.redeem(&right.to_string(), &device("phone-1")).await;
    let PairingRefusal::TooManyWrongCodes { retry_after } = refusal(locked) else {
        panic!("even the right code waits");
    };
    assert_eq!(retry_after, jiff::SignedDuration::from_mins(10));

    app.advance_minutes(9);
    assert!(
        pairing
            .redeem(&right.to_string(), &device("phone-1"))
            .await
            .is_err()
    );
    app.advance_minutes(1);
    // The brake is off, but the code itself has now expired.
    let expired = pairing.redeem(&right.to_string(), &device("phone-1")).await;
    assert_eq!(refusal(expired), PairingRefusal::Expired);
}

/// Guesses sent all at once are braked like guesses sent one after another: of twenty
/// concurrent wrong codes, five are looked up and the rest are refused without a lookup.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn twenty_wrong_codes_at_once_get_five_guesses() {
    let app = TestApp::new().await;
    let mut guesses = tokio::task::JoinSet::new();
    for n in 0..20 {
        let state = app.state.clone();
        guesses.spawn(async move {
            let guess = format!("AAAA-AA{n:02}");
            state
                .pairing()
                .redeem(&guess, &device(&format!("guesser-{n}")))
                .await
        });
    }
    let (mut unknown, mut braked) = (0, 0);
    while let Some(result) = guesses.join_next().await {
        match refusal(result.unwrap()) {
            PairingRefusal::Unknown => unknown += 1,
            PairingRefusal::TooManyWrongCodes { .. } => braked += 1,
            other => panic!("unexpected {other:?}"),
        }
    }
    assert_eq!((unknown, braked), (5, 15));
}

#[tokio::test]
async fn codes_that_exist_do_not_count_against_the_brake() {
    let app = TestApp::new().await;
    let pairing = app.state.pairing();
    let used = pairing.new_code(CodeMaker::CommandLine).await.unwrap().code;
    pairing
        .redeem(&used.to_string(), &device("phone-1"))
        .await
        .unwrap();
    for _ in 0..4 {
        let wrong = pairing.redeem("AAAA-AAAA", &device("guesser")).await;
        assert_eq!(refusal(wrong), PairingRefusal::Unknown);
    }
    for _ in 0..10 {
        let again = pairing.redeem(&used.to_string(), &device("phone-2")).await;
        assert_eq!(refusal(again), PairingRefusal::Used);
    }
    let fifth = pairing.redeem("AAAA-AAAA", &device("guesser")).await;
    assert_eq!(
        refusal(fifth),
        PairingRefusal::Unknown,
        "still the fifth wrong code"
    );
}
