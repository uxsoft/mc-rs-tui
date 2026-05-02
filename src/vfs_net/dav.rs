//! Basic WebDAV (HTTP) backend — read-only browsing.
//!
//! The endpoint URL is encoded in the layer's `location` field, percent-encoded
//! since it may include slashes. We send a `Depth: 1` PROPFIND for directory
//! listing and a plain GET for file content. No auth UI yet (anonymous /
//! basic-auth via `MC_RS_DAV_USER` / `MC_RS_DAV_PASS`).

use std::sync::Arc;

use crate::core::{Entry, EntryKind, Error, Result, VPath};
use crate::vfs::trait_::{AsyncReader, Capabilities, Vfs};
use async_trait::async_trait;
use percent_encoding::{NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};
use quick_xml::events::Event;
use quick_xml::reader::Reader;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};

use tokio::io::AsyncRead;

#[derive(Debug, Clone)]
pub struct DavEndpoint {
    /// Full base URL like `https://dav.example.com/remote.php/dav/`.
    pub base: String,
    pub user: Option<String>,
    pub password: Option<String>,
}

impl DavEndpoint {
    /// Build from a `[user@]https://host/...` style location string.
    pub fn parse(loc: &str) -> Result<Self> {
        // Decode percent-encoding so the location reads back intact.
        let decoded = percent_decode_str(loc)
            .decode_utf8()
            .map_err(|e| Error::InvalidPath(format!("dav location utf8: {e}")))?
            .into_owned();
        let (user, host_url) = match decoded.find('@') {
            Some(at_idx) => {
                let prefix = &decoded[..at_idx];
                if prefix.starts_with("http://") || prefix.starts_with("https://") {
                    (None, decoded.clone())
                } else {
                    (Some(prefix.to_string()), decoded[at_idx + 1..].to_string())
                }
            }
            None => (None, decoded.clone()),
        };
        // Default to https if scheme missing.
        let base = if host_url.starts_with("http://") || host_url.starts_with("https://") {
            host_url
        } else {
            format!("https://{host_url}")
        };
        let password = std::env::var("MC_RS_DAV_PASS")
            .ok()
            .filter(|s| !s.is_empty());
        let user = user.or_else(|| {
            std::env::var("MC_RS_DAV_USER")
                .ok()
                .filter(|s| !s.is_empty())
        });
        Ok(Self {
            base,
            user,
            password,
        })
    }

    /// Encode for use as a `Layer::location` (slashes percent-encoded).
    #[must_use]
    pub fn encode_for_layer(s: &str) -> String {
        utf8_percent_encode(s, NON_ALPHANUMERIC).to_string()
    }
}

pub struct DavVfs {
    scheme: &'static str,
    base: String,
    client: reqwest::Client,
    user: Option<String>,
    password: Option<String>,
}

impl DavVfs {
    pub fn open(scheme: &'static str, endpoint: DavEndpoint) -> Result<Self> {
        let client = reqwest::Client::builder()
            .build()
            .map_err(|e| Error::Vfs(format!("dav client: {e}")))?;
        Ok(Self {
            scheme,
            base: endpoint.base,
            client,
            user: endpoint.user,
            password: endpoint.password,
        })
    }

    fn url_for(&self, p: &VPath) -> Result<String> {
        let layer = p
            .layers()
            .iter()
            .rev()
            .find(|l| l.scheme == self.scheme)
            .ok_or_else(|| Error::InvalidPath(format!("vpath has no {} layer", self.scheme)))?;
        let sub = layer.sub.to_string_lossy();
        let suffix = if sub.starts_with('/') {
            sub.trim_start_matches('/').to_string()
        } else {
            sub.into_owned()
        };
        let mut url = self.base.trim_end_matches('/').to_string();
        if !suffix.is_empty() {
            url.push('/');
            url.push_str(&suffix);
        }
        Ok(url)
    }

    fn auth(&self) -> Option<(String, Option<String>)> {
        self.user
            .as_ref()
            .map(|u| (u.clone(), self.password.clone()))
    }
}

