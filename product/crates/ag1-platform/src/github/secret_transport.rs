//! Fixed GitHub origin; plaintext exists only in native entry and sealing.
use super::*;
use crate::github::secrets::{PrivateEntry, encrypted_body, public_key_operation, set_receipt};
async fn set_to(
    client: &Client,
    token: &Token,
    operation: &Operation,
    body: Value,
    control: &ScanControl<'_>,
    origin: &str,
) -> Result<Reply> {
    check(control)?;
    let mut encrypted = operation.clone();
    encrypted.body = Some(body);
    // This native request is never a Prepared value, renderer reply or log.
    super::super::schemas::validate(&Operation {
        id: "native.secret.write".into(),
        ..encrypted.clone()
    })
    .map_err(|_| {
        ApiError::new(
            "schema",
            "Secret request does not match the pinned GitHub schema.",
        )
    })?;
    let reply = send_to(client, token, &encrypted, control, origin).await?;
    set_receipt(operation, reply.status)
}
pub(super) fn execute(
    expected: &Account,
    operation: &Operation,
    entry: &dyn PrivateEntry,
    control: &ScanControl<'_>,
) -> Result<Reply> {
    network(|| {
        let token = token(control)?;
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| ApiError::new("transport", "Cannot start GitHub worker."))?;
        rt.block_on(async {
            let client=client()?;
            if identify(&client,&token,control).await?!=*expected{return Err(ApiError::new("stale_account","GitHub account changed before private entry."));}
            let key=send(&client,&token,&public_key_operation(operation)?,control).await?;
            crate::github::secrets::validate_public_key(&key.data)?;
            // Validate public key before asking for a value. No value preview.
            let secret=entry.enter(expected,operation,control)?;
            check(control)?;
            let body=encrypted_body(operation,&secret,&key.data)?;drop(secret);
            // Entry can take time: obtain credentials again and compare both
            // identity and credential incarnation before any secret write.
            let fresh=super::token(control)?;
            if identify(&client,&fresh,control).await?!=*expected{return Err(ApiError::new("stale_account","GitHub account or credential changed while entering the secret. The value was discarded."));}
            set_to(&client,&fresh,operation,body,control,"https://api.github.com").await
        })
    })
}
// This fixture verifies the Linux native secret-entry path against libsodium.
// AG1W deliberately leaves secret writes unavailable until it has an equivalent
// Windows-native entry flow, so it must not pull a Unix-only linker dependency.
#[cfg(all(test, target_os = "linux"))]
#[path = "secret_transport_fixture.rs"]
mod fixture;
