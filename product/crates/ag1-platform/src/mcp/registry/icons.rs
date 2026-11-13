//! Publisher avatars from registry metadata, never renderer-provided URLs.
use super::*;
use crate::web_http::{SystemResolver, WebPolicy, WebResolver};
use std::net::SocketAddr;
const MAX_BYTES: usize = 128 * 1024;
#[derive(Default)]
pub(super) struct Icons {
    cache: VecDeque<(String, Option<String>, u64)>,
    requests: VecDeque<u64>,
    inflight: std::collections::HashSet<String>,
}
/// Only numeric GitHub publisher-avatar identifiers. Strip all supplied query
/// fields and force a small rendition. No repository paths, SVG or arbitrary hosts.
fn approved(value: &str) -> Option<String> {
    let url = reqwest::Url::parse(value).ok()?;
    if url.scheme() != "https"
        || url.host_str() != Some("avatars.githubusercontent.com")
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    let id = url.path().strip_prefix("/u/")?;
    if id.is_empty() || id.len() > 20 || !id.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(format!(
        "https://avatars.githubusercontent.com/u/{id}?s=64&v=4"
    ))
}
pub(super) fn source(server: &serde_json::Map<String, Value>) -> Option<String> {
    let github = server
        .get("_meta")?
        .get("io.modelcontextprotocol.registry/publisher-provided")?
        .get("github")?;
    approved(github.get("ownerAvatarUrl")?.as_str()?)
}
impl Directory {
    pub fn icon(&self, key: &str) -> Result<Value, DomainError> {
        self.icon_with(key, fetch)
    }
    fn icon_with(
        &self,
        key: &str,
        read: impl FnOnce(&str) -> Result<String, DomainError>,
    ) -> Result<Value, DomainError> {
        let url = {
            let state = self.state.lock().map_err(|_| DomainError::Conflict)?;
            find(&state, key)?.0.icon_url.clone()
        };
        let Some(url) = url.and_then(|u| approved(&u)) else {
            return Ok(json!({"dataUrl":null}));
        };
        let mut icons = self.icons.lock().map_err(|_| DomainError::Conflict)?;
        loop {
            let current = now()?;
            if let Some((_, data, _)) = icons
                .cache
                .iter()
                .find(|(u, _, until)| u == &url && *until > current)
            {
                return Ok(json!({"dataUrl":data}));
            }
            if icons.inflight.contains(&url) || icons.inflight.len() >= 4 {
                icons = self
                    .icon_ready
                    .wait(icons)
                    .map_err(|_| DomainError::Conflict)?;
                continue;
            }
            icons.requests.retain(|t| current.saturating_sub(*t) < 60);
            if icons.requests.len() >= 60 {
                return Ok(json!({"dataUrl":null}));
            }
            icons.requests.push_back(current);
            icons.inflight.insert(url.clone());
            break;
        }
        // Do not hold the cache lock during DNS, TLS or decoding. At most four
        // fetches run concurrently; duplicate publisher URLs share one result.
        drop(icons);
        let data = read(&url).ok();
        let mut icons = self.icons.lock().map_err(|_| DomainError::Conflict)?;
        icons.inflight.remove(&url);
        self.icon_ready.notify_all();
        let until = now()? + if data.is_some() { 86400 } else { 300 };
        icons.cache.retain(|(u, _, _)| u != &url);
        icons.cache.push_back((url, data.clone(), until));
        while icons.cache.len() > 128 {
            icons.cache.pop_front();
        }
        Ok(json!({"dataUrl":data}))
    }
}
fn fetch(url: &str) -> Result<String, DomainError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| invalid("icon runtime"))?;
    runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(3), async {
            let url = reqwest::Url::parse(url).map_err(|_| invalid("icon URL"))?;
            let host = "avatars.githubusercontent.com";
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
                .resolve_to_addrs(host, &addresses)
                .build()
                .map_err(|_| invalid("icon TLS"))?;
            let mut response = client
                .get(url)
                .header("accept", "image/png,image/jpeg")
                .header("user-agent", "AG1-MCP/0.1")
                .send()
                .await
                .map_err(|_| invalid("icon response"))?;
            let h = response.headers();
            if response.status() != 200
                || h.len() > 64
                || h.iter()
                    .map(|(k, v)| k.as_str().len() + v.as_bytes().len())
                    .sum::<usize>()
                    > 16384
                || h.get("content-encoding").is_some_and(|v| v != "identity")
            {
                return Err(invalid("icon response headers"));
            }
            let mime = h
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.split(';').next())
                .filter(|s| matches!(*s, "image/png" | "image/jpeg"))
                .ok_or_else(|| invalid("icon MIME"))?
                .to_owned();
            if response
                .content_length()
                .is_some_and(|n| n > MAX_BYTES as u64)
            {
                return Err(invalid("icon bytes"));
            }
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(|_| invalid("icon body"))? {
                if bytes.len() + chunk.len() > MAX_BYTES {
                    return Err(invalid("icon bytes"));
                }
                bytes.extend_from_slice(&chunk);
            }
            crate::image_tools::registry_icon(&bytes, &mime)
        })
        .await
        .map_err(|_| DomainError::TimedOut)?
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_registry_numeric_publisher_avatars_are_admitted() {
        assert_eq!(
            approved("https://avatars.githubusercontent.com/u/6154722?v=4"),
            Some("https://avatars.githubusercontent.com/u/6154722?s=64&v=4".into())
        );
        for url in [
            "http://avatars.githubusercontent.com/u/1",
            "https://avatars.githubusercontent.com.evil/u/1",
            "https://127.0.0.1/u/1",
            "https://avatars.githubusercontent.com/a/logo.svg",
            "https://x@avatars.githubusercontent.com/u/1",
            "https://avatars.githubusercontent.com/u/1/extra",
            "https://avatars.githubusercontent.com/u/1#x",
        ] {
            assert!(approved(url).is_none(), "{url}");
        }
        let mut server = json!({"name":"owner/test","version":"1","_meta":{"io.modelcontextprotocol.registry/publisher-provided":{"github":{"ownerAvatarUrl":"https://avatars.githubusercontent.com/u/1"}}}});
        assert!(source(server.as_object().unwrap()).is_some());
        server["_meta"]["io.modelcontextprotocol.registry/publisher-provided"]["github"]["ownerAvatarUrl"] =
            json!("https://advertised-server.invalid/logo.png");
        assert!(source(server.as_object().unwrap()).is_none());
    }
    #[test]
    fn icon_metadata_does_not_change_setup_digest_and_old_cache_remains_readable() {
        let make = |avatar: &str| {
            serde_json::to_vec(&json!({"servers":[{"server":{"name":"owner/test","version":"1","_meta":{"io.modelcontextprotocol.registry/publisher-provided":{"github":{"ownerAvatarUrl":avatar}}}}}],"metadata":{}})).unwrap()
        };
        let a = parse_page(
            &make("https://avatars.githubusercontent.com/u/1"),
            "",
            "",
            0,
        )
        .unwrap();
        let b = parse_page(
            &make("https://avatars.githubusercontent.com/u/2"),
            "",
            "",
            0,
        )
        .unwrap();
        assert_eq!(a.entries[0].digest, b.entries[0].digest);
        assert_ne!(a.entries[0].icon_url, b.entries[0].icon_url);
        let mut old = serde_json::to_value(&a).unwrap();
        old.as_object_mut().unwrap().remove("iconVersion");
        old["entries"][0].as_object_mut().unwrap().remove("iconUrl");
        let old: Page = serde_json::from_value(old).unwrap();
        assert_eq!(old.icon_version, 0);
        assert!(old.entries[0].icon_url.is_none());
    }
    #[test]
    fn avatar_decoder_rasterizes_and_rejects_mime_and_pixel_bombs() {
        use std::io::Cursor;
        let image = image::DynamicImage::new_rgba8(128, 128);
        let mut bytes = Cursor::new(Vec::new());
        image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        let uri = crate::image_tools::registry_icon(bytes.get_ref(), "image/png").unwrap();
        assert!(uri.starts_with("data:image/png;base64,"));
        assert!(crate::image_tools::registry_icon(bytes.get_ref(), "image/jpeg").is_err());
        assert!(crate::image_tools::registry_icon(b"<svg/>", "image/png").is_err());
        let image = image::DynamicImage::new_rgba8(1025, 1);
        let mut bytes = Cursor::new(Vec::new());
        image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        assert!(crate::image_tools::registry_icon(bytes.get_ref(), "image/png").is_err());
    }

    #[test]
    fn different_publishers_fetch_concurrently_and_duplicates_share_cache() {
        use std::sync::{
            Arc, Condvar, Mutex,
            atomic::{AtomicUsize, Ordering},
            mpsc,
        };
        let directory = Arc::new(Directory::with_reader(|_| {
            Ok((serde_json::to_vec(&json!({"servers":(1..=2).map(|i|json!({"server":{"name":format!("org/{i}"),"version":"1","_meta":{"io.modelcontextprotocol.registry/publisher-provided":{"github":{"ownerAvatarUrl":format!("https://avatars.githubusercontent.com/u/{i}")}}}}})).collect::<Vec<_>>(),"metadata":{}})).unwrap(),0))
        }));
        let page = directory.search("", "", true, false).unwrap();
        let keys: Vec<_> = page["page"]["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["key"].as_str().unwrap().to_owned())
            .collect();
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let calls = Arc::new(AtomicUsize::new(0));
        let (tx, rx) = mpsc::channel();
        let workers: Vec<_> = (0..8)
            .map(|i| {
                let directory = directory.clone();
                let key = keys[i % 2].clone();
                let gate = gate.clone();
                let calls = calls.clone();
                let tx = tx.clone();
                std::thread::spawn(move || {
                    directory
                        .icon_with(&key, |_| {
                            calls.fetch_add(1, Ordering::SeqCst);
                            tx.send(()).unwrap();
                            let (lock, ready) = &*gate;
                            let mut open = lock.lock().unwrap();
                            while !*open {
                                open = ready.wait(open).unwrap();
                            }
                            Ok("data:image/png;base64,owned".into())
                        })
                        .unwrap()
                })
            })
            .collect();
        let concurrent = rx.recv_timeout(Duration::from_secs(2)).is_ok()
            && rx.recv_timeout(Duration::from_secs(2)).is_ok();
        *gate.0.lock().unwrap() = true;
        gate.1.notify_all();
        for worker in workers {
            assert_eq!(
                worker.join().unwrap()["dataUrl"],
                "data:image/png;base64,owned"
            );
        }
        assert!(
            concurrent,
            "publisher requests must not serialize behind the icon cache lock"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
}
