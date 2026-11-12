//! Understanding what a drag from another program carries (pure parsing, no OS calls).
//!
//! Explorer drags files (`CF_HDROP`). A browser dragging an image offers the image URL in
//! several shapes, depending on the browser: `HTML Format` (Chrome, Edge: an HTML fragment with
//! the `<img>` tag and the page's `SourceURL`), `text/html` (Firefox), `UniformResourceLocatorW`,
//! `text/x-moz-url`, `CF_UNICODETEXT`. `ole_drop.rs` reads those formats from the OLE data
//! object; [`classify`] decides what to do with them.

use std::path::PathBuf;

/// An image to download.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageSource {
    pub url: String,
    /// The page the image was dragged from; many sites refuse hot-linking without it.
    pub referer: Option<String>,
}

/// What a drop means.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Dropped {
    Files(Vec<PathBuf>),
    Image(ImageSource),
    Unsupported(String),
}

/// The decoded clipboard formats of one drag; every field is optional.
#[derive(Clone, Debug, Default)]
pub struct Formats {
    pub files: Vec<PathBuf>,
    /// `HTML Format` (UTF-8 with a header) or Firefox's `text/html` (UTF-16, no header).
    pub html: Option<Html>,
    /// `UniformResourceLocatorW` / `UniformResourceLocator`.
    pub url: Option<String>,
    /// `text/x-moz-url`: "url\ntitle".
    pub moz_url: Option<String>,
    /// `application/x-moz-file-promise-url`: Firefox's address of the dragged image.
    pub moz_image_url: Option<String>,
    /// `CF_UNICODETEXT`.
    pub text: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Html {
    pub source_url: Option<String>,
    pub fragment: String,
}

/// Decides what the drop is: files win; otherwise the best image address.
pub fn classify(f: &Formats) -> Dropped {
    if !f.files.is_empty() {
        return Dropped::Files(f.files.clone());
    }
    let page = f.html.as_ref().and_then(|h| h.source_url.clone());
    let from_html = f
        .html
        .as_ref()
        .and_then(|h| html_img_src(&h.fragment))
        .and_then(|src| resolve_url(page.as_deref(), &src));
    let first_line = |s: &Option<String>| -> Option<String> {
        let line = s
            .as_deref()?
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty())?;
        let line = line.trim_start_matches('<').trim_end_matches('>');
        is_http_url(line).then(|| line.to_owned())
    };
    let url = from_html
        .or_else(|| first_line(&f.moz_image_url))
        .or_else(|| first_line(&f.url))
        .or_else(|| first_line(&f.moz_url))
        .or_else(|| first_line(&f.text));
    match url {
        Some(url) => Dropped::Image(ImageSource { url, referer: page }),
        None => Dropped::Unsupported("nothing to copy or download in this drop".to_owned()),
    }
}

pub fn is_http_url(s: &str) -> bool {
    let l = s.to_ascii_lowercase();
    (l.starts_with("http://") || l.starts_with("https://")) && !s.contains(char::is_whitespace)
}

/// Files of a `DROPFILES` block (the payload of `CF_HDROP`).
pub fn parse_dropfiles(bytes: &[u8]) -> Vec<PathBuf> {
    let le32 = |o: usize| -> Option<usize> {
        Some(u32::from_le_bytes(bytes.get(o..o + 4)?.try_into().ok()?) as usize)
    };
    let (Some(start), Some(wide)) = (le32(0), le32(16)) else {
        return Vec::new();
    };
    let Some(list) = bytes.get(start..) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if wide != 0 {
        let units: Vec<u16> = list
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        for name in units.split(|&u| u == 0) {
            if name.is_empty() {
                break; // the list ends with an empty string
            }
            out.push(PathBuf::from(String::from_utf16_lossy(name)));
        }
    } else {
        for name in list.split(|&b| b == 0) {
            if name.is_empty() {
                break;
            }
            out.push(PathBuf::from(String::from_utf8_lossy(name).into_owned()));
        }
    }
    out
}

