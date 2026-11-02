//! Knowledge sources that aren't code: files the person picked and web pages. Each is kept as
//! a folder of text snapshots in `repos/<source id>`, readable by this user only, which the
//! audit reads like a code folder. Nothing here goes anywhere but the files and pages named:
//! the snapshots reach only the AI the person chose, when they run an audit.

use crate::error::AppResult;
use crate::repos::private_dir;
use sha2::{Digest, Sha256};
use std::collections::{HashSet, VecDeque};
use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::time::Duration;
use url::Url;

/// The largest file read.
pub const MAX_FILE_BYTES: u64 = 20 * 1024 * 1024;
/// The most files in one source.
pub const MAX_FILES: usize = 50;
/// The most pages a crawl reads.
pub const MAX_PAGES: usize = 50;
/// The most a crawl downloads, all pages together.
pub const MAX_WEB_BYTES: usize = 5 * 1024 * 1024;
/// Read as they are.
const TEXT_EXTENSIONS: &[&str] = &["md", "txt", "csv", "tsv", "json", "yaml", "yml", "sql"];
/// Every kind of file SayQL can read, for the file picker.
pub const FILE_EXTENSIONS: &[&str] = &[
    "md", "txt", "csv", "tsv", "json", "yaml", "yml", "sql", "html", "htm", "pdf", "docx",
];
/// The width HTML is wrapped at.
const TEXT_WIDTH: usize = 100;
const PAGE_TIME: Duration = Duration::from_secs(20);
/// Redirects followed for one page, all on the same site.
const MAX_REDIRECTS: usize = 5;

/// One text snapshot: its name in the source's folder (what the audit cites) and its text.
pub type Doc = (String, String);

fn extension(name: &str) -> String {
    Path::new(name)
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

/// The text of a file named `name`; an error says why it can't be read.
pub fn extract(name: &str, bytes: &[u8]) -> Result<String, String> {
    let ext = extension(name);
    let text = match ext.as_str() {
        e if TEXT_EXTENSIONS.contains(&e) => String::from_utf8_lossy(bytes).into_owned(),
        "html" | "htm" => html_text(bytes),
        "pdf" => pdf_extract::extract_text_from_mem(bytes)
            .map_err(|e| format!("{name} isn't a PDF SayQL can read ({e})."))?,
        "docx" => docx_text(bytes)
            .map_err(|e| format!("{name} isn't a Word file SayQL can read ({e})."))?,
        _ => {
            return Err(format!(
                "SayQL can't read .{ext} files. It reads {}.",
                FILE_EXTENSIONS.join(", ")
            ));
        }
    };
    if text.trim().is_empty() {
        return Err(format!(
            "{name} has no text SayQL can read (a scanned PDF is only images)."
        ));
    }
    Ok(text)
}

/// A web page as plain text.
pub fn html_text(bytes: &[u8]) -> String {
    html2text::config::plain_no_decorate()
        .string_from_read(bytes, TEXT_WIDTH)
        .unwrap_or_else(|_| String::from_utf8_lossy(bytes).into())
}

/// The text of a Word file: `word/document.xml`, a line per paragraph.
fn docx_text(bytes: &[u8]) -> Result<String, String> {
    use std::io::Read;
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let mut xml = String::new();
    zip.by_name("word/document.xml")
        .map_err(|e| e.to_string())?
        .take(MAX_FILE_BYTES * 4)
        .read_to_string(&mut xml)
        .map_err(|e| e.to_string())?;
    let mut out = String::new();
    let mut rest = xml.as_str();
    while let Some(start) = rest.find('<') {
        let Some(len) = rest[start..].find('>') else {
            break;
        };
        let tag = &rest[start + 1..start + len];
        let after = &rest[start + len + 1..];
        let name = tag.split([' ', '/']).next().unwrap_or("");
        match name {
            "w:t" if !tag.ends_with('/') => {
                let end = after.find("</w:t>").unwrap_or(after.len());
                out.push_str(&xml_unescape(&after[..end]));
                rest = &after[end..];
                continue;
            }
            "w:tab" => out.push('\t'),
            "w:br" | "w:cr" => out.push('\n'),
            _ if tag == "/w:p" => out.push('\n'),
            _ => {}
        }
        rest = after;
    }
    Ok(out)
}

fn xml_unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// The version of a snapshot: a sha256 over its names and texts, in name order.
pub fn version(docs: &[Doc]) -> String {
    let mut sorted: Vec<&Doc> = docs.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    let mut hash = Sha256::new();
    for (name, text) in sorted {
        hash.update(name.as_bytes());
        hash.update([0]);
        hash.update(text.as_bytes());
        hash.update([0]);
    }
    hash.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// Replaces the snapshot in `dest` with `docs` (written beside it first, so a failure leaves
/// the old one); its version.
pub fn write_snapshot(dest: &Path, docs: &[Doc]) -> AppResult<String> {
    let parent = dest.parent().ok_or("That source has no folder.")?;
    private_dir(parent)?;
    let mut fresh = dest.as_os_str().to_owned();
    fresh.push(".new");
    let fresh = std::path::PathBuf::from(fresh);
    if fresh.exists() {
        std::fs::remove_dir_all(&fresh)?;
    }
    private_dir(&fresh)?;
    let written = docs.iter().try_for_each(|(name, text)| {
        let path = fresh.join(name);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, text)
    });
    if let Err(e) = written {
        let _ = std::fs::remove_dir_all(&fresh);
        return Err(e.into());
    }
    if dest.exists() {
        std::fs::remove_dir_all(dest)?;
    }
    std::fs::rename(&fresh, dest)?;
    Ok(version(docs))
}

/// `name` with only safe characters, never `.` or `..`.
fn safe_part(name: &str) -> String {
    let clean: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '-'
            }
        })
        .collect();
    let clean = clean.trim_matches('.').to_string();
    if clean.is_empty() {
        "file".into()
    } else {
        clean
    }
}

