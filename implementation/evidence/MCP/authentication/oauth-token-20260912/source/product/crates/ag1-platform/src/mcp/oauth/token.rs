//! Native token exchange and refresh. No automatic operation replay, DTO tokens,
//! unsigned JWT account labels, alternate issuer or credential fallback.
use super::{
    Network, invalid,
    login::{AuthorizedCode, form},
    network::{Reply, check},
    private_json::{self, Value},
    scopes, url,
};
use crate::{
    mcp::auth::{Binding, Grant, Material, Pending, Target, Vault},
    provider::http::CancelToken,
};
use ag1_core::{DomainError, SecretBytes};
use std::{collections::BTreeSet, sync::Arc, time::Duration};
use tokio::time::Instant;

/// A native reservation guard retires failed or abandoned exchanges, including
/// futures dropped after send. Cleanup cannot invalidate a newer generation.
struct Reservation {
    vault: Arc<Vault>,
    pending: Option<Pending>,
}
impl Drop for Reservation {
    fn drop(&mut self) {
        if let Some(pending) = &self.pending {
            let _ = self.vault.abort_pending(pending);
        }
    }
}
impl Reservation {
    fn pending(&self) -> &Pending {
        self.pending.as_ref().expect("owned reservation")
    }
    fn check(&self) -> Result<(), DomainError> {
        self.vault.check_pending(self.pending())
    }
}
/// Verified private response awaiting the manager's exact configuration/session
/// CAS. Drop revokes this reservation; commit is one-use and generation-fenced.
pub struct Tokens {
    reservation: Reservation,
    material: Material,
    grant: Grant,
}
impl Tokens {
    pub fn binding(&self) -> &Binding {
        self.reservation.pending().binding()
    }
    pub fn grant(&self) -> &Grant {
        &self.grant
    }
    pub fn commit(mut self, now_ms: u64) -> Result<u64, DomainError> {
        let result = self.reservation.vault.commit_reserved(
            self.reservation.pending(),
            self.material,
            self.grant,
            now_ms,
        );
        if result.is_ok() {
            self.reservation.pending.take();
        }
        result
    }
}
fn endpoint(target: &Target) -> Result<(&str, &str, &str), DomainError> {
    let Target::OAuth {
        token_endpoint,
        client_id,
        resource,
        ..
    } = target
    else {
        return Err(DomainError::PermissionDenied);
    };
    let parsed = url(token_endpoint, true)?;
    if parsed.query_pairs().any(|(k, _)| {
        [
            "grant_type",
            "client_id",
            "redirect_uri",
            "scope",
            "resource",
            "state",
            "code_challenge",
            "code_challenge_method",
        ]
        .contains(&k.as_ref())
    }) {
        return Err(invalid("OAuth token endpoint overrides client parameters"));
    }
    Ok((token_endpoint, client_id, resource))
}
impl Network {
    pub async fn exchange(
        &self,
        code: AuthorizedCode,
        vault: Arc<Vault>,
        pending: Pending,
        cancel: &mut CancelToken,
    ) -> Result<Tokens, DomainError> {
        // Reject a foreign reservation before taking custody (no revocation of it).
        if pending.binding() != &code.binding || pending.refresh_token().is_some() {
            return Err(DomainError::PermissionDenied);
        }
        let reservation = Reservation {
            vault,
            pending: Some(pending),
        };
        reservation.check()?;
        code.auth.check_code_flow()?;
        let (target, client_id, resource) = endpoint(&code.binding.target)?;
        if target != code.auth.view().token_endpoint || client_id != code.registration.client_id() {
            return Err(DomainError::PermissionDenied);
        }
        let body = form(&[
            ("grant_type", b"authorization_code"),
            ("client_id", client_id.as_bytes()),
            ("code", code.code.expose()),
            ("redirect_uri", code.redirect_uri.as_bytes()),
            ("code_verifier", code.verifier.expose()),
            ("resource", resource.as_bytes()),
        ])?;
        let deadline = code.deadline.min(Instant::now() + Duration::from_secs(20));
        let now = crate::mcp::http::credential_now()?;
        let admission = || reservation.check();
        let reply = self
            .request_checked(
                reqwest::Method::POST,
                target,
                Some(("application/x-www-form-urlencoded", body)),
                false,
                deadline,
                cancel,
                Some(&admission),
            )
            .await?;
        check(deadline, cancel)?;
        reservation.check()?;
        let (material, grant) = parse(
            reply,
            &code.binding.target,
            &code.auth.resource().scopes,
            None,
            now,
        )?;
        Ok(Tokens {
            reservation,
            material,
            grant,
        })
    }
    pub async fn refresh(
        &self,
        vault: Arc<Vault>,
        pending: Pending,
        cancel: &mut CancelToken,
    ) -> Result<Tokens, DomainError> {
        if pending.refresh_token().is_none() {
            return Err(DomainError::PermissionDenied);
        }
        let reservation = Reservation {
            vault,
            pending: Some(pending),
        };
        reservation.check()?;
        let pending = reservation.pending();
        pending.binding().validate()?;
        let (target, client_id, resource) = endpoint(&pending.binding().target)?;
        let body = form(&[
            ("grant_type", b"refresh_token"),
            ("client_id", client_id.as_bytes()),
            (
                "refresh_token",
                pending
                    .refresh_token()
                    .ok_or(DomainError::PermissionDenied)?
                    .expose(),
            ),
            ("resource", resource.as_bytes()),
        ])?;
        let deadline = Instant::now() + Duration::from_secs(20);
        let now = crate::mcp::http::credential_now()?;
        let admission = || reservation.check();
        let reply = self
            .request_checked(
                reqwest::Method::POST,
                target,
                Some(("application/x-www-form-urlencoded", body)),
                false,
                deadline,
                cancel,
                Some(&admission),
            )
            .await?;
        check(deadline, cancel)?;
        reservation.check()?;
        let (material, mut grant) = parse(
            reply,
            &pending.binding().target,
            &pending.grant().scopes,
            pending.refresh_token(),
            now,
        )?;
        if grant.scopes != pending.grant().scopes {
            return Err(DomainError::PermissionDenied);
        }
        grant.account = pending.grant().account.clone();
        material.reject_echo(&serde_json::to_value(&grant).map_err(|_| invalid("OAuth grant"))?)?;
        Ok(Tokens {
            reservation,
            material,
            grant,
        })
    }
}
fn required(value: &mut Value, key: &[u8]) -> Result<Value, DomainError> {
    value
        .take(key)
        .ok_or_else(|| invalid("OAuth token response required field missing"))
}
fn parse(
    reply: Reply,
    target: &Target,
    requested: &BTreeSet<String>,
    old_refresh: Option<&SecretBytes>,
    now: u64,
) -> Result<(Material, Grant), DomainError> {
    let mut types = reply.headers.get_all("content-type").iter();
    if types
        .next()
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.split(';').next())
        .is_none_or(|s| !s.trim().eq_ignore_ascii_case("application/json"))
        || types.next().is_some()
    {
        return Err(invalid("OAuth token response content type"));
    }
    let mut value = private_json::decode(reply.body.expose())?;
    if let Some(error) = value.take(b"error") {
        if value.take(b"access_token").is_some() || reply.status == 200 {
            return Err(invalid("ambiguous OAuth token response"));
        }
        return Err(DomainError::Unavailable(
            match error.text()? {
                "invalid_grant" => "MCP OAuth grant expired or revoked; sign in again",
                "invalid_client" | "unauthorized_client" => {
                    "MCP OAuth client registration was rejected"
                }
                "invalid_scope" => "MCP OAuth scopes were rejected; review sign-in",
                _ => "MCP OAuth token request was rejected",
            }
            .into(),
        ));
    }
    if reply.status != 200 {
        return Err(DomainError::Unavailable(
            "MCP OAuth token endpoint unavailable; sign in again".into(),
        ));
    }
    if !required(&mut value, b"token_type")?
        .text()?
        .eq_ignore_ascii_case("Bearer")
    {
        return Err(invalid("unsupported OAuth token type"));
    }
    let access = required(&mut value, b"access_token")?.secret()?;
    let refresh = match value.take(b"refresh_token") {
        Some(v) => Some(v.secret()?),
        None => old_refresh.map(|s| SecretBytes::new(s.expose().to_vec())),
    };
    let expires_at_ms = match value.take(b"expires_in") {
        None => None,
        Some(Value::Number(seconds)) if seconds > 0 && seconds <= 365 * 24 * 60 * 60 => Some(
            now.checked_add(seconds * 1000)
                .ok_or_else(|| invalid("OAuth expiry overflow"))?,
        ),
        _ => {
            return Err(invalid(
                "OAuth expires_in must be positive integer seconds within one year",
            ));
        }
    };
    let granted = match value.take(b"scope") {
        Some(s) => scopes(s.text()?)?,
        None => requested.clone(),
    };
    if !granted.is_subset(requested) {
        return Err(DomainError::PermissionDenied);
    }
    let material = Material::Bearer { access, refresh };
    material.validate(target)?;
    let grant = Grant {
        account: None,
        scopes: granted,
        expires_at_ms,
    };
    material.reject_echo(&serde_json::to_value(&grant).map_err(|_| invalid("OAuth grant"))?)?;
    Ok((material, grant))
}
