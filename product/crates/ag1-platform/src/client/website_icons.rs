//! Optional public favicon decoration, bound to an existing web-tool result.
//! Never fetch from the renderer, send search text/credentials, or alter tool evidence.
use super::*;
use crate::web_http::{SystemResolver, WebPolicy, WebResolver};
use reqwest::Url;
use std::{
    collections::{HashSet, VecDeque},
    net::SocketAddr,
    sync::LazyLock,
    time::Instant,
};

pub(super) fn sources(entry: &Value) -> Vec<Value> {
    let m = &entry["message"];
    if m["role"] != "toolResult"
        || m["isError"] == true
        || m["details"]["error"].is_string()
        || m["details"]["fallbackUsed"] == true
    {
        return vec![];
    }
    let details = &m["details"];
    let urls: Vec<_> = match m["toolName"].as_str() {
        Some("webSearch") => details["sourceUrls"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .take(4)
            .collect(),
        Some("webFetch") => details["finalUrl"]
            .as_str()
            .or_else(|| details["url"].as_str())
            .into_iter()
            .collect(),
        _ => return vec![],
    };
    let mut seen = HashSet::new();
    urls.into_iter()
        .filter_map(|raw| {
            if raw.len() > 2048 {
                return None;
            }
            let url = Url::parse(raw).ok()?;
            if !matches!(url.scheme(), "http" | "https")
                || !url.username().is_empty()
                || url.password().is_some()
                || !seen.insert(url.to_string())
            {
                return None;
            }
            let title = details["results"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|row| row["url"] == raw)
                .and_then(|row| row["title"].as_str())
                .unwrap_or_else(|| url.host_str().unwrap_or("Webpage"));
            Some(json!({"url":url.as_str(),"title":title.chars().take(240).collect::<String>()}))
        })
        .collect()
}
fn origin(raw: &str) -> Result<Url> {
    let mut url = Url::parse(raw).map_err(|_| DomainError::PermissionDenied)?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
    {
        return Err(DomainError::PermissionDenied);
    }
    url.set_scheme("https")
        .map_err(|_| DomainError::PermissionDenied)?;
    url.set_path("/");
    url.set_query(None);
    url.set_fragment(None);
    Ok(url)
}
#[derive(Default)]
struct Cache {
    values: VecDeque<(String, Option<String>, Instant)>,
    inflight: HashSet<String>,
    requests: VecDeque<Instant>,
}
static CACHE: LazyLock<Mutex<Cache>> = LazyLock::new(|| Mutex::new(Cache::default()));
pub(super) fn icon(raw: &str) -> Result<Option<String>> {
    cached_icon(&CACHE, raw, |root| {
        fetch(root.clone()).or_else(|_| fallback(root))
    })
}
fn cached_icon(
    cache: &Mutex<Cache>,
    raw: &str,
    fetch: impl FnOnce(Url) -> Result<String>,
) -> Result<Option<String>> {
    let root = origin(raw)?;
    let key = root.to_string();
    let now = Instant::now();
    let stale;
    {
        let mut c = cache.lock().map_err(|_| DomainError::Conflict)?;
        c.values
            .retain(|(_, data, until)| data.is_some() || *until > now);
        if let Some((_, data, until)) = c.values.iter().find(|(k, _, _)| k == &key) {
            if *until > now {
                return Ok(data.clone());
            }
            stale = data.clone();
        } else {
            stale = None;
        }
        c.requests
            .retain(|t| now.duration_since(*t) < Duration::from_secs(60));
        if c.inflight.len() >= 4 || c.inflight.contains(&key) || c.requests.len() >= 60 {
            return if stale.is_some() {
                Ok(stale)
            } else {
                Err(DomainError::QueueFull)
            };
        }
        c.inflight.insert(key.clone());
        c.requests.push_back(now);
    }
    let data = fetch(root).ok().or(stale);
    let mut c = cache.lock().map_err(|_| DomainError::Conflict)?;
    c.inflight.remove(&key);
    c.values.retain(|(k, _, _)| k != &key);
    let ttl = if data.is_some() { 3600 } else { 30 };
    c.values
        .push_back((key, data.clone(), Instant::now() + Duration::from_secs(ttl)));
    while c.values.len() > 128 {
        c.values.pop_front();
    }
    Ok(data)
}
fn fetch(root: Url) -> Result<String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| DomainError::Unavailable("icon runtime".into()))?;
    runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(24), async {
            // Validate the source before using even a domain-only public icon fallback.
            let host = root.host_str().ok_or(DomainError::PermissionDenied)?;
            let addresses = SystemResolver.resolve(host, 443).await?;
            WebPolicy::default().validate(root.as_str(), &addresses)?;
            let conventional = root
                .join("favicon.ico")
                .map_err(|_| DomainError::PermissionDenied)?;
            if let Ok((_, bytes)) = download(conventional, false).await {
                if let Ok(data) = crate::image_tools::website_icon(&bytes) {
                    return Ok(data);
                }
            }
            // Site-declared icons may live at hashed paths or on a separate CDN.
            if let Ok((page, html)) = download(root.clone(), true).await {
                for url in declared_icons(&page, &String::from_utf8_lossy(&html)) {
                    if let Ok((_, bytes)) = download(url, false).await {
                        if let Ok(data) = crate::image_tools::website_icon(&bytes) {
                            return Ok(data);
                        }
                    }
                }
            }
            for path in ["apple-touch-icon.png", "favicon.png", "favicon.svg"] {
                if let Ok((_, bytes)) = download(
                    root.join(path).map_err(|_| DomainError::PermissionDenied)?,
                    false,
                )
                .await
                {
                    if let Ok(data) = crate::image_tools::website_icon(&bytes) {
                        return Ok(data);
                    }
                }
            }
            Err(DomainError::NotFound)
        })
        .await
        .map_err(|_| DomainError::TimedOut)?
    })
}