/// `name`, or `name-2`, `name-3`… before its extension when `taken` has it.
fn unique_name(name: String, taken: &HashSet<String>) -> String {
    if !taken.contains(&name) {
        return name;
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (name.clone(), String::new()),
    };
    (2..)
        .map(|n| format!("{stem}-{n}{ext}"))
        .find(|n| !taken.contains(n))
        .expect("some number is free")
}

/// Reads each file into a snapshot: text files keep their name, others get `.txt` added
/// (`guide.pdf.txt`). Files that can't be read are left out, with why.
pub fn read_files(paths: &[String]) -> (Vec<Doc>, Vec<String>) {
    let mut docs = Vec::new();
    let mut problems = Vec::new();
    let mut taken = HashSet::new();
    for path in paths.iter().take(MAX_FILES) {
        let p = Path::new(path);
        let file_name = p
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.clone());
        let read = match std::fs::metadata(p) {
            Err(_) => Err(format!("{file_name} isn't there any more.")),
            Ok(m) if !m.is_file() => Err(format!("{file_name} isn't a file.")),
            Ok(m) if m.len() > MAX_FILE_BYTES => Err(format!(
                "{file_name} is over {} MB.",
                MAX_FILE_BYTES / 1024 / 1024
            )),
            Ok(_) => std::fs::read(p)
                .map_err(|e| format!("Couldn't read {file_name}: {e}"))
                .and_then(|bytes| extract(&file_name, &bytes)),
        };
        match read {
            Ok(text) => {
                let mut name = safe_part(&file_name);
                if !TEXT_EXTENSIONS.contains(&extension(&name).as_str()) {
                    name.push_str(".txt");
                }
                let name = unique_name(name, &taken);
                taken.insert(name.clone());
                docs.push((name, text));
            }
            Err(e) => problems.push(e),
        }
    }
    (docs, problems)
}

/// A web address SayQL may read: http or https, a host name, no user name or password.
pub fn check_web_url(url: &str) -> Result<Url, String> {
    let url = url.trim();
    let mut parsed = Url::parse(url)
        .map_err(|_| "Enter the page's full address, starting with https://.".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("SayQL reads http:// and https:// pages only.".into());
    }
    if parsed.host_str().is_none_or(str::is_empty) {
        return Err("Enter the page's full address, starting with https://.".into());
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(
            "Leave the user name and password out of the address: SayQL reads public pages only."
                .into(),
        );
    }
    parsed.set_fragment(None);
    Ok(parsed)
}

/// Not this computer, a private network, or a link-local or other special address.
fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            !(v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_multicast()
                || a == 0
                // Shared address space (carrier NAT) and benchmarking.
                || (a == 100 && (64..128).contains(&b))
                || (a == 198 && (b == 18 || b == 19)))
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_public(IpAddr::V4(v4));
            }
            let first = v6.segments()[0];
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                // Unique local (fc00::/7) and link-local (fe80::/10).
                || (first & 0xfe00) == 0xfc00
                || (first & 0xffc0) == 0xfe80)
        }
    }
}

