use std::time::Duration;

use base64::{engine::general_purpose, Engine as _};
use reqwest::{header::CONTENT_TYPE, Client, Response, Url};

pub const MAX_IMAGE_BYTES: u64 = 8 * 1024 * 1024;
pub const IMAGE_DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, thiserror::Error, PartialEq, Eq)]
pub enum ImageError {
    #[error("Bitte häng eine PNG-, JPEG- oder WebP-Datei von Discord an.")]
    Invalid,
    #[error("Das Bild muss zwischen 1 Byte und 8 MiB groß sein.")]
    TooLarge,
    #[error("Das Bild konnte nicht geladen werden. Bitte häng es erneut an.")]
    Download,
}

pub fn supported_media_type(value: &str) -> bool {
    matches!(value, "image/png" | "image/jpeg" | "image/webp")
}

pub fn valid_attachment_url(raw: &str) -> bool {
    if raw.trim() != raw || raw.contains('\\') || raw.chars().any(char::is_control) {
        return false;
    }
    let Ok(url) = Url::parse(raw) else {
        return false;
    };
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port_or_known_default() != Some(443)
        || url.fragment().is_some()
        || !matches!(
            url.host_str(),
            Some("cdn.discordapp.com" | "media.discordapp.net")
        )
    {
        return false;
    }
    let segments: Vec<_> = url.path().split('/').collect();
    segments.len() == 5
        && segments[1] == "attachments"
        && segments[2..4]
            .iter()
            .all(|id| !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit()))
        && !segments[4].is_empty()
        && !segments[4].to_ascii_lowercase().contains("%2f")
        && !segments[4].to_ascii_lowercase().contains("%5c")
}

pub fn validate_attachment(url: &str, media_type: &str, size: u64) -> Result<(), ImageError> {
    if !valid_attachment_url(url) || !supported_media_type(media_type) {
        return Err(ImageError::Invalid);
    }
    if size == 0 || size > MAX_IMAGE_BYTES {
        return Err(ImageError::TooLarge);
    }
    Ok(())
}

fn download_client() -> Result<Client, ImageError> {
    Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .timeout(IMAGE_DOWNLOAD_TIMEOUT)
        .build()
        .map_err(|_| ImageError::Download)
}

pub async fn download_data_uri(raw: &str) -> Result<String, ImageError> {
    if !valid_attachment_url(raw) {
        return Err(ImageError::Invalid);
    }
    let client = download_client()?;
    tokio::time::timeout(IMAGE_DOWNLOAD_TIMEOUT, async {
        let response = client
            .get(raw)
            .send()
            .await
            .map_err(|_| ImageError::Download)?;
        response_data_uri(response).await
    })
    .await
    .map_err(|_| ImageError::Download)?
}

