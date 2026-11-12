//! Downloading one image into a folder. The HTTP client is behind the [`Fetch`] trait (the real
//! one lives in `http.rs`), so everything else here is plain code that is tested with fakes.
//!
//! Rules: the response must be an image (content type and the file's first bytes are both
//! checked, so a web page is never saved as `.jpg`); the name comes from `Content-Disposition`,
//! then the URL, with the extension corrected to the real format; an existing file is never
//! overwritten (`name (2).ext`); a failed or cancelled download leaves nothing behind.

use std::{
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

/// Largest image accepted.
pub const MAX_BYTES: u64 = 512 << 20;

pub struct Response {
    /// Where the request ended up after redirects.
    pub final_url: String,
    /// `Content-Type` without parameters, lower case.
    pub content_type: Option<String>,
    pub disposition: Option<String>,
    pub length: Option<u64>,
    pub body: Box<dyn Read + Send>,
}

pub trait Fetch {
    fn get(&self, url: &str, referer: Option<&str>) -> io::Result<Response>;
}

/// Where a download reports to (the job's counters in production).
pub trait Sink {
    fn named(&self, name: &str);
    fn total(&self, bytes: u64);
    fn advance(&self, bytes: u64);
    fn cancelled(&self) -> bool;
}

/// Downloads `url` into `dest`; returns the file written.
pub fn fetch_image(
    fetch: &dyn Fetch,
    url: &str,
    referer: Option<&str>,
    dest: &Path,
    sink: &dyn Sink,
) -> io::Result<PathBuf> {
    if !dest.is_dir() {
        return Err(io::Error::other("the destination folder is not available"));
    }
    let mut resp = fetch.get(url, referer)?;
    let mime = resp.content_type.clone().unwrap_or_default();
    if mime.starts_with("text/")
        || mime.contains("html")
        || mime.contains("json")
        || mime.contains("xml") && mime != "image/svg+xml"
    {
        return Err(io::Error::other(format!(
            "not an image (the server sent {mime})"
        )));
    }
    if resp.length.is_some_and(|n| n > MAX_BYTES) {
        return Err(io::Error::other("the image is larger than 512 MB"));
    }

    // Read enough of the start to recognise the format.
    let mut head = vec![0u8; 512];
    let mut got = 0;
    while got < 32 {
        let n = resp.body.read(&mut head[got..])?;
        if n == 0 {
            break;
        }
        got += n;
    }
    head.truncate(got);
    if head.is_empty() {
        return Err(io::Error::other("the server sent an empty file"));
    }
    let sniffed = sniff_image(&head);
    if sniffed.is_none() && (!mime.starts_with("image/") || looks_like_html(&head)) {
        return Err(io::Error::other(if mime.is_empty() {
            "not an image".to_owned()
        } else {
            format!("not an image (the server sent {mime})")
        }));
    }

    let name = file_name_for(
        url,
        &resp.final_url,
        resp.disposition.as_deref(),
        Some(&mime),
        sniffed,
    );
    let (mut file, path) = create_unique(dest, &name)?;
    sink.named(
        &path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or(name),
    );
    if let Some(n) = resp.length {
        sink.total(n);
    }

    let result = (|| -> io::Result<()> {
        let mut written = 0u64;
        file.write_all(&head)?;
        written += head.len() as u64;
        sink.advance(head.len() as u64);
        let mut buf = vec![0u8; 64 << 10];
        loop {
            if sink.cancelled() {
                return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
            }
            let n = resp.body.read(&mut buf)?;
            if n == 0 {
                break;
            }
            written += n as u64;
            if written > MAX_BYTES {
                return Err(io::Error::other("the image is larger than 512 MB"));
            }
            file.write_all(&buf[..n])?;
            sink.advance(n as u64);
        }
        if resp.length.is_some_and(|l| l != written) {
            return Err(io::Error::other("the download was cut short"));
        }
        file.flush()
    })();
    drop(file);
    match result {
        Ok(()) => Ok(path),
        Err(e) => {
            let _ = fs::remove_file(&path);
            Err(e)
        }
    }
}

/// Creates `dest/name`, or `dest/name (2)` and so on when it exists. Never overwrites: the
/// check and the creation are one atomic step.
fn create_unique(dest: &Path, name: &str) -> io::Result<(fs::File, PathBuf)> {
    let first = dest.join(name);
    let stem = first
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = first
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    for n in 1..10_000u32 {
        let path = if n == 1 {
            first.clone()
        } else {
            dest.join(format!("{stem} ({n}){ext}"))
        };
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(f) => return Ok((f, path)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::other("too many files with that name"))
}

// --- Recognising images ------------------------------------------------------------------------

/// Extension of the image format whose magic bytes `b` start with.
pub fn sniff_image(b: &[u8]) -> Option<&'static str> {
    let starts = |p: &[u8]| b.starts_with(p);
    if starts(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if starts(&[0xFF, 0xD8, 0xFF]) {
        Some("jpg")
    } else if starts(b"GIF87a") || starts(b"GIF89a") {
        Some("gif")
    } else if b.len() >= 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        Some("webp")
    } else if b.len() >= 14 && starts(b"BM") && b[6..10] == [0, 0, 0, 0] {
        Some("bmp")
    } else if starts(&[0, 0, 1, 0]) {
        Some("ico")
    } else if starts(b"II*\0") || starts(b"MM\0*") {
        Some("tiff")
    } else if b.len() >= 12 && &b[4..8] == b"ftyp" {
        match &b[8..12] {
            b"avif" | b"avis" => Some("avif"),
            b"heic" | b"heix" | b"mif1" | b"msf1" => Some("heic"),
            _ => None,
        }
    } else {
        let text = String::from_utf8_lossy(&b[..b.len().min(512)]);
        let t = text
            .trim_start_matches('\u{feff}')
            .trim_start()
            .to_ascii_lowercase();
        (t.starts_with("<svg") || (t.starts_with("<?xml") && t.contains("<svg"))).then_some("svg")
    }
}

fn looks_like_html(b: &[u8]) -> bool {
    let text = String::from_utf8_lossy(&b[..b.len().min(256)]).to_ascii_lowercase();
    let t = text.trim_start();
    t.starts_with("<!doctype html")
        || t.starts_with("<html")
        || t.starts_with("<head")
        || t.starts_with("<body")
}

fn ext_for_mime(mime: &str) -> Option<&'static str> {
    Some(match mime {
        "image/jpeg" | "image/jpg" | "image/pjpeg" => "jpg",
        "image/png" | "image/apng" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/bmp" | "image/x-ms-bmp" => "bmp",
        "image/svg+xml" => "svg",
        "image/x-icon" | "image/vnd.microsoft.icon" => "ico",
        "image/avif" => "avif",
        "image/heic" | "image/heif" => "heic",
        "image/tiff" => "tiff",
        _ => return None,
    })
}