/// The addresses `url`'s host has, all public unless `allow_private` (tests only).
async fn public_addrs(url: &Url, allow_private: bool) -> AppResult<Vec<SocketAddr>> {
    let host = url.host_str().unwrap_or_default().trim_matches(['[', ']']);
    let port = url.port_or_known_default().unwrap_or(443);
    let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host, port))
        .await
        .map_err(|_| format!("SayQL couldn't find {host}. Check the address."))?
        .collect();
    if addrs.is_empty() {
        return Err(format!("SayQL couldn't find {host}. Check the address.").into());
    }
    if !allow_private && addrs.iter().any(|a| !is_public(a.ip())) {
        return Err(format!(
            "SayQL reads public web pages only, and {host} is on this computer or a private network."
        )
        .into());
    }
    Ok(addrs)
}

/// What a crawl may read: the start page's site and its folder.
struct Scope {
    host: String,
    /// Only when the address names one: http and https of a site are the same site.
    port: Option<u16>,
    /// `/docs/` for `/docs/intro.html` or `/docs`, `/` for the whole site.
    prefix: String,
}

impl Scope {
    fn of(start: &Url) -> Scope {
        let path = start.path();
        let prefix = if path.ends_with('/') {
            path.to_string()
        } else {
            let last = path.rsplit('/').next().unwrap_or("");
            if last.contains('.') {
                path[..path.len() - last.len()].to_string()
            } else {
                format!("{path}/")
            }
        };
        Scope {
            host: start.host_str().unwrap_or_default().to_ascii_lowercase(),
            port: start.port(),
            prefix,
        }
    }

    fn same_site(&self, url: &Url) -> bool {
        matches!(url.scheme(), "http" | "https")
            && url.host_str().map(str::to_ascii_lowercase).as_deref() == Some(self.host.as_str())
            && url.port() == self.port
    }

    /// In the start page's folder (`/docs` itself counts for `/docs/`).
    fn contains(&self, url: &Url) -> bool {
        self.same_site(url)
            && (url.path().starts_with(&self.prefix)
                || url.path() == self.prefix.trim_end_matches('/'))
    }
}

/// robots.txt rules for SayQL: `(allow, pattern)`, from the group naming SayQL, else `*`.
fn robots_rules(text: &str) -> Vec<(bool, String)> {
    let mut star = Vec::new();
    let mut ours = Vec::new();
    let mut named_ours = false;
    let (mut agents, mut in_rules): (Vec<String>, bool) = (Vec::new(), false);
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let (key, value) = (key.trim().to_ascii_lowercase(), value.trim());
        match key.as_str() {
            "user-agent" => {
                if in_rules {
                    agents.clear();
                    in_rules = false;
                }
                agents.push(value.to_ascii_lowercase());
            }
            "allow" | "disallow" => {
                in_rules = true;
                if value.is_empty() {
                    continue;
                }
                let rule = (key == "allow", value.to_string());
                if agents.iter().any(|a| a.contains("sayql")) {
                    named_ours = true;
                    ours.push(rule.clone());
                }
                if agents.iter().any(|a| a == "*") {
                    star.push(rule);
                }
            }
            _ => {}
        }
    }
    if named_ours { ours } else { star }
}

/// `pattern` matches the start of `path`; `*` is any text and a final `$` the end.
fn robots_match(pattern: &str, path: &str) -> bool {
    let (pattern, anchored) = match pattern.strip_suffix('$') {
        Some(p) => (p, true),
        None => (pattern, false),
    };
    let parts: Vec<&str> = pattern.split('*').collect();
    let mut at = 0;
    for (i, part) in parts.iter().enumerate() {
        if i == 0 {
            if !path.starts_with(part) {
                return false;
            }
            at = part.len();
        } else if let Some(found) = path[at..].find(part) {
            at += found + part.len();
        } else {
            return false;
        }
    }
    if anchored {
        // The last part must end the path.
        parts.len() == 1 && at == path.len()
            || parts.len() > 1 && path.ends_with(parts[parts.len() - 1])
    } else {
        true
    }
}

