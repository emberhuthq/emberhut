//! The shared connect vectors (`tests/fixtures/connect/`, written by `gen/gen.py`): keys, ids,
//! encodings, normal forms, names, signed statements, the verification rule, probe answers and
//! their bindings, pairing links, and the wire JSON of the connect routes, the relay protocol
//! and the directory. Every vector is checked here (the raw-message signature cases in
//! emberhut-identity); the TypeScript client checks the same files.

use std::fmt::{Debug, Write as _};

use emberhut_api::auth::{AuthReply, PairRequest};
use emberhut_api::connect::{
    LinkStatementRequest, MachineIdentity, ProbeAnswer, ProbeError, ProbeQuery, Route, RouteKind,
    RouteList, SignedLink, SshForward,
};
use emberhut_api::directory::{DirectoryError, MachineList, UnlinkRequest};
use emberhut_api::relay::{Auth, CloseCode, CloseReason, CloseReasonError, Control, RelayError};
use emberhut_identity::{
    AccountId, AccountIdError, DecodeError, Host, HostError, KeyError, LinkStatement, LocalAddr,
    LocalAddrError, MachineId, MachineName, MachineNameError, Nonce, PairingLink, PairingLinkError,
    ProbeKind, ProbeStatement, PublicKey, Reached, ReachedError, RelayAuthStatement, RelayDomain,
    RelayDomainError, Scheme, SecretKey, Signature, Statement, TimeOutOfRange, UnixMillis,
    UnlinkStatement, Via, ViaError,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::support::{fixture, from_value, to_value};

fn vectors(name: &str) -> Value {
    fixture(&format!("connect/{name}"))
}

fn list(value: &Value) -> &[Value] {
    value
        .as_array()
        .unwrap_or_else(|| panic!("{value} is a list"))
}

fn text(value: &Value) -> &str {
    value.as_str().unwrap_or_else(|| panic!("{value} is text"))
}

fn hex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect()
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut out, byte| {
        write!(out, "{byte:02x}").unwrap();
        out
    })
}

fn time(value: &Value) -> UnixMillis {
    UnixMillis::new(value.as_u64().unwrap()).unwrap()
}

/// A key of `keys.json`, by name.
fn key(name: &str) -> SecretKey {
    let keys = vectors("keys.json");
    let entry = list(&keys["keys"])
        .iter()
        .find(|key| key["name"] == name)
        .unwrap_or_else(|| panic!("no key {name}"));
    SecretKey::from_bytes(&hex(text(&entry["seed"])).try_into().unwrap())
}

fn machine() -> MachineId {
    key("machine").machine_id()
}

fn decode_error(error: &DecodeError) -> &'static str {
    match error {
        DecodeError::Length { .. } => "length",
        DecodeError::Symbol { .. } => "symbol",
        DecodeError::TrailingBits => "trailingBits",
        other => panic!("unexpected {other:?}"),
    }
}

fn key_error(error: &KeyError) -> &'static str {
    match error {
        KeyError::Decode(error) => decode_error(error),
        KeyError::NotAPoint => "notAPoint",
        KeyError::Weak => "weak",
        KeyError::NotTorsionFree => "notTorsionFree",
        other => panic!("unexpected {other:?}"),
    }
}

fn host_error(error: &HostError) -> &'static str {
    match error {
        HostError::Empty => "empty",
        HostError::Port => "port",
        HostError::Ipv6 => "ipv6",
        HostError::Name => "name",
        HostError::NumericName => "numericName",
        HostError::NotNormal { .. } => "notNormal",
        other => panic!("unexpected {other:?}"),
    }
}

fn scheme(case: &Value) -> Scheme {
    match text(&case["scheme"]) {
        "http" => Scheme::Http,
        "https" => Scheme::Https,
        other => panic!("scheme {other}"),
    }
}

/// `value` reads as `T` and writes back exactly.
fn round_trip<T: DeserializeOwned + Serialize>(value: &Value) -> T {
    let read: T = from_value(value.clone());
    assert_eq!(&to_value(&read), value, "doesn't round-trip");
    read
}

/// `plain` reads as `T` and writes back exactly, and `extended` (the same with fields no
/// version knows) reads as the same value.
fn reads<T: DeserializeOwned + Serialize + PartialEq + Debug>(
    plain: &Value,
    extended: &Value,
) -> T {
    let read: T = round_trip(plain);
    assert_eq!(
        from_value::<T>(extended.clone()),
        read,
        "unknown fields are ignored"
    );
    read
}

