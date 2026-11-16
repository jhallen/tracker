//! Multipart form parsing and photo storage.

use std::{collections::HashMap, path::Path};

use axum::extract::Multipart;
use chrono::Utc;

use crate::{error::AppError, models::NewAttachment};

/// Field name used by the forms for photo inputs.
pub const PHOTO_FIELD: &str = "photos";

pub struct UploadedFile {
    pub original_name: String,
    pub content_type: &'static str,
    pub data: Vec<u8>,
}

/// A parsed multipart form: text fields plus any photos.
pub struct ParsedForm {
    pub fields: HashMap<String, String>,
    pub photos: Vec<UploadedFile>,
}

impl ParsedForm {
    pub fn get(&self, k: &str) -> Option<&String> {
        self.fields.get(k)
    }
}

/// Recognise the image formats phones and browsers produce by their magic
/// bytes; the client-supplied content type is not trusted.
pub fn sniff_image(data: &[u8]) -> Option<(&'static str, &'static str)> {
    if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(("image/jpeg", "jpg"))
    } else if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(("image/png", "png"))
    } else if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        Some(("image/gif", "gif"))
    } else if data.len() > 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        Some(("image/webp", "webp"))
    } else if data.len() > 12
        && &data[4..8] == b"ftyp"
        && matches!(&data[8..12], b"heic" | b"heix" | b"mif1" | b"msf1" | b"heim" | b"heis")
    {
        Some(("image/heic", "heic"))
    } else {
        None
    }
}

pub async fn parse_multipart(
    mut multipart: Multipart,
    max_file_bytes: usize,
    max_files: usize,
) -> Result<ParsedForm, AppError> {
    let mut fields = HashMap::new();
    let mut photos = Vec::new();
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(format!("Upload failed: {e}")))?
    {
        let name = field.name().unwrap_or("").to_string();
        if name == PHOTO_FIELD {
            let original_name = field.file_name().unwrap_or("photo").to_string();
            let data = field
                .bytes()
                .await
                .map_err(|e| AppError::BadRequest(format!("Upload failed: {e}")))?;
            if data.is_empty() {
                continue; // empty <input type=file>
            }
            if data.len() > max_file_bytes {
                return Err(AppError::BadRequest(format!(
                    "\"{original_name}\" is larger than the {} MB limit.",
                    max_file_bytes / 1024 / 1024
                )));
            }
            let Some((content_type, _)) = sniff_image(&data) else {
                return Err(AppError::BadRequest(format!(
                    "\"{original_name}\" is not a supported image (JPEG, PNG, GIF, WebP, HEIC)."
                )));
            };
            if photos.len() >= max_files {
                return Err(AppError::BadRequest(format!(
                    "At most {max_files} photos can be attached at once."
                )));
            }
            photos.push(UploadedFile {
                original_name: sanitize_filename(&original_name),
                content_type,
                data: data.to_vec(),
            });
        } else {
            let text = field
                .text()
                .await
                .map_err(|e| AppError::BadRequest(format!("Bad form field: {e}")))?;
            fields.insert(name, text);
        }
    }
    Ok(ParsedForm { fields, photos })
}

fn sanitize_filename(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or("photo");
    let cleaned: String = base
        .chars()
        .filter(|c| !c.is_control())
        .take(120)
        .collect();
    if cleaned.trim().is_empty() {
        "photo".into()
    } else {
        cleaned
    }
}

/// Write photos to `dir` and return attachment rows ready to insert
/// (with `issue_id`/`comment_id` filled in by the caller).
pub async fn store_photos(
    dir: &Path,
    photos: Vec<UploadedFile>,
    uploader_id: i32,
) -> Result<Vec<NewAttachment>, AppError> {
    let mut rows = Vec::new();
    for p in photos {
        let ext = sniff_image(&p.data).map(|(_, e)| e).unwrap_or("bin");
        let stored_name = format!("{}.{ext}", uuid::Uuid::new_v4());
        tokio::fs::write(dir.join(&stored_name), &p.data).await?;
        rows.push(NewAttachment {
            issue_id: 0,
            comment_id: None,
            uploader_id,
            original_name: p.original_name,
            stored_name,
            content_type: p.content_type.to_string(),
            size_bytes: p.data.len() as i32,
            created_at: Utc::now().naive_utc(),
        });
    }
    Ok(rows)
}

/// Best-effort cleanup when the database insert after `store_photos` fails.
pub async fn discard(dir: &Path, rows: &[NewAttachment]) {
    for r in rows {
        let _ = tokio::fs::remove_file(dir.join(&r.stored_name)).await;
    }
}
