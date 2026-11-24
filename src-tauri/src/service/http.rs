use crate::models::{ProxyKind, ProxySettings};
use crate::error::AppError;
use crate::Result;
use std::time::Duration;

/// 所有 HTTP 客户端的唯一构造点。
///
/// 之前 scraper 和 downloader 各建一个 `Client`，加代理时必须同时改两处，
/// 漏一个就会出现「爬取走代理、下载不走」的怪现象。
pub fn build_client(proxy: &ProxySettings, user_agent: &str, timeout: Duration) -> Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder()
        .user_agent(user_agent.to_string())
        .timeout(timeout)
        .tcp_keepalive(Some(Duration::from_secs(60)));

    if proxy.enabled {
        let host = proxy.host.trim();
        // 空 host 视为「跳过代理」而不是报错：用户可能刚勾上还没填。
        if !host.is_empty() {
            let port = if proxy.port.trim().is_empty() {
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
            let mut rule = reqwest::Proxy::all(&url).map_err(|err| {
                AppError::InvalidSettings(format!("invalid proxy url {url}: {err}"))
            })?;

            // 用户名非空才带认证；密码可以是空串（有些代理只校验用户名）。
            if !proxy.username.trim().is_empty() {
                rule = rule.basic_auth(proxy.username.trim(), &proxy.password);
            }
            builder = builder.proxy(rule);
        }
    }

    builder
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
}
