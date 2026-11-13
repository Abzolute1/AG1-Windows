//! Private values have dedicated native channels. Ordinary operation JSON and
//! advanced GraphQL must not become an alternate credential-entry surface.
pub(super) fn sensitive(name: &str) -> bool {
    let name = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect::<String>();
    [
        "token",
        "secret",
        "authorization",
        "password",
        "clientsecret",
        "privatekey",
        "encryptedvalue",
        "pat",
        "githubpat",
        "encodedjitconfig",
        "generatejitconfig",
    ]
    .contains(&name.as_str())
        || name.ends_with("token")
        || ["password", "privatekey", "clientsecret"]
            .iter()
            .any(|suffix| name.ends_with(suffix))
}
/// This scanner only denies sensitive GraphQL names; it never decides whether
/// a query is read-only or valid GraphQL. GitHub still validates the graph, and
/// every advanced graph request remains consequential. Comments and literal
/// strings cannot introduce executable field names, so skip them precisely.
pub(super) fn graph(query: &str) -> Result<(), String> {
    let b = query.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'#' {
            while i < b.len() && b[i] != b'\n' && b[i] != b'\r' {
                i += 1;
            }
            continue;
        }
        if b[i] == b'"' {
            if b[i..].starts_with(b"\"\"\"") {
                i += 3;
                let mut closed = false;
                while i < b.len() {
                    if b[i..].starts_with(b"\\\"\"\"") {
                        i += 4;
                        continue;
                    }
                    if b[i..].starts_with(b"\"\"\"") {
                        i += 3;
                        closed = true;
                        break;
                    }
                    i += 1;
                }
                if !closed {
                    return Err("Unclosed GraphQL block string.".into());
                }
            } else {
                i += 1;
                let mut closed = false;
                while i < b.len() {
                    if b[i] == b'\\' {
                        i += 2;
                    } else if b[i] == b'"' {
                        i += 1;
                        closed = true;
                        break;
                    } else {
                        i += 1;
                    }
                }
                if !closed {
                    return Err("Unclosed GraphQL string.".into());
                }
            }
            continue;
        }
        if b[i].is_ascii_alphabetic() || b[i] == b'_' {
            let start = i;
            i += 1;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            if sensitive(&query[start..i]) {
                return Err("Credential fields require a dedicated private channel; they are not accepted in advanced GraphQL.".into());
            }
        } else {
            i += 1;
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn private_api_templates_do_not_confuse_resource_names_with_credentials() {
        use crate::github::operations::{Request, endpoint};
        for path in [
            "repos/owned/secrets/issues",
            "repos/keys/repo/contents/secrets/config.json",
            "repos/owned/repo/labels/keys",
            "user/codespaces",
            "user/installations",
        ] {
            assert!(endpoint(path).is_ok(), "{path}");
        }
        for path in [
            "repos/owned/repo/actions/runners/registration-token",
            "orgs/owned/actions/runners/generate-jitconfig",
            "repos/owned/repo/actions/secrets",
            "user/codespaces/secrets",
            "user/keys",
        ] {
            assert!(endpoint(path).is_err(), "{path}");
        }
        let request:Request=serde_json::from_value(json!({"operation":"api.rest","parameters":{"method":"GET","endpoint":"user/codespaces"}})).unwrap();
        let operation = request.compile().unwrap();
        crate::github::schemas::validate(&operation).unwrap();
        assert_eq!(
            operation.risk,
            crate::github::operations::Risk::Consequential
        );
    }
    #[test]
    fn credential_spellings_and_graph_literals_cannot_bypass_private_entry() {
        for key in [
            "secret",
            "accessToken",
            "sourceAccessToken",
            "githubPat",
            "refresh_token",
            "clientSecret",
            "private-key",
            "PASSWORD",
            "encrypted_value",
        ] {
            assert!(sensitive(key), "{key}");
            let request:crate::github::operations::Request=serde_json::from_value(json!({"operation":"api.rest","parameters":{"method":"POST","endpoint":"repos/owned/repo/hooks"},"body":{"config":{key:"owned synthetic private value"}}})).unwrap();
            assert!(request.compile().is_err(), "{key}");
            let request:crate::github::operations::Request=serde_json::from_value(json!({"operation":"api.graphql","body":{"query":"mutation($input:CreateMigrationSourceInput!){createMigrationSource(input:$input){id}}","variables":{"input":{key:"owned synthetic private value"}}}})).unwrap();
            assert!(request.compile().is_err(), "{key}");
        }
        for query in [
            "mutation { createMigrationSource(input:{accessToken:\"owned\"}){id}}",
            "mutation { x:createMigrationSource(input:{githubPat:$p}){id}}",
            "query { viewer { x:accessToken } }",
            "mutation { x(input:{password:\"owned\"}){id}}",
        ] {
            assert!(graph(query).is_err(), "{query}");
        }
        for query in [
            "query { viewer { login } }",
            "mutation{createIssue(input:{body:\"accessToken: text example\"}){issue{id}}}",
            "query { viewer { login } } # accessToken",
            "mutation{x(input:{body:\"\"\"password in prose with \\\"\"\" escape\"\"\"}){id}}",
        ] {
            assert!(graph(query).is_ok(), "{query}");
        }
        assert!(graph("query{x(name:\"unterminated)}").is_err());
        for key in [
            "secret_name",
            "token_permissions",
            "clientMutationId",
            "cursor",
            "body",
            "content",
        ] {
            assert!(!sensitive(key), "{key}");
        }
    }
}
