use super::*;
use crate::web_http::{WebPurpose, WebResponse};
use ag1_core::tools::websearch::duckduckgo_web;

fn checked_data_url(value: &str, endpoint: &str, query: &str) -> Result<String, DomainError> {
    let base = reqwest::Url::parse(endpoint).map_err(|_| DomainError::PermissionDenied)?;
    let url = base
        .join(value)
        .map_err(|_| DomainError::PermissionDenied)?;
    let terms = url
        .query_pairs()
        .filter(|(key, _)| key == "q")
        .map(|(_, v)| v.into_owned())
        .collect::<Vec<_>>();
    if url.as_str().len() > 8192
        || url.origin() != base.origin()
        || url.path() != base.path()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || terms != [query]
    {
        return Err(DomainError::PermissionDenied);
    }
    Ok(url.to_string())
}
pub(super) struct WebAttempt<'a, A> {
    pub query: &'a str,
    pub deadline: Instant,
    pub cancel: &'a mut CancelToken,
    pub artifacts: &'a mut A,
    pub evidence: &'a mut Vec<String>,
    pub call_id: &'a str,
}
impl<R: WebResolver> SearchService<R> {
    async fn web_search_response(
        &self,
        url: &str,
        attempt: &mut WebAttempt<'_, impl ContextArtifactPort>,
    ) -> Result<WebResponse, DomainError> {
        let response = self
            .web
            .http
            .fetch_for(
                url,
                None,
                WebPurpose::SearchWeb,
                attempt.deadline,
                attempt.cancel,
            )
            .await?;
        let raw = json!({"version":1,"provider":"duckduckgo-web","initialRequestMethod":"GET","initialRequestTarget":url,
            "searchQuery":attempt.query,"requestedUrl":url,"finalUrl":response.final_url,"redirectChain":response.redirect_chain,
            "statusCode":response.status,"contentType":response.content_type,"bodyBase64":STANDARD.encode(&response.bytes),
            "bodySha256":format!("{:x}",Sha256::digest(&response.bytes)),"bodyRepresentation":"decoded-entity","trust":"untrusted"});
        let artifact = attempt.artifacts.put_evidence(
            &raw.to_string(),
            "web-response",
            &json!({"type":"tool","name":"webSearch","callId":attempt.call_id}),
            &[],
            None,
        )?;
        attempt.evidence.push(artifact.id);
        Ok(response)
    }
    pub(super) async fn duckduckgo_web(
        &self,
        landing: &str,
        data_endpoint: &str,
        max: usize,
        attempt: &mut WebAttempt<'_, impl ContextArtifactPort>,
    ) -> Result<Vec<websearch::ResultRow>, DomainError> {
        let query = attempt.query;
        let url = search_url(landing, query, None);
        let page = self.web_search_response(&url, attempt).await?;
        let html = String::from_utf8_lossy(&page.bytes);
        if page.status != 200 || websearch::duckduckgo_page_status(&html) == "blocked" {
            return Err(DomainError::Unavailable("DuckDuckGo web blocked".into()));
        }
        let link = duckduckgo_web::preload(&html)?;
        let url = checked_data_url(&link, data_endpoint, query)?;
        let mut response = self.web_search_response(&url, attempt).await?;
        if response.status == 202 {
            let script = String::from_utf8_lossy(&response.bytes);
            let continuation = duckduckgo_web::arithmetic_continuation(&script)
                .map_err(|_| DomainError::Unavailable("DuckDuckGo web blocked".into()))?;
            let url = checked_data_url(&continuation, data_endpoint, query)?;
            response = self.web_search_response(&url, attempt).await?;
        }
        if response.status != 200
            || websearch::duckduckgo_page_status(&String::from_utf8_lossy(&response.bytes))
                == "blocked"
        {
            return Err(DomainError::Unavailable("DuckDuckGo web blocked".into()));
        }
        duckduckgo_web::results(&String::from_utf8_lossy(&response.bytes), max)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn continuation_cannot_change_origin_path_or_query() {
        let endpoint = "https://links.duckduckgo.com/d.js";
        assert!(checked_data_url("/d.js?q=a%2Bb&jsa=17", endpoint, "a+b").is_ok());
        for value in [
            "https://evil.test/d.js?q=a%2Bb",
            "//127.0.0.1/d.js?q=a%2Bb",
            "/other?q=a%2Bb",
            "/d.js?q=other",
            "/d.js?q=a%2Bb&q=a%2Bb",
            "https://user@links.duckduckgo.com/d.js?q=a%2Bb",
            "/d.js?q=a%2Bb#fragment",
        ] {
            assert!(checked_data_url(value, endpoint, "a+b").is_err(), "{value}");
        }
    }
}