/// The error the vectors say refuses `message` (`refusedBy`), as its text, which the serde error
/// must carry.
fn refusal(refused_by: &str, message: &Value) -> String {
    let field = |name: &str| text(&message[name]).to_owned();
    match refused_by {
        "wrongKey" => {
            let machine = message.get("machine").unwrap_or(message);
            let (id, key) = (text(&machine["id"]), text(&machine["publicKey"]));
            MachineIdentity::new(id.parse().unwrap(), key.parse().unwrap())
                .unwrap_err()
                .to_string()
        }
        "reached" => {
            let via: Via = field("via").parse().unwrap();
            let local = field("local");
            let local = (!local.is_empty()).then(|| local.parse().unwrap());
            assert_eq!(Reached::new(via, local), Err(ReachedError));
            ReachedError.to_string()
        }
        "host" => field("host").parse::<Host>().unwrap_err().to_string(),
        "local" => field("local").parse::<LocalAddr>().unwrap_err().to_string(),
        "publicKey" => field("publicKey")
            .parse::<PublicKey>()
            .unwrap_err()
            .to_string(),
        "via" => {
            assert_eq!(field("via").parse::<Via>(), Err(ViaError));
            ViaError.to_string()
        }
        "name" => field("name")
            .parse::<MachineName>()
            .unwrap_err()
            .to_string(),
        "time" => TimeOutOfRange.to_string(),
        other => panic!("refusedBy {other}"),
    }
}

/// `message` doesn't read as `T`, and what refuses it is what `refused_by` names.
fn refused<T: DeserializeOwned + Debug>(message: &Value, refused_by: &Value) {
    let error = serde_json::from_value::<T>(message.clone())
        .unwrap_err()
        .to_string();
    let expected = refusal(text(refused_by), message);
    assert!(error.contains(&expected), "{error:?} is not {expected:?}");
}

#[test]
fn keys_come_from_their_seeds_and_ids_from_their_keys() {
    let keys = vectors("keys.json");
    for entry in list(&keys["keys"]) {
        let public = key(text(&entry["name"])).public_key();
        assert_eq!(public.to_string(), text(&entry["publicKey"]));
        assert_eq!(to_hex(public.as_bytes()), text(&entry["publicKeyHex"]));
        let mut input = b"emberhut machine id v1\0\0\0\0\x20".to_vec();
        input.extend_from_slice(public.as_bytes());
        assert_eq!(to_hex(&input), text(&entry["machineIdInput"]));
        let id = public.machine_id();
        assert_eq!(id.to_string(), text(&entry["machineId"]));
        assert_eq!(to_hex(id.as_bytes()), text(&entry["machineIdHash"])[..40]);
        assert_eq!(text(&entry["publicKey"]).parse::<PublicKey>(), Ok(public));
    }
}

#[test]
fn binary_values_and_times_have_one_spelling() {
    let encodings = vectors("encodings.json");
    for case in list(&encodings["nonces"]["valid"]) {
        let nonce: Nonce = text(&case["text"]).parse().unwrap();
        assert_eq!(to_hex(nonce.as_bytes()), text(&case["hex"]));
        assert_eq!(nonce.to_string(), text(&case["text"]));
    }
    for case in list(&encodings["nonces"]["invalid"]) {
        let error = text(&case["text"]).parse::<Nonce>().unwrap_err();
        assert_eq!(decode_error(&error), case["error"], "{case}");
    }
    for case in list(&encodings["signatures"]["valid"]) {
        let signature: Signature = text(&case["text"]).parse().unwrap();
        assert_eq!(to_hex(&signature.to_bytes()), text(&case["hex"]));
        assert_eq!(signature.to_string(), text(&case["text"]));
    }
    for case in list(&encodings["signatures"]["invalid"]) {
        let error = text(&case["text"]).parse::<Signature>().unwrap_err();
        assert_eq!(decode_error(&error), case["error"], "{case}");
    }
    for id in list(&encodings["machineIds"]["valid"]) {
        assert_eq!(text(id).parse::<MachineId>().unwrap().to_string(), text(id));
    }
    for case in list(&encodings["machineIds"]["invalid"]) {
        let error = text(&case["text"]).parse::<MachineId>().unwrap_err();
        assert_eq!(decode_error(&error), case["error"], "{case}");
    }
    for case in list(&encodings["publicKeys"]["invalid"]) {
        let error = text(&case["text"]).parse::<PublicKey>().unwrap_err();
        assert_eq!(key_error(&error), case["error"], "{case}");
    }
    for millis in list(&encodings["times"]["valid"]) {
        assert_eq!(from_value::<UnixMillis>(millis.clone()), time(millis));
    }
    for case in list(&encodings["times"]["invalid"]) {
        assert!(UnixMillis::new(case["millis"].as_u64().unwrap()).is_err());
        assert!(serde_json::from_value::<UnixMillis>(case["millis"].clone()).is_err());
    }
}

