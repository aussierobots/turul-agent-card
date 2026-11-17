//! The address rule: refuse every address that is not globally reachable.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use crate::fetch::FetchPolicy;

pub(crate) fn is_allowed(ip: IpAddr, policy: &FetchPolicy) -> bool {
    match ip {
        IpAddr::V4(v4) => allowed_v4(v4, policy),
        IpAddr::V6(v6) => allowed_v6(v6, policy),
    }
}

/// The first disallowed address of a resolution, if any.
pub(crate) fn first_disallowed(addrs: &[IpAddr], policy: &FetchPolicy) -> Option<IpAddr> {
    addrs.iter().copied().find(|ip| !is_allowed(*ip, policy))
}

fn allowed_v4(ip: Ipv4Addr, policy: &FetchPolicy) -> bool {
    let [a, b, c, _] = ip.octets();
    if a == 127 {
        return policy.permit_loopback_http;
    }
    !(a == 0
        || a == 10
        || (a == 100 && (64..=127).contains(&b))
        || (a == 169 && b == 254)
        || (a == 172 && (16..=31).contains(&b))
        || (a == 192 && b == 0 && (c == 0 || c == 2))
        || (a == 192 && b == 88 && c == 99)
        || (a == 192 && b == 168)
        || (a == 198 && (b == 18 || b == 19))
        || (a == 198 && b == 51 && c == 100)
        || (a == 203 && b == 0 && c == 113)
        || a >= 224)
}

fn allowed_v6(ip: Ipv6Addr, policy: &FetchPolicy) -> bool {
    let s = ip.segments();
    let embedded = |hi: u16, lo: u16| {
        let [a, b] = hi.to_be_bytes();
        let [c, d] = lo.to_be_bytes();
        allowed_v4(Ipv4Addr::new(a, b, c, d), policy)
    };
    // ::ffff:0:0/96 (IPv4-mapped) and 64:ff9b::/96 (NAT64) are vetted as the embedded IPv4.
    if s[..5] == [0; 5] && s[5] == 0xffff {
        return embedded(s[6], s[7]);
    }
    if s[0] == 0x64 && s[1] == 0xff9b && s[2..6] == [0; 4] {
        return embedded(s[6], s[7]);
    }
    if ip == Ipv6Addr::LOCALHOST {
        return policy.permit_loopback_http;
    }
    s[0] & 0xe000 == 0x2000
        && !(s[0] == 0x2001 && (s[1] < 0x0200 || s[1] == 0x0db8))
        && s[0] != 0x2002
        && !(s[0] == 0x3fff && s[1] < 0x1000)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn special_purpose_blocks_are_refused() {
        let p = FetchPolicy::default();
        for s in [
            "0.0.0.1",
            "10.1.2.3",
            "100.64.0.1",
            "100.100.100.200",
            "127.0.0.1",
            "169.254.169.254",
            "172.16.0.1",
            "172.31.255.255",
            "192.0.0.192",
            "192.0.2.1",
            "192.88.99.1",
            "192.168.1.1",
            "198.18.0.1",
            "198.19.255.255",
            "198.51.100.1",
            "203.0.113.1",
            "224.0.0.1",
            "240.0.0.1",
            "255.255.255.255",
            "::",
            "::1",
            "fc00::1",
            "fd00:ec2::254",
            "fe80::1",
            "fec0::1",
            "ff02::1",
            "100::1",
            "64:ff9b:1::1",
            "2001::1",
            "2001:db8::1",
            "2002::1",
            "3fff::1",
        ] {
            assert!(!is_allowed(ip(s), &p), "{s}");
        }
    }

    #[test]
    fn global_unicast_is_allowed() {
        let p = FetchPolicy::default();
        for s in [
            "93.184.216.34",
            "8.8.8.8",
            "172.32.0.1",
            "100.128.0.1",
            "2606:2800:220:1::1",
            "2001:4860::1",
        ] {
            assert!(is_allowed(ip(s), &p), "{s}");
        }
    }

    #[test]
    fn embedded_ipv4_is_vetted_as_ipv4() {
        let p = FetchPolicy::default();
        assert!(!is_allowed(ip("::ffff:127.0.0.1"), &p));
        assert!(!is_allowed(ip("64:ff9b::a00:1"), &p));
        assert!(is_allowed(ip("::ffff:93.184.216.34"), &p));
        assert!(is_allowed(ip("64:ff9b::5db8:d822"), &p));
        let lo = FetchPolicy {
            permit_loopback_http: true,
            ..FetchPolicy::default()
        };
        assert!(is_allowed(ip("127.0.0.1"), &lo));
        assert!(is_allowed(ip("::1"), &lo));
        assert!(!is_allowed(ip("10.0.0.1"), &lo));
    }

    #[test]
    fn resolution_mixing_public_and_private_is_refused() {
        let p = FetchPolicy::default();
        let mixed = [ip("93.184.216.34"), ip("10.0.0.1")];
        assert_eq!(first_disallowed(&mixed, &p), Some(ip("10.0.0.1")));
        assert_eq!(first_disallowed(&mixed[..1], &p), None);
    }
}