const IMAGE_EXTS: &[&str] = &[
    "jpg", "jpeg", "jfif", "png", "gif", "webp", "bmp", "svg", "ico", "avif", "heic", "heif",
    "tif", "tiff",
];

fn same_format(a: &str, b: &str) -> bool {
    let canon = |e: &str| match e.to_ascii_lowercase().as_str() {
        "jpeg" | "jfif" | "jpg" => "jpg".to_owned(),
        "tif" | "tiff" => "tiff".to_owned(),
        "heif" | "heic" => "heic".to_owned(),
        other => other.to_owned(),
    };
    canon(a) == canon(b)
}

// --- File names --------------------------------------------------------------------------------

/// The name to save as. Order: `Content-Disposition`, the requested URL, the URL after
/// redirects, `image`. The extension always matches the real format (sniffed bytes first, then
/// the content type): `photo.php` becomes `photo.jpg`, a `.jpg` that is really a PNG becomes `.png`.
pub fn file_name_for(
    url: &str,
    final_url: &str,
    disposition: Option<&str>,
    mime: Option<&str>,
    sniffed: Option<&str>,
) -> String {
    let has_image_ext = |n: &str| {
        Path::new(n).extension().is_some_and(|e| {
            IMAGE_EXTS.contains(&e.to_string_lossy().to_ascii_lowercase().as_str())
        })
    };
    let candidates = [
        disposition.and_then(disposition_name),
        url_name(url),
        url_name(final_url),
    ];
    let base = candidates
        .iter()
        .flatten()
        .find(|n| has_image_ext(n))
        .or_else(|| candidates.iter().flatten().next())
        .cloned()
        .unwrap_or_else(|| "image".to_owned());

    let real = sniffed.or_else(|| mime.and_then(ext_for_mime));
    let path = Path::new(&base);
    let ext = path.extension().map(|e| e.to_string_lossy().into_owned());
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let name = match (real, ext) {
        (Some(real), Some(e)) if has_image_ext(&base) && same_format(&e, real) => base,
        (Some(real), Some(e)) if has_image_ext(&base) => {
            // Wrong extension for the real format: replace it (keep the stem).
            let _ = e;
            format!("{stem}.{real}")
        }
        (Some(real), Some(_)) => format!("{base}.{real}"), // `photo.php` -> `photo.php.jpg`
        (Some(real), None) => format!("{base}.{real}"),
        (None, _) => base,
    };
    // `photo.php.jpg` is ugly: drop a leading script-like extension.
    let name = strip_script_ext(&name);
    sanitize_file_name(&name).unwrap_or_else(|| "image".to_owned())
}