#[test]
fn hosts_have_one_normal_form() {
    let hosts = vectors("hosts.json");
    for case in list(&hosts["normalize"]) {
        let host = Host::normalize(text(&case["input"]), scheme(case)).unwrap();
        assert_eq!(host.as_str(), text(&case["normal"]), "{case}");
        assert_eq!(host.as_str().parse::<Host>(), Ok(host.clone()), "{case}");
    }
    let differences = list(&hosts["whatwgDifferences"]["cases"]);
    for case in list(&hosts["invalid"]).iter().chain(differences) {
        let error = Host::normalize(text(&case["input"]), scheme(case)).unwrap_err();
        assert_eq!(host_error(&error), case["error"], "{case}");
    }
    for case in list(&hosts["parse"]) {
        match text(&case["text"]).parse::<Host>() {
            Ok(host) => assert_eq!(host.as_str(), text(&case["normal"]), "{case}"),
            Err(error) => {
                assert_eq!(host_error(&error), case["error"], "{case}");
                if let HostError::NotNormal { normal } = error {
                    assert_eq!(normal, text(&case["normalForm"]), "{case}");
                }
            }
        }
    }
    let domains = &hosts["relayDomains"];
    for case in list(&domains["valid"]) {
        let domain = RelayDomain::normalize(text(&case["input"])).unwrap();
        assert_eq!(domain.as_str(), text(&case["normal"]));
        assert_eq!(domain.as_str().parse::<RelayDomain>(), Ok(domain.clone()));
    }
    for case in list(&domains["invalid"]) {
        let error = match RelayDomain::normalize(text(&case["input"])).unwrap_err() {
            RelayDomainError::Host(_) => "host",
            RelayDomainError::NotAName => "notAName",
            RelayDomainError::TooLong => "tooLong",
            other => panic!("unexpected {other:?}"),
        };
        assert_eq!(error, case["error"], "{case}");
    }
}

#[test]
fn socket_addresses_have_one_normal_form() {
    let addresses = vectors("socket_addresses.json");
    for case in list(&addresses["normalize"]) {
        let addr: std::net::SocketAddr = text(&case["input"]).parse().unwrap();
        assert_eq!(LocalAddr::new(addr).to_string(), text(&case["normal"]));
    }
    for case in list(&addresses["parse"]) {
        match text(&case["text"]).parse::<LocalAddr>() {
            Ok(local) => assert_eq!(local.to_string(), text(&case["normal"])),
            Err(LocalAddrError::Invalid) => assert_eq!(case["error"], "invalid", "{case}"),
            Err(LocalAddrError::NotNormal { normal }) => {
                assert_eq!(case["error"], "notNormal", "{case}");
                assert_eq!(normal, text(&case["normalForm"]));
            }
            Err(other) => panic!("unexpected {other:?}"),
        }
    }
}

