//! Pictures in tool replies. Slack's `slack_read_file` answers a picture
//! with an MCP `image` block; agents get it as one too, after a check.
//!
//! A tool handler returns JSON only, so the checked images ride in that JSON
//! under [`IMAGES_KEY`]. [`take`] lifts them out in the server before the
//! call is audited, so the audit log never holds image data, and they go
//! out as `image` content blocks next to the structured result.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use pr_nexus_providers::slack_mcp::schema::ToolImage;
use rmcp::model::ContentBlock;
use serde_json::{Value, json};

/// Where a handler's images wait for the server. Never in a structured result.
pub const IMAGES_KEY: &str = "_images";
/// Largest picture passed on, decoded.
pub const MAX_IMAGE_BYTES: usize = 5 * 1024 * 1024;
/// Largest total of pictures in one call, decoded.
pub const MAX_CALL_BYTES: usize = 15 * 1024 * 1024;

/// Image types agents' clients read, with the bytes each file starts with.
const KINDS: &[(&str, &[u8])] = &[
    ("image/png", b"\x89PNG\r\n\x1a\n"),
    ("image/jpeg", b"\xff\xd8\xff"),
    ("image/gif", b"GIF8"),
    ("image/webp", b"RIFF"),
];

/// The pictures one call passes on, within [`MAX_CALL_BYTES`].
#[derive(Debug, Default)]
pub struct Collected {
    images: Vec<ToolImage>,
    bytes: usize,
}

impl Collected {
    /// Check `image` and keep it. `Ok` is what the agent reads about it
    /// (`index` of its image block, type, size); `Err` says why it was left
    /// out (with the type Slack claimed).
    pub fn admit(&mut self, image: &ToolImage) -> Result<Value, Value> {
        let skipped = |reason: &str| json!({ "mime_type": image.mime_type, "reason": reason });
        let mime = image.mime_type.trim().to_ascii_lowercase();
        let Some((_, magic)) = KINDS.iter().find(|(kind, _)| *kind == mime) else {
            return Err(skipped("unsupported_type"));
        };
        // Base64 grows data by a third: refuse before decoding a huge string.
        if image.data.len() > MAX_IMAGE_BYTES / 3 * 4 + 4 {
            return Err(skipped("too_large"));
        }
        let compact: String = image
            .data
            .chars()
            .filter(|c| !c.is_ascii_whitespace())
            .collect();
        let Ok(bytes) = STANDARD.decode(&compact) else {
            return Err(skipped("not_base64"));
        };
        let webp_ok = mime != "image/webp" || bytes.get(8..12) == Some(b"WEBP".as_slice());
        if !bytes.starts_with(magic) || !webp_ok {
            return Err(skipped("not_an_image"));
        }
        if bytes.len() > MAX_IMAGE_BYTES {
            return Err(skipped("too_large"));
        }
        if self.bytes + bytes.len() > MAX_CALL_BYTES {
            return Err(skipped("call_limit"));
        }
        self.bytes += bytes.len();
        self.images.push(ToolImage {
            data: compact,
            mime_type: mime.clone(),
        });
        Ok(json!({
            "index": self.images.len() - 1,
            "mime_type": mime,
            "bytes": bytes.len(),
        }))
    }

    /// Put the kept pictures in `value` for [`take`]. Nothing when none.
    pub fn attach(self, value: &mut Value) {
        if self.images.is_empty() {
            return;
        }
        if let Value::Object(map) = value {
            let images = self
                .images
                .into_iter()
                .map(|image| json!({ "data": image.data, "mime_type": image.mime_type }))
                .collect();
            map.insert(IMAGES_KEY.into(), Value::Array(images));
        }
    }
}

/// Remove the pictures [`Collected::attach`] put in `value`, as `image`
/// content blocks in their `index` order.
pub fn take(value: &mut Value) -> Vec<ContentBlock> {
    let Some(Value::Array(images)) = value.as_object_mut().and_then(|map| map.remove(IMAGES_KEY))
    else {
        return Vec::new();
    };
    images
        .into_iter()
        .filter_map(|image| {
            Some(ContentBlock::image(
                image["data"].as_str()?.to_owned(),
                image["mime_type"].as_str()?.to_owned(),
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";

    fn image(mime: &str, bytes: &[u8]) -> ToolImage {
        ToolImage {
            data: STANDARD.encode(bytes),
            mime_type: mime.into(),
        }
    }

    #[test]
    fn a_real_picture_is_kept_and_lifted_out_whole() {
        let mut collected = Collected::default();
        let kept = collected.admit(&image("image/png", PNG)).unwrap();
        assert_eq!(
            kept,
            json!({ "index": 0, "mime_type": "image/png", "bytes": PNG.len() })
        );
        let mut value = json!({ "results": [] });
        collected.attach(&mut value);
        let blocks = take(&mut value);
        assert_eq!(
            value,
            json!({ "results": [] }),
            "no image data stays behind"
        );
        assert_eq!(blocks.len(), 1);
        let block = blocks[0].as_image().unwrap();
        assert_eq!(block.mime_type, "image/png");
        assert_eq!(STANDARD.decode(&block.data).unwrap(), PNG);
    }

    #[test]
    fn wrong_types_bytes_and_sizes_are_left_out() {
        let mut collected = Collected::default();
        let reason = |result: Result<Value, Value>| result.unwrap_err()["reason"].clone();
        assert_eq!(
            reason(collected.admit(&image("image/svg+xml", b"<svg/>"))),
            "unsupported_type"
        );
        assert_eq!(
            reason(collected.admit(&image("application/pdf", b"%PDF"))),
            "unsupported_type"
        );
        assert_eq!(
            reason(collected.admit(&image("image/png", b"<html>"))),
            "not_an_image"
        );
        assert_eq!(
            reason(collected.admit(&image("image/webp", b"RIFF0000WAVE"))),
            "not_an_image"
        );
        let bad = ToolImage {
            data: "%%%".into(),
            mime_type: "image/png".into(),
        };
        assert_eq!(reason(collected.admit(&bad)), "not_base64");
        let mut big = PNG.to_vec();
        big.resize(MAX_IMAGE_BYTES + 1, 0);
        assert_eq!(
            reason(collected.admit(&image("image/png", &big))),
            "too_large"
        );
        let mut value = json!({});
        collected.attach(&mut value);
        assert_eq!(value, json!({}));
    }

    #[test]
    fn one_call_carries_at_most_its_byte_budget() {
        let mut collected = Collected::default();
        let mut four_mb = PNG.to_vec();
        four_mb.resize(4 * 1024 * 1024, 0);
        for _ in 0..3 {
            collected.admit(&image("image/png", &four_mb)).unwrap();
        }
        let over = collected.admit(&image("image/png", &four_mb)).unwrap_err();
        assert_eq!(over["reason"], "call_limit");
        let small = collected.admit(&image("IMAGE/PNG", PNG)).unwrap();
        assert_eq!(small["index"], 3);
    }

    #[test]
    fn take_without_images_changes_nothing() {
        let mut value = json!({ "a": 1 });
        assert!(take(&mut value).is_empty());
        assert_eq!(value, json!({ "a": 1 }));
    }
}
