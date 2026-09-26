//! Decode DDG's web result envelope and a narrowly recognized arithmetic check.
//! Downloaded JavaScript is data only and is never executed.
use super::*;

fn invalid() -> DomainError {
    DomainError::InvalidInput("unrecognized DuckDuckGo web response".into())
}
pub fn preload(html: &str) -> Result<String, DomainError> {
    if html.len() > 1_000_000 {
        return Err(invalid());
    }
    let doc = dom_query::Document::from(html);
    let scripts = doc.select("script#deep_preload_script");
    if scripts.length() != 1 {
        return Err(invalid());
    }
    scripts
        .attr("src")
        .map(|s| s.to_string())
        .ok_or_else(invalid)
}
pub fn results(script: &str, max: usize) -> Result<Vec<ResultRow>, DomainError> {
    if script.len() > 1_000_000 || max > 256 {
        return Err(invalid());
    }
    if max == 0 {
        return Ok(vec![]);
    }
    let (_, tail) = script
        .split_once("DDG.pageLayout.load('d',")
        .ok_or_else(invalid)?;
    let value = serde_json::Deserializer::from_str(tail)
        .into_iter::<Value>()
        .next()
        .ok_or_else(invalid)?
        .map_err(|_| invalid())?;
    let items = value.as_array().ok_or_else(invalid)?;
    if items.len() > 256 {
        return Err(invalid());
    }
    if items.len() == 1 && items[0]["t"] == "DEEP_SIMPLE_NO_RESULTS" && items[0]["u"] == "" {
        return Ok(vec![]);
    }
    let mut rows = vec![];
    let mut seen = BTreeSet::new();
    for item in items {
        let (Some(title), Some(url)) = (item["t"].as_str(), item["u"].as_str()) else {
            continue;
        };
        let url = normalize_result_url(url);
        let title = decode_html(title);
        if title.is_empty() || !usable(&url) || !seen.insert(canonical(&url)) {
            continue;
        }
        rows.push(ResultRow {
            title: title.chars().take(4096).collect(),
            url,
            snippet: decode_html(item["a"].as_str().unwrap_or_default())
                .chars()
                .take(4096)
                .collect(),
        });
        if rows.len() >= max {
            break;
        }
    }
    if rows.is_empty() && !items.is_empty() {
        return Err(invalid());
    }
    Ok(rows)
}
/// Only accept the observed grammar: integer seed, up to 64 declared arithmetic
/// helpers, up to 64 invocations, and a literal DDG continuation. Unknown code
/// fails closed. The caller separately authorizes the returned URL and query.
pub fn arithmetic_continuation(script: &str) -> Result<String, DomainError> {
    if script.len() > 16384 {
        return Err(invalid());
    }
    let prefix =
        rx(r"(?s)^window\.execDeep = function\(\) \{\s*let jsa = ([0-9]{1,9});\s*try \{\s*");
    let caps = prefix.captures(script).ok_or_else(invalid)?;
    let mut value: u64 = caps[1].parse().map_err(|_| invalid())?;
    let mut rest = &script[caps.get(0).unwrap().end()..];
    let helper = rx(r"^let ([A-Za-z]{1,32}) = function\(num\) \{([^{}]{1,256})\};\s*");
    let multiply = rx(r"^return num \* ([0-9]{1,3});$");
    let fragment = rx(
        r"^const el = document\.createElement\('div'\);el\.innerHTML = `([^`]{1,80})`;return num \+ el\.innerHTML\.length;$",
    );
    let mut operations = BTreeMap::new();
    while let Some(c) = helper.captures(rest) {
        if operations.len() >= 64 {
            return Err(invalid());
        }
        let operation = if let Some(m) = multiply.captures(&c[2]) {
            (true, m[1].parse::<u64>().map_err(|_| invalid())?)
        } else {
            let m = fragment.captures(&c[2]).ok_or_else(invalid)?;
            // Browser HTML-fragment serialization lengths, independently checked
            // against Chromium. No arbitrary markup or script evaluation.
            let n = match &m[1] {
                "<div><div></div><div></div" => 33,
                "<br><div></br><br></div" => 23,
                "<li><div></li><li></div" => 29,
                "<p><div></p><p></div" => 32,
                _ => return Err(invalid()),
            };
            (false, n)
        };
        if operations.insert(c[1].to_string(), operation).is_some() {
            return Err(invalid());
        }
        rest = &rest[c.get(0).unwrap().end()..];
    }
    let step = rx(r"^jsa = ([A-Za-z]{1,32})\(jsa\);\s*");
    let mut count = 0;
    while let Some(c) = step.captures(rest) {
        count += 1;
        if count > 64 {
            return Err(invalid());
        }
        let (multiply, n) = operations.get(&c[1]).ok_or_else(invalid)?;
        value = if *multiply {
            value.checked_mul(*n)
        } else {
            value.checked_add(*n)
        }
        .ok_or_else(invalid)?;
        if value > 9_007_199_254_740_991 {
            return Err(invalid());
        }
        rest = &rest[c.get(0).unwrap().end()..];
    }
    if count == 0 {
        return Err(invalid());
    }
    let suffix = rx(
        r"(?s)^\} catch \(e\) \{ jsa = -1; \}\s*DDG\.deep\.initialize\('([^'\s\\]+)' \+ jsa, false\);\s*return \{isJsaChallenge: true\};\s*\};\s*$",
    );
    let c = suffix.captures(rest).ok_or_else(invalid)?;
    if c[1].len() > 8192 || !c[1].ends_with("&jsa=") {
        return Err(invalid());
    }
    Ok(format!("{}{value}", &c[1]))
}