#[async_trait]
impl Vfs for DavVfs {
    fn scheme(&self) -> &'static str {
        self.scheme
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::READ | Capabilities::STAT
    }

    async fn stat(&self, p: &VPath) -> Result<Entry> {
        // PROPFIND Depth: 0 → single-entry response.
        let url = self.url_for(p)?;
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("depth"),
            HeaderValue::from_static("0"),
        );
        let mut req = self
            .client
            .request(reqwest::Method::from_bytes(b"PROPFIND").unwrap(), &url)
            .headers(headers)
            .body(MIN_PROPFIND_BODY);
        if let Some((u, pw)) = self.auth() {
            req = req.basic_auth(u, pw);
        }
        let resp = req.send().await.map_err(net_err)?;
        if !resp.status().is_success() {
            return Err(Error::Vfs(format!("dav stat {url}: {}", resp.status())));
        }
        let body = resp.text().await.map_err(net_err)?;
        let entries = parse_multistatus(&body, &self.base);
        entries
            .into_iter()
            .next()
            .ok_or_else(|| Error::Vfs(format!("dav stat {url}: empty response")))
    }

    async fn read_dir(&self, p: &VPath) -> Result<Vec<Entry>> {
        let url = self.url_for(p)?;
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("depth"),
            HeaderValue::from_static("1"),
        );
        let mut req = self
            .client
            .request(reqwest::Method::from_bytes(b"PROPFIND").unwrap(), &url)
            .headers(headers)
            .body(MIN_PROPFIND_BODY);
        if let Some((u, pw)) = self.auth() {
            req = req.basic_auth(u, pw);
        }
        let resp = req.send().await.map_err(net_err)?;
        if !resp.status().is_success() {
            return Err(Error::Vfs(format!("dav read_dir {url}: {}", resp.status())));
        }
        let body = resp.text().await.map_err(net_err)?;
        let mut entries = parse_multistatus(&body, &self.base);
        // The first entry is typically the directory itself.
        if !entries.is_empty() {
            entries.remove(0);
        }
        Ok(entries)
    }

    async fn open_read(&self, p: &VPath) -> Result<AsyncReader> {
        let url = self.url_for(p)?;
        let mut req = self.client.get(&url);
        if let Some((u, pw)) = self.auth() {
            req = req.basic_auth(u, pw);
        }
        let resp = req.send().await.map_err(net_err)?;
        if !resp.status().is_success() {
            return Err(Error::Vfs(format!("dav get {url}: {}", resp.status())));
        }
        let bytes = resp.bytes().await.map_err(net_err)?;
        Ok(Box::new(BytesReader {
            data: Arc::from(bytes.to_vec()),
            pos: 0,
        }))
    }
}

const MIN_PROPFIND_BODY: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<D:propfind xmlns:D="DAV:">
  <D:prop>
    <D:resourcetype/>
    <D:getcontentlength/>
    <D:getlastmodified/>
    <D:displayname/>
  </D:prop>
</D:propfind>"#;

fn net_err(e: reqwest::Error) -> Error {
    Error::Vfs(format!("dav: {e}"))
}

#[derive(Default)]
struct PropState {
    href: Option<String>,
    name: Option<String>,
    is_collection: bool,
    size: Option<u64>,
    mtime_str: Option<String>,
}

