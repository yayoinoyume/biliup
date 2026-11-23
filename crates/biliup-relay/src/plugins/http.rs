use std::io;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

/// A small pool of clients keyed by proxy. Client clones share connections;
/// proxy changes cannot grow this cache without bound.
pub fn pooled_client(proxy: Option<&str>) -> Result<reqwest::Client, reqwest::Error> {
    type ClientEntry = (Option<String>, reqwest::Client);
    static CLIENTS: OnceLock<Mutex<Vec<ClientEntry>>> = OnceLock::new();
    let proxy = proxy.filter(|proxy| !proxy.is_empty());
    let mut clients = CLIENTS
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if let Some((_, client)) = clients.iter().find(|(key, _)| key.as_deref() == proxy) {
        return Ok(client.clone());
    }
    let mut builder = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30));
    if let Some(proxy) = proxy {
        builder = builder.proxy(reqwest::Proxy::all(proxy)?);
    }
    let client = builder.build()?;
    if clients.len() == 4 {
        clients.remove(0);
    }
    clients.push((proxy.map(str::to_owned), client.clone()));
    Ok(client)
}

/// Enforce the limit while streaming, including responses without a length.
pub async fn response_bytes_limited(
    mut response: reqwest::Response,
    limit: usize,
) -> io::Result<Vec<u8>> {
    let oversized = || {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "upstream response exceeds byte limit",
        )
    };
    if response
        .content_length()
        .is_some_and(|size| size > limit as u64)
    {
        return Err(oversized());
    }
    let mut bytes = Vec::new();
    // Without the URL: some carry API keys, and this error ends up in logs.
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| io::Error::other(e.without_url()))?
    {
        if chunk.len() > limit.saturating_sub(bytes.len()) {
            return Err(oversized());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

pub async fn response_json_limited<T: serde::de::DeserializeOwned>(
    response: reqwest::Response,
) -> io::Result<T> {
    let bytes = response_bytes_limited(response, 8 * 1024 * 1024).await?;
    serde_json::from_slice(&bytes)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn caps_chunked_response_without_content_length() {
        crate::install_crypto_provider();
        use tokio::io::AsyncWriteExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut body = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
            for _ in 0..4 {
                body.extend_from_slice(b"200\r\n");
                body.extend_from_slice(&[b'x'; 512]);
                body.extend_from_slice(b"\r\n");
            }
            body.extend_from_slice(b"0\r\n\r\n");
            let _ = socket.write_all(&body).await;
            let _ = socket.shutdown().await;
        });
        let response = pooled_client(None)
            .unwrap()
            .get(format!("http://{address}/"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.content_length(), None);
        let error = response_bytes_limited(response, 1024).await.unwrap_err();
        server.await.unwrap();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }
}