fn strip_script_ext(name: &str) -> String {
    const SCRIPTS: &[&str] = &[
        "php", "asp", "aspx", "jsp", "cgi", "ashx", "axd", "do", "action", "html", "htm",
    ];
    let p = Path::new(name);
    if let (Some(stem), Some(ext)) = (p.file_stem(), p.extension())
        && IMAGE_EXTS.contains(&ext.to_string_lossy().to_ascii_lowercase().as_str())
    {
        let inner = Path::new(stem);
        if let (Some(s2), Some(e2)) = (inner.file_stem(), inner.extension())
            && SCRIPTS.contains(&e2.to_string_lossy().to_ascii_lowercase().as_str())
        {
            return format!("{}.{}", s2.to_string_lossy(), ext.to_string_lossy());
        }
    }
    name.to_owned()
}

/// Last path segment of a URL, percent-decoded.
fn url_name(url: &str) -> Option<String> {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let path = rest.split(['?', '#']).next()?;
    let path = path.split_once('/').map_or("", |(_, p)| p);
    let seg = path.rsplit('/').find(|s| !s.is_empty())?;
    sanitize_file_name(&percent_decode(seg))
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && let Some(h) = b
                .get(i + 1..i + 3)
                .and_then(|h| std::str::from_utf8(h).ok())
            && let Ok(v) = u8::from_str_radix(h, 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `filename*=UTF-8''na%20me.jpg` (preferred) or `filename="name.jpg"`.
fn disposition_name(header: &str) -> Option<String> {
    let lower = header.to_ascii_lowercase();
    if let Some(at) = lower.find("filename*=") {
        let v = header[at + 10..]
            .split(';')
            .next()?
            .trim()
            .trim_matches('"');
        let v = v.split_once("''").map_or(v, |(_, rest)| rest);
        if let Some(n) = sanitize_file_name(&percent_decode(v)) {
            return Some(n);
        }
    }
    // `filename=`, but not the tail of `filename*=`.
    let mut from = 0;
    while let Some(p) = lower[from..].find("filename=") {
        let at = from + p;
        from = at + 9;
        let v = header[at + 9..].trim_start();
        let v = if let Some(q) = v.strip_prefix('"') {
            q.split('"').next().unwrap_or("")
        } else {
            v.split(';').next().unwrap_or("").trim()
        };
        if let Some(n) = sanitize_file_name(&percent_decode(v)) {
            return Some(n);
        }
    }
    None
}

/// A name Windows accepts, or `None` when nothing usable is left.
pub fn sanitize_file_name(name: &str) -> Option<String> {
    let mut s: String = name
        .chars()
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    s = s.trim().trim_end_matches(['.', ' ']).to_owned();
    if s.chars().all(|c| c == '_' || c == '.') {
        return None;
    }
    if s.chars().count() > 150 {
        let p = Path::new(&s);
        let ext = p
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy()))
            .unwrap_or_default();
        let keep = 150usize.saturating_sub(ext.chars().count());
        let stem: String = p
            .file_stem()
            .map(|x| x.to_string_lossy().chars().take(keep).collect())
            .unwrap_or_default();
        s = format!("{stem}{ext}");
    }
    let stem_upper = s.split('.').next().unwrap_or("").to_ascii_uppercase();
    const RESERVED: &[&str] = &["CON", "PRN", "AUX", "NUL"];
    let numbered = |p: &str| {
        stem_upper
            .strip_prefix(p)
            .is_some_and(|d| d.len() == 1 && d != "0" && d.bytes().all(|b| b.is_ascii_digit()))
    };
    if RESERVED.contains(&stem_upper.as_str()) || numbered("COM") || numbered("LPT") {
        s = format!("_{s}");
    }
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        cell::{Cell, RefCell},
        io::Cursor,
    };

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n0123456789abcdefghij0123456789abcdef";
    const JPG: &[u8] = &[
        0xFF, 0xD8, 0xFF, 0xE0, 0, 16, b'J', b'F', b'I', b'F', 0, 1, 1, 0, 0, 1, 0, 1, 0, 0, 1, 2,
        3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14,
    ];

    #[derive(Default)]
    struct Fake {
        mime: Option<String>,
        disposition: Option<String>,
        body: Vec<u8>,
        claim_len: Option<u64>,
        final_url: Option<String>,
        seen: RefCell<Vec<(String, Option<String>)>>,
    }
    impl Fetch for Fake {
        fn get(&self, url: &str, referer: Option<&str>) -> io::Result<Response> {
            self.seen
                .borrow_mut()
                .push((url.to_owned(), referer.map(str::to_owned)));
            Ok(Response {
                final_url: self.final_url.clone().unwrap_or_else(|| url.to_owned()),
                content_type: self.mime.clone(),
                disposition: self.disposition.clone(),
                length: self.claim_len.or(Some(self.body.len() as u64)),
                body: Box::new(Cursor::new(self.body.clone())),
            })
        }
    }

    #[derive(Default)]
    struct Probe {
        name: RefCell<String>,
        total: Cell<u64>,
        done: Cell<u64>,
        cancel_after: Cell<Option<u64>>,
    }
    impl Sink for Probe {
        fn named(&self, n: &str) {
            *self.name.borrow_mut() = n.to_owned();
        }
        fn total(&self, b: u64) {
            self.total.set(b);
        }
        fn advance(&self, b: u64) {
            self.done.set(self.done.get() + b);
        }
        fn cancelled(&self) -> bool {
            self.cancel_after
                .get()
                .is_some_and(|n| self.done.get() >= n)
        }
    }

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("qf-dl-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn fake(mime: &str, body: &[u8]) -> Fake {
        Fake {
            mime: Some(mime.to_owned()),
            body: body.to_vec(),
            ..Fake::default()
        }
    }

    #[test]
    fn saves_the_image_with_the_name_from_the_url_and_reports_progress() {
        let d = tmp("ok");
        let p = Probe::default();
        let f = fake("image/png", PNG);
        let path = fetch_image(
            &f,
            "https://ex.com/a/cat%20one.png?x=1",
            Some("https://ex.com/page"),
            &d,
            &p,
        )
        .unwrap();
        assert_eq!(path.file_name().unwrap(), "cat one.png");
        assert_eq!(fs::read(&path).unwrap(), PNG);
        assert_eq!(
            (p.total.get(), p.done.get()),
            (PNG.len() as u64, PNG.len() as u64)
        );
        assert_eq!(&*p.name.borrow(), "cat one.png");
        assert_eq!(
            f.seen.borrow()[0].1.as_deref(),
            Some("https://ex.com/page"),
            "the page is sent as Referer"
        );
        let _ = fs::remove_dir_all(d);
    }

    #[test]
    fn never_overwrites() {
        let d = tmp("dup");
        fs::write(d.join("a.png"), b"mine").unwrap();
        let f = fake("image/png", PNG);
        let p1 = fetch_image(&f, "https://ex.com/a.png", None, &d, &Probe::default()).unwrap();
        let p2 = fetch_image(&f, "https://ex.com/a.png", None, &d, &Probe::default()).unwrap();
        assert_eq!(p1.file_name().unwrap(), "a (2).png");
        assert_eq!(p2.file_name().unwrap(), "a (3).png");
        assert_eq!(fs::read(d.join("a.png")).unwrap(), b"mine");
        let _ = fs::remove_dir_all(d);
    }

    #[test]
    fn refuses_web_pages_even_when_they_claim_to_be_images() {
        let d = tmp("html");
        let html = b"<!DOCTYPE html><html><body>hello there, this is a page</body></html>";
        for (mime, what) in [
            ("text/html", "type"),
            ("image/jpeg", "lying type"),
            ("application/octet-stream", "no type"),
        ] {
            let e = fetch_image(
                &fake(mime, html),
                "https://ex.com/x.jpg",
                None,
                &d,
                &Probe::default(),
            )
            .unwrap_err();
            assert!(e.to_string().contains("not an image"), "{what}: {e}");
        }
        assert_eq!(fs::read_dir(&d).unwrap().count(), 0, "nothing was saved");
        let _ = fs::remove_dir_all(d);
    }

    #[test]
    fn octet_stream_is_fine_when_the_bytes_are_an_image() {
        let d = tmp("octet");
        let p = fetch_image(
            &fake("application/octet-stream", JPG),
            "https://cdn.ex.com/get?id=7",
            None,
            &d,
            &Probe::default(),
        )
        .unwrap();
        assert_eq!(p.file_name().unwrap(), "get.jpg", "{p:?}");
        let _ = fs::remove_dir_all(d);
    }

    #[test]
    fn unknown_formats_are_trusted_when_the_server_says_image() {
        let d = tmp("trust");
        let body = vec![7u8; 64];
        let p = fetch_image(
            &fake("image/jxl", &body),
            "https://ex.com/x.jxl",
            None,
            &d,
            &Probe::default(),
        )
        .unwrap();
        assert_eq!(p.file_name().unwrap(), "x.jxl");
        let _ = fs::remove_dir_all(d);
    }

    #[test]
    fn a_truncated_or_cancelled_download_leaves_nothing() {
        let d = tmp("cut");
        let mut f = fake("image/png", PNG);
        f.claim_len = Some(PNG.len() as u64 + 100);
        let e = fetch_image(&f, "https://ex.com/a.png", None, &d, &Probe::default()).unwrap_err();
        assert!(e.to_string().contains("cut short"), "{e}");
        assert_eq!(fs::read_dir(&d).unwrap().count(), 0);

        let mut big = PNG.to_vec();
        big.extend(vec![0u8; 300_000]);
        let p = Probe::default();
        p.cancel_after.set(Some(PNG.len() as u64 + 1));
        let e = fetch_image(
            &fake("image/png", &big),
            "https://ex.com/b.png",
            None,
            &d,
            &p,
        )
        .unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::Interrupted);
        assert_eq!(fs::read_dir(&d).unwrap().count(), 0);
        let _ = fs::remove_dir_all(d);
    }

    #[test]
    fn missing_destination_and_empty_bodies_are_errors() {
        let e = fetch_image(
            &fake("image/png", PNG),
            "https://a/b.png",
            None,
            Path::new(r"Z:\nope\none"),
            &Probe::default(),
        )
        .unwrap_err();
        assert!(e.to_string().contains("destination"));
        let d = tmp("empty");
        let e = fetch_image(
            &fake("image/png", b""),
            "https://a/b.png",
            None,
            &d,
            &Probe::default(),
        )
        .unwrap_err();
        assert!(e.to_string().contains("empty"));
        let _ = fs::remove_dir_all(d);
    }

    #[test]
    fn sniffing() {
        assert_eq!(sniff_image(PNG), Some("png"));
        assert_eq!(sniff_image(JPG), Some("jpg"));
        assert_eq!(sniff_image(b"GIF89a\x01\0\x01\0"), Some("gif"));
        assert_eq!(sniff_image(b"RIFF\x10\0\0\0WEBPVP8 "), Some("webp"));
        assert_eq!(sniff_image(b"\0\0\0\x1cftypavif\0\0\0\0"), Some("avif"));
        assert_eq!(sniff_image(b"\xEF\xBB\xBF  <svg xmlns='x'/>"), Some("svg"));
        assert_eq!(sniff_image(b"<?xml version='1.0'?><svg/>"), Some("svg"));
        assert_eq!(
            sniff_image(b"BM hello this is text"),
            None,
            "BM alone is not a bitmap"
        );
        assert_eq!(sniff_image(b"<html></html>"), None);
        assert_eq!(sniff_image(b""), None);
    }

    #[test]
    fn names() {
        let n = |url: &str, cd: Option<&str>, mime: Option<&str>, sniff: Option<&str>| {
            file_name_for(url, url, cd, mime, sniff)
        };
        assert_eq!(
            n(
                "https://e.com/dir/photo.JPG?w=100",
                None,
                Some("image/jpeg"),
                Some("jpg")
            ),
            "photo.JPG"
        );
        assert_eq!(
            n("https://e.com/photo.jpeg", None, None, Some("jpg")),
            "photo.jpeg",
            "jpeg == jpg"
        );
        assert_eq!(
            n(
                "https://e.com/photo.jpg",
                None,
                Some("image/png"),
                Some("png")
            ),
            "photo.png",
            "wrong extension is fixed"
        );
        assert_eq!(
            n(
                "https://e.com/render.php?id=3",
                None,
                Some("image/webp"),
                Some("webp")
            ),
            "render.webp"
        );
        assert_eq!(
            n("https://e.com/", None, Some("image/gif"), None),
            "image.gif"
        );
        assert_eq!(
            n(
                "https://e.com/x",
                Some("attachment; filename=\"Sunset 1.png\""),
                None,
                Some("png")
            ),
            "Sunset 1.png"
        );
        assert_eq!(
            n(
                "https://e.com/x",
                Some("inline; filename*=UTF-8''caf%C3%A9.png; filename=\"cafe.png\""),
                None,
                Some("png")
            ),
            "café.png"
        );
        assert_eq!(
            n("https://e.com/%E7%8C%AB.png", None, None, Some("png")),
            "猫.png"
        );
        assert_eq!(
            n("https://e.com/CON.png", None, None, Some("png")),
            "_CON.png"
        );
        assert_eq!(
            n("https://e.com/a%2Fb%3Fc.png", None, None, Some("png")),
            "a_b_c.png"
        );
        assert_eq!(
            n("https://e.com/pic", None, Some("image/svg+xml"), None),
            "pic.svg"
        );
    }

    #[test]
    fn sanitizing() {
        assert_eq!(
            sanitize_file_name("a<b>c.png").as_deref(),
            Some("a_b_c.png")
        );
        assert_eq!(sanitize_file_name("  name. ").as_deref(), Some("name"));
        assert_eq!(sanitize_file_name("..."), None);
        assert_eq!(sanitize_file_name("LPT1").as_deref(), Some("_LPT1"));
        assert_eq!(sanitize_file_name("COM0").as_deref(), Some("COM0"));
        let long = format!("{}.png", "a".repeat(400));
        let s = sanitize_file_name(&long).unwrap();
        assert!(s.chars().count() <= 150 && s.ends_with(".png"));
    }
}
