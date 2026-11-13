use std::collections::HashMap;
use std::io::Cursor;
use std::time::{Duration, Instant};

use brain_contracts::image::{
    ImageMediaType, ImageOrigin, QueryImage, MAX_IMAGE_BYTES, MAX_QUERY_IMAGES,
    MAX_TOTAL_IMAGE_BYTES,
};
use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, ImageFormat, ImageReader, Limits};
use reqwest::{Client, Url};

use crate::BrainError;

pub const IMAGE_ONLY_QUESTION: &str = "Was sagst du zu meinem Bild?";
const MAX_DECODE_EDGE: u32 = 12_000;
const MAX_DECODE_ALLOC: u64 = 512 * 1024 * 1024;
const MIN_EDGE: u32 = 384;
pub const IMAGE_COUNT_RANGE: std::ops::RangeInclusive<usize> = 1..=MAX_QUERY_IMAGES;
pub const IMAGE_DOWNLOAD_BYTES_RANGE: std::ops::RangeInclusive<u64> = 1..=MAX_DECODE_ALLOC;
pub const IMAGE_EDGE_RANGE: std::ops::RangeInclusive<u32> = MIN_EDGE..=MAX_DECODE_EDGE;
pub const IMAGE_JPEG_QUALITY_RANGE: std::ops::RangeInclusive<u8> = 40..=95;
const MAX_REMEMBERED: usize = 512;
const MAX_REMEMBERED_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Default, PartialEq, Eq)]
pub struct AttachedImages {
    channel_id: u64,
    urls: Vec<String>,
}

impl AttachedImages {
    pub fn new(channel_id: u64, urls: impl IntoIterator<Item = String>) -> Self {
        Self {
            channel_id,
            urls: urls.into_iter().collect(),
        }
    }

    pub fn transient(urls: impl IntoIterator<Item = String>) -> Self {
        Self::new(0, urls)
    }

    pub fn is_empty(&self) -> bool {
        self.urls.is_empty()
    }

    pub fn len(&self) -> usize {
        self.urls.len()
    }
}

impl std::fmt::Debug for AttachedImages {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AttachedImages")
            .field("count", &self.urls.len())
            .finish()
    }
}