/// UTF-16LE text up to the first NUL (a leading BOM is dropped).
pub fn utf16_text(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .take_while(|&u| u != 0)
        .collect();
    let s = String::from_utf16_lossy(&units);
    s.strip_prefix('\u{feff}').unwrap_or(&s).to_owned()
}

/// ANSI / UTF-8 text up to the first NUL.
pub fn ansi_text(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

/// The `HTML Format` clipboard format: a text header (`SourceURL:`, `StartFragment:` and
/// `EndFragment:` byte offsets) followed by the document.
pub fn parse_cf_html(bytes: &[u8]) -> Html {
    let text = ansi_text(bytes);
    let mut source_url = None;
    let (mut start, mut end) = (None, None);
    for line in text.lines().take(16) {
        if let Some(v) = line.strip_prefix("SourceURL:") {
            source_url = Some(v.trim().to_owned()).filter(|v| is_http_url(v));
        } else if let Some(v) = line.strip_prefix("StartFragment:") {
            start = v.trim().parse::<usize>().ok();
        } else if let Some(v) = line.strip_prefix("EndFragment:") {
            end = v.trim().parse::<usize>().ok();
        }
    }
    let raw = text.as_bytes();
    let fragment = match (start, end) {
        (Some(s), Some(e)) if s <= e && e <= raw.len() => {
            String::from_utf8_lossy(&raw[s..e]).into_owned()
        }
        _ => text,
    };
    Html {
        source_url,
        fragment,
    }
}

/// `src` of the first `<img>` in an HTML fragment (entities decoded).
pub fn html_img_src(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let mut from = 0;
    while let Some(at) = lower[from..].find("<img") {
        let tag_start = from + at + 4;
        // `<img` must be followed by whitespace or `/`, not be e.g. `<imgx`.
        let next = lower.as_bytes().get(tag_start).copied().unwrap_or(b'>');
        if next.is_ascii_whitespace() || next == b'/' {
            let tag_end = lower[tag_start..]
                .find('>')
                .map_or(lower.len(), |e| tag_start + e);
            if let Some(v) = attribute(&html[tag_start..tag_end], "src") {
                let v = decode_entities(v.trim());
                if !v.is_empty() {
                    return Some(v);
                }
            }
        }
        from = tag_start;
    }
    None
}

/// Value of attribute `name` in the inside of a tag (quoted or bare).
fn attribute(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let b = tag.as_bytes();
    let mut i = 0;
    while let Some(p) = lower[i..].find(name) {
        let at = i + p;
        let before_ok =
            at == 0 || b[at - 1].is_ascii_whitespace() || b[at - 1] == b'"' || b[at - 1] == b'\'';
        let mut j = at + name.len();
        while j < b.len() && b[j].is_ascii_whitespace() {
            j += 1;
        }
        if before_ok && j < b.len() && b[j] == b'=' {
            j += 1;
            while j < b.len() && b[j].is_ascii_whitespace() {
                j += 1;
            }
            let rest = &tag[j..];
            return match rest.chars().next()? {
                q @ ('"' | '\'') => {
                    let inner = &rest[1..];
                    Some(inner[..inner.find(q).unwrap_or(inner.len())].to_owned())
                }
                _ => {
                    let end = rest
                        .find(|c: char| c.is_whitespace() || c == '>')
                        .unwrap_or(rest.len());
                    Some(rest[..end].to_owned())
                }
            };
        }
        i = at + name.len();
    }
    None
}

fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        let Some(semi) = rest.find(';').filter(|&e| e <= 10) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let ent = &rest[1..semi];
        let ch = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => ent
                .strip_prefix("#x")
                .or_else(|| ent.strip_prefix("#X"))
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| ent.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &rest[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Makes `href` (as found in a page at `base`) absolute. Only http(s) results are returned:
/// `data:`, `blob:` and the like cannot be downloaded.
pub fn resolve_url(base: Option<&str>, href: &str) -> Option<String> {
    let href = href.trim();
    if is_http_url(href) {
        return Some(href.to_owned());
    }
    let base = base.filter(|b| is_http_url(b))?;
    let scheme_end = base.find("://")? + 3;
    let origin_end = base[scheme_end..]
        .find(['/', '?', '#'])
        .map_or(base.len(), |e| scheme_end + e);
    let origin = &base[..origin_end];
    let out = if let Some(rest) = href.strip_prefix("//") {
        format!("{}//{rest}", &base[..scheme_end - 2])
    } else if href.starts_with('/') {
        format!("{origin}{href}")
    } else if href.contains(':') && !href.starts_with("./") {
        return None; // another scheme (data:, blob:, javascript:)
    } else {
        let path = &base[origin_end..];
        let path = path.split(['?', '#']).next().unwrap_or("");
        let dir = &path[..path.rfind('/').map_or(0, |i| i + 1)];
        let dir = if dir.is_empty() { "/" } else { dir };
        format!("{origin}{dir}{}", href.trim_start_matches("./"))
    };
    is_http_url(&out).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16(s: &str) -> Vec<u8> {
        s.encode_utf16()
            .chain([0])
            .flat_map(u16::to_le_bytes)
            .collect()
    }

    fn dropfiles(names: &[&str], wide: bool) -> Vec<u8> {
        let mut b = vec![0u8; 20];
        b[0] = 20;
        b[16] = u8::from(wide);
        for n in names {
            if wide {
                b.extend(n.encode_utf16().chain([0]).flat_map(u16::to_le_bytes));
            } else {
                b.extend(n.bytes().chain([0]));
            }
        }
        b.extend(if wide { vec![0, 0] } else { vec![0] });
        b
    }

    #[test]
    fn reads_a_hdrop_list_wide_and_ansi() {
        for wide in [true, false] {
            let got = parse_dropfiles(&dropfiles(&[r"C:\a b\c.txt", r"D:\dir"], wide));
            assert_eq!(
                got,
                [PathBuf::from(r"C:\a b\c.txt"), PathBuf::from(r"D:\dir")]
            );
        }
        assert!(parse_dropfiles(&[1, 2, 3]).is_empty());
        assert!(parse_dropfiles(&dropfiles(&[], true)).is_empty());
    }

    #[test]
    fn text_decoding() {
        assert_eq!(utf16_text(&utf16("héllo")), "héllo");
        assert_eq!(utf16_text(&[0xff, 0xfe, b'a', 0, 0, 0, b'z', 0]), "a");
        assert_eq!(ansi_text(b"abc\0def"), "abc");
    }

    const CHROME: &str = "Version:0.9\r\nStartHTML:00000097\r\nEndHTML:00000300\r\n\
        StartFragment:00000131\r\nEndFragment:00000250\r\n\
        SourceURL:https://example.com/gallery/page.html\r\n\
        <html>\r\n<body>\r\n<!--StartFragment--><img src=\"/img/cat%20one.jpg?x=1&amp;y=2\" alt=\"cat\"><!--EndFragment-->\r\n</body>\r\n</html>";

    #[test]
    fn chrome_html_format_gives_the_page_and_the_image() {
        // Offsets in real data are byte offsets; recompute them for the sample.
        let start = CHROME.find("<!--StartFragment-->").unwrap();
        let end = CHROME.find("<!--EndFragment-->").unwrap() + "<!--EndFragment-->".len();
        let text = CHROME
            .replace("00000131", &format!("{start:08}"))
            .replace("00000250", &format!("{end:08}"));
        let h = parse_cf_html(text.as_bytes());
        assert_eq!(
            h.source_url.as_deref(),
            Some("https://example.com/gallery/page.html")
        );
        assert!(h.fragment.starts_with("<!--StartFragment-->"));
        let src = html_img_src(&h.fragment).unwrap();
        assert_eq!(src, "/img/cat%20one.jpg?x=1&y=2");
        assert_eq!(
            resolve_url(h.source_url.as_deref(), &src).as_deref(),
            Some("https://example.com/img/cat%20one.jpg?x=1&y=2")
        );
    }

    #[test]
    fn img_tag_variants() {
        assert_eq!(html_img_src("<IMG SRC='a.png'>").as_deref(), Some("a.png"));
        assert_eq!(
            html_img_src("<img alt=x src=a.png width=3>").as_deref(),
            Some("a.png")
        );
        assert_eq!(
            html_img_src("<a href='/page'><img\n  class=\"c\"\n  src = \"b.gif\" /></a>")
                .as_deref(),
            Some("b.gif")
        );
        assert_eq!(html_img_src("<imgx src='no'>"), None);
        assert_eq!(
            html_img_src("<img data-src='lazy.png' src='real.png'>").as_deref(),
            Some("real.png")
        );
        assert_eq!(html_img_src("<p>no image</p>"), None);
        assert_eq!(html_img_src("<img alt=''>"), None);
    }

    #[test]
    fn resolving_urls() {
        let base = Some("https://ex.com/a/b/page.html?q=1#top");
        assert_eq!(
            resolve_url(base, "http://o.com/x.png").as_deref(),
            Some("http://o.com/x.png")
        );
        assert_eq!(
            resolve_url(base, "//cdn.com/x.png").as_deref(),
            Some("https://cdn.com/x.png")
        );
        assert_eq!(
            resolve_url(base, "/x.png").as_deref(),
            Some("https://ex.com/x.png")
        );
        assert_eq!(
            resolve_url(base, "x.png").as_deref(),
            Some("https://ex.com/a/b/x.png")
        );
        assert_eq!(
            resolve_url(base, "./x.png").as_deref(),
            Some("https://ex.com/a/b/x.png")
        );
        assert_eq!(resolve_url(base, "data:image/png;base64,AAAA"), None);
        assert_eq!(resolve_url(base, "blob:https://ex.com/uuid"), None);
        assert_eq!(resolve_url(None, "x.png"), None);
        assert_eq!(
            resolve_url(Some("https://ex.com"), "x.png").as_deref(),
            Some("https://ex.com/x.png")
        );
    }

    #[test]
    fn entities() {
        assert_eq!(
            decode_entities("a&amp;b&lt;&#65;&#x42;&bogus;&"),
            "a&b<AB&bogus;&"
        );
    }

    #[test]
    fn files_win_over_everything() {
        let f = Formats {
            files: vec![PathBuf::from(r"C:\x.png")],
            url: Some("https://a.com/x.png".into()),
            ..Formats::default()
        };
        assert_eq!(
            classify(&f),
            Dropped::Files(vec![PathBuf::from(r"C:\x.png")])
        );
    }

    #[test]
    fn the_image_beats_the_link_around_it() {
        // Dragging a linked image: the URL formats carry the *page*, the HTML has the image.
        let f = Formats {
            url: Some("https://ex.com/article".into()),
            html: Some(Html {
                source_url: Some("https://ex.com/list".into()),
                fragment: "<a href=\"/article\"><img src=\"https://cdn.ex.com/t.jpg\"></a>".into(),
            }),
            ..Formats::default()
        };
        assert_eq!(
            classify(&f),
            Dropped::Image(ImageSource {
                url: "https://cdn.ex.com/t.jpg".into(),
                referer: Some("https://ex.com/list".into())
            })
        );
    }

    #[test]
    fn falls_back_to_url_formats_and_rejects_the_rest() {
        let f = Formats {
            moz_url: Some("https://a.com/p.webp\nTitle".into()),
            ..Formats::default()
        };
        assert!(
            matches!(classify(&f), Dropped::Image(i) if i.url == "https://a.com/p.webp" && i.referer.is_none())
        );
        let f = Formats {
            text: Some("  <https://a.com/q.png>\n".into()),
            ..Formats::default()
        };
        assert!(matches!(classify(&f), Dropped::Image(i) if i.url == "https://a.com/q.png"));
        let f = Formats {
            text: Some("just some words".into()),
            ..Formats::default()
        };
        assert!(matches!(classify(&f), Dropped::Unsupported(_)));
        let f = Formats {
            html: Some(Html {
                source_url: Some("https://ex.com/".into()),
                fragment: "<img src='data:image/png;base64,AA'>".into(),
            }),
            ..Formats::default()
        };
        assert!(
            matches!(classify(&f), Dropped::Unsupported(_)),
            "data: URLs are not downloadable"
        );
    }
}