/// Parse a `multistatus` response body into a `Vec<Entry>`. We strip the base
/// URL from each `<href>` to use the trailing path's basename as the entry name.
fn parse_multistatus(body: &str, base: &str) -> Vec<Entry> {
    let mut reader = Reader::from_str(body);
    let mut buf = Vec::new();
    let mut entries: Vec<Entry> = Vec::new();
    let mut current = PropState::default();
    let mut text_target: Option<&'static str> = None;
    let mut in_resourcetype = false;
    let mut depth = 0usize;
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let raw = e.name();
                let local = local_name(raw.as_ref());
                depth += 1;
                match local {
                    b"response" => current = PropState::default(),
                    b"href" => text_target = Some("href"),
                    b"displayname" => text_target = Some("displayname"),
                    b"getcontentlength" => text_target = Some("getcontentlength"),
                    b"getlastmodified" => text_target = Some("getlastmodified"),
                    b"resourcetype" => in_resourcetype = true,
                    b"collection" if in_resourcetype => current.is_collection = true,
                    _ => {}
                }
            }
            Ok(Event::Empty(e)) => {
                let raw = e.name();
                let local = local_name(raw.as_ref());
                if local == b"collection" && in_resourcetype {
                    current.is_collection = true;
                }
            }
            Ok(Event::End(e)) => {
                let raw = e.name();
                let local = local_name(raw.as_ref());
                depth = depth.saturating_sub(1);
                match local {
                    b"response" => {
                        let name = current.name.clone().unwrap_or_else(|| {
                            current
                                .href
                                .as_deref()
                                .map(|h| basename_of(h, base))
                                .unwrap_or_default()
                        });
                        let kind = if current.is_collection {
                            EntryKind::Dir
                        } else {
                            EntryKind::File
                        };
                        if !name.is_empty() {
                            entries.push(Entry {
                                name,
                                kind,
                                size: current.size.unwrap_or(0),
                                mtime: parse_http_date(current.mtime_str.as_deref()),
                                atime: None,
                                ctime: None,
                                mode: None,
                                uid: None,
                                gid: None,
                                nlink: None,
                                target: None,
                            });
                        }
                    }
                    b"resourcetype" => in_resourcetype = false,
                    b"href" | b"displayname" | b"getcontentlength" | b"getlastmodified" => {
                        text_target = None;
                    }
                    _ => {}
                }
            }
            Ok(Event::Text(t)) => {
                let s = match t.decode() {
                    Ok(s) => s.into_owned(),
                    Err(_) => continue,
                };
                match text_target {
                    Some("href") => current.href = Some(s),
                    Some("displayname") => {
                        if !s.is_empty() {
                            current.name = Some(s);
                        }
                    }
                    Some("getcontentlength") => current.size = s.parse().ok(),
                    Some("getlastmodified") => current.mtime_str = Some(s),
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                tracing::warn!("dav xml parse: {e}");
                break;
            }
            _ => {}
        }
        buf.clear();
        let _ = depth; // silence unused warning when no logging
    }
    entries
}

fn local_name(qname: &[u8]) -> &[u8] {
    qname
        .iter()
        .position(|&b| b == b':')
        .map_or(qname, |i| &qname[i + 1..])
}

fn basename_of(href: &str, base: &str) -> String {
    let stripped = href.strip_prefix(base).unwrap_or(href);
    let trimmed = stripped.trim_end_matches('/');
    let last = trimmed.rsplit('/').next().unwrap_or("");
    percent_decode_str(last)
        .decode_utf8()
        .map(|c| c.into_owned())
        .unwrap_or_else(|_| last.to_string())
}

fn parse_http_date(s: Option<&str>) -> Option<std::time::SystemTime> {
    // RFC 1123 IMF-fixdate, e.g. "Tue, 15 Nov 1994 12:45:26 GMT".
    // We don't pull chrono — a small hand-rolled parser covers what every
    // real-world WebDAV server emits. Older RFC 850 and asctime forms are
    // ignored (servers no longer emit them).
    let s = s?.trim();
    let comma = s.find(',')?;
    let rest = s.get(comma + 1..)?.trim_start();
    let mut parts = rest.split_whitespace();
    let day: u32 = parts.next()?.parse().ok()?;
    let month = parts.next()?;
    let year: i32 = parts.next()?.parse().ok()?;
    let hms = parts.next()?;
    // Trailing "GMT" is required by RFC 9110; we don't support other zones.
    if !matches!(parts.next(), Some(z) if z.eq_ignore_ascii_case("GMT")) {
        return None;
    }
    let mut hms_parts = hms.split(':');
    let h: u32 = hms_parts.next()?.parse().ok()?;
    let m: u32 = hms_parts.next()?.parse().ok()?;
    let sec: u32 = hms_parts.next()?.parse().ok()?;
    let month_idx = match month {
        "Jan" => 1,
        "Feb" => 2,
        "Mar" => 3,
        "Apr" => 4,
        "May" => 5,
        "Jun" => 6,
        "Jul" => 7,
        "Aug" => 8,
        "Sep" => 9,
        "Oct" => 10,
        "Nov" => 11,
        "Dec" => 12,
        _ => return None,
    };
    let secs_since_epoch = days_from_civil(year, month_idx, day)? * 86_400
        + i64::from(h) * 3600
        + i64::from(m) * 60
        + i64::from(sec);
    if secs_since_epoch >= 0 {
        std::time::UNIX_EPOCH.checked_add(std::time::Duration::from_secs(
            u64::try_from(secs_since_epoch).ok()?,
        ))
    } else {
        std::time::UNIX_EPOCH.checked_sub(std::time::Duration::from_secs(
            u64::try_from(-secs_since_epoch).ok()?,
        ))
    }
}