#[test]
fn names_kinds_and_account_ids_are_bounded() {
    let names = vectors("names.json");
    let name_error = |error: MachineNameError| match error {
        MachineNameError::Empty => "empty",
        MachineNameError::TooLong => "tooLong",
        MachineNameError::NotTrimmed => "notTrimmed",
        MachineNameError::Control => "control",
        other => panic!("unexpected {other:?}"),
    };
    for case in list(&names["machineNames"]["cut"]) {
        match MachineName::cut(text(&case["input"])) {
            Ok(name) => assert_eq!(name.as_str(), text(&case["name"]), "{case}"),
            Err(error) => assert_eq!(name_error(error), case["error"], "{case}"),
        }
    }
    for case in list(&names["machineNames"]["parse"]) {
        match text(&case["text"]).parse::<MachineName>() {
            Ok(name) => assert_eq!(name.as_str(), text(&case["name"]), "{case}"),
            Err(error) => assert_eq!(name_error(error), case["error"], "{case}"),
        }
    }
    for kind in list(&names["probeKinds"]["valid"]) {
        assert_eq!(
            text(kind).parse::<ProbeKind>().unwrap().as_str(),
            text(kind)
        );
    }
    for kind in list(&names["probeKinds"]["invalid"]) {
        assert!(text(kind).parse::<ProbeKind>().is_err(), "{kind}");
    }
    for id in list(&names["accountIds"]["valid"]) {
        assert_eq!(AccountId::new(text(id)).unwrap().as_str(), text(id));
    }
    for case in list(&names["accountIds"]["invalid"]) {
        let error = match AccountId::new(text(&case["text"])).unwrap_err() {
            AccountIdError::Length => "length",
            AccountIdError::Character => "character",
            other => panic!("unexpected {other:?}"),
        };
        assert_eq!(error, case["error"], "{case}");
    }
}

/// Checks a statement against its vector: its bytes, and the signer's signature over them.
fn check_statement<S: Statement>(statement: &S, case: &Value) {
    assert_eq!(S::PREFIX, text(&case["prefix"]));
    assert_eq!(
        to_hex(&statement.to_bytes()),
        text(&case["bytes"]),
        "{case}"
    );
    let signer = key(text(&case["signer"]));
    let signature: Signature = text(&case["signature"]).parse().unwrap();
    assert_eq!(signer.sign(statement), signature, "{case}");
    assert_eq!(signer.public_key().verify(statement, &signature), Ok(()));
}

fn unlink_statement(inputs: &Value) -> UnlinkStatement {
    UnlinkStatement::new(
        text(&inputs["machineId"]).parse().unwrap(),
        text(&inputs["relayDomain"]).parse().unwrap(),
        time(&inputs["time"]),
    )
}

#[test]
fn statements_encode_and_sign_as_the_vectors_say() {
    let statements = vectors("statements.json");
    for case in list(&statements["statements"]) {
        let inputs = &case["inputs"];
        let parse = |field: &str| text(&inputs[field]).to_owned();
        match text(&case["prefix"]) {
            "emberhut probe v1" => {
                let local = parse("local");
                let via = parse("via").parse().unwrap();
                let local = (!local.is_empty()).then(|| local.parse().unwrap());
                let statement = ProbeStatement::new(
                    parse("nonce").parse().unwrap(),
                    parse("machineId").parse().unwrap(),
                    parse("kind").parse().unwrap(),
                    Reached::new(via, local).unwrap(),
                    parse("host").parse().unwrap(),
                );
                check_statement(&statement, case);
            }
            "emberhut relay auth v1" => {
                let statement = RelayAuthStatement::new(
                    parse("relayDomain").parse().unwrap(),
                    parse("nonce").parse().unwrap(),
                    parse("name").parse().unwrap(),
                    time(&inputs["time"]),
                );
                check_statement(&statement, case);
            }
            "emberhut link v1" => {
                let statement = LinkStatement::new(
                    parse("account").parse().unwrap(),
                    parse("relayDomain").parse().unwrap(),
                    parse("name").parse().unwrap(),
                    time(&inputs["time"]),
                );
                check_statement(&statement, case);
            }
            "emberhut unlink v1" => check_statement(&unlink_statement(inputs), case),
            other => panic!("prefix {other}"),
        }
    }
}

#[test]
fn signatures_follow_the_one_verification_rule() {
    let signatures = vectors("signatures.json");
    let statement = unlink_statement(&signatures["statement"]["inputs"]);
    assert_eq!(
        to_hex(&statement.to_bytes()),
        text(&signatures["statement"]["bytes"])
    );
    for case in list(&signatures["cases"])
        .iter()
        .chain(list(&signatures["crossKind"]))
    {
        let signature: Signature = text(&case["signature"]).parse().unwrap();
        let accepted = match text(&case["publicKey"]).parse::<PublicKey>() {
            Ok(key) => key.verify(&statement, &signature).is_ok(),
            Err(error) => {
                assert_eq!(key_error(&error), case["keyError"], "{case}");
                false
            }
        };
        assert_eq!(Some(accepted), case["accept"].as_bool(), "{case}");
    }
}

