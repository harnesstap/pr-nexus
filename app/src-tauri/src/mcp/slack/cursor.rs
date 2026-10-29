//! The page cursor of a Slack tool: every workspace's upstream cursor in
//! one opaque, signed string, so an agent cannot point a cursor at a
//! workspace or tool it was not issued for.

use std::collections::BTreeMap;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use crate::error::AppError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Composite {
    /// Tool the cursor pages.
    pub tool: String,
    /// Upstream cursor per workspace qualifier; `None` once it ran out.
    pub pages: BTreeMap<String, Option<String>>,
}

impl Composite {
    /// Whether any workspace has another page.
    pub fn has_more(&self) -> bool {
        self.pages.values().any(Option::is_some)
    }
}

fn mac(key: &[u8], body: &str) -> Hmac<Sha256> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("any key length");
    mac.update(body.as_bytes());
    mac
}

pub fn encode(key: &[u8], cursor: &Composite) -> String {
    let body = URL_SAFE_NO_PAD.encode(serde_json::to_vec(cursor).expect("cursor serializes"));
    let tag = URL_SAFE_NO_PAD.encode(mac(key, &body).finalize().into_bytes());
    format!("{body}.{tag}")
}

/// The cursor, when it was signed by this app run for `tool`.
pub fn decode(key: &[u8], tool: &str, text: &str) -> Result<Composite, AppError> {
    let invalid = || {
        AppError::new(
            "validation",
            "Invalid arguments: cursor is not one this tool returned (or the app restarted). Start again without a cursor.",
        )
    };
    let (body, tag) = text.split_once('.').ok_or_else(invalid)?;
    let tag = URL_SAFE_NO_PAD.decode(tag).map_err(|_| invalid())?;
    mac(key, body).verify_slice(&tag).map_err(|_| invalid())?;
    let bytes = URL_SAFE_NO_PAD.decode(body).map_err(|_| invalid())?;
    let cursor: Composite = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
    if cursor.tool != tool {
        return Err(invalid());
    }
    Ok(cursor)
}
