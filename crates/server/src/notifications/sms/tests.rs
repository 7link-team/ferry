use super::*;
fn config() -> Config {
    Config {
        sms_access_key: "fixtureAccess".into(),
        sms_secret_key: "fixtureSecret".into(),
        sms_region: "ap-guangzhou".into(),
        sms_app_id: "1400000000".into(),
        sms_sign_name: "Fixture".into(),
        sms_template_code: "123456".into(),
        sms_parameter_names: vec!["second".into(), "first".into()],
    }
}
fn numbers() -> Vec<String> {
    vec!["+8613800000000".into()]
}
fn params() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("first".into(), "A & B".into()),
        ("second".into(), "1234".into()),
    ])
}
#[test]
fn validation_requires_e164_unique_parameter_names_and_provider_fields() {
    for provider in ["Aliyun SMS", "Tencent SMS"] {
        assert!(config().validate(provider, &numbers()));
        for number in [
            "13800000000",
            "+01234567",
            "+86138 00000000",
            "+123",
            "+8613800000000\n",
        ] {
            assert!(!config().validate(provider, &[number.into()]));
        }
        let mut c = config();
        c.sms_parameter_names.push("first".into());
        assert!(!c.validate(provider, &numbers()));
        assert!(config().parameters(&BTreeMap::new()).is_none());
    }
    assert!(!config().validate("Aliyun SMS", &["+14155552671".into()]));
    assert!(config().validate("Tencent SMS", &["+14155552671".into()]));
    let mut c = config();
    c.sms_app_id.clear();
    assert!(!c.validate("Tencent SMS", &numbers()));
}
#[test]
fn signing_uses_sha256_hmac_and_exact_wire_bytes() {
    // RFC 4231 test case 1.
    assert_eq!(
        hex::encode(mac(&[0x0b; 20], "Hi There")),
        "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
    );
    assert_eq!(encode("A & B/+~"), "A%20%26%20B%2F%2B~");
    let c = config();
    let ali = request(
        "Aliyun SMS",
        &c,
        &numbers(),
        &params(),
        1700000000,
        "fixture-nonce",
    );
    assert!(ali.url.contains(
        "PhoneNumbers=13800000000&SignName=Fixture&TemplateCode=123456&TemplateParam=%7B"
    ));
    assert!(ali.body.is_empty());
    assert_eq!(ali.headers["x-acs-date"], "2023-11-14T22:13:20Z");
    assert_eq!(
        ali.headers["authorization"],
        include_str!("aliyun-vector.txt").trim()
    );
    let tc = request(
        "Tencent SMS",
        &c,
        &numbers(),
        &params(),
        1700000000,
        "unused",
    );
    assert_eq!(
        serde_json::from_str::<Value>(&tc.body).unwrap()["TemplateParamSet"],
        json!(["1234", "A & B"])
    );
    assert_eq!(
        tc.headers["authorization"],
        include_str!("tencent-vector.txt").trim()
    );
}
#[test]
fn provider_acceptance_rejection_partial_and_malformed_are_distinct() {
    assert_eq!(
        outcome("Aliyun SMS", &json!({"Code":"OK"}), &numbers()),
        "succeeded"
    );
    assert_eq!(
        outcome(
            "Aliyun SMS",
            &json!({"Code":"isv.BUSINESS_LIMIT_CONTROL"}),
            &numbers()
        ),
        "failed"
    );
    assert_eq!(outcome("Aliyun SMS", &json!({}), &numbers()), "unknown");
    let reply = |code: &str| json!({"Response":{"SendStatusSet":[{"PhoneNumber":"+8613800000000","Code":code}]}});
    assert_eq!(
        outcome("Tencent SMS", &reply("Ok"), &numbers()),
        "succeeded"
    );
    assert_eq!(
        outcome("Tencent SMS", &reply("FailedOperation"), &numbers()),
        "failed"
    );
    assert_eq!(
        outcome(
            "Tencent SMS",
            &json!({"Response":{"Error":{"Code":"AuthFailure"}}}),
            &numbers()
        ),
        "failed"
    );
    assert_eq!(
        outcome("Tencent SMS", &reply("Ok"), &["+8613800000001".into()]),
        "unknown"
    );
    let mut mixed = reply("Ok");
    mixed["Response"]["SendStatusSet"]
        .as_array_mut()
        .unwrap()
        .push(json!({"PhoneNumber":"+8613800000001","Code":"FailedOperation"}));
    assert_eq!(
        outcome(
            "Tencent SMS",
            &mixed,
            &["+8613800000000".into(), "+8613800000001".into()]
        ),
        "unknown"
    );
}
#[tokio::test]
async fn signed_http_fixture_handles_success_errors_and_invalid_replies() {
    use axum::{Router, body::Bytes, http::HeaderMap, routing::post};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let count = Arc::new(AtomicUsize::new(0));
    let seen = count.clone();
    let router=Router::new().route("/",post(move |headers:HeaderMap,body:Bytes| { let seen=seen.clone();async move {
        assert!(headers["authorization"].to_str().unwrap().contains("SignedHeaders=content-type;host"));
        assert_eq!(headers["x-tc-action"],"SendSms");
        assert_eq!(headers["host"],"sms.tencentcloudapi.com");
        let payload:Value=serde_json::from_slice(&body).unwrap();assert_eq!(payload["TemplateParamSet"],json!(["1234","A & B"]));
        match seen.fetch_add(1,Ordering::SeqCst) {
            0 => json!({"Response":{"SendStatusSet":[{"PhoneNumber":"+8613800000000","Code":"Ok"}]}}).to_string(),
            1 => json!({"Response":{"Error":{"Code":"AuthFailure"}}}).to_string(),
            _ => "malformed response".into()
        }
    }}));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    for expected in ["succeeded", "failed", "unknown"] {
        let mut r = request(
            "Tencent SMS",
            &config(),
            &numbers(),
            &params(),
            1700000000,
            "fixture",
        );
        r.url = format!("http://{address}/");
        assert_eq!(
            execute(&client, r, "Tencent SMS", &numbers()).await,
            expected
        );
    }
    assert_eq!(count.load(Ordering::SeqCst), 3);
    task.abort();
}

