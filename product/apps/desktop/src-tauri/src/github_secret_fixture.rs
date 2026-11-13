//! Owned native dialog probe: only a fixed synthetic value is accepted.
use super::*;
use base64::Engine;
#[link(name = "sodium")]
unsafe extern "C" {
    fn sodium_init() -> i32;
    fn crypto_box_seed_keypair(pk: *mut u8, sk: *mut u8, seed: *const u8) -> i32;
    fn crypto_box_seal_open(
        m: *mut u8,
        c: *const u8,
        len: u64,
        pk: *const u8,
        sk: *const u8,
    ) -> i32;
}
pub(super) fn set(
    f: &Fixture,
    account: &Account,
    op: &Operation,
    entry: &dyn ag1_platform::github::secrets::PrivateEntry,
    c: &ScanControl<'_>,
) -> Result<Reply> {
    if f.account(c)? != *account {
        return Err(ApiError::new("fixture", "Account changed."));
    }
    let secret = entry.enter(account, op, c)?;
    check(c)?;
    let mut pk = [0; 32];
    let mut sk = [0; 32];
    unsafe {
        assert!(sodium_init() >= 0);
        assert_eq!(
            crypto_box_seed_keypair(pk.as_mut_ptr(), sk.as_mut_ptr(), [7; 32].as_ptr()),
            0
        );
    }
    let encoded = secret.seal(&base64::engine::general_purpose::STANDARD.encode(pk))?;
    drop(secret);
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .unwrap();
    let mut plain = vec![0; bytes.len() - 48];
    unsafe {
        assert_eq!(
            crypto_box_seal_open(
                plain.as_mut_ptr(),
                bytes.as_ptr(),
                bytes.len() as u64,
                pk.as_ptr(),
                sk.as_ptr()
            ),
            0
        );
    }
    let matches = plain == b"ownedfixturevalue";
    plain.fill(0);
    sk.fill(0);
    if !matches {
        return Err(ApiError::new(
            "fixture",
            "Only the owned synthetic fixture value is admitted.",
        ));
    }
    let operation = serde_json::to_value(op).unwrap();
    let name = operation["endpoint"]
        .as_str()
        .unwrap()
        .rsplit('/')
        .next()
        .unwrap();
    let mut rows = f.secrets.lock().unwrap();
    rows.retain(|s| s["name"] != name);
    rows.push(json!({"name":name,"created_at":"2026-09-10T12:00:00Z","updated_at":"2026-09-10T12:00:00Z","visibility":operation["body"]["visibility"]}));
    let mut serial = f.serial.lock().unwrap();
    *serial += 1;
    let mut log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(f.root.join("github-requests.jsonl"))
        .unwrap();
    writeln!(
        log,
        "{}",
        json!({"serial":*serial,"operation":operation,"native_entry_and_sealed_roundtrip":true})
    )
    .unwrap();
    ag1_platform::github::secrets::set_receipt(op, 204)
}
pub(super) fn metadata(f: &Fixture, value: &serde_json::Value, _op: &Operation) -> Result<Reply> {
    let id = value["id"].as_str().unwrap();
    let endpoint = value["endpoint"].as_str().unwrap();
    let mut rows = f.secrets.lock().unwrap();
    let data = match id {
        "secrets.list" => json!({"total_count":rows.len(),"secrets":*rows}),
        "secrets.get" => rows
            .iter()
            .find(|r| endpoint.ends_with(r["name"].as_str().unwrap()))
            .cloned()
            .unwrap_or(json!({"name":"OWNED_SECRET"})),
        "secrets.delete" => {
            rows.retain(|r| !endpoint.ends_with(r["name"].as_str().unwrap()));
            serde_json::Value::Null
        }
        "secrets.repositories" => {
            json!({"total_count":1,"repositories":[{"id":42,"full_name":"owned/fixture"}]})
        }
        "secrets.set_repositories" => serde_json::Value::Null,
        _ => {
            return Err(ApiError::new(
                "fixture",
                "Unsupported secret fixture operation.",
            ));
        }
    };
    Ok(Reply {
        data,
        status: if value["method"] == "GET" { 200 } else { 204 },
        next: None,
        rate_remaining: Some(4999),
        rate_reset: None,
        rate_limit: None,
        rate_resource: None,
        outcome: if value["method"] == "GET" {
            Outcome::Read
        } else {
            Outcome::Applied
        },
    })
}
