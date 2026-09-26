//! DOM extraction and bounded, query-focused source excerpts.
use super::*;
use dom_query::Document;

pub(super) struct Extracted {
    pub markdown: String,
    pub title: Option<String>,
    pub description: Option<String>,
}
pub(super) fn extract(content: &str, html: bool, base: &str) -> Extracted {
    if !html {
        return Extracted {
            markdown: content.into(),
            title: None,
            description: None,
        };
    }
    let doc = Document::from(content);
    let meta = |selector: &str| {
        doc.select(selector)
            .attr("content")
            .map(|s| compact(&s))
            .filter(|s| !s.is_empty())
    };
    let title = meta("meta[property='og:title']")
        .or_else(|| meta("meta[name='twitter:title']"))
        .or_else(|| {
            let t = compact(&doc.select("title").text());
            (!t.is_empty()).then_some(t)
        });
    let description =
        meta("meta[name='description']").or_else(|| meta("meta[property='og:description']"));
    doc.select("script, style, noscript, template, svg, nav, [role='navigation'], [hidden], [aria-hidden='true'], button, input, select, textarea, iframe").remove();
    if let Ok(base) = url::Url::parse(base) {
        for a in doc.select("a[href]").iter() {
            if let Some(href) = a.attr("href") {
                match base.join(&href) {
                    Ok(url)
                        if matches!(url.scheme(), "http" | "https")
                            && url.username().is_empty()
                            && url.password().is_none() =>
                    {
                        a.set_attr("href", url.as_str());
                    }
                    _ => a.remove_attr("href"),
                }
            }
        }
    }
    // Prefer the complete main region to a small nested article or teaser.
    let mut region = doc.select("main, [role='main']");
    if region.is_empty() {
        region = doc.select("article");
    }
    if region.is_empty() {
        region = doc.select("body");
    }
    let markdown = if let Some(node) = region.nodes().iter().max_by_key(|n| n.text().len()) {
        let fragment = Document::from(node.html().as_ref());
        fragment.select("header, footer, aside, .sphinxsidebar, .sidebar, .toc, .toctree-wrapper, .headerlink").remove();
        fragment.md(None).to_string()
    } else {
        doc.md(None).to_string()
    };
    Extracted {
        markdown: markdown.trim().into(),
        title,
        description,
    }
}

pub(super) struct Excerpt {
    pub text: String,
    pub truncated: bool,
}
/// Keep paragraphs, tables and fenced examples together. Rank blocks plus their
/// immediate context, then restore document order. Omissions are explicit.
pub(super) fn excerpt(markdown: &str, focus: &[String], budget: usize) -> Excerpt {
    if markdown.chars().count() <= budget {
        return Excerpt {
            text: markdown.into(),
            truncated: false,
        };
    }
    let mut blocks = Vec::<String>::new();
    let mut block = String::new();
    let mut fence: Option<(char, usize)> = None;
    for line in markdown.lines() {
        let trimmed = line.trim_start();
        let marker = trimmed.chars().next().filter(|c| *c == '`' || *c == '~');
        if let Some(c) = marker {
            let count = trimmed.chars().take_while(|v| *v == c).count();
            if count >= 3 {
                match fence {
                    None => fence = Some((c, count)),
                    Some((open, n)) if open == c && count >= n => fence = None,
                    _ => {}
                }
            }
        }
        if line.trim().is_empty() && fence.is_none() {
            if !block.trim().is_empty() {
                blocks.push(std::mem::take(&mut block));
            }
        } else {
            block.push_str(line);
            block.push('\n');
        }
    }
    if !block.trim().is_empty() {
        blocks.push(block);
    }
    let terms = focus_terms(focus);
    // Markdown escapes must not hide API identifiers from matching. Weight
    // terms by their rarity on this page so a specific method can outrank the
    // project/type names repeated throughout a large reference document.
    let normalized: Vec<_> = blocks
        .iter()
        .map(|b| b.to_lowercase().replace('\\', ""))
        .collect();
    let weights: Vec<_> = terms
        .iter()
        .map(|t| {
            (blocks.len() / (1 + normalized.iter().filter(|b| b.contains(t.as_str())).count()))
                .clamp(1, 32)
        })
        .collect();
    let scores: Vec<_> = normalized
        .iter()
        .map(|b| {
            terms
                .iter()
                .zip(&weights)
                .filter(|(t, _)| b.contains(t.as_str()))
                .map(|(_, weight)| *weight)
                .sum::<usize>()
        })
        .collect();
    let mut order: Vec<_> = (0..blocks.len()).collect();
    order.sort_by_key(|&i| {
        std::cmp::Reverse(
            scores[i] * 10
                + if i > 0 { scores[i - 1] * 3 } else { 0 }
                + scores.get(i + 1).copied().unwrap_or(0) * 2,
        )
    });
    let mut chosen = BTreeSet::new();
    // Allow separators and the final omission note within the hard character cap.
    let mut remaining = budget.saturating_sub(160);
    for i in order {
        let size = blocks[i].chars().count() + 40;
        if size <= remaining {
            chosen.insert(i);
            remaining -= size;
        }
    }
    let mut text = String::new();
    let mut previous = None;
    for &i in &chosen {
        if previous.map_or(i != 0, |p| i != p + 1) {
            text.push_str("\n[Source passage omitted]\n\n");
        }
        text.push_str(blocks[i].trim_end());
        text.push_str("\n\n");
        previous = Some(i);
    }
    if chosen.is_empty() {
        // An oversized single paragraph/example cannot consume unlimited context.
        // Preserve a verbatim prefix; explicitly label that it is incomplete.
        let best = scores
            .iter()
            .enumerate()
            .max_by_key(|(_, s)| **s)
            .map(|(i, _)| i)
            .unwrap_or(0);
        if let Some(b) = blocks.get(best) {
            text.extend(b.chars().take(budget.saturating_sub(160)));
        }
    }
    text.push_str("\n[Excerpt incomplete; other source content omitted. Raw response retained in network evidence.]");
    Excerpt {
        text,
        truncated: true,
    }
}