/// Whether robots.txt lets SayQL read `path`: the longest matching rule wins, allow on a tie.
fn robots_allow(rules: &[(bool, String)], path: &str) -> bool {
    rules
        .iter()
        .filter(|(_, p)| robots_match(p, path))
        .max_by_key(|(allow, p)| (p.len(), *allow))
        .is_none_or(|(allow, _)| *allow)
}

/// The links in a page, as absolute addresses without a fragment or query.
fn links(base: &Url, html: &str) -> Vec<Url> {
    let lower = html.to_ascii_lowercase();
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(i) = lower[from..].find("href") {
        let mut at = from + i + 4;
        from = at;
        let rest = &html[at..];
        let trimmed = rest.trim_start();
        if !trimmed.starts_with('=') {
            continue;
        }
        at += rest.len() - trimmed.len() + 1;
        let value = html[at..].trim_start();
        let link = match value.chars().next() {
            Some(q @ ('"' | '\'')) => value[1..].split(q).next().unwrap_or(""),
            _ => value
                .split(|c: char| c.is_whitespace() || c == '>')
                .next()
                .unwrap_or(""),
        };
        let link = link.replace("&amp;", "&");
        if let Ok(mut url) = base.join(link.trim()) {
            url.set_fragment(None);
            url.set_query(None);
            out.push(url);
        }
    }
    out
}

/// A page's file in the snapshot: `docs/billing/refunds.txt`, `index.txt` for `/`.
fn page_name(url: &Url) -> String {
    let parts: Vec<String> = url
        .path_segments()
        .map(|s| s.filter(|p| !p.is_empty()).map(safe_part).collect())
        .unwrap_or_default();
    let mut name = if parts.is_empty() {
        "index".to_string()
    } else {
        parts.join("/")
    };
    if url.path().ends_with('/') && !parts.is_empty() {
        name.push_str("/index");
    }
    format!("{name}.txt")
}

/// Limits for [`fetch_web_with`]: the crate's defaults, smaller in tests.
pub struct Limits {
    pub pages: usize,
    pub bytes: usize,
}

const LIMITS: Limits = Limits {
    pages: MAX_PAGES,
    bytes: MAX_WEB_BYTES,
};

/// One page, or the pages in its folder on the same site (`crawl`), as text snapshots.
pub async fn fetch_web(url: &str, crawl: bool) -> AppResult<Vec<Doc>> {
    fetch_web_with(url, crawl, &LIMITS, false).await
}

struct Page {
    url: Url,
    html: bool,
    body: Vec<u8>,
}

/// [`fetch_web`] with its limits, and private addresses allowed for tests.
pub async fn fetch_web_with(
    url: &str,
    crawl: bool,
    limits: &Limits,
    allow_private: bool,
) -> AppResult<Vec<Doc>> {
    let start = check_web_url(url)?;
    let addrs = public_addrs(&start, allow_private).await?;
    let host = start.host_str().unwrap_or_default().to_string();
    // No cookies, no redirects followed by reqwest (each is checked here), and the host
    // pinned to the addresses checked above so a second lookup can't point elsewhere.
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .resolve_to_addrs(&host, &addrs)
        .timeout(PAGE_TIME)
        .user_agent(concat!(
            "SayQL/",
            env!("CARGO_PKG_VERSION"),
            " (knowledge source)"
        ))
        .build()
        .map_err(|e| format!("Could not set up networking: {e}"))?;
    let scope = Scope::of(&start);
    let rules = if crawl {
        let mut robots = start.clone();
        robots.set_path("/robots.txt");
        robots.set_query(None);
        match get(&client, &scope, robots, 512 * 1024, false).await {
            Ok(Some(page)) if !page.html => robots_rules(&String::from_utf8_lossy(&page.body)),
            _ => Vec::new(),
        }
    } else {
        Vec::new()
    };
    if crawl && !robots_allow(&rules, start.path()) {
        return Err(
            format!("{host}'s robots.txt asks programs like SayQL not to read that page.").into(),
        );
    }

    let mut docs: Vec<Doc> = Vec::new();
    let mut taken = HashSet::new();
    let mut seen = HashSet::from([start.to_string()]);
    let mut queue = VecDeque::from([start.clone()]);
    let mut used = 0usize;
    while let Some(next) = queue.pop_front() {
        if docs.len() >= limits.pages || used >= limits.bytes {
            break;
        }
        let first = docs.is_empty() && next == start;
        let page = match get(&client, &scope, next, limits.bytes - used, crawl).await {
            Ok(Some(page)) => page,
            Ok(None) if first => {
                return Err("That address isn't a web page or text SayQL can read.".into());
            }
            Err(e) if first => return Err(e),
            _ => continue,
        };
        used += page.body.len();
        let text = if page.html {
            html_text(&page.body)
        } else {
            String::from_utf8_lossy(&page.body).into_owned()
        };
        if crawl && page.html {
            let body = String::from_utf8_lossy(&page.body);
            for link in links(&page.url, &body) {
                if scope.contains(&link)
                    && robots_allow(&rules, link.path())
                    && seen.insert(link.to_string())
                {
                    queue.push_back(link);
                }
            }
        }
        if text.trim().is_empty() {
            continue;
        }
        let name = unique_name(page_name(&page.url), &taken);
        taken.insert(name.clone());
        docs.push((name, format!("{}\n\n{text}", page.url)));
    }
    if docs.is_empty() {
        return Err("That page has no text SayQL can read.".into());
    }
    Ok(docs)
}