// A second bounded route still works when the source blocks non-browser traffic.
// Only a validated public hostname goes to DuckDuckGo: never the page or query.
fn fallback(root: Url) -> Result<String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| DomainError::Unavailable("icon runtime".into()))?;
    runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(5), async {
            let host = root.host_str().ok_or(DomainError::PermissionDenied)?;
            let addresses = SystemResolver.resolve(host, 443).await?;
            WebPolicy::default().validate(root.as_str(), &addresses)?;
            let mut url = Url::parse("https://icons.duckduckgo.com/ip3/")
                .map_err(|_| DomainError::PermissionDenied)?;
            url.path_segments_mut()
                .map_err(|_| DomainError::PermissionDenied)?
                .pop_if_empty()
                .push(&format!("{host}.ico"));
            let (_, bytes) = download(url, false).await?;
            crate::image_tools::website_icon(&bytes)
        })
        .await
        .map_err(|_| DomainError::TimedOut)?
    })
}

fn declared_icons(page: &Url, html: &str) -> Vec<Url> {
    // Tokenize bounded link tags, respecting quoted '>' and attribute ordering.
    // Never interpret scripts/comments as markup or execute any document content.
    static OMIT: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"(?is)<!--.*?(?:-->|$)|<(script|style)\b[^>]*>.*?</(?:script|style)\s*>")
            .unwrap()
    });
    static TAG: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r#"(?is)<(?:link|base)\b(?:[^>"']|"[^"]*"|'[^']*')*>"#).unwrap()
    });
    static ATTR: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r#"(?is)([^\s=<>/]+)\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+))"#).unwrap()
    });
    let cleaned = OMIT.replace_all(html, "");
    let mut base = page.clone();
    let mut has_base = false;
    let mut result = Vec::new();
    for tag in TAG.find_iter(&cleaned).take(128) {
        let attrs: std::collections::HashMap<_, _> = ATTR
            .captures_iter(tag.as_str())
            .map(|c| {
                (
                    c[1].to_ascii_lowercase(),
                    ag1_core::tools::websearch::decode_html(
                        c.get(2)
                            .or_else(|| c.get(3))
                            .or_else(|| c.get(4))
                            .unwrap()
                            .as_str(),
                    ),
                )
            })
            .collect();
        let Some(href) = attrs.get("href") else {
            continue;
        };
        if tag.as_str()[..5].eq_ignore_ascii_case("<base") {
            if !has_base {
                if let Ok(url) = page.join(href) {
                    base = url;
                    has_base = true;
                }
            }
            continue;
        }
        if !attrs.get("rel").is_some_and(|rel| {
            rel.split_ascii_whitespace().any(|s| {
                s.eq_ignore_ascii_case("icon")
                    || s.eq_ignore_ascii_case("apple-touch-icon")
                    || s.eq_ignore_ascii_case("apple-touch-icon-precomposed")
            })
        }) {
            continue;
        }
        let Ok(url) = base.join(href) else {
            continue;
        };
        if safe_icon_url(&url) && !result.contains(&url) {
            result.push(url);
        }
        if result.len() == 6 {
            break;
        }
    }
    result
}
fn safe_icon_url(url: &Url) -> bool {
    url.as_str().len() <= 2048
        && url.scheme() == "https"
        && url.port_or_known_default() == Some(443)
        && url.username().is_empty()
        && url.password().is_none()
}
async fn download(mut url: Url, html: bool) -> Result<(Url, Vec<u8>)> {
    tokio::time::timeout(Duration::from_secs(4), async {
        for redirects in 0..=3 {
            if !safe_icon_url(&url) {
                return Err(DomainError::PermissionDenied);
            }
            let host = url.host_str().ok_or(DomainError::PermissionDenied)?;
            let addresses = SystemResolver.resolve(host, 443).await?;
            WebPolicy::default().validate(url.as_str(), &addresses)?;
            let addresses: Vec<_> = addresses
                .into_iter()
                .map(|ip| SocketAddr::new(ip, 443))
                .collect();
            let client = reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .retry(reqwest::retry::never())
                .pool_max_idle_per_host(0)
                .connect_timeout(Duration::from_secs(2))
                .timeout(Duration::from_secs(4))
                .resolve_to_addrs(host, &addresses)
                .build()
                .map_err(|_| DomainError::Unavailable("icon TLS".into()))?;
            let mut response = client
                .get(url.clone())
                .header("accept", if html { "text/html" } else { "image/*" })
                .header("user-agent", "AG1/0.1")
                .send()
                .await
                .map_err(|_| DomainError::Unavailable("icon request".into()))?;
            let h = response.headers();
            if h.len() > 64
                || h.iter()
                    .map(|(k, v)| k.as_str().len() + v.as_bytes().len())
                    .sum::<usize>()
                    > 16384
            {
                return Err(DomainError::LimitExceeded("icon headers".into()));
            }
            if response.status().is_redirection() && redirects < 3 {
                url = h
                    .get("location")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| url.join(v).ok())
                    .ok_or(DomainError::PermissionDenied)?;
                continue;
            }
            if response.status() != 200 {
                return Err(DomainError::NotFound);
            }
            let limit = if html { 512 * 1024 } else { 128 * 1024 };
            if !html && response.content_length().is_some_and(|n| n > limit as u64) {
                return Err(DomainError::LimitExceeded("icon bytes".into()));
            }
            let mut bytes = Vec::new();
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|_| DomainError::Unavailable("icon body".into()))?
            {
                if bytes.len() + chunk.len() > limit {
                    if html {
                        bytes.extend_from_slice(&chunk[..limit - bytes.len()]);
                        break;
                    }
                    return Err(DomainError::LimitExceeded("icon bytes".into()));
                }
                bytes.extend_from_slice(&chunk);
                if html && bytes.windows(7).any(|w| w.eq_ignore_ascii_case(b"</head>")) {
                    break;
                }
            }
            return Ok((url, bytes));
        }
        Err(DomainError::NotFound)
    })
    .await
    .map_err(|_| DomainError::TimedOut)?
}
#[cfg(test)]
fn legacy_fetch(root: Url) -> Result<String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| DomainError::Unavailable("icon runtime".into()))?;
    runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(6), async {
            for path in ["favicon.ico", "apple-touch-icon.png"] {
                let mut url = root.join(path).map_err(|_| DomainError::PermissionDenied)?;
                for redirects in 0..=2 {
                    if url.scheme() != "https"
                        || url.port_or_known_default() != Some(443)
                        || !url.username().is_empty()
                        || url.password().is_some()
                    {
                        break;
                    }
                    let host = url.host_str().ok_or(DomainError::PermissionDenied)?;
                    let addresses = SystemResolver.resolve(host, 443).await?;
                    WebPolicy::default().validate(url.as_str(), &addresses)?;
                    let addresses: Vec<_> = addresses
                        .into_iter()
                        .map(|ip| SocketAddr::new(ip, 443))
                        .collect();
                    let client = reqwest::Client::builder()
                        .no_proxy()
                        .no_gzip()
                        .no_brotli()
                        .no_deflate()
                        .redirect(reqwest::redirect::Policy::none())
                        .retry(reqwest::retry::never())
                        .pool_max_idle_per_host(0)
                        .connect_timeout(Duration::from_secs(2))
                        .timeout(Duration::from_secs(3))
                        .resolve_to_addrs(host, &addresses)
                        .build()
                        .map_err(|_| DomainError::Unavailable("icon TLS".into()))?;
                    let response = client
                        .get(url.clone())
                        .header("accept", "image/*")
                        .header("user-agent", "AG1/0.1")
                        .send()
                        .await;
                    let Ok(mut response) = response else {
                        break;
                    };
                    let h = response.headers();
                    if h.len() > 64
                        || h.iter()
                            .map(|(k, v)| k.as_str().len() + v.as_bytes().len())
                            .sum::<usize>()
                            > 16384
                        || h.get("content-encoding").is_some_and(|v| v != "identity")
                    {
                        break;
                    }
                    if response.status().is_redirection() && redirects < 2 {
                        let Some(next) = h
                            .get("location")
                            .and_then(|v| v.to_str().ok())
                            .and_then(|v| url.join(v).ok())
                        else {
                            break;
                        };
                        url = next;
                        continue;
                    }
                    if response.status() != 200
                        || response.content_length().is_some_and(|n| n > 128 * 1024)
                    {
                        break;
                    }
                    let mut bytes = Vec::new();
                    while let Some(chunk) = response
                        .chunk()
                        .await
                        .map_err(|_| DomainError::Unavailable("icon body".into()))?
                    {
                        if bytes.len() + chunk.len() > 128 * 1024 {
                            return Err(DomainError::LimitExceeded("icon bytes".into()));
                        }
                        bytes.extend_from_slice(&chunk);
                    }
                    if let Ok(data) = crate::image_tools::website_icon(&bytes) {
                        return Ok(data);
                    }
                    break;
                }
            }
            Err(DomainError::NotFound)
        })
        .await
        .map_err(|_| DomainError::TimedOut)?
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sources_are_bounded_to_successful_native_web_results() {
        let mut e = json!({"message":{"role":"toolResult","toolName":"webSearch","details":{"sourceUrls":["https://example.com/a?q=x","javascript:alert(1)","https://secret@example.com/","https://example.com/a?q=x"],"results":[{"url":"https://example.com/a?q=x","title":"Example source"}]}}});
        assert_eq!(
            sources(&e),
            vec![json!({"url":"https://example.com/a?q=x","title":"Example source"})]
        );
        e["message"]["isError"] = json!(true);
        assert!(sources(&e).is_empty());
        e["message"]["isError"] = json!(false);
        e["message"]["toolName"] = json!("bash");
        assert!(sources(&e).is_empty());
    }
    #[test]
    fn old_fallback_suggestions_and_failed_fetches_do_not_become_source_badges() {
        let mut e = json!({"message":{"role":"toolResult","toolName":"webSearch","isError":false,"details":{"fallbackUsed":true,"sourceUrls":["https://docs.python.org/3/"]}}});
        assert!(sources(&e).is_empty());
        e["message"]["toolName"] = json!("webFetch");
        e["message"]["details"] =
            json!({"error":"404 Not Found","url":"https://example.org/missing"});
        assert!(sources(&e).is_empty());
    }
    #[test]
    fn icons_strip_page_paths_queries_and_reject_private_networks() {
        assert_eq!(
            origin("http://example.com/private?q=secret#fragment")
                .unwrap()
                .as_str(),
            "https://example.com/"
        );
        for raw in [
            "file:///tmp/icon",
            "https://user:secret@example.com/",
            "https://example.com:8443/a",
        ] {
            assert!(origin(raw).is_err());
        }
        for ip in ["127.0.0.1", "10.0.0.1", "169.254.169.254", "::1"] {
            assert!(
                WebPolicy::default()
                    .validate("https://example.com/favicon.ico", &[ip.parse().unwrap()])
                    .is_err()
            );
        }
    }
    #[test]
    fn favicon_decode_returns_only_bounded_png_and_refuses_active_content() {
        use image::{DynamicImage, ImageFormat};
        use std::io::Cursor;
        for format in [ImageFormat::Png, ImageFormat::Ico] {
            let mut bytes = Cursor::new(Vec::new());
            DynamicImage::new_rgba8(32, 32)
                .write_to(&mut bytes, format)
                .unwrap();
            assert!(
                crate::image_tools::website_icon(bytes.get_ref())
                    .unwrap()
                    .starts_with("data:image/png;base64,")
            );
        }
        for bad in [
            b"<svg onload='alert(1)'/>".as_slice(),
            b"<html>error</html>",
            b"\x89PNG\r\n\x1a\n",
        ] {
            assert!(crate::image_tools::website_icon(bad).is_err());
        }
    }
    #[test]
    fn discovers_declared_cdn_icons_and_resolves_base_without_script_decoys() {
        let root = Url::parse("https://example.com/news/").unwrap();
        let html = r#"<!-- <link rel=icon href=/comment.ico> --><script>let x='<link rel=icon href=/script.ico>';</script>
        <base href="https://cdn.example.com/assets/">
        <LINK sizes="32x32" HREF='brand.svg?a=1&amp;b=2' REL="shortcut ICON">
        <link rel=apple-touch-icon href=/touch.png><link rel=stylesheet href=/style.css>
        <link rel=icon href="data:image/svg+xml,abc"><link rel=icon href="https://user:password@example.com/a">
        <link rel=icon href="brand.svg?a=1&amp;b=2">"#;
        let urls = declared_icons(&root, html)
            .into_iter()
            .map(|u| u.to_string())
            .collect::<Vec<_>>();
        assert_eq!(
            urls,
            vec![
                "https://cdn.example.com/assets/brand.svg?a=1&b=2",
                "https://cdn.example.com/touch.png"
            ]
        );
        assert!(!safe_icon_url(&Url::parse("http://example.com/a").unwrap()));
        assert!(!safe_icon_url(
            &Url::parse("https://example.com:8443/a").unwrap()
        ));
    }
    #[test]
    fn svg_icons_become_png_but_active_or_external_content_is_refused() {
        let good=br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="#123456" d="M0 0h24v24H0z"/></svg>"##;
        let png = crate::image_tools::website_icon(good).unwrap();
        assert!(png.starts_with("data:image/png;base64,"));
        for body in [
            "<script>alert(1)</script>",
            "<image href='file:///etc/passwd'/>",
            "<path onload='alert(1)'/>",
            "<rect fill='url(https://example.com/a)'/>",
        ] {
            assert!(
                crate::image_tools::website_icon(
                    format!("<svg xmlns='http://www.w3.org/2000/svg'>{body}</svg>").as_bytes()
                )
                .is_err()
            );
        }
    }
    #[test]
    #[ignore = "Explicit bounded live icon smoke; needs public network"]
    fn live_website_icon_smoke() {
        for host in [
            "openai.com",
            "docs.python.org",
            "github.com",
            "news.ycombinator.com",
            "duckduckgo.com",
            "findskill.ai",
        ] {
            let start = Instant::now();
            let root = origin(&format!("https://{host}/")).unwrap();
            let legacy = legacy_fetch(root.clone());
            println!(
                "ICON_LEGACY {}",
                json!({"host":host,"success":legacy.is_ok(),"error":legacy.as_ref().err().map(|e|format!("{e:?}")),"seconds":start.elapsed().as_secs_f64()})
            );
            let start = Instant::now();
            let direct = fetch(root.clone());
            let route = if direct.is_ok() {
                "direct"
            } else {
                "domain-fallback"
            };
            let data = direct.or_else(|_| fallback(root));
            println!(
                "ICON_SMOKE {}",
                json!({"host":host,"route":route,"success":data.is_ok(),"seconds":start.elapsed().as_secs_f64(),"png_bytes":data.as_ref().map(|s|s.len()).unwrap_or(0)})
            );
            assert!(data.is_ok(), "{host}: {data:?}");
            if let Ok(directory) = std::env::var("AG1_ICON_SMOKE_DIR") {
                use base64::Engine;
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(
                        data.unwrap()
                            .strip_prefix("data:image/png;base64,")
                            .unwrap(),
                    )
                    .unwrap();
                std::fs::write(
                    std::path::Path::new(&directory).join(format!("{host}.png")),
                    bytes,
                )
                .unwrap();
            }
            if host == "openai.com" {
                let data = fallback(origin(&format!("https://{host}/")).unwrap()).unwrap();
                assert!(data.starts_with("data:image/png;base64,"));
                println!("ICON_FALLBACK openai.com success");
            }
        }
    }

    #[test]
    fn transient_failure_keeps_previous_logo_and_misses_expire() {
        let cache = Mutex::new(Cache::default());
        let url = "https://example.com/a";
        assert_eq!(
            cached_icon(&cache, url, |_| Ok("logo".into())).unwrap(),
            Some("logo".into())
        );
        cache.lock().unwrap().values[0].2 = Instant::now() - Duration::from_secs(1);
        assert_eq!(
            cached_icon(&cache, url, |_| Err(DomainError::TimedOut)).unwrap(),
            Some("logo".into())
        );
        let cache = Mutex::new(Cache::default());
        assert_eq!(
            cached_icon(&cache, url, |_| Err(DomainError::TimedOut)).unwrap(),
            None
        );
        assert_eq!(
            cached_icon(&cache, url, |_| panic!("negative cache still fresh")).unwrap(),
            None
        );
        cache.lock().unwrap().values[0].2 = Instant::now() - Duration::from_secs(1);
        assert_eq!(
            cached_icon(&cache, url, |_| Ok("recovered".into())).unwrap(),
            Some("recovered".into())
        );
    }
    #[test]
    fn saturation_is_retryable_and_does_not_poison_icon_cache() {
        let cache = Mutex::new(Cache::default());
        cache
            .lock()
            .unwrap()
            .inflight
            .extend((0..4).map(|i| format!("host-{i}")));
        assert!(matches!(
            cached_icon(&cache, "https://example.com/", |_| panic!("no slot")),
            Err(DomainError::QueueFull)
        ));
        assert!(cache.lock().unwrap().values.is_empty());
        cache.lock().unwrap().inflight.clear();
        assert_eq!(
            cached_icon(&cache, "https://example.com/", |_| Ok("logo".into())).unwrap(),
            Some("logo".into())
        );
    }
}