fn probe_query(nonce: &Value, kind: &Value) -> (Nonce, ProbeKind) {
    (text(nonce).parse().unwrap(), text(kind).parse().unwrap())
}

#[test]
fn probe_answers_verify_or_fail_as_the_vectors_say() {
    let probes = vectors("probes.json");
    for case in list(&probes["probes"]) {
        let (nonce, kind) = probe_query(&case["query"]["nonce"], &case["query"]["kind"]);
        let expected: MachineId = text(&case["expected"]).parse().unwrap();
        if case["error"] == "decode" {
            refused::<ProbeAnswer>(&case["answer"], &case["refusedBy"]);
            continue;
        }
        let answer: ProbeAnswer = from_value(case["answer"].clone());
        match answer.verify(&expected, nonce, &kind) {
            Ok(verified) => {
                assert_eq!(case["valid"], true, "{case}");
                assert_eq!(to_value(&answer), case["answer"], "it writes back the same");
                let statement = verified.statement();
                let signed = ProbeAnswer::sign(
                    &key("machine"),
                    nonce,
                    &kind,
                    *statement.reached(),
                    statement.host().clone(),
                );
                assert_eq!(signed, answer, "signing is deterministic");
            }
            Err(ProbeError::OtherMachine { .. }) => assert_eq!(case["error"], "otherMachine"),
            Err(ProbeError::Signature(_)) => assert_eq!(case["error"], "signature", "{case}"),
            Err(other) => panic!("unexpected {other:?}"),
        }
    }
}

#[test]
fn verified_answers_bind_to_their_own_route_only() {
    let probes = vectors("probes.json");
    for case in list(&probes["bindings"]) {
        let named = &probes["answers"][text(&case["answer"])];
        let (nonce, kind) = probe_query(&probes["nonce"], &named["kind"]);
        let answer: ProbeAnswer = from_value(named["answer"].clone());
        let verified = answer.verify(&machine(), nonce, &kind).unwrap();
        let route: Route = from_value(case["route"].clone());
        assert_eq!(
            Some(verified.binding_matches(&route)),
            case["matches"].as_bool(),
            "{case}"
        );
    }
}

#[test]
fn ssh_answers_bind_to_their_own_forward_only() {
    let probes = vectors("probes.json");
    for case in list(&probes["sshBindings"]) {
        let named = &probes["answers"][text(&case["answer"])];
        let (nonce, kind) = probe_query(&probes["nonce"], &named["kind"]);
        let answer: ProbeAnswer = from_value(named["answer"].clone());
        let verified = answer.verify(&machine(), nonce, &kind).unwrap();
        let port = |name: &str| u16::try_from(case["forward"][name].as_u64().unwrap()).unwrap();
        let forward = SshForward {
            local_port: port("localPort"),
            machine_port: port("machinePort"),
        };
        assert_eq!(
            Some(verified.forward_matches(forward)),
            case["matches"].as_bool(),
            "{case}"
        );
    }
}

#[test]
fn pairing_links_read_and_write_as_the_vectors_say() {
    let links = vectors("pairing_links.json");
    for case in list(&links["valid"]) {
        let link = PairingLink::parse(text(&case["link"])).unwrap();
        assert_eq!(link.id().to_string(), text(&case["id"]));
        assert_eq!(link.code(), text(&case["code"]));
        let routes: Vec<&str> = list(&case["routes"]).iter().map(text).collect();
        assert_eq!(link.routes(), routes);
        if case["canonical"] == true {
            assert_eq!(link.to_url(), text(&case["link"]));
        }
        assert_eq!(PairingLink::parse(&link.to_url()), Ok(link));
    }
    for case in list(&links["invalid"]) {
        let error = match PairingLink::parse(text(&case["link"])).unwrap_err() {
            PairingLinkError::NotAPairingLink => "notAPairingLink",
            PairingLinkError::TooLong => "tooLong",
            PairingLinkError::Missing(_) => "missing",
            PairingLinkError::Repeated(_) => "repeated",
            PairingLinkError::TooManyRoutes => "tooManyRoutes",
            PairingLinkError::EmptyRoute => "emptyRoute",
            PairingLinkError::Id(_) => "id",
            other => panic!("unexpected {other:?}"),
        };
        assert_eq!(error, case["error"], "{case}");
    }
}