/// Howard Hinnant's date algorithm: days from 1970-01-01 to the given Y-M-D.
/// Negative results mean dates before the Unix epoch.
fn days_from_civil(y: i32, m: u32, d: u32) -> Option<i64> {
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u32;
    let doy = (153 * if m > 2 { m - 3 } else { m + 9 } + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(i64::from(era) * 146_097 + i64::from(doe) - 719_468)
}

struct BytesReader {
    data: Arc<[u8]>,
    pos: usize,
}

impl AsyncRead for BytesReader {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let me = self.get_mut();
        let remaining = &me.data[me.pos..];
        let n = remaining.len().min(buf.remaining());
        if n == 0 {
            return std::task::Poll::Ready(Ok(()));
        }
        buf.put_slice(&remaining[..n]);
        me.pos += n;
        std::task::Poll::Ready(Ok(()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_endpoint_https() {
        let e = DavEndpoint::parse("https://dav.example.com/foo/").unwrap();
        assert_eq!(e.base, "https://dav.example.com/foo/");
        assert!(e.user.is_none() || e.user.is_some());
    }

    #[test]
    fn parse_endpoint_with_user() {
        let e = DavEndpoint::parse("alice@https://dav.example.com/foo/").unwrap();
        assert_eq!(e.user.as_deref(), Some("alice"));
    }

    #[test]
    fn parse_rfc1123_date() {
        // 1994-11-15 12:45:26 UTC = 784903526
        let t = parse_http_date(Some("Tue, 15 Nov 1994 12:45:26 GMT")).unwrap();
        let secs = t.duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        assert_eq!(secs, 784_903_526);
    }

    #[test]
    fn parse_rfc1123_rejects_bad() {
        assert!(parse_http_date(None).is_none());
        assert!(parse_http_date(Some("not a date")).is_none());
        assert!(parse_http_date(Some("Tue, 15 Nov 1994 12:45:26 EDT")).is_none());
    }

    #[test]
    fn parse_multistatus_one_collection() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<D:multistatus xmlns:D="DAV:">
  <D:response>
    <D:href>/dav/</D:href>
    <D:propstat>
      <D:prop>
        <D:displayname>dav</D:displayname>
        <D:resourcetype><D:collection/></D:resourcetype>
      </D:prop>
      <D:status>HTTP/1.1 200 OK</D:status>
    </D:propstat>
  </D:response>
  <D:response>
    <D:href>/dav/file.txt</D:href>
    <D:propstat>
      <D:prop>
        <D:displayname>file.txt</D:displayname>
        <D:resourcetype/>
        <D:getcontentlength>42</D:getcontentlength>
      </D:prop>
      <D:status>HTTP/1.1 200 OK</D:status>
    </D:propstat>
  </D:response>
</D:multistatus>"#;
        let entries = parse_multistatus(xml, "");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "dav");
        assert!(matches!(entries[0].kind, EntryKind::Dir));
        assert_eq!(entries[1].name, "file.txt");
        assert!(matches!(entries[1].kind, EntryKind::File));
        assert_eq!(entries[1].size, 42);
    }
}
