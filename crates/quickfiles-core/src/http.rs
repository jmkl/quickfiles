//! The real HTTP client behind [`crate::download::Fetch`] (blocking `ureq`, rustls; follows
//! redirects, so http and https images both work).

use std::{io, time::Duration};

use crate::download::{Fetch, Response};

pub struct Http {
    agent: ureq::Agent,
}

impl Http {
    pub fn new() -> Self {
        Self {
            agent: ureq::AgentBuilder::new()
                .timeout_connect(Duration::from_secs(15))
                .timeout_read(Duration::from_secs(30))
                .redirects(8)
                // Some CDNs answer unknown clients with a placeholder or a 403.
                .user_agent(
                    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) \
                     Chrome/124.0 Safari/537.36",
                )
                .build(),
        }
    }
}

impl Default for Http {
    fn default() -> Self {
        Self::new()
    }
}

impl Fetch for Http {
    fn get(&self, url: &str, referer: Option<&str>) -> io::Result<Response> {
        let mut req = self.agent.get(url).set(
            "Accept",
            "image/avif,image/webp,image/apng,image/svg+xml,image/*,*/*;q=0.8",
        );
        if let Some(r) = referer {
            req = req.set("Referer", r);
        }
        let resp = match req.call() {
            Ok(r) => r,
            Err(ureq::Error::Status(code, _)) => {
                return Err(io::Error::other(format!("the server answered HTTP {code}")));
            }
            Err(e) => return Err(io::Error::other(format!("could not download: {e}"))),
        };
        Ok(Response {
            final_url: resp.get_url().to_owned(),
            content_type: Some(resp.content_type().to_ascii_lowercase()).filter(|c| !c.is_empty()),
            disposition: resp.header("content-disposition").map(str::to_owned),
            length: resp
                .header("content-length")
                .and_then(|v| v.trim().parse().ok()),
            body: Box::new(resp.into_reader()),
        })
    }
}
