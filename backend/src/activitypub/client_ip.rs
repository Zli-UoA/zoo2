//! CF-Connecting-IP is trusted only on an explicitly isolated Tunnel origin.
use super::*;
use axum::http::HeaderMap;
use std::net::IpAddr;
pub(super) fn resolve(
    peer: Option<IpAddr>,
    headers: &HeaderMap,
    tunnel: bool,
) -> Result<Option<IpAddr>, AppError> {
    let peer = peer.map(|ip| ip.to_canonical());
    if tunnel && peer.is_none() {
        return Err(AppError::BadRequest(
            "Client address unavailable from Tunnel".into(),
        ));
    }
    let internal = peer.is_some_and(|ip| match ip {
        IpAddr::V4(v) => v.is_private() || v.is_loopback(),
        IpAddr::V6(v) => v.is_unique_local() || v.is_loopback(),
    });
    if !tunnel || !internal {
        return Ok(peer);
    }
    let value = header_ip(headers, "cf-connecting-ip");
    // Cloudflare's Pseudo IPv4 overwrite mode preserves the real IPv6 here.
    let value = match value {
        Some(IpAddr::V4(ip)) if ip.octets()[0] >= 240 => {
            header_ip(headers, "cf-connecting-ipv6").filter(IpAddr::is_ipv6)
        }
        other => other,
    };
    value
        .filter(|ip| crate::apps::delivery::public_ip(*ip))
        .map(Some)
        .ok_or_else(|| AppError::BadRequest("Client address unavailable from Tunnel".into()))
}
fn header_ip(headers: &HeaderMap, name: &str) -> Option<IpAddr> {
    let mut values = headers.get_all(name).iter();
    let ip = values
        .next()?
        .to_str()
        .ok()?
        .parse::<IpAddr>()
        .ok()?
        .to_canonical();
    values.next().is_none().then_some(ip)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pseudo_ipv4_uses_the_preserved_real_ipv6() {
        let proxy = Some("172.18.0.2".parse().unwrap());
        let mut headers = HeaderMap::new();
        headers.insert("cf-connecting-ip", "240.1.2.3".parse().unwrap());
        assert!(resolve(proxy, &headers, true).is_err());
        headers.insert(
            "cf-connecting-ipv6",
            "2606:4700:4700::1111".parse().unwrap(),
        );
        assert_eq!(
            resolve(proxy, &headers, true).unwrap(),
            Some("2606:4700:4700::1111".parse().unwrap())
        );
    }
    #[test]
    fn tunnel_addresses_are_separate_and_untrusted_headers_are_ignored() {
        let proxy = Some("172.18.0.2".parse().unwrap());
        let mut headers = HeaderMap::new();
        headers.insert("cf-connecting-ip", "8.8.8.8".parse().unwrap());
        assert_eq!(
            resolve(proxy, &headers, true).unwrap(),
            Some("8.8.8.8".parse().unwrap())
        );
        headers.insert("cf-connecting-ip", "1.1.1.1".parse().unwrap());
        assert_eq!(
            resolve(proxy, &headers, true).unwrap(),
            Some("1.1.1.1".parse().unwrap())
        );
        assert_eq!(resolve(proxy, &headers, false).unwrap(), proxy);
        let direct = Some("9.9.9.9".parse().unwrap());
        assert_eq!(resolve(direct, &headers, true).unwrap(), direct);
    }
    #[test]
    fn missing_spoofed_and_multiple_tunnel_headers_fail_without_a_shared_proxy_bucket() {
        let proxy = Some("172.18.0.2".parse().unwrap());
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "8.8.8.8".parse().unwrap());
        assert!(resolve(proxy, &headers, true).is_err());
        assert!(resolve(None, &headers, true).is_err());
        for value in ["127.0.0.1", "8.8.8.8, 1.1.1.1", "invalid"] {
            headers.insert("cf-connecting-ip", value.parse().unwrap());
            assert!(resolve(proxy, &headers, true).is_err());
        }
        headers.insert("cf-connecting-ip", "8.8.8.8".parse().unwrap());
        headers.append("cf-connecting-ip", "1.1.1.1".parse().unwrap());
        assert!(resolve(proxy, &headers, true).is_err());
    }
}