/// A query string as a JSON object, as a query extractor reads it.
fn query(text: &str) -> Value {
    Value::Object(
        text.split('&')
            .map(|pair| {
                let (key, value) = pair.split_once('=').unwrap();
                (key.to_owned(), Value::from(value))
            })
            .collect(),
    )
}

#[test]
fn connect_messages_read_as_sent_and_inconsistent_ones_dont() {
    let connect = vectors("connect.json");
    let (plain, extended) = (&connect["messages"], &connect["withUnknownFields"]);
    let both = |name: &str| (&plain[name], &extended[name]);
    let (a, b) = both("routeList");
    let list: RouteList = reads(a, b);
    assert_eq!(list.machine.id(), &machine());
    assert_eq!(list.name.as_str(), "Main computer");
    assert_eq!(list.routes[0].kind, RouteKind::Loopback);
    assert_eq!(list.routes[2].label.as_deref(), Some("WireGuard"));
    assert!(
        !list.routes[5].kind.is_known(),
        "kinds it doesn't know are kept"
    );
    let (a, b) = both("pairRequest");
    let request: PairRequest = reads(a, b);
    assert_eq!(request.public_key, Some(key("device").public_key()));
    let (a, b) = both("pairAnswer");
    let answer: AuthReply = reads(a, b);
    assert_eq!(
        answer.machine,
        Some(MachineIdentity::of(key("machine").public_key()))
    );
    let (a, b) = both("pairAnswerLegacy");
    let legacy: AuthReply = reads(a, b);
    assert_eq!(legacy.machine, None, "a legacy host's answer");
    assert!(legacy.into_grant().is_ok());
    let (a, b) = both("linkStatementRequest");
    let _request: LinkStatementRequest = reads(a, b);
    let (a, b) = both("signedLink");
    let _link: SignedLink = reads(a, b);
    let (a, b) = both("probeAnswer");
    let _answer: ProbeAnswer = reads(a, b);
    for (name, case) in connect["unreadable"].as_object().unwrap() {
        let (message, refused_by) = (&case["message"], &case["refusedBy"]);
        if name.starts_with("pairAnswer") {
            refused::<AuthReply>(message, refused_by);
        } else if name.starts_with("routeList") {
            refused::<RouteList>(message, refused_by);
        } else if name.starts_with("signedLink") {
            refused::<SignedLink>(message, refused_by);
        } else {
            panic!("unreadable {name}");
        }
    }
    let query_ok: ProbeQuery = from_value(query(text(&connect["probeQuery"])));
    assert_eq!(query_ok.kind.as_str(), "lan");
    let bad = query(text(&connect["probeQueryBadKind"]));
    assert!(
        serde_json::from_value::<ProbeQuery>(bad).is_err(),
        "the server answers 400"
    );
}

#[test]
fn link_statements_verify_for_their_account_and_relay_only() {
    let connect = vectors("connect.json");
    let link: SignedLink = from_value(connect["messages"]["signedLink"].clone());
    let request: LinkStatementRequest =
        from_value(connect["messages"]["linkStatementRequest"].clone());
    let signed = SignedLink::sign(
        &key("machine"),
        link.name.clone(),
        request.account,
        request.relay_domain,
        link.time,
    );
    assert_eq!(signed, link, "signing is deterministic");
    for case in list(&connect["linkChecks"]) {
        let link: SignedLink = match text(&case["link"]) {
            "signedLink" => from_value(connect["messages"]["signedLink"].clone()),
            name => from_value(connect[name].clone()),
        };
        let account: AccountId = text(&case["account"]).parse().unwrap();
        let domain: RelayDomain = text(&case["relayDomain"]).parse().unwrap();
        match link.verify(&account, &domain) {
            Ok(id) => {
                assert_eq!(case["expect"], "ok", "{case}");
                assert_eq!(id, machine());
            }
            Err(error) => assert_eq!(error.code().as_str(), text(&case["expect"]), "{case}"),
        }
    }
}