async fn response_data_uri(mut response: Response) -> Result<String, ImageError> {
    if !response.status().is_success() {
        return Err(ImageError::Download);
    }
    if response
        .content_length()
        .is_some_and(|size| size > MAX_IMAGE_BYTES)
    {
        return Err(ImageError::TooLarge);
    }
    let media_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map(str::trim)
        .filter(|value| supported_media_type(value))
        .ok_or(ImageError::Invalid)?
        .to_owned();
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| ImageError::Download)? {
        if chunk.len() > MAX_IMAGE_BYTES as usize - bytes.len() {
            return Err(ImageError::TooLarge);
        }
        bytes.extend_from_slice(&chunk);
    }
    let valid_signature = match media_type.as_str() {
        "image/png" => bytes.starts_with(b"\x89PNG\r\n\x1a\n") && bytes.len() >= 24,
        "image/jpeg" => bytes.starts_with(&[0xff, 0xd8, 0xff]) && bytes.len() >= 4,
        "image/webp" => bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP"),
        _ => false,
    };
    if !valid_signature {
        return Err(ImageError::Invalid);
    }
    Ok(format!(
        "data:{media_type};base64,{}",
        general_purpose::STANDARD.encode(bytes)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, response::Response as ServerResponse, routing::get, Router};

    const URL: &str =
        "https://cdn.discordapp.com/attachments/123/456/screenshot.png?ex=1&is=2&hm=3";

    #[test]
    fn accepts_supported_discord_attachment() {
        assert_eq!(
            validate_attachment(URL, "image/png", MAX_IMAGE_BYTES),
            Ok(())
        );
        assert!(valid_attachment_url(
            "https://media.discordapp.net/attachments/1/2/a.webp"
        ));
    }

    #[test]
    fn rejects_untrusted_origins_and_paths() {
        for url in [
            "http://cdn.discordapp.com/attachments/1/2/a.png",
            "https://cdn.discordapp.com.evil.test/attachments/1/2/a.png",
            "https://cdn.discordapp.com@127.0.0.1/attachments/1/2/a.png",
            "https://u:p@cdn.discordapp.com/attachments/1/2/a.png",
            "https://cdn.discordapp.com:444/attachments/1/2/a.png",
            "https://cdn.discordapp.com/avatars/1/2/a.png",
            "https://cdn.discordapp.com/attachments/1/2/a.png#ignored",
            "https://cdn.discordapp.com/attachments/1/2/a%2fb.png",
            "https://cdn.discordapp.com/attachments/a/2/a.png",
            "https://127.0.0.1/attachments/1/2/a.png",
            "data:image/png;base64,AAAA",
            "file:///tmp/a.png",
        ] {
            assert!(!valid_attachment_url(url), "{url}");
        }
    }

    #[test]
    fn rejects_bad_types_and_sizes() {
        for mime in ["image/svg+xml", "image/gif", "text/plain", "", "image/pngx"] {
            assert!(validate_attachment(URL, mime, 1).is_err());
        }
        for size in [0, MAX_IMAGE_BYTES + 1, u64::MAX] {
            assert_eq!(
                validate_attachment(URL, "image/png", size),
                Err(ImageError::TooLarge)
            );
        }
    }

    async fn fixture_response(
        status: u16,
        mime: &str,
        body: Vec<u8>,
        length: Option<u64>,
    ) -> Response {
        let mime = mime.to_owned();
        let router = Router::new().route(
            "/",
            get(move || {
                let mime = mime.clone();
                let body = body.clone();
                async move {
                    let mut response = ServerResponse::builder()
                        .status(status)
                        .header(CONTENT_TYPE, mime);
                    if let Some(length) = length {
                        response = response.header("content-length", length);
                    }
                    response
                        .header("location", "http://127.0.0.1:1/private")
                        .body(Body::from(body))
                        .expect("gültige HTTP-Testantwort")
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("lokaler Testport");
        let address = listener.local_addr().expect("Adresse des Testservers");
        tokio::spawn(async move {
            axum::serve(listener, router)
                .await
                .expect("lokaler Testserver")
        });
        download_client()
            .expect("HTTP-Testclient")
            .get(format!("http://{address}/"))
            .send()
            .await
            .expect("Antwort des lokalen Testservers")
    }

    #[tokio::test]
    async fn valid_bytes_are_passed_as_data_not_remote_url() {
        let png = b"\x89PNG\r\n\x1a\n0000000000000000".to_vec();
        let response = fixture_response(200, "image/png", png.clone(), None).await;
        assert_eq!(
            response_data_uri(response)
                .await
                .expect("gültige PNG-Testbytes"),
            format!(
                "data:image/png;base64,{}",
                general_purpose::STANDARD.encode(png)
            )
        );
    }

    #[tokio::test]
    async fn redirect_is_not_followed() {
        let response = fixture_response(302, "image/png", Vec::new(), None).await;
        assert_eq!(response.status().as_u16(), 302);
        assert_eq!(response_data_uri(response).await, Err(ImageError::Download));
    }

    #[tokio::test]
    async fn invalid_signature_and_header_are_rejected() {
        let response = fixture_response(200, "image/png", b"not a png".to_vec(), None).await;
        assert_eq!(response_data_uri(response).await, Err(ImageError::Invalid));
        let response = fixture_response(200, "text/html", b"not an image".to_vec(), None).await;
        assert_eq!(response_data_uri(response).await, Err(ImageError::Invalid));
    }

    #[tokio::test]
    async fn chunked_response_is_bounded_without_content_length() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("lokaler Testport");
        let address = listener.local_addr().expect("Adresse des Testservers");
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("Testverbindung");
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") && request.len() < 8192 {
                let mut byte = [0u8; 1];
                if socket.read_exact(&mut byte).await.is_err() {
                    return;
                }
                request.push(byte[0]);
            }
            if socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n").await.is_err() { return; }
            let chunk = vec![0; 1024 * 1024];
            for _ in 0..9 {
                if socket.write_all(b"100000\r\n").await.is_err()
                    || socket.write_all(&chunk).await.is_err()
                    || socket.write_all(b"\r\n").await.is_err()
                {
                    return;
                }
            }
            let _ = socket.write_all(b"0\r\n\r\n").await;
        });
        let response = download_client()
            .expect("HTTP-Testclient")
            .get(format!("http://{address}/"))
            .send()
            .await
            .expect("Chunked-Testantwort");
        assert_eq!(response.content_length(), None);
        assert_eq!(response_data_uri(response).await, Err(ImageError::TooLarge));
    }

    #[tokio::test]
    async fn oversized_response_is_rejected() {
        let response = fixture_response(
            200,
            "image/png",
            vec![0; MAX_IMAGE_BYTES as usize + 1],
            None,
        )
        .await;
        assert_eq!(response_data_uri(response).await, Err(ImageError::TooLarge));
    }
}
