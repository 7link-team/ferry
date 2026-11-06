use super::*;

#[test]
fn challenge_tokens_answers_and_bindings_cannot_be_swapped() {
    let id = "opaque-test-token";
    let stored = StoredChallenge {
        token_hash: fingerprint(id),
        answer_hash: answer_hash(id, "A7XYZ"),
        kind: "image".into(),
    };
    let mut proof = Proof {
        login_challenge_id: id.into(),
        login_challenge_answer: "a7xyz".into(),
        ..Default::default()
    };
    assert!(valid_answer(&stored, &proof));
    proof.login_challenge_answer = "A7XY2".into();
    assert!(!valid_answer(&stored, &proof));
    proof.login_challenge_answer = "A7XYZ".into();
    proof.login_challenge_id = "other-token".into();
    assert!(!valid_answer(&stored, &proof));
    assert_ne!(
        binding("one@example.invalid", "127.0.0.1"),
        binding("two@example.invalid", "127.0.0.1")
    );
    assert_ne!(
        binding("one@example.invalid", "127.0.0.1"),
        binding("one@example.invalid", "127.0.0.2")
    );
    let slider = StoredChallenge {
        token_hash: fingerprint(id),
        answer_hash: answer_hash(id, "120"),
        kind: "slider".into(),
    };
    proof.login_challenge_id = id.into();
    for (answer, expected) in [
        ("117", true),
        ("123", true),
        ("116", false),
        ("124", false),
        ("999999999999", false),
        ("", false),
    ] {
        proof.login_challenge_answer = answer.into();
        assert_eq!(valid_answer(&slider, &proof), expected);
    }
}

#[test]
fn turnstile_rejects_wrong_hostname_action_and_failed_verdict() {
    let config = Configuration {
        mode: "turnstile".into(),
        hostname: "example.invalid".into(),
        ..Default::default()
    };
    let ok = json!({"success":true,"hostname":"example.invalid","action":"login"});
    assert!(valid_turnstile(&ok, &config));
    for (key, value) in [
        ("hostname", json!("evil.invalid")),
        ("action", json!("register")),
        ("success", json!(false)),
    ] {
        let mut invalid = ok.clone();
        invalid[key] = value;
        assert!(!valid_turnstile(&invalid, &config));
    }
    assert!(!valid_turnstile(&json!({}), &config));
}

#[test]
fn all_local_challenges_produce_real_rasters_and_valid_server_answers() {
    use base64::Engine;
    let mut kinds = std::collections::BTreeSet::new();
    for _ in 0..128 {
        let generated = captcha::generate();
        kinds.insert(generated.kind);
        let encoded = generated
            .image
            .strip_prefix("data:image/bmp;base64,")
            .unwrap();
        let bmp = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .unwrap();
        assert_eq!(&bmp[..2], b"BM");
        assert_eq!(bmp.len(), 54 + 280 * 80 * 3);
        assert!(bmp[54..].iter().any(|v| *v < 100));
        if generated.kind == "slider" {
            let path = generated.slider_path.unwrap();
            assert!((0.08..=0.12).contains(&path.slope.abs()));
            for x in [0.0, 256.0] {
                let top = path.start_y + x * path.slope;
                assert!(top >= 0.0 && top + 24.0 <= 80.0);
            }
            let x: f64 = generated.answer.parse().unwrap();
            let y = (path.start_y + x * path.slope).round() as usize;
            let offset = 54 + ((79 - y) * 280 + x as usize) * 3;
            assert_eq!(&bmp[offset..offset + 3], &[80, 55, 30]);
        }
        if generated.kind == "select" {
            assert_eq!(generated.choices.len(), 8);
            assert_eq!(generated.answer.len(), 3);
            assert!(
                generated
                    .answer
                    .chars()
                    .all(|c| generated.choices.contains(&c.to_string()))
            );
        }
        let stored = StoredChallenge {
            token_hash: fingerprint("test"),
            answer_hash: answer_hash("test", &generated.answer),
            kind: generated.kind.into(),
        };
        assert!(valid_answer(
            &stored,
            &Proof {
                login_challenge_id: "test".into(),
                login_challenge_answer: generated.answer,
                ..Default::default()
            }
        ));
    }
    assert_eq!(kinds.len(), 4);
}