#[test]
fn relay_control_messages_read_as_sent() {
    let relay = vectors("relay.json");
    let domain: RelayDomain = text(&relay["relayDomain"]).parse().unwrap();
    let (plain, extended) = (&relay["messages"], &relay["withUnknownFields"]);
    for name in ["hello", "auth", "welcome", "drain"] {
        let _message: Control = reads(&plain[name], &extended[name]);
    }
    let Control::Hello(hello) = from_value(plain["hello"].clone()) else {
        panic!("a hello");
    };
    assert_eq!(hello.version, emberhut_api::relay::VERSION);
    let Control::Auth(auth) = from_value(plain["auth"].clone()) else {
        panic!("an auth");
    };
    assert_eq!(auth.verify(domain.clone(), hello.nonce), Ok(machine()));
    let signed = Auth::sign(
        &key("machine"),
        auth.name.clone(),
        domain.clone(),
        &hello,
        auth.time,
    );
    assert_eq!(signed, *auth, "signing is deterministic");
    let Control::Auth(renamed) = from_value(relay["authRenamed"].clone()) else {
        panic!("an auth");
    };
    assert!(
        renamed.verify(domain, hello.nonce).is_err(),
        "the name is signed"
    );
    let Control::Welcome(welcome) = from_value(plain["welcome"].clone()) else {
        panic!("a welcome");
    };
    assert_eq!((welcome.limits.streams, welcome.ping), (256, 20));
    let unknown: Control = from_value(relay["unknownType"].clone());
    assert_eq!(unknown, Control::Unknown, "unknown types are ignored");
    for code in list(&relay["closeCodes"]) {
        let code = u16::try_from(code.as_u64().unwrap()).unwrap();
        assert_eq!(CloseCode::try_from(code).map(u16::from), Ok(code));
    }
    for word in list(&relay["relayErrors"]) {
        assert!(RelayError::from(text(word)).is_known(), "{word}");
    }
}

#[test]
fn close_reasons_read_and_write_as_the_vectors_say() {
    let relay = vectors("relay.json");
    for case in list(&relay["closeReasons"]["valid"]) {
        let reason: CloseReason = text(&case["text"]).parse().unwrap();
        assert_eq!(reason.reason(), text(&case["reason"]));
        assert_eq!(reason.time().map(UnixMillis::get), case["time"].as_u64());
        assert_eq!(reason.retry().map(u64::from), case["retry"].as_u64());
        let written = case.get("written").unwrap_or(&case["text"]);
        assert_eq!(reason.to_string(), text(written));
    }
    for case in list(&relay["closeReasons"]["invalid"]) {
        let error = match text(&case["text"]).parse::<CloseReason>().unwrap_err() {
            CloseReasonError::TooLong => "tooLong",
            CloseReasonError::Malformed => "malformed",
            CloseReasonError::NoReason => "noReason",
            CloseReasonError::Word => "word",
            other => panic!("unexpected {other:?}"),
        };
        assert_eq!(error, case["error"], "{case}");
    }
}

#[test]
fn directory_messages_read_as_sent_and_unlinks_check_their_context() {
    let directory = vectors("directory.json");
    let (plain, extended) = (&directory["messages"], &directory["withUnknownFields"]);
    let machines: MachineList = reads(&plain["machines"], &extended["machines"]);
    assert_eq!(machines.machines[1].last_seen, None);
    assert_eq!(machines.machines[0].machine.id(), &machine());
    let unlink: UnlinkRequest = reads(&plain["unlink"], &extended["unlink"]);
    let _error: DirectoryError = reads(&plain["error"], &extended["error"]);
    let wrong = &directory["unreadable"]["machinesWrongKey"];
    let error = serde_json::from_value::<MachineList>(wrong["message"].clone()).unwrap_err();
    let expected = refusal(text(&wrong["refusedBy"]), &wrong["message"]["machines"][0]);
    assert!(error.to_string().contains(&expected), "{error}");
    let signed = UnlinkRequest::sign(&key("machine"), unlink.relay_domain.clone(), unlink.time);
    assert_eq!(signed, unlink, "signing is deterministic");
    for case in list(&directory["unlinkChecks"]) {
        let path: MachineId = text(&case["path"]).parse().unwrap();
        let domain: RelayDomain = text(&case["relayDomain"]).parse().unwrap();
        match unlink.verify(&path, &domain) {
            Ok(()) => assert_eq!(case["expect"], "ok", "{case}"),
            Err(error) => assert_eq!(error.code().as_str(), text(&case["expect"]), "{case}"),
        }
    }
    let errors: Vec<DirectoryError> = list(&directory["errors"]).iter().map(round_trip).collect();
    assert!(errors[0].sign_in);
    assert!(errors[..3].iter().all(|error| error.code.is_known()));
    assert!(!errors[3].code.is_known(), "a code from a newer directory");
}
