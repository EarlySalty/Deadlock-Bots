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
const MAX_REMEMBERED: usize = 512;

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
}

impl Default for ImageLimits {
    fn default() -> Self {
        Self {
            max_images: MAX_QUERY_IMAGES,
            max_download_bytes: 10 * 1024 * 1024,
            max_edge: 1568,
            jpeg_quality: 85,
            followup_ttl: Duration::from_secs(10 * 60),
        }
    }
}

impl ImageLimits {
    fn bounded(self) -> Self {
        Self {
            max_images: self.max_images.clamp(1, MAX_QUERY_IMAGES),
            max_download_bytes: self.max_download_bytes.max(1),
            max_edge: self.max_edge.max(MIN_EDGE),
            jpeg_quality: self.jpeg_quality.clamp(40, 95),
            followup_ttl: self.followup_ttl,
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
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .build()
            .map_err(|_| image_error())?;
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
            prepared.push(self.prepare(bytes, budget).await?);
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

    async fn prepare(&self, bytes: Vec<u8>, budget: usize) -> Result<QueryImage, BrainError> {
        let limits = self.limits;
        tokio::task::spawn_blocking(move || encode_for_brain(&bytes, limits, budget))
            .await
            .map_err(|_| image_error())?
    }
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
        && segments[1] == "attachments"
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
) -> Result<QueryImage, BrainError> {
    let decoded = decode(bytes)?;
    let mut edge = limits.max_edge;
    loop {
        let jpeg = to_jpeg(&decoded, edge, limits.jpeg_quality)?;
        if jpeg.len() <= budget {
            return QueryImage::from_bytes(ImageMediaType::Jpeg, &jpeg, ImageOrigin::Current)
                .map_err(|_| image_error());
        }
        if edge <= MIN_EDGE {
            return Err(image_error());
        }
        edge = (edge * 3 / 4).max(MIN_EDGE);
    }
}

fn decode(bytes: &[u8]) -> Result<DynamicImage, BrainError> {
    let format = image::guess_format(bytes).map_err(|_| image_error())?;
    if !matches!(
        format,
        ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::WebP
    ) {
        return Err(image_error());
    }
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_DECODE_EDGE);
    limits.max_image_height = Some(MAX_DECODE_EDGE);
    limits.max_alloc = Some(MAX_DECODE_ALLOC);
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    reader.limits(limits);
    reader.decode().map_err(|_| image_error())
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
        for url in [
            "http://cdn.discordapp.com/attachments/1/2/a.png",
            "https://cdn.discordapp.com.evil.test/attachments/1/2/a.png",
            "https://u:p@cdn.discordapp.com/attachments/1/2/a.png",
            "https://cdn.discordapp.com:444/attachments/1/2/a.png",
            "https://cdn.discordapp.com/avatars/1/2/a.png",
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
        .expect("prepared image");
        assert_eq!(image.media_type, ImageMediaType::Jpeg);
        assert!(image.decoded_len().expect("valid image") <= MAX_IMAGE_BYTES / 2);
        assert!(format!("{image:?}").contains("data_len"));
    }

    #[test]
    fn follow_up_reuses_only_the_same_users_images_as_earlier() {
        let fetcher = ImageFetcher::new(ImageLimits::default()).expect("fetcher");
        let image = encode_for_brain(&png(64, 64), ImageLimits::default(), MAX_IMAGE_BYTES)
            .expect("prepared image");
        fetcher.remember(1, 10, &[image]);
        let recalled = fetcher.recall(1, 10);
        assert_eq!(recalled.len(), 1);
        assert_eq!(recalled[0].origin, ImageOrigin::Earlier);
        assert!(fetcher.recall(2, 10).is_empty());
        assert!(fetcher.recall(1, 11).is_empty());
        fetcher.forget_user(1);
        assert!(fetcher.recall(1, 10).is_empty());
    }
}