#[tokio::test]
async fn aliyun_http_fixture_checks_signed_query_and_ambiguous_http_response() {
    use axum::{
        Router,
        extract::RawQuery,
        http::{HeaderMap, StatusCode},
        routing::post,
    };
    let app = Router::new().route(
        "/",
        post(|headers: HeaderMap, RawQuery(query): RawQuery| async move {
            assert_eq!(headers["x-acs-action"], "SendSms");
            assert_eq!(headers["x-acs-version"], "2017-05-25");
            assert_eq!(headers["host"], "dysmsapi.aliyuncs.com");
            assert!(
                headers["authorization"]
                    .to_str()
                    .unwrap()
                    .starts_with("ACS3-HMAC-SHA256 Credential=fixtureAccess,")
            );
            let query: BTreeMap<String, String> =
                reqwest::Url::parse(&format!("http://localhost/?{}", query.unwrap()))
                    .unwrap()
                    .query_pairs()
                    .into_owned()
                    .collect();
            assert_eq!(query["PhoneNumbers"], "13800000000");
            assert_eq!(
                serde_json::from_str::<Value>(&query["TemplateParam"]).unwrap(),
                json!({"first":"A & B","second":"1234"})
            );
            (StatusCode::INTERNAL_SERVER_ERROR, "{\"Code\":\"OK\"}")
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let mut r = request(
        "Aliyun SMS",
        &config(),
        &numbers(),
        &params(),
        1700000000,
        "fixture",
    );
    let query = r.url.split_once('?').unwrap().1.to_owned();
    r.url = format!("http://{address}/?{query}");
    assert_eq!(
        execute(&reqwest::Client::new(), r, "Aliyun SMS", &numbers()).await,
        "unknown"
    );
    task.abort();
}