/// Fetches `url`, following redirects that stay on the site (and in the folder, when
/// `in_folder`). `None`: not a page (an error status, or not HTML or text). At most `max`
/// bytes are read: a longer page is an error.
async fn get(
    client: &reqwest::Client,
    scope: &Scope,
    mut url: Url,
    max: usize,
    in_folder: bool,
) -> AppResult<Option<Page>> {
    let mut hops = 0;
    let mut res = loop {
        let res = client
            .get(url.clone())
            .send()
            .await
            .map_err(|e| format!("Couldn't read {url}: {}", e.without_url()))?;
        if !res.status().is_redirection() {
            break res;
        }
        let Some(next) = res
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|l| l.to_str().ok())
            .and_then(|l| url.join(l).ok())
        else {
            return Ok(None);
        };
        hops += 1;
        let allowed = if in_folder {
            scope.contains(&next)
        } else {
            scope.same_site(&next)
        };
        if !allowed {
            return Err(format!(
                "{url} moves to {next}, on another site. Add that address instead."
            )
            .into());
        }
        if hops > MAX_REDIRECTS {
            return Err(format!("{url} redirects too many times.").into());
        }
        url = next;
    };
    if !res.status().is_success() {
        return Ok(None);
    }
    let kind = res
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|t| t.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    let html = kind.contains("html");
    if !(html || kind.starts_with("text/") || kind.is_empty()) {
        return Ok(None);
    }
    let mut body = Vec::new();
    while let Some(chunk) = res
        .chunk()
        .await
        .map_err(|e| format!("Couldn't read {url}: {}", e.without_url()))?
    {
        if body.len() + chunk.len() > max {
            return Err(format!(
                "{url} is more than SayQL reads from a site ({} MB in all).",
                MAX_WEB_BYTES / 1024 / 1024
            )
            .into());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(Some(Page { url, html, body }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::io::Write;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn reads_text_files_as_they_are() {
        let md = "# Orders\n\n`status`: 1 = paid, 2 = refunded.\n";
        assert_eq!(extract("orders.md", md.as_bytes()).unwrap(), md);
        let csv = "code,meaning\n1,paid\n2,refunded\n";
        assert_eq!(extract("codes.CSV", csv.as_bytes()).unwrap(), csv);
        assert!(
            extract("x.exe", b"MZ")
                .unwrap_err()
                .contains("can't read .exe")
        );
        assert!(
            extract("empty.txt", b"  \n")
                .unwrap_err()
                .contains("no text")
        );
    }

    #[test]
    fn html_becomes_text() {
        let html = "<html><head><style>p{color:red}</style><script>alert(1)</script></head>\
                    <body><h1>Refunds</h1><p>Amounts are in <b>cents</b>.</p></body></html>";
        let text = extract("guide.html", html.as_bytes()).unwrap();
        assert!(
            text.contains("Refunds") && text.contains("Amounts are in cents."),
            "{text}"
        );
        assert!(
            !text.contains("<p>") && !text.contains("alert") && !text.contains("color:red"),
            "{text}"
        );
    }

    /// A one-page PDF with `text` in Helvetica, its cross-reference table worked out here.
    fn pdf(text: &str) -> Vec<u8> {
        let stream = format!("BT /F1 12 Tf 72 720 Td ({text}) Tj ET");
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_string(),
            format!("<< /Length {} >>\nstream\n{stream}\nendstream", stream.len()),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_string(),
        ];
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (i, body) in objects.iter().enumerate() {
            offsets.push(out.len());
            write!(out, "{} 0 obj\n{body}\nendobj\n", i + 1).unwrap();
        }
        let xref = out.len();
        write!(out, "xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).unwrap();
        for o in offsets {
            writeln!(out, "{o:010} 00000 n ").unwrap();
        }
        write!(
            out,
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .unwrap();
        out
    }

    #[test]
    fn pdf_text_is_extracted() {
        let text = extract("spec.pdf", &pdf("Refunds are stored in cents")).unwrap();
        assert!(text.contains("Refunds are stored in cents"), "{text:?}");
        assert!(extract("bad.pdf", b"not a pdf").is_err());
    }

    fn docx(document_xml: &str) -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        let mut zip = zip::ZipWriter::new(&mut buf);
        let opts = zip::write::SimpleFileOptions::default();
        zip.start_file("[Content_Types].xml", opts).unwrap();
        zip.write_all(b"<Types/>").unwrap();
        zip.start_file("word/document.xml", opts).unwrap();
        zip.write_all(document_xml.as_bytes()).unwrap();
        zip.finish().unwrap();
        buf.into_inner()
    }

    #[test]
    fn docx_text_is_extracted() {
        let xml = r#"<?xml version="1.0"?><w:document xmlns:w="x"><w:body>
            <w:p><w:r><w:t>Status codes</w:t></w:r></w:p>
            <w:p><w:r><w:t xml:space="preserve">1 = paid </w:t></w:r><w:r><w:t>&amp; settled</w:t></w:r><w:r><w:tab/><w:t>(final)</w:t></w:r></w:p>
            <w:p><w:r><w:t/></w:r></w:p>
            </w:body></w:document>"#;
        let text = extract("Codes.docx", &docx(xml)).unwrap();
        assert_eq!(
            text.trim(),
            "Status codes\n1 = paid & settled\t(final)",
            "{text:?}"
        );
        assert!(extract("x.docx", b"PK nope").is_err());
    }

    #[test]
    fn files_become_a_snapshot_with_a_version() {
        let dir = std::env::temp_dir().join(format!("sayql-files-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("a")).unwrap();
        std::fs::write(dir.join("notes.md"), "# Notes").unwrap();
        std::fs::write(dir.join("a/notes.md"), "# Other notes").unwrap();
        std::fs::write(dir.join("guide.html"), "<p>Guide</p>").unwrap();
        std::fs::write(dir.join("big.bin"), "x").unwrap();
        let paths: Vec<String> = ["notes.md", "a/notes.md", "guide.html", "big.bin", "gone.md"]
            .iter()
            .map(|p| dir.join(p).to_string_lossy().into_owned())
            .collect();
        let (docs, problems) = read_files(&paths);
        let names: Vec<&str> = docs.iter().map(|d| d.0.as_str()).collect();
        assert_eq!(names, ["notes.md", "notes-2.md", "guide.html.txt"]);
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert!(problems[1].contains("gone.md isn't there"));

        let dest = dir.join("repos/src_abc");
        let v1 = write_snapshot(&dest, &docs).unwrap();
        assert_eq!(v1.len(), 64);
        assert_eq!(
            std::fs::read_to_string(dest.join("notes-2.md")).unwrap(),
            "# Other notes"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&dest).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700);
        }
        // Refresh: the same files give the same version; a changed one, a new version.
        assert_eq!(write_snapshot(&dest, &read_files(&paths).0).unwrap(), v1);
        std::fs::write(dir.join("notes.md"), "# Notes, changed").unwrap();
        let v2 = write_snapshot(&dest, &read_files(&paths).0).unwrap();
        assert_ne!(v1, v2);
        assert_eq!(
            std::fs::read_to_string(dest.join("notes.md")).unwrap(),
            "# Notes, changed"
        );
        assert!(!dir.join("repos/src_abc.new").exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn web_addresses() {
        assert!(check_web_url("https://docs.acme.com/db").is_ok());
        assert!(check_web_url("http://acme.com").is_ok());
        for bad in [
            "ftp://acme.com",
            "file:///etc/passwd",
            "acme.com",
            "https://me:pw@acme.com/",
        ] {
            assert!(check_web_url(bad).is_err(), "{bad}");
        }
        for ip in [
            "127.0.0.1",
            "10.1.2.3",
            "192.168.0.1",
            "172.16.5.5",
            "169.254.169.254",
            "0.0.0.0",
            "100.64.0.1",
            "::1",
            "fd00::1",
            "fe80::1",
            "::ffff:127.0.0.1",
        ] {
            assert!(!is_public(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["93.184.216.34", "2606:2800:220:1::1"] {
            assert!(is_public(ip.parse().unwrap()), "{ip}");
        }
    }

    #[tokio::test]
    async fn this_computer_and_private_networks_are_refused() {
        for url in [
            "http://127.0.0.1:9/",
            "http://localhost/",
            "http://[::1]/",
            "http://10.0.0.1/",
        ] {
            let e = fetch_web(url, false).await.unwrap_err().to_string();
            assert!(e.contains("public web pages only"), "{url}: {e}");
        }
    }

    #[test]
    fn robots_rules_for_sayql() {
        let rules = robots_rules(
            "User-agent: Googlebot\nDisallow: /\n\nUser-agent: *\nDisallow: /docs/private\nAllow: /docs/private/ok\nDisallow: /*.pdf$\n",
        );
        assert!(robots_allow(&rules, "/docs/intro"));
        assert!(!robots_allow(&rules, "/docs/private/x"));
        assert!(robots_allow(&rules, "/docs/private/ok/y"));
        assert!(!robots_allow(&rules, "/docs/a.pdf"));
        assert!(robots_allow(&rules, "/docs/a.pdf.html"));
        let ours =
            robots_rules("User-agent: *\nDisallow: /\nUser-agent: SayQL\nDisallow: /admin\n");
        assert!(robots_allow(&ours, "/docs") && !robots_allow(&ours, "/admin/x"));
        assert!(robots_allow(
            &robots_rules("User-agent: *\nDisallow:\n"),
            "/x"
        ));
    }

    #[test]
    fn crawl_scope_and_page_names() {
        let u = |s: &str| Url::parse(s).unwrap();
        let docs = Scope::of(&u("https://acme.com/docs"));
        assert_eq!(docs.prefix, "/docs/");
        assert!(docs.contains(&u("https://acme.com/docs")));
        assert!(docs.contains(&u("http://acme.com/docs/billing")));
        assert!(!docs.contains(&u("https://acme.com/docs-old/x")));
        assert!(!docs.contains(&u("https://acme.com/blog")));
        assert!(!docs.contains(&u("https://evil.com/docs/x")));
        assert!(!docs.contains(&u("https://acme.com:8443/docs/x")));
        assert_eq!(
            Scope::of(&u("https://acme.com/docs/intro.html")).prefix,
            "/docs/"
        );
        assert_eq!(Scope::of(&u("https://acme.com")).prefix, "/");
        assert_eq!(page_name(&u("https://acme.com/")), "index.txt");
        assert_eq!(page_name(&u("https://acme.com/docs/")), "docs/index.txt");
        assert_eq!(
            page_name(&u("https://acme.com/docs/a%20b/../c")),
            "docs/c.txt"
        );
        assert_eq!(
            page_name(&u("https://acme.com/docs/x.html")),
            "docs/x.html.txt"
        );
        let found = links(
            &u("https://acme.com/docs/"),
            r#"<a href="billing">B</a> <a HREF = 'https://acme.com/docs/x?y=1#z'>X</a> <a href=/blog>no</a>"#,
        );
        let found: Vec<String> = found.iter().map(Url::to_string).collect();
        assert_eq!(
            found,
            [
                "https://acme.com/docs/billing",
                "https://acme.com/docs/x",
                "https://acme.com/blog"
            ]
        );
    }

    /// A web server on this computer serving `pages` (path → (content type, body)); every
    /// path asked for is recorded.
    async fn serve(
        pages: HashMap<&'static str, (&'static str, String)>,
    ) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let asked = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = asked.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    return;
                };
                let mut buf = vec![0; 8192];
                let n = sock.read(&mut buf).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).into_owned();
                let path = req.split_whitespace().nth(1).unwrap_or("/").to_string();
                let has_cookie = req.to_ascii_lowercase().contains("\ncookie:");
                log.lock().unwrap().push(if has_cookie {
                    format!("{path} (cookie)")
                } else {
                    path.clone()
                });
                let reply = match pages.get(path.as_str()) {
                    Some(("redirect", to)) => format!(
                        "HTTP/1.1 302 Found\r\nLocation: {to}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    ),
                    Some((kind, body)) => format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nSet-Cookie: s=1\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    ),
                    None => {
                        "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                            .to_string()
                    }
                };
                let _ = sock.write_all(reply.as_bytes()).await;
            }
        });
        (format!("http://{addr}"), asked)
    }

    const HTML: &str = "text/html; charset=utf-8";

    #[tokio::test]
    async fn crawls_only_its_folder_and_respects_robots_and_caps() {
        let page = |links: &str, text: &str| {
            (
                HTML,
                format!("<html><body><p>{text}</p>{links}</body></html>"),
            )
        };
        let mut pages = HashMap::from([
            (
                "/robots.txt",
                (
                    "text/plain",
                    "User-agent: *\nDisallow: /docs/secret\n".to_string(),
                ),
            ),
            (
                "/docs/",
                page(
                    r#"<a href="a">a</a><a href="/docs/b">b</a><a href="/blog">blog</a><a href="secret">s</a><a href="/docs/old">old</a><a href="/docs/img.png">img</a>"#,
                    "Start",
                ),
            ),
            (
                "/docs/a",
                page(
                    r#"<a href="/docs/">up</a><a href="http://example.com/docs/x">away</a>"#,
                    "Page A",
                ),
            ),
            ("/docs/b", page("", "Page B")),
            ("/docs/old", ("redirect", "/blog/moved".to_string())),
            ("/docs/img.png", ("image/png", "PNG".to_string())),
            ("/docs/secret", page("", "Secret")),
            ("/blog", page("", "Blog")),
        ]);
        let (base, asked) = serve(pages.clone()).await;
        let limits = Limits {
            pages: 50,
            bytes: 1024 * 1024,
        };
        let docs = fetch_web_with(&format!("{base}/docs/"), true, &limits, true)
            .await
            .unwrap();
        let names: Vec<&str> = docs.iter().map(|d| d.0.as_str()).collect();
        assert_eq!(names, ["docs/index.txt", "docs/a.txt", "docs/b.txt"]);
        assert!(
            docs[1].1.starts_with(&format!("{base}/docs/a\n\n")) && docs[1].1.contains("Page A")
        );
        let asked = asked.lock().unwrap().clone();
        assert!(
            !asked
                .iter()
                .any(|p| p.contains("secret") || p.starts_with("/blog")),
            "{asked:?}"
        );
        assert!(
            !asked.iter().any(|p| p.contains("cookie")),
            "no cookies are kept: {asked:?}"
        );
        assert_ne!(version(&docs), "");

        // Caps: pages, then bytes.
        let two = fetch_web_with(
            &format!("{base}/docs/"),
            true,
            &Limits {
                pages: 2,
                bytes: 1024 * 1024,
            },
            true,
        )
        .await
        .unwrap();
        assert_eq!(two.len(), 2);
        pages.insert("/docs/b", (HTML, format!("<p>{}</p>", "x".repeat(4000))));
        let (base, _) = serve(pages.clone()).await;
        let small = fetch_web_with(
            &format!("{base}/docs/"),
            true,
            &Limits {
                pages: 50,
                bytes: 2000,
            },
            true,
        )
        .await
        .unwrap();
        assert!(small.iter().all(|d| d.0 != "docs/b.txt"), "{small:?}");

        // One page: no links followed, and robots.txt isn't read.
        let (base, asked) = serve(pages.clone()).await;
        let one = fetch_web_with(&format!("{base}/docs/"), false, &limits, true)
            .await
            .unwrap();
        assert_eq!(one.len(), 1);
        assert_eq!(asked.lock().unwrap().as_slice(), ["/docs/"]);

        // A start page robots.txt refuses, one that isn't a page, and a missing one.
        let e = fetch_web_with(&format!("{base}/docs/secret"), true, &limits, true)
            .await
            .unwrap_err();
        assert!(e.to_string().contains("robots.txt"), "{e}");
        assert!(
            fetch_web_with(&format!("{base}/docs/img.png"), false, &limits, true)
                .await
                .is_err()
        );
        assert!(
            fetch_web_with(&format!("{base}/nothing"), false, &limits, true)
                .await
                .is_err()
        );
    }
}
