use crate::error::AppError;
use crate::models::{ProxyKind, ProxySettings};
use crate::Result;
use std::time::Duration;

/// 一次性响应的客户端：爬列表、读目录 JSON 这类「拿完就完」的请求。
///
/// `timeout` 是**整个请求**的时限（reqwest 0.11 `.timeout()` 的语义：
/// 从连接开始一直到响应体读完）。大文件下载绝不能用它——见
/// [`build_streaming_client`]。
pub fn build_client(
    proxy: &ProxySettings,
    user_agent: &str,
    timeout: Duration,
) -> Result<reqwest::Client> {
    finish(
        with_proxy(reqwest::Client::builder().timeout(timeout), proxy)?,
        user_agent,
    )
}

/// 流式下载专用客户端：只限连接建立，不限总时长。
///
/// 之前的 bug：下载器把 30 秒的「连接超时」当成了整包时限传进来，
/// 于是任何 30 秒下不完的文件（Blender 稳定版 400 MB）必然
/// `operation timed out`，与网速无关。这是设计错误，不是网络问题。
///
/// 挂死由两处兜底：
/// 1. `connect_timeout` 拦住连不上；
/// 2. 下载循环每次 `chunk()` 前有读超时（`downloader::READ_IDLE_TIMEOUT`），
///    对端长时间不吐数据就报错，而不是永久等待。
pub fn build_streaming_client(
    proxy: &ProxySettings,
    user_agent: &str,
    connect_timeout: Duration,
) -> Result<reqwest::Client> {
    finish(
        with_proxy(
            reqwest::Client::builder().connect_timeout(connect_timeout),
            proxy,
        )?,
        user_agent,
    )
}

/// 把代理规则装进 builder。两个构造函数共用，避免「爬取走代理、
/// 下载不走」这种只改一处的漏配。
fn with_proxy(
    mut builder: reqwest::ClientBuilder,
    proxy: &ProxySettings,
) -> Result<reqwest::ClientBuilder> {
    if !proxy.enabled {
        return Ok(builder);
    }
    let host = proxy.host.trim();
    // 空 host 视为「跳过代理」而不是报错：用户可能刚勾上还没填。
    if host.is_empty() {
        return Ok(builder);
    }

    let port =
        if proxy.port.trim().is_empty() {
            match proxy.kind {
                ProxyKind::Http => 8080,
                ProxyKind::Https => 8443,
            }
        } else {
            proxy.port.trim().parse::<u16>().map_err(|_| {
                AppError::InvalidSettings(format!("invalid proxy port: {}", proxy.port))
            })?
        };

    let scheme = match proxy.kind {
        ProxyKind::Http => "http",
        ProxyKind::Https => "https",
    };
    let url = format!("{scheme}://{host}:{port}");
    let mut rule = reqwest::Proxy::all(&url)
        .map_err(|err| AppError::InvalidSettings(format!("invalid proxy url {url}: {err}")))?;

    // 用户名非空才带认证；密码可以是空串（有些代理只校验用户名）。
    if !proxy.username.trim().is_empty() {
        rule = rule.basic_auth(proxy.username.trim(), &proxy.password);
    }
    builder = builder.proxy(rule);
    Ok(builder)
}

fn finish(builder: reqwest::ClientBuilder, user_agent: &str) -> Result<reqwest::Client> {
    builder
        .user_agent(user_agent.to_string())
        .tcp_keepalive(Some(Duration::from_secs(60)))
        .build()
        .map_err(|err| AppError::Network(format!("failed to build HTTP client: {err}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proxy(enabled: bool, kind: ProxyKind, host: &str, port: &str) -> ProxySettings {
        ProxySettings {
            enabled,
            kind,
            host: host.to_string(),
            port: port.to_string(),
            username: String::new(),
            password: String::new(),
        }
    }

    #[test]
    fn test_disabled_proxy_builds_client() {
        let client = build_client(
            &proxy(false, ProxyKind::Http, "127.0.0.1", "8080"),
            "test",
            Duration::from_secs(5),
        );
        assert!(client.is_ok());
    }

    #[test]
    fn test_empty_host_is_ignored_not_fatal() {
        // 勾了代理但没填 host：不应当拖垮整个客户端构造。
        let client = build_client(
            &proxy(true, ProxyKind::Http, "   ", ""),
            "test",
            Duration::from_secs(5),
        );
        assert!(client.is_ok());
    }

    #[test]
    fn test_invalid_port_is_reported() {
        let err = build_client(
            &proxy(true, ProxyKind::Http, "127.0.0.1", "not-a-port"),
            "test",
            Duration::from_secs(5),
        )
        .unwrap_err();
        assert!(err.to_string().contains("invalid proxy port"));
    }

    #[test]
    fn test_valid_manual_proxy_builds_client() {
        let mut settings = proxy(true, ProxyKind::Http, "127.0.0.1", "3128");
        settings.username = "user".into();
        settings.password = "pass".into();
        assert!(build_client(&settings, "test", Duration::from_secs(5)).is_ok());
    }

    #[test]
    fn test_streaming_client_accepts_long_downloads() {
        // 只验证能构造：真正「不限总时长」的行为无法在单测里断言，
        // 它是 reqwest 的文档语义（不设 `.timeout()` 就没有总时限）。
        assert!(build_streaming_client(
            &proxy(false, ProxyKind::Http, "", ""),
            "test",
            Duration::from_secs(30),
        )
        .is_ok());
    }
}