pub fn question_or_image_default(question: &str, has_images: bool) -> Option<String> {
    let question = question.trim();
    if !question.is_empty() {
        Some(question.to_owned())
    } else if !has_images {
        None
    } else {
        Some(IMAGE_ONLY_QUESTION.to_owned())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageLimits {
    pub max_images: usize,
    pub max_download_bytes: u64,
    pub max_edge: u32,
    pub jpeg_quality: u8,
    pub followup_ttl: Duration,
    pub http: dl_bridges::http::HttpLimits,
}

impl Default for ImageLimits {
    fn default() -> Self {
        Self {
            max_images: MAX_QUERY_IMAGES,
            max_download_bytes: 10 * 1024 * 1024,
            max_edge: 1568,
            jpeg_quality: 85,
            followup_ttl: Duration::from_secs(10 * 60),
            http: dl_bridges::http::HttpLimits::default(),
        }
    }
}

impl ImageLimits {
    fn bounded(self) -> Self {
        Self {
            max_images: self
                .max_images
                .clamp(*IMAGE_COUNT_RANGE.start(), *IMAGE_COUNT_RANGE.end()),
            max_download_bytes: self.max_download_bytes.clamp(
                *IMAGE_DOWNLOAD_BYTES_RANGE.start(),
                *IMAGE_DOWNLOAD_BYTES_RANGE.end(),
            ),
            max_edge: self
                .max_edge
                .clamp(*IMAGE_EDGE_RANGE.start(), *IMAGE_EDGE_RANGE.end()),
            jpeg_quality: self.jpeg_quality.clamp(
                *IMAGE_JPEG_QUALITY_RANGE.start(),
                *IMAGE_JPEG_QUALITY_RANGE.end(),
            ),
            followup_ttl: self.followup_ttl,
            http: self.http,
        }
    }
}

type RecentImages = HashMap<(u64, u64), (Instant, Vec<QueryImage>)>;

pub(crate) struct ImageFetcher {
    client: Client,
    limits: ImageLimits,
    recent: std::sync::Mutex<RecentImages>,
}

impl ImageFetcher {
    pub(crate) fn new(limits: ImageLimits) -> Result<Self, BrainError> {
        let client = dl_bridges::http::internal_client(limits.http)
            .map_err(|error| BrainError::Backend(format!("bild: {error}")))?;
        Ok(Self {
            client,
            limits: limits.bounded(),
            recent: std::sync::Mutex::default(),
        })
    }

    pub(crate) async fn for_turn(
        &self,
        user_id: u64,
        images: &AttachedImages,
        follow_up: bool,
    ) -> Result<Vec<QueryImage>, BrainError> {
        let channel_id = images.channel_id;
        if images.is_empty() {
            return Ok(if follow_up && channel_id != 0 {
                self.recall(user_id, channel_id)
            } else {
                Vec::new()
            });
        }
        let urls = &images.urls[..images.len().min(self.limits.max_images)];
        if urls.len() < images.len() {
            tracing::info!(
                attached = images.len(),
                used = urls.len(),
                "Brain-Bilder: nur die ersten Bilder werden genutzt"
            );
        }
        let budget = (MAX_TOTAL_IMAGE_BYTES / urls.len()).min(MAX_IMAGE_BYTES);
        let mut prepared = Vec::with_capacity(urls.len());
        for url in urls {
            let bytes = self.download(url).await?;
            if let Some(image) = self.prepare(bytes, budget).await? {
                prepared.push(image);
            }
        }
        if prepared.len() < urls.len() {
            tracing::info!(
                attached = urls.len(),
                used = prepared.len(),
                "Brain-Bilder: Bildformat nicht unterstützt"
            );
        }
        if channel_id != 0 {
            self.remember(user_id, channel_id, &prepared);
        }
        Ok(prepared)
    }

    pub(crate) fn forget_user(&self, user_id: u64) {
        if let Ok(mut recent) = self.recent.lock() {
            recent.retain(|(user, _), _| *user != user_id);
        }
    }

    fn remember(&self, user_id: u64, channel_id: u64, images: &[QueryImage]) {
        let Ok(mut recent) = self.recent.lock() else {
            return;
        };
        let now = Instant::now();
        let ttl = self.limits.followup_ttl;
        recent.retain(|_, (at, _)| now.duration_since(*at) < ttl);
        if recent.len() >= MAX_REMEMBERED {
            if let Some(oldest) = recent
                .iter()
                .min_by_key(|(_, (at, _))| *at)
                .map(|(key, _)| *key)
            {
                recent.remove(&oldest);
            }
        }
        if !ttl.is_zero() {
            recent.insert((user_id, channel_id), (now, images.to_vec()));
        }
        while remembered_bytes(&recent) > MAX_REMEMBERED_BYTES {
            let Some(oldest) = recent
                .iter()
                .filter(|(key, _)| **key != (user_id, channel_id))
                .min_by_key(|(_, (at, _))| *at)
                .map(|(key, _)| *key)
            else {
                break;
            };
            recent.remove(&oldest);
        }
    }

    fn recall(&self, user_id: u64, channel_id: u64) -> Vec<QueryImage> {
        let Ok(recent) = self.recent.lock() else {
            return Vec::new();
        };
        recent
            .get(&(user_id, channel_id))
            .filter(|(at, _)| at.elapsed() < self.limits.followup_ttl)
            .map(|(_, images)| {
                images
                    .iter()
                    .cloned()
                    .map(|mut image| {
                        image.origin = ImageOrigin::Earlier;
                        image
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    async fn download(&self, raw: &str) -> Result<Vec<u8>, BrainError> {
        let url = attachment_url(raw).ok_or_else(image_error)?;
        let mut response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|_| image_error())?;
        let cap = self.limits.max_download_bytes;
        if !response.status().is_success()
            || response.content_length().is_some_and(|size| size > cap)
        {
            return Err(image_error());
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| image_error())? {
            if (bytes.len() + chunk.len()) as u64 > cap {
                return Err(image_error());
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }

    async fn prepare(
        &self,
        bytes: Vec<u8>,
        budget: usize,
    ) -> Result<Option<QueryImage>, BrainError> {
        let limits = self.limits;
        tokio::task::spawn_blocking(move || encode_for_brain(&bytes, limits, budget))
            .await
            .map_err(|_| image_error())?
    }
}

fn remembered_bytes(recent: &RecentImages) -> usize {
    recent
        .values()
        .flat_map(|(_, images)| images)
        .map(|image| image.data.len())
        .sum()
}

pub fn unusable_image_turn(
    question: &str,
    attached: &AttachedImages,
    prepared: &[QueryImage],
) -> bool {
    !attached.is_empty()
        && prepared.is_empty()
        && (question.trim().is_empty() || question.trim() == IMAGE_ONLY_QUESTION)
}

fn image_error() -> BrainError {
    BrainError::Backend("bild".into())
}

fn attachment_url(raw: &str) -> Option<Url> {
    if raw.trim() != raw || raw.contains('\\') || raw.chars().any(char::is_control) {
        return None;
    }
    let url = Url::parse(raw).ok()?;
    let host_ok = matches!(
        url.host_str(),
        Some("cdn.discordapp.com" | "media.discordapp.net")
    );
    if url.scheme() != "https"
        || !host_ok
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port_or_known_default() != Some(443)
        || url.fragment().is_some()
    {
        return None;
    }
    let segments: Vec<&str> = url.path().split('/').collect();
    let ids_ok = segments.len() == 5
        && matches!(segments[1], "attachments" | "ephemeral-attachments")
        && segments[2..4]
            .iter()
            .all(|id| !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit()));
    let name = segments.get(4).map(|name| name.to_ascii_lowercase());
    let name_ok = name
        .as_deref()
        .is_some_and(|name| !name.is_empty() && !name.contains("%2f") && !name.contains("%5c"));
    (ids_ok && name_ok).then_some(url)
}

fn encode_for_brain(
    bytes: &[u8],
    limits: ImageLimits,
    budget: usize,
) -> Result<Option<QueryImage>, BrainError> {
    let Some(decoded) = decode(bytes)? else {
        return Ok(None);
    };
    let mut edge = limits.max_edge;
    loop {
        let jpeg = to_jpeg(&decoded, edge, limits.jpeg_quality)?;
        if jpeg.len() <= budget {
            return QueryImage::from_bytes(ImageMediaType::Jpeg, &jpeg, ImageOrigin::Current)
                .map(Some)
                .map_err(|_| image_error());
        }
        if edge <= MIN_EDGE {
            return Err(image_error());
        }
        edge = (edge * 3 / 4).max(MIN_EDGE);
    }
}

fn decode(bytes: &[u8]) -> Result<Option<DynamicImage>, BrainError> {
    let Ok(format) = image::guess_format(bytes) else {
        return Ok(None);
    };
    if !matches!(
        format,
        ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::WebP | ImageFormat::Gif
    ) {
        return Ok(None);
    }
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_DECODE_EDGE);
    limits.max_image_height = Some(MAX_DECODE_EDGE);
    limits.max_alloc = Some(MAX_DECODE_ALLOC);
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    reader.limits(limits);
    reader.decode().map(Some).map_err(|_| image_error())
}

fn to_jpeg(image: &DynamicImage, edge: u32, quality: u8) -> Result<Vec<u8>, BrainError> {
    let scaled = if image.width() > edge || image.height() > edge {
        image.resize(edge, edge, image::imageops::FilterType::Triangle)
    } else {
        image.clone()
    };
    let mut out = Vec::new();
    JpegEncoder::new_with_quality(&mut out, quality)
        .encode_image(&DynamicImage::ImageRgb8(scaled.to_rgb8()))
        .map_err(|_| image_error())?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(width: u32, height: u32) -> Vec<u8> {
        let image = image::RgbImage::from_fn(width, height, |x, y| {
            image::Rgb([(x % 256) as u8, (y % 256) as u8, ((x ^ y) % 256) as u8])
        });
        let mut out = Cursor::new(Vec::new());
        DynamicImage::ImageRgb8(image)
            .write_to(&mut out, ImageFormat::Png)
            .expect("png fixture");
        out.into_inner()
    }

    #[test]
    fn only_discord_attachment_urls_are_fetched() {
        assert!(attachment_url("https://cdn.discordapp.com/attachments/1/2/a.png?ex=1").is_some());
        assert!(attachment_url("https://media.discordapp.net/attachments/1/2/a.webp").is_some());
        assert!(
            attachment_url("https://cdn.discordapp.com/ephemeral-attachments/1/2/a.png").is_some()
        );
        for url in [
            "http://cdn.discordapp.com/attachments/1/2/a.png",
            "https://cdn.discordapp.com.evil.test/attachments/1/2/a.png",
            "https://u:p@cdn.discordapp.com/attachments/1/2/a.png",
            "https://cdn.discordapp.com:444/attachments/1/2/a.png",
            "https://cdn.discordapp.com/avatars/1/2/a.png",
            "https://cdn.discordapp.com/ephemeral-attachments/x/2/a.png",
            "https://cdn.discordapp.com/attachments/1/2/a%2fb.png",
            "https://127.0.0.1/attachments/1/2/a.png",
        ] {
            assert!(attachment_url(url).is_none(), "{url}");
        }
    }

    #[test]
    fn large_screenshots_are_scaled_into_the_brain_contract() {
        let image = encode_for_brain(
            &png(2560, 1440),
            ImageLimits::default(),
            MAX_IMAGE_BYTES / 2,
        )
        .expect("prepared image")
        .expect("supported format");
        assert_eq!(image.media_type, ImageMediaType::Jpeg);
        assert!(image.decoded_len().expect("valid image") <= MAX_IMAGE_BYTES / 2);
        assert!(format!("{image:?}").contains("data_len"));
    }

    #[test]
    fn follow_up_reuses_only_the_same_users_images_as_earlier() {
        let fetcher = ImageFetcher::new(ImageLimits::default()).expect("fetcher");
        let image = encode_for_brain(&png(64, 64), ImageLimits::default(), MAX_IMAGE_BYTES)
            .expect("prepared image")
            .expect("supported format");
        fetcher.remember(1, 10, &[image]);
        let recalled = fetcher.recall(1, 10);
        assert_eq!(recalled.len(), 1);
        assert_eq!(recalled[0].origin, ImageOrigin::Earlier);
        assert!(fetcher.recall(2, 10).is_empty());
        assert!(fetcher.recall(1, 11).is_empty());
        fetcher.forget_user(1);
        assert!(fetcher.recall(1, 10).is_empty());
    }

    #[test]
    fn gif_is_decoded_and_unknown_formats_are_skipped_not_failed() {
        let mut gif = Cursor::new(Vec::new());
        DynamicImage::ImageRgb8(image::RgbImage::new(8, 8))
            .write_to(&mut gif, ImageFormat::Gif)
            .expect("gif fixture");
        let limits = ImageLimits::default();
        assert!(encode_for_brain(gif.get_ref(), limits, MAX_IMAGE_BYTES)
            .expect("gif")
            .is_some());
        assert!(
            encode_for_brain(b"BM-not-supported", limits, MAX_IMAGE_BYTES)
                .expect("unsupported is skipped")
                .is_none()
        );
        assert!(encode_for_brain(b"plain text", limits, MAX_IMAGE_BYTES)
            .expect("unknown is skipped")
            .is_none());
        let mut broken = png(16, 16);
        broken.truncate(40);
        assert!(encode_for_brain(&broken, limits, MAX_IMAGE_BYTES).is_err());
        let attached = AttachedImages::new(1, ["u".to_owned()]);
        assert!(unusable_image_turn(IMAGE_ONLY_QUESTION, &attached, &[]));
        assert!(unusable_image_turn(" ", &attached, &[]));
        assert!(!unusable_image_turn(
            "welches item ist das?",
            &attached,
            &[]
        ));
        assert!(!unusable_image_turn(
            IMAGE_ONLY_QUESTION,
            &AttachedImages::default(),
            &[]
        ));
    }

    #[test]
    fn remembered_images_stay_within_the_byte_cap() {
        let fetcher = ImageFetcher::new(ImageLimits::default()).expect("fetcher");
        let big = QueryImage {
            media_type: ImageMediaType::Jpeg,
            data: "A".repeat(MAX_REMEMBERED_BYTES / 3 + 1),
            origin: ImageOrigin::Current,
        };
        for user in 1..=4 {
            fetcher.remember(user, 10, std::slice::from_ref(&big));
        }
        let recent = fetcher.recent.lock().expect("lock");
        assert!(remembered_bytes(&recent) <= MAX_REMEMBERED_BYTES);
        assert!(recent.contains_key(&(4, 10)));
    }
}
